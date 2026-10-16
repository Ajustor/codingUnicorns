//! Where each band of a pull request's diff sits. The diff view only lays
//! out what is on screen, so it needs every item's height up front: rows
//! have a fixed height, threads are estimated until drawn and measured.

use crate::git::compare::{DiffContent, DiffRow, FileDiff, TextDiff};
use crate::github::{ReviewThread, Side};
use std::collections::HashMap;
use std::ops::Range;

pub const HEADER_HEIGHT: f32 = 30.0;
pub const NOTICE_HEIGHT: f32 = 28.0;
pub const TITLE_HEIGHT: f32 = 24.0;
/// Height of a thread until it is drawn and measured.
pub const THREAD_ESTIMATE: f32 = 80.0;

/// Where a thread is listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Under the row showing its line, on its side.
    Line,
    /// In "Comments on the file": a file-level thread, or a line thread on a
    /// file shown without rows (binary, too large, unavailable).
    File,
    /// In "Outdated comments": its line isn't in the diff any more.
    Outdated,
}

/// One horizontal band of the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    /// Title above the threads on files this diff doesn't show (first in the view).
    OrphanTitle,
    /// Thread `thread` on a file this diff doesn't show.
    OrphanThread { thread: usize },
    /// Path, counts and "viewed" checkbox of `file`.
    FileHeader { file: usize },
    /// A binary or large file: a line of text instead of its diff.
    Notice { file: usize },
    /// Title above the threads on the whole of `file` ("Comments on the file").
    FileCommentsTitle { file: usize },
    /// Title above the outdated threads of `file`.
    OutdatedTitle { file: usize },
    /// Row `row` of the file's [`TextDiff::rows`].
    Row { file: usize, row: usize },
    /// Review thread `thread` (index in the pull request's threads).
    Thread {
        file: usize,
        thread: usize,
        placement: Placement,
    },
}

impl Item {
    /// The file the item belongs to; `None` for the orphan items.
    pub fn file(self) -> Option<usize> {
        match self {
            Self::OrphanTitle | Self::OrphanThread { .. } => None,
            Self::FileHeader { file }
            | Self::Notice { file }
            | Self::FileCommentsTitle { file }
            | Self::OutdatedTitle { file }
            | Self::Row { file, .. }
            | Self::Thread { file, .. } => Some(file),
        }
    }
}

/// The threads of one file: on the whole file, under the row they target,
/// or outdated.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Anchors {
    pub file_level: Vec<usize>,
    pub by_row: HashMap<usize, Vec<usize>>,
    pub outdated: Vec<usize>,
}

/// Threads placed in their file, and those whose file isn't in the diff.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Placed {
    /// One `Anchors` per file.
    pub files: Vec<Anchors>,
    /// Threads on files this diff doesn't show, in thread order.
    pub orphans: Vec<usize>,
}

/// The file a thread on `path` is about: the file at that path, or the one
/// renamed from it.
fn find_file(files: &[FileDiff], path: &str) -> Option<usize> {
    files.iter().position(|f| f.path() == path).or_else(|| {
        files
            .iter()
            .position(|f| f.old_path.as_deref() == Some(path))
    })
}

/// Place each thread in the file it is about. A thread on a text file is
/// outdated when GitHub reports no line or the line has no row; on a file
/// without rows (binary, too large) a thread with a line goes to the file's
/// comments. A thread on a file absent from the diff is an orphan.
pub fn anchor_threads(files: &[FileDiff], threads: &[ReviewThread]) -> Placed {
    let mut placed = Placed {
        files: vec![Anchors::default(); files.len()],
        orphans: Vec::new(),
    };
    for (index, thread) in threads.iter().enumerate() {
        let Some(file) = find_file(files, &thread.path) else {
            placed.orphans.push(index);
            continue;
        };
        let anchors = &mut placed.files[file];
        if thread.file_level {
            anchors.file_level.push(index);
            continue;
        }
        match (&files[file].content, thread_line(thread)) {
            (DiffContent::Text(text), Some(line)) => match row_of_line(text, thread.side, line) {
                Some(row) => anchors.by_row.entry(row).or_default().push(index),
                None => anchors.outdated.push(index),
            },
            (DiffContent::Text(_), None) => anchors.outdated.push(index),
            (_, Some(_)) => anchors.file_level.push(index),
            (_, None) => anchors.outdated.push(index),
        }
    }
    placed
}

/// Unfold the folds hiding a thread's line, so that it gets a row.
pub fn unfold_for_threads(files: &mut [FileDiff], threads: &[ReviewThread]) {
    for thread in threads {
        let Some(line) = thread_line(thread) else {
            continue;
        };
        let Some(file) = find_file(files, &thread.path) else {
            continue;
        };
        let DiffContent::Text(text) = &mut files[file].content else {
            continue;
        };
        if row_of_line(text, thread.side, line).is_some() {
            continue;
        }
        let fold = text.rows.iter().position(|row| match (thread.side, *row) {
            (Side::Left, DiffRow::Fold { old, len, .. }) => (old..old + len).contains(&line),
            (Side::Right, DiffRow::Fold { new, len, .. }) => (new..new + len).contains(&line),
            _ => false,
        });
        if let Some(row) = fold {
            text.unfold(row);
        }
    }
}

/// 0-based line of a thread, unless it is outdated.
fn thread_line(thread: &ReviewThread) -> Option<usize> {
    thread.line.and_then(|l| (l as usize).checked_sub(1))
}

/// The row showing 0-based `line` of `side`, if any.
fn row_of_line(text: &TextDiff, side: Side, line: usize) -> Option<usize> {
    text.rows.iter().position(|row| match (side, *row) {
        (
            Side::Left,
            DiffRow::Context { old, .. } | DiffRow::Modified { old, .. } | DiffRow::Removed { old },
        ) => old == line,
        (
            Side::Right,
            DiffRow::Context { new, .. } | DiffRow::Modified { new, .. } | DiffRow::Added { new },
        ) => new == line,
        _ => false,
    })
}

/// The items of the view and their geometry.
pub struct DiffLayout {
    items: Vec<Item>,
    heights: Vec<f32>,
    /// `offsets[i]`: top of item `i`; the last entry is the total height.
    offsets: Vec<f32>,
    /// Items starting a block of changed rows, in order: the F7 stops.
    /// Only rows count: a thread between two changed rows doesn't split a stop.
    change_starts: Vec<usize>,
}

impl DiffLayout {
    /// `collapsed[f]`: file `f` collapses to its header. `thread_heights`:
    /// measured heights by thread index.
    pub fn build(
        files: &[FileDiff],
        threads: &[ReviewThread],
        collapsed: &[bool],
        row_height: f32,
        thread_heights: &HashMap<usize, f32>,
    ) -> Self {
        let placed = anchor_threads(files, threads);
        let mut items = Vec::new();
        let mut change_starts = Vec::new();
        if !placed.orphans.is_empty() {
            items.push(Item::OrphanTitle);
            items.extend(
                placed
                    .orphans
                    .iter()
                    .map(|&thread| Item::OrphanThread { thread }),
            );
        }
        for (file, f) in files.iter().enumerate() {
            items.push(Item::FileHeader { file });
            if collapsed.get(file).copied().unwrap_or(false) {
                continue;
            }
            let a = &placed.files[file];
            if !a.file_level.is_empty() {
                items.push(Item::FileCommentsTitle { file });
                items.extend(a.file_level.iter().map(|&thread| Item::Thread {
                    file,
                    thread,
                    placement: Placement::File,
                }));
            }
            if !a.outdated.is_empty() {
                items.push(Item::OutdatedTitle { file });
                items.extend(a.outdated.iter().map(|&thread| Item::Thread {
                    file,
                    thread,
                    placement: Placement::Outdated,
                }));
            }
            let DiffContent::Text(text) = &f.content else {
                items.push(Item::Notice { file });
                continue;
            };
            let mut in_change = false;
            for (row, r) in text.rows.iter().enumerate() {
                if r.is_change() && !in_change {
                    change_starts.push(items.len());
                }
                in_change = r.is_change();
                items.push(Item::Row { file, row });
                if let Some(anchored) = a.by_row.get(&row) {
                    items.extend(anchored.iter().map(|&thread| Item::Thread {
                        file,
                        thread,
                        placement: Placement::Line,
                    }));
                }
            }
        }
        let heights = items
            .iter()
            .map(|item| match *item {
                Item::FileHeader { .. } => HEADER_HEIGHT,
                Item::Notice { .. } => NOTICE_HEIGHT,
                Item::OrphanTitle | Item::FileCommentsTitle { .. } | Item::OutdatedTitle { .. } => {
                    TITLE_HEIGHT
                }
                Item::Row { .. } => row_height,
                Item::Thread { thread, .. } | Item::OrphanThread { thread } => thread_heights
                    .get(&thread)
                    .copied()
                    .unwrap_or(THREAD_ESTIMATE),
            })
            .collect();
        let mut layout = Self {
            items,
            heights,
            offsets: Vec::new(),
            change_starts,
        };
        layout.recompute_offsets_from(0);
        layout
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Item indexes starting a block of changed rows, in order: the F7 stops.
    pub fn change_starts(&self) -> &[usize] {
        &self.change_starts
    }

    pub fn total_height(&self) -> f32 {
        self.offsets.last().copied().unwrap_or(0.0)
    }

    /// Distance from the top of the view to the top of `item`.
    pub fn top(&self, item: usize) -> f32 {
        self.offsets[item]
    }

    /// Height of `item`: fixed, measured or estimated.
    pub fn height(&self, item: usize) -> f32 {
        self.heights[item]
    }

    /// The items overlapping `[top, bottom)`.
    pub fn visible(&self, top: f32, bottom: f32) -> Range<usize> {
        let n = self.items.len();
        let start = self.offsets[1..].partition_point(|&b| b <= top);
        let end = self.offsets[..n]
            .partition_point(|&t| t < bottom)
            .max(start);
        start..end
    }

    /// Record the measured height of `item`; true when it changed. A change
    /// of half a pixel or less is ignored (drawing is not pixel-exact, and a
    /// re-layout every frame would never settle); a non-finite height is
    /// ignored and a negative one counts as 0.
    pub fn set_height(&mut self, item: usize, height: f32) -> bool {
        if !height.is_finite() {
            return false;
        }
        let height = height.max(0.0);
        if (self.heights[item] - height).abs() <= 0.5 {
            return false;
        }
        self.heights[item] = height;
        self.recompute_offsets_from(item);
        true
    }

    /// Index of the header of `file` in [`Self::items`].
    pub fn header_item(&self, file: usize) -> Option<usize> {
        self.items
            .iter()
            .position(|item| *item == Item::FileHeader { file })
    }

    /// F7. With a cursor (`current`, an index into [`Self::change_starts`]):
    /// the stop after it, staying on the last. Without: the first stop whose
    /// top is at or below `from_y`, the last one if none.
    pub fn next_change(&self, current: Option<usize>, from_y: f32) -> Option<usize> {
        let last = self.change_starts.len().checked_sub(1)?;
        Some(match current {
            Some(c) => c.saturating_add(1).min(last),
            None => self
                .change_starts
                .iter()
                .position(|&item| self.offsets[item] >= from_y)
                .unwrap_or(last),
        })
    }

    /// Shift+F7. With a cursor: the stop before it, staying on the first.
    /// Without: the last stop whose top is above `from_y`, the first one if
    /// none.
    pub fn prev_change(&self, current: Option<usize>, from_y: f32) -> Option<usize> {
        let last = self.change_starts.len().checked_sub(1)?;
        Some(match current {
            Some(c) => c.saturating_sub(1).min(last),
            None => self
                .change_starts
                .iter()
                .rposition(|&item| self.offsets[item] < from_y)
                .unwrap_or(0),
        })
    }

    /// Recompute `offsets[item + 1..]`; `offsets[..=item]` are unchanged.
    fn recompute_offsets_from(&mut self, item: usize) {
        self.offsets.resize(self.heights.len() + 1, 0.0);
        for i in item..self.heights.len() {
            self.offsets[i + 1] = self.offsets[i] + self.heights[i];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::compare::{binary_file, text_file};
    use crate::github::api::thread;

    fn build(files: &[FileDiff], threads: &[ReviewThread], collapsed: &[bool]) -> DiffLayout {
        DiffLayout::build(files, threads, collapsed, 20.0, &HashMap::new())
    }

    fn line(thread: usize) -> Item {
        Item::Thread {
            file: 0,
            thread,
            placement: Placement::Line,
        }
    }

    #[test]
    fn threads_sit_under_their_line_on_their_side() {
        // Rows: Context(0,0) Modified(1,1) Added(2) Context(2,3).
        let files = vec![text_file("a.rs", "a\nb\nc\n", "a\nB\nX\nc\n")];
        let threads = vec![
            thread("a.rs", Side::Left, Some(2)),
            thread("a.rs", Side::Right, Some(3)),
            thread("a.rs", Side::Right, None),
            thread("other.rs", Side::Right, Some(1)),
        ];
        let a = anchor_threads(&files, &threads);
        assert_eq!(a.files[0].by_row.get(&1), Some(&vec![0]));
        assert_eq!(a.files[0].by_row.get(&2), Some(&vec![1]));
        assert_eq!(a.files[0].outdated, vec![2]);
        assert_eq!(a.files.len(), 1);
        assert_eq!(a.orphans, vec![3], "a thread on another file is an orphan");
    }

    #[test]
    fn a_thread_inside_a_fold_unfolds_it() {
        let old: String = (0..12).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l11\n", "L11\n");
        let mut files = vec![text_file("a.rs", &old, &new)];
        let threads = vec![thread("a.rs", Side::Right, Some(2))];
        assert_eq!(anchor_threads(&files, &threads).files[0].outdated, vec![0]);
        unfold_for_threads(&mut files, &threads);
        assert_eq!(
            anchor_threads(&files, &threads).files[0].by_row.get(&1),
            Some(&vec![0])
        );
    }

    #[test]
    fn items_list_headers_threads_and_rows_in_order() {
        let files = vec![
            text_file("a.rs", "a\nb\nc\n", "a\nB\nc\n"),
            binary_file("img.png"),
        ];
        let threads = vec![
            thread("a.rs", Side::Right, Some(2)),
            thread("a.rs", Side::Left, None),
        ];
        let l = build(&files, &threads, &[false, false]);
        assert_eq!(
            l.items(),
            [
                Item::FileHeader { file: 0 },
                Item::OutdatedTitle { file: 0 },
                Item::Thread {
                    file: 0,
                    thread: 1,
                    placement: Placement::Outdated
                },
                Item::Row { file: 0, row: 0 },
                Item::Row { file: 0, row: 1 },
                line(0),
                Item::Row { file: 0, row: 2 },
                Item::FileHeader { file: 1 },
                Item::Notice { file: 1 },
            ]
        );
        assert_eq!(l.change_starts(), [4]);
        assert_eq!(l.header_item(1), Some(7));
        assert_eq!(l.items()[5].file(), Some(0));
    }

    #[test]
    fn file_level_threads_have_their_own_section() {
        let files = vec![text_file("a.rs", "a\n", "b\n")];
        let mut on_file = thread("a.rs", Side::Right, None);
        on_file.file_level = true;
        let threads = vec![on_file, thread("a.rs", Side::Right, None)];
        assert_eq!(
            anchor_threads(&files, &threads).files[0].file_level,
            vec![0]
        );
        let l = build(&files, &threads, &[false]);
        assert_eq!(
            l.items()[..5],
            [
                Item::FileHeader { file: 0 },
                Item::FileCommentsTitle { file: 0 },
                Item::Thread {
                    file: 0,
                    thread: 0,
                    placement: Placement::File
                },
                Item::OutdatedTitle { file: 0 },
                Item::Thread {
                    file: 0,
                    thread: 1,
                    placement: Placement::Outdated
                },
            ]
        );
    }

    #[test]
    fn a_thread_on_a_file_outside_the_diff_is_listed_first() {
        let files = vec![text_file("a.rs", "a\n", "b\n")];
        let threads = vec![thread("gone.rs", Side::Right, Some(1))];
        let l = build(&files, &threads, &[false]);
        assert_eq!(
            l.items()[..3],
            [
                Item::OrphanTitle,
                Item::OrphanThread { thread: 0 },
                Item::FileHeader { file: 0 }
            ]
        );
        assert_eq!(l.items()[0].file(), None);
        assert_eq!(l.height(0), TITLE_HEIGHT);
        assert_eq!(l.height(1), THREAD_ESTIMATE);
        assert_eq!(l.top(2), TITLE_HEIGHT + THREAD_ESTIMATE);
        assert_eq!(
            l.change_starts(),
            [3],
            "title, orphan, header, then the row"
        );
        let measured = HashMap::from([(0, 33.0)]);
        let l = DiffLayout::build(&files, &threads, &[false], 20.0, &measured);
        assert_eq!(l.height(1), 33.0);
    }

    #[test]
    fn a_thread_on_the_old_path_of_a_renamed_file_is_placed_in_it() {
        let mut file = text_file("new.rs", "a\n", "b\n");
        file.old_path = Some("old.rs".into());
        let files = vec![file];
        let threads = vec![thread("old.rs", Side::Left, Some(1))];
        let a = anchor_threads(&files, &threads);
        assert_eq!(a.files[0].by_row.get(&0), Some(&vec![0]));
        assert!(a.orphans.is_empty());

        // Unfolding finds the renamed file by its old path too.
        let old: String = (0..12).map(|i| format!("l{i}\n")).collect();
        let mut renamed = text_file("new.rs", &old, &old.replace("l11\n", "L11\n"));
        renamed.old_path = Some("old.rs".into());
        let mut files = vec![renamed];
        let threads = vec![thread("old.rs", Side::Left, Some(2))];
        assert_eq!(anchor_threads(&files, &threads).files[0].outdated, vec![0]);
        unfold_for_threads(&mut files, &threads);
        assert_eq!(
            anchor_threads(&files, &threads).files[0].by_row.get(&1),
            Some(&vec![0])
        );
    }

    #[test]
    fn a_line_thread_on_a_file_without_rows_goes_to_the_file_comments() {
        let files = vec![binary_file("img.png")];
        let threads = vec![thread("img.png", Side::Right, Some(3))];
        assert_eq!(
            anchor_threads(&files, &threads).files[0].file_level,
            vec![0]
        );
        let l = build(&files, &threads, &[false]);
        assert_eq!(
            l.items(),
            [
                Item::FileHeader { file: 0 },
                Item::FileCommentsTitle { file: 0 },
                Item::Thread {
                    file: 0,
                    thread: 0,
                    placement: Placement::File
                },
                Item::Notice { file: 0 },
            ]
        );
    }

    #[test]
    fn two_threads_on_a_row_keep_their_order() {
        let files = vec![text_file("a.rs", "a\n", "b\n")];
        let threads = vec![
            thread("a.rs", Side::Right, Some(1)),
            thread("a.rs", Side::Right, Some(1)),
        ];
        assert_eq!(
            anchor_threads(&files, &threads).files[0].by_row.get(&0),
            Some(&vec![0, 1])
        );
        let l = build(&files, &threads, &[false]);
        assert_eq!(l.items()[2..], [line(0), line(1)]);
    }

    #[test]
    fn a_collapsed_file_is_only_its_header() {
        let files = vec![text_file("a.rs", "a\n", "b\n")];
        let l = build(&files, &[], &[true]);
        assert_eq!(l.items(), [Item::FileHeader { file: 0 }]);
        assert!(l.change_starts().is_empty());
    }

    #[test]
    fn each_block_of_changes_is_one_stop() {
        // File a: Modified, Context, Context, Removed. File b: Modified, Modified, Added.
        let files = vec![
            text_file("a", "a\nb\nc\nd\n", "A\nb\nc\n"),
            text_file("b", "a\nb\n", "A\nB\nC\n"),
        ];
        let l = build(&files, &[], &[false, false]);
        assert_eq!(l.change_starts(), [1, 4, 6]);
    }

    #[test]
    fn a_thread_between_changed_rows_keeps_one_stop() {
        // Rows: Modified, Modified, Context.
        let files = vec![text_file("a.rs", "a\nb\nc\n", "A\nB\nc\n")];
        let threads = vec![thread("a.rs", Side::Right, Some(1))];
        let l = build(&files, &threads, &[false]);
        assert_eq!(
            l.items()[1..4],
            [
                Item::Row { file: 0, row: 0 },
                line(0),
                Item::Row { file: 0, row: 1 }
            ]
        );
        assert_eq!(l.change_starts(), [1]);
    }

    #[test]
    fn change_navigation_stops_at_both_ends() {
        let files = vec![text_file("a", "a\nb\nc\nd\n", "A\nb\nc\n")];
        let l = build(&files, &[], &[false]);
        assert_eq!(l.next_change(None, 0.0), Some(0));
        assert_eq!(l.next_change(Some(0), 0.0), Some(1));
        assert_eq!(l.next_change(Some(1), 0.0), Some(1));
        assert_eq!(l.next_change(Some(usize::MAX), 0.0), Some(1));
        assert_eq!(l.prev_change(Some(1), 0.0), Some(0));
        assert_eq!(l.prev_change(Some(0), 0.0), Some(0));
        assert_eq!(
            l.prev_change(Some(9), 0.0),
            Some(1),
            "a stale index is clamped"
        );
        assert_eq!(build(&[], &[], &[]).next_change(None, 0.0), None);
        assert_eq!(build(&[], &[], &[]).prev_change(Some(0), 0.0), None);
    }

    #[test]
    fn change_navigation_without_a_cursor_starts_from_a_position() {
        // Items: header (0..30), 4 rows of 20: the stops are items 1 and 4,
        // at tops 30.0 and 90.0.
        let files = vec![text_file("a", "a\nb\nc\nd\n", "A\nb\nc\n")];
        let l = build(&files, &[], &[false]);
        assert_eq!(l.change_starts(), [1, 4]);
        assert_eq!(l.next_change(None, 0.0), Some(0));
        assert_eq!(l.next_change(None, 30.0), Some(0), "a stop at the position");
        assert_eq!(l.next_change(None, 31.0), Some(1));
        assert_eq!(l.next_change(None, 500.0), Some(1), "past the last stop");
        assert_eq!(l.prev_change(None, 500.0), Some(1));
        assert_eq!(l.prev_change(None, 90.0), Some(0), "not above itself");
        assert_eq!(l.prev_change(None, 60.0), Some(0));
        assert_eq!(l.prev_change(None, 10.0), Some(0), "before the first stop");
    }

    #[test]
    fn offsets_follow_heights_and_measured_threads() {
        let files = vec![text_file("a.rs", "a\nb\nc\n", "a\nB\nc\n")];
        let threads = vec![thread("a.rs", Side::Right, Some(2))];
        // Items: header, row 0, row 1, thread, row 2.
        let mut l = build(&files, &threads, &[false]);
        assert_eq!(l.top(1), HEADER_HEIGHT);
        assert_eq!(l.total_height(), HEADER_HEIGHT + 60.0 + THREAD_ESTIMATE);
        assert_eq!(l.visible(0.0, 1.0), 0..1);
        assert_eq!(l.visible(HEADER_HEIGHT + 25.0, HEADER_HEIGHT + 45.0), 2..4);
        assert!(l.set_height(3, 50.0));
        assert!(!l.set_height(3, 50.2), "sub-pixel changes are ignored");
        assert_eq!(l.total_height(), HEADER_HEIGHT + 60.0 + 50.0);
        assert_eq!(l.top(4), HEADER_HEIGHT + 40.0 + 50.0);
        let measured = HashMap::from([(0, 42.0)]);
        let l = DiffLayout::build(&files, &threads, &[false], 20.0, &measured);
        assert_eq!(l.height(3), 42.0);
        assert_eq!(build(&[], &[], &[]).visible(0.0, 100.0), 0..0);
    }

    #[test]
    fn set_height_ignores_non_finite_and_clamps_negative() {
        let files = vec![text_file("a.rs", "a\nb\nc\n", "a\nB\nc\n")];
        let threads = vec![thread("a.rs", Side::Right, Some(2))];
        let mut l = build(&files, &threads, &[false]);
        let total = l.total_height();
        assert!(!l.set_height(3, f32::NAN));
        assert!(!l.set_height(3, f32::INFINITY));
        assert_eq!(l.total_height(), total);
        assert!(l.set_height(3, -5.0));
        assert_eq!(l.height(3), 0.0);
        assert_eq!(l.total_height(), total - THREAD_ESTIMATE);
    }

    #[test]
    fn visible_is_exact_at_item_boundaries() {
        let files = vec![text_file("a.rs", "a\nb\n", "a\nB\n")];
        // Items: header (0..30), row 0 (30..50), row 1 (50..70).
        let l = build(&files, &[], &[false]);
        assert_eq!(l.visible(30.0, 50.0), 1..2, "edges are half-open");
        assert_eq!(l.visible(29.9, 30.1), 0..2);
        assert_eq!(l.visible(1000.0, 1100.0), 3..3, "past the end");
    }
}
