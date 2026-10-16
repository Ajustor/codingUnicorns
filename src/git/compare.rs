//! The diff of a pull request, computed locally from its commits: the files
//! it changes and, for each text file, its lines paired side by side.

use super::word_diff::{changed_ranges, WordChanges, MAX_LINE};
use super::GitStatus;
use crate::editor::{LARGE_FILE_BYTES, LARGE_FILE_LINES};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;

/// Unchanged lines shown around each change; the rest is folded.
pub const CONTEXT: usize = 3;

/// Summed `old.len() * new.len()` of the `Modified` pairs of one file whose
/// words are compared (a conservative bound: the real work is smaller);
/// beyond it the remaining rows get no word highlights
/// (the painter treats a missing entry as none).
const WORD_DIFF_BUDGET: usize = 250_000_000;

/// One row of a side-by-side diff. Indexes are 0-based lines of
/// [`TextDiff::old`] and [`TextDiff::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffRow {
    /// The same line on both sides.
    Context { old: usize, new: usize },
    /// A removed line facing the added line that replaces it.
    Modified { old: usize, new: usize },
    /// A removed line facing a filler.
    Removed { old: usize },
    /// An added line facing a filler.
    Added { new: usize },
    /// `len` unchanged lines starting at `old` / `new`, folded.
    Fold { old: usize, new: usize, len: usize },
}

impl DiffRow {
    pub fn is_change(self) -> bool {
        matches!(
            self,
            Self::Modified { .. } | Self::Removed { .. } | Self::Added { .. }
        )
    }
}

/// A text file's two versions and their rows.
#[derive(Debug, Clone, PartialEq)]
pub struct TextDiff {
    /// Lines to display: tabs are expanded to 4 spaces.
    pub old: Vec<String>,
    /// Lines to display: tabs are expanded to 4 spaces.
    pub new: Vec<String>,
    pub rows: Vec<DiffRow>,
    /// Changed words of each `Modified` row, by its old line.
    pub words: HashMap<usize, WordChanges>,
}

impl TextDiff {
    /// Diff `old` against `new` (both with `\n` line endings). The texts are
    /// compared as they are: a tab and four spaces differ, as in git.
    pub fn new(old: &str, new: &str) -> Self {
        let old_lines = split_lines(old);
        let new_lines = split_lines(new);
        let rows = align(old, new, old_lines.len(), new_lines.len());
        let old: Vec<String> = old_lines.iter().map(|l| expand_tabs(l)).collect();
        let new: Vec<String> = new_lines.iter().map(|l| expand_tabs(l)).collect();
        let words = word_changes(&rows, &old, &new, WORD_DIFF_BUDGET);
        Self {
            old,
            new,
            rows,
            words,
        }
    }

    /// Replace the fold at `row` by its lines. False when `row` isn't a fold.
    pub fn unfold(&mut self, row: usize) -> bool {
        let Some(&DiffRow::Fold { old, new, len }) = self.rows.get(row) else {
            return false;
        };
        self.rows.splice(
            row..=row,
            (0..len).map(|k| DiffRow::Context {
                old: old + k,
                new: new + k,
            }),
        );
        true
    }

    /// Lines added and removed; a modified line counts as both.
    pub fn counts(&self) -> (usize, usize) {
        self.rows.iter().fold((0, 0), |(add, del), row| match row {
            DiffRow::Modified { .. } => (add + 1, del + 1),
            DiffRow::Added { .. } => (add + 1, del),
            DiffRow::Removed { .. } => (add, del + 1),
            _ => (add, del),
        })
    }
}

/// What the view shows for a file.
#[derive(Debug, Clone, PartialEq)]
pub enum DiffContent {
    Text(TextDiff),
    /// NUL bytes or not UTF-8.
    Binary,
    /// Over the editor's large-file limits: shown on demand ("Show anyway").
    TooLarge,
    /// A blob is missing locally (shallow or partial clone): the reason.
    Unavailable(String),
}

/// The content to show for a file going from `old` to `new`; `force` ignores
/// the size limits.
pub fn text_content(old: &[u8], new: &[u8], force: bool) -> DiffContent {
    if old.contains(&0) || new.contains(&0) {
        return DiffContent::Binary;
    }
    let (Ok(old), Ok(new)) = (std::str::from_utf8(old), std::str::from_utf8(new)) else {
        return DiffContent::Binary;
    };
    // Checked before the copies.
    if !force && (old.len() > LARGE_FILE_BYTES || new.len() > LARGE_FILE_BYTES) {
        return DiffContent::TooLarge;
    }
    let (old, new) = (normalize_newlines(old), normalize_newlines(new));
    if !force && (too_many_lines(&old) || too_many_lines(&new)) {
        return DiffContent::TooLarge;
    }
    DiffContent::Text(TextDiff::new(&old, &new))
}

/// `\n` line endings.
fn normalize_newlines(text: &str) -> String {
    if text.contains('\r') {
        text.replace("\r\n", "\n")
    } else {
        text.to_string()
    }
}

fn too_many_lines(text: &str) -> bool {
    text.split_terminator('\n').count() > LARGE_FILE_LINES
}

/// Lines as git splits them: only `\n` ends a line, so a lone `\r` stays.
fn split_lines(text: &str) -> Vec<&str> {
    text.split_terminator('\n').collect()
}

fn expand_tabs(line: &str) -> String {
    line.replace('\t', "    ")
}

/// Changed words of each `Modified` row, until the summed cost of the pairs
/// (`old.len() * new.len()`) would exceed `budget`.
fn word_changes(
    rows: &[DiffRow],
    old: &[String],
    new: &[String],
    budget: usize,
) -> HashMap<usize, WordChanges> {
    let mut spent = 0usize;
    let mut words = HashMap::new();
    for row in rows {
        let DiffRow::Modified { old: o, new: n } = *row else {
            continue;
        };
        let (Some(old_line), Some(new_line)) = (old.get(o), new.get(n)) else {
            continue;
        };
        // Too long to compare: costs nothing.
        if old_line.len() > MAX_LINE || new_line.len() > MAX_LINE {
            continue;
        }
        spent = spent.saturating_add(old_line.len().saturating_mul(new_line.len()));
        if spent > budget {
            break;
        }
        if let Some(changes) = changed_ranges(old_line, new_line) {
            words.insert(o, changes);
        }
    }
    words
}

/// A change between two texts: the lines it replaces and the lines replacing
/// them. For an empty side, the range is empty and its start is the 0-based
/// index of the line where the other side's lines land (the line after which
/// they are inserted, counting from 1).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hunk {
    old: Range<usize>,
    new: Range<usize>,
}

/// Rows of `old` → `new` for the hunks libgit2 found.
fn align(old: &str, new: &str, old_len: usize, new_len: usize) -> Vec<DiffRow> {
    rows_of(hunks(old, new), old_len, new_len, old != new)
}

/// Rows for `hunks`: each paired line by line, unchanged runs shown
/// [`CONTEXT`] lines around the hunks and folded beyond. `differ` tells
/// whether the texts differ: with no hunk then, or with hunks that don't fit
/// the files, everything is shown as replaced.
fn rows_of(hunks: Vec<Hunk>, old_len: usize, new_len: usize, differ: bool) -> Vec<DiffRow> {
    let replace_all = || {
        vec![Hunk {
            old: 0..old_len,
            new: 0..new_len,
        }]
    };
    if hunks.is_empty() && differ {
        log::warn!("diff found no change between two different texts: showing them as replaced");
        return build_rows(&replace_all(), old_len, new_len).unwrap_or_default();
    }
    build_rows(&hunks, old_len, new_len).unwrap_or_else(|| {
        log::warn!("diff hunks don't fit the files: showing them as replaced");
        build_rows(&replace_all(), old_len, new_len).unwrap_or_default()
    })
}

/// The rows of `hunks`, or `None` when they are not ordered, overlap, or
/// leave the two files with different amounts of unchanged lines.
fn build_rows(hunks: &[Hunk], old_len: usize, new_len: usize) -> Option<Vec<DiffRow>> {
    let mut rows = Vec::new();
    let (mut o, mut n) = (0, 0);
    for (index, hunk) in hunks.iter().enumerate() {
        debug_assert!(hunk.old.start >= o && hunk.new.start >= n);
        let gap = hunk.old.start.checked_sub(o)?;
        if hunk.new.start.checked_sub(n)? != gap {
            return None;
        }
        push_gap(&mut rows, o, n, gap, index == 0, false);
        let (old_count, new_count) = (hunk.old.len(), hunk.new.len());
        let paired = old_count.min(new_count);
        rows.extend((0..paired).map(|k| DiffRow::Modified {
            old: hunk.old.start + k,
            new: hunk.new.start + k,
        }));
        rows.extend((paired..old_count).map(|k| DiffRow::Removed {
            old: hunk.old.start + k,
        }));
        rows.extend((paired..new_count).map(|k| DiffRow::Added {
            new: hunk.new.start + k,
        }));
        o = hunk.old.end;
        n = hunk.new.end;
    }
    let rest = old_len.checked_sub(o)?;
    debug_assert_eq!(Some(rest), new_len.checked_sub(n));
    if new_len.checked_sub(n)? != rest {
        return None;
    }
    push_gap(&mut rows, o, n, rest, hunks.is_empty(), true);
    Some(rows)
}

/// Hunks of `old` → `new` without context, 0-based.
fn hunks(old: &str, new: &str) -> Vec<Hunk> {
    let mut opts = git2::DiffOptions::new();
    opts.context_lines(0).indent_heuristic(true);
    let patch = match git2::Patch::from_buffers(
        old.as_bytes(),
        None,
        new.as_bytes(),
        None,
        Some(&mut opts),
    ) {
        Ok(patch) => patch,
        Err(e) => {
            log::warn!("diff failed: {e}");
            return Vec::new();
        }
    };
    (0..patch.num_hunks())
        .filter_map(|h| patch.hunk(h).ok())
        .map(|(hunk, _)| {
            // libgit2 numbers lines from 1; an empty side's start is the line
            // *before* the change (0 = top), already the 0-based index.
            let range = |start: u32, count: u32| {
                let start = (if count == 0 { start } else { start - 1 }) as usize;
                start..start + count as usize
            };
            Hunk {
                old: range(hunk.old_start(), hunk.old_lines()),
                new: range(hunk.new_start(), hunk.new_lines()),
            }
        })
        .collect()
}

/// `len` unchanged lines starting at `old` / `new`: [`CONTEXT`] lines after
/// the previous hunk (none at the top of the file) and before the next one
/// (none at the end), the rest folded. A single line is never folded: the
/// fold row would take as much room as the line.
fn push_gap(
    rows: &mut Vec<DiffRow>,
    old: usize,
    new: usize,
    len: usize,
    at_start: bool,
    at_end: bool,
) {
    let mut head = if at_start { 0 } else { CONTEXT.min(len) };
    let tail = if at_end { 0 } else { CONTEXT.min(len - head) };
    let mut folded = len - head - tail;
    if folded == 1 {
        head += 1;
        folded = 0;
    }
    rows.extend((0..head).map(|k| DiffRow::Context {
        old: old + k,
        new: new + k,
    }));
    if folded > 0 {
        rows.push(DiffRow::Fold {
            old: old + head,
            new: new + head,
            len: folded,
        });
    }
    rows.extend((len - tail..len).map(|k| DiffRow::Context {
        old: old + k,
        new: new + k,
    }));
}

// ── Files of a pull request ─────────────────────────────────────────────────

/// Why a commit of the pull request is missing locally.
pub const CHANGED: &str = "The pull request changed while loading, refresh it.";

/// How a pull request changes a file. A type change (file to symlink, file to
/// submodule…) is `Modified`: each path appears once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDiffKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// One file the pull request changes.
#[derive(Debug, Clone, PartialEq)]
pub struct FileDiff {
    /// `None` for an added file.
    pub old_path: Option<String>,
    /// `None` for a deleted file.
    pub new_path: Option<String>,
    pub kind: FileDiffKind,
    pub additions: usize,
    pub deletions: usize,
    /// `None` when the old side is missing or a submodule.
    pub old_blob: Option<git2::Oid>,
    /// `None` when the new side is missing or a submodule: never a commit id,
    /// so [`load_text`] never reads a commit as a blob. Key a "viewed" mark
    /// with [`FileDiff::version`] instead.
    pub new_blob: Option<git2::Oid>,
    /// The commit a submodule points to in the pull request.
    pub new_commit: Option<git2::Oid>,
    pub content: DiffContent,
}

impl FileDiff {
    /// The path in the pull request (the old one for a deleted file).
    pub fn path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or("")
    }

    /// `old → new` for a rename, the path otherwise.
    pub fn display_path(&self) -> String {
        match (self.kind, &self.old_path, &self.new_path) {
            (FileDiffKind::Renamed, Some(old), Some(new)) => format!("{old} → {new}"),
            _ => self.path().to_string(),
        }
    }

    /// What the file is in the pull request: its blob, or the commit of a
    /// submodule. A file marked viewed stays viewed while this is the same.
    pub fn version(&self) -> Option<git2::Oid> {
        self.new_blob.or(self.new_commit)
    }

    /// Set the content, and the counts that go with it.
    pub fn set_content(&mut self, content: DiffContent) {
        (self.additions, self.deletions) = match &content {
            DiffContent::Text(text) => text.counts(),
            _ => (0, 0),
        };
        self.content = content;
    }
}

/// Like git's `diff.renameLimit`: inexact rename detection is skipped when
/// there are more than `RENAME_LIMIT²` (added × deleted) candidate pairs.
const RENAME_LIMIT: usize = 1000;

/// The files `head_sha` changes since its merge base with `base_sha`
/// (GitHub's three-dot diff), renames detected. Each path appears once.
pub fn diff_pull(workdir: &Path, base_sha: &str, head_sha: &str) -> Result<Vec<FileDiff>, String> {
    diff_pull_limited(workdir, base_sha, head_sha, RENAME_LIMIT)
}

fn diff_pull_limited(
    workdir: &Path,
    base_sha: &str,
    head_sha: &str,
    rename_limit: usize,
) -> Result<Vec<FileDiff>, String> {
    let repo = git2::Repository::discover(workdir).map_err(|e| format!("Repo error: {e}"))?;
    let base = find_commit(&repo, base_sha)?;
    let head = find_commit(&repo, head_sha)?;
    let merge_base = repo.merge_base(base.id(), head.id()).map_err(|e| {
        if e.code() == git2::ErrorCode::NotFound && repo.is_shallow() {
            "The local clone is shallow and doesn't reach the pull request's base: \
             run `git fetch --unshallow`."
                .to_string()
        } else {
            format!(
                "Could not find the pull request's merge base: {}",
                e.message()
            )
        }
    })?;
    let old_tree = repo
        .find_commit(merge_base)
        .and_then(|c| c.tree())
        .map_err(|e| format!("Can't read the merge base: {}", e.message()))?;
    let new_tree = head
        .tree()
        .map_err(|e| format!("Can't read the pull request's head: {}", e.message()))?;
    let mut old_index = TreeIndex::new(&repo, old_tree.clone());
    let mut new_index = TreeIndex::new(&repo, new_tree.clone());
    let changes = detect_renames(&repo, &old_tree, &new_tree, &mut old_index, rename_limit)?;
    Ok(changes
        .iter()
        .map(|change| file_diff(&repo, &mut old_index, &mut new_index, change))
        .collect())
}

/// A delta of a tree-to-tree diff, owned so that the results of several
/// diffs can be merged. A missing side has a zero id.
#[derive(Debug, Clone)]
struct Change {
    status: git2::Delta,
    old_path: Vec<u8>,
    old_id: git2::Oid,
    new_path: Vec<u8>,
    new_id: git2::Oid,
}

impl Change {
    fn of(delta: &git2::DiffDelta) -> Self {
        let path = |file: git2::DiffFile| file.path_bytes().unwrap_or_default().to_vec();
        Self {
            status: delta.status(),
            old_path: path(delta.old_file()),
            old_id: delta.old_file().id(),
            new_path: path(delta.new_file()),
            new_id: delta.new_file().id(),
        }
    }

    /// The path in the pull request (the old one for a deleted file).
    fn path(&self) -> &[u8] {
        if self.status == git2::Delta::Deleted {
            &self.old_path
        } else {
            &self.new_path
        }
    }
}

fn changes_of(diff: &git2::Diff) -> Vec<Change> {
    diff.deltas().map(|delta| Change::of(&delta)).collect()
}

/// The changes between the trees with their renames, as `git diff -M` finds
/// them, sorted by path.
fn detect_renames(
    repo: &git2::Repository,
    old_tree: &git2::Tree,
    new_tree: &git2::Tree,
    old_index: &mut TreeIndex,
    rename_limit: usize,
) -> Result<Vec<Change>, String> {
    let make = |paths: &[Vec<u8>]| {
        let mut opts = git2::DiffOptions::new();
        // A type change is one delta, not an addition and a deletion.
        opts.include_typechange(true);
        if !paths.is_empty() {
            opts.disable_pathspec_match(true);
            for path in paths {
                opts.pathspec(path.clone());
            }
        }
        repo.diff_tree_to_tree(Some(old_tree), Some(new_tree), Some(&mut opts))
            .map_err(|e| format!("Can't compare the trees: {}", e.message()))
    };
    let plain = changes_of(&make(&[])?);
    let count = |status| plain.iter().filter(|c| c.status == status).count();
    // libgit2 compares each added file with up to `rename_limit` candidates,
    // exact matches included: all the files that can be a rename's source.
    let typechanged: Vec<&Change> = plain
        .iter()
        .filter(|c| c.status == git2::Delta::Typechange)
        .collect();
    let blob_typechanges = typechanged
        .iter()
        .filter(|c| !old_index.is_gitlink(&c.old_path))
        .count();
    let sources = count(git2::Delta::Deleted) + blob_typechanges;
    let find = |limit: usize, exact: bool| {
        let mut find = git2::DiffFindOptions::new();
        find.renames(true).rename_limit(limit.max(1));
        if exact {
            find.exact_match_only(true);
        }
        find
    };

    // Pass 1: exact renames, whatever the number of files (ids only).
    let mut diff = make(&[])?;
    let mut changes = match diff.find_similar(Some(&mut find(sources, true))) {
        Ok(()) => changes_of(&diff),
        Err(e) => {
            log::warn!("rename detection failed ({e}): showing no renames");
            return Ok(sorted(plain));
        }
    };

    // Pass 2: inexact renames among what pass 1 left, if git would try.
    let leftover = |status| {
        changes
            .iter()
            .filter(|c| c.status == status)
            .map(|c| c.path().to_vec())
            .collect::<Vec<_>>()
    };
    let (added, deleted) = (leftover(git2::Delta::Added), leftover(git2::Delta::Deleted));
    if !added.is_empty() && !deleted.is_empty() {
        if added.len().saturating_mul(deleted.len()) > rename_limit * rename_limit {
            log::warn!("inexact rename detection skipped: too many files");
        } else if let Some(found) = inexact_renames(&make, &find, &added, &deleted) {
            merge_renames(&mut changes, found);
        }
    }

    // A rename libgit2 builds from a type change repeats a path: show what
    // the plain diff shows for these paths instead.
    let typechange_paths: HashSet<Vec<u8>> =
        typechanged.iter().map(|c| c.path().to_vec()).collect();
    let mut affected = typechange_paths.clone();
    for change in &changes {
        if change.status == git2::Delta::Renamed
            && (typechange_paths.contains(&change.old_path)
                || typechange_paths.contains(&change.new_path))
        {
            affected.insert(change.old_path.clone());
            affected.insert(change.new_path.clone());
        }
    }
    if !affected.is_empty() {
        changes.retain(|c| !affected.contains(c.path()) && !affected.contains(&c.old_path));
        changes.extend(
            plain
                .iter()
                .filter(|c| affected.contains(c.path()))
                .cloned(),
        );
    }
    Ok(sorted(changes))
}

fn sorted(mut changes: Vec<Change>) -> Vec<Change> {
    changes.sort_by(|a, b| a.path().cmp(b.path()));
    changes
}

/// The renames among `added` and `deleted` files, by similarity: in a diff of
/// those paths only (`find_similar` can't run twice on a diff).
fn inexact_renames<'r>(
    make: &impl Fn(&[Vec<u8>]) -> Result<git2::Diff<'r>, String>,
    find: &impl Fn(usize, bool) -> git2::DiffFindOptions,
    added: &[Vec<u8>],
    deleted: &[Vec<u8>],
) -> Option<Vec<Change>> {
    let paths: Vec<Vec<u8>> = added.iter().chain(deleted).cloned().collect();
    let mut diff = make(&paths)
        .map_err(|e| log::warn!("inexact rename detection failed ({e})"))
        .ok()?;
    diff.find_similar(Some(&mut find(deleted.len(), false)))
        .map_err(|e| log::warn!("inexact rename detection failed ({e})"))
        .ok()?;
    Some(
        changes_of(&diff)
            .into_iter()
            .filter(|c| c.status == git2::Delta::Renamed)
            .collect(),
    )
}

/// Replace the added and deleted entries of `changes` that `renames` pair up.
/// An entry the sub-diff doesn't mention stays as it is.
fn merge_renames(changes: &mut Vec<Change>, renames: Vec<Change>) {
    let position = |changes: &[Change], status, path: &[u8]| {
        changes
            .iter()
            .position(|c| c.status == status && c.path() == path)
    };
    for rename in renames {
        let new = position(changes, git2::Delta::Added, &rename.new_path);
        let old = position(changes, git2::Delta::Deleted, &rename.old_path);
        let (Some(new), Some(old)) = (new, old) else {
            continue;
        };
        changes.swap_remove(new.max(old));
        changes.swap_remove(new.min(old));
        changes.push(rename);
    }
}

/// Reload a file's content as text whatever its size ("Show anyway").
pub fn load_text(
    workdir: &Path,
    old_blob: Option<git2::Oid>,
    new_blob: Option<git2::Oid>,
) -> Result<DiffContent, String> {
    let repo = git2::Repository::discover(workdir).map_err(|e| format!("Repo error: {e}"))?;
    Ok(text_content(
        &blob_bytes(&repo, old_blob)?,
        &blob_bytes(&repo, new_blob)?,
        true,
    ))
}

fn find_commit<'r>(repo: &'r git2::Repository, sha: &str) -> Result<git2::Commit<'r>, String> {
    git2::Oid::from_str(sha)
        .ok()
        .and_then(|oid| repo.find_commit(oid).ok())
        .ok_or_else(|| CHANGED.to_string())
}

/// The directories of a tree, each read once: libgit2 doesn't cache the big
/// ones, and looking a path up from the root would read them for every file.
struct TreeIndex<'r> {
    repo: &'r git2::Repository,
    root: git2::Tree<'r>,
    /// By path; `None` when there is no such directory.
    dirs: HashMap<Vec<u8>, Option<git2::Tree<'r>>>,
}

impl<'r> TreeIndex<'r> {
    fn new(repo: &'r git2::Repository, root: git2::Tree<'r>) -> Self {
        Self {
            repo,
            root,
            dirs: HashMap::new(),
        }
    }

    /// Whether `path` is a submodule. (`DiffFile::mode` panics on a mode
    /// that isn't one of git's canonical ones, so the tree is asked.)
    fn is_gitlink(&mut self, path: &[u8]) -> bool {
        let (dir, name) = split_path(path);
        self.dir(dir)
            .and_then(|tree| tree.get_name_bytes(name))
            .is_some_and(|entry| entry.filemode_raw() & 0o170000 == 0o160000)
    }

    fn dir(&mut self, path: &[u8]) -> Option<&git2::Tree<'r>> {
        if path.is_empty() {
            return Some(&self.root);
        }
        if !self.dirs.contains_key(path) {
            let (parent, name) = split_path(path);
            let id = self
                .dir(parent)
                .and_then(|tree| tree.get_name_bytes(name))
                .filter(|entry| entry.kind() == Some(git2::ObjectType::Tree))
                .map(|entry| entry.id());
            let tree = id.and_then(|id| self.repo.find_tree(id).ok());
            self.dirs.insert(path.to_vec(), tree);
        }
        self.dirs[path].as_ref()
    }
}

/// `dir/name` split at the last `/`.
fn split_path(path: &[u8]) -> (&[u8], &[u8]) {
    match path.iter().rposition(|&b| b == b'/') {
        Some(slash) => (&path[..slash], &path[slash + 1..]),
        None => (&[], path),
    }
}

/// What one side of a delta holds.
#[derive(Default)]
struct Side {
    /// A regular file or a symlink.
    blob: Option<git2::Oid>,
    /// A submodule: the id is a commit of the submodule, not a blob.
    commit: Option<git2::Oid>,
}

impl Side {
    fn of(id: git2::Oid, path: &[u8], tree: &mut TreeIndex) -> Self {
        if id.is_zero() {
            Self::default()
        } else if tree.is_gitlink(path) {
            Self {
                blob: None,
                commit: Some(id),
            }
        } else {
            Self {
                blob: Some(id),
                commit: None,
            }
        }
    }

    /// The bytes to compare: what git shows for a submodule, the blob otherwise.
    fn bytes(&self, repo: &git2::Repository) -> Result<Vec<u8>, String> {
        match self.commit {
            Some(commit) => Ok(format!("Subproject commit {commit}\n").into_bytes()),
            None => blob_bytes(repo, self.blob),
        }
    }
}

fn file_diff(
    repo: &git2::Repository,
    old_tree: &mut TreeIndex,
    new_tree: &mut TreeIndex,
    change: &Change,
) -> FileDiff {
    let kind = match change.status {
        git2::Delta::Added => FileDiffKind::Added,
        git2::Delta::Deleted => FileDiffKind::Deleted,
        git2::Delta::Renamed => FileDiffKind::Renamed,
        // Modified, and typechange.
        _ => FileDiffKind::Modified,
    };
    // libgit2's paths use `/`; a `\` is part of a name.
    let path = |path: &[u8]| String::from_utf8_lossy(path).into_owned();
    let old = Side::of(change.old_id, &change.old_path, old_tree);
    let new = Side::of(change.new_id, &change.new_path, new_tree);
    let mut file = FileDiff {
        old_path: (kind != FileDiffKind::Added).then(|| path(&change.old_path)),
        new_path: (kind != FileDiffKind::Deleted).then(|| path(&change.new_path)),
        kind,
        additions: 0,
        deletions: 0,
        old_blob: old.blob,
        new_blob: new.blob,
        new_commit: new.commit,
        content: DiffContent::Binary,
    };
    file.set_content(sides_content(repo, &old, &new));
    file
}

/// What the view shows for two sides; a blob on both sides (a pure rename
/// or mode change) is read once.
fn sides_content(repo: &git2::Repository, old: &Side, new: &Side) -> DiffContent {
    let old_bytes = match old.bytes(repo) {
        Ok(bytes) => bytes,
        Err(reason) => return DiffContent::Unavailable(reason),
    };
    if old.blob.is_some() && old.blob == new.blob {
        return text_content(&old_bytes, &old_bytes, false);
    }
    match new.bytes(repo) {
        Ok(new_bytes) => text_content(&old_bytes, &new_bytes, false),
        Err(reason) => DiffContent::Unavailable(reason),
    }
}

fn blob_bytes(repo: &git2::Repository, blob: Option<git2::Oid>) -> Result<Vec<u8>, String> {
    match blob {
        None => Ok(Vec::new()),
        Some(oid) => repo
            .find_blob(oid)
            .map(|b| b.content().to_vec())
            .map_err(|e| {
                format!(
                    "A file is missing locally (shallow or partial clone): {}",
                    e.message()
                )
            }),
    }
}

// ── Fetch and checkout ──────────────────────────────────────────────────────

/// Fetch pull request `number`'s head into `refs/remotes/origin/pr/<number>`,
/// and its base branch, from `origin`. Works for pull requests from forks:
/// GitHub keeps every pull request's head under `refs/pull/` in the base repo.
/// Leaves `FETCH_HEAD` alone. Fails when `origin` doesn't publish the pull
/// ref (a mirror or proxy may hide it), since libgit2 fetches nothing then
/// without an error.
pub fn fetch_pull(workdir: &Path, number: u64, base_ref: &str) -> Result<(), String> {
    let repo = git2::Repository::discover(workdir).map_err(|e| format!("Repo error: {e}"))?;
    let head = format!("+refs/pull/{number}/head:refs/remotes/origin/pr/{number}");
    let base = format!("+refs/heads/{base_ref}:refs/remotes/origin/{base_ref}");
    super::remote::fetch_remote_with(&repo, &[head.as_str(), base.as_str()], false)?;
    if repo
        .find_reference(&format!("refs/remotes/origin/pr/{number}"))
        .is_err()
    {
        return Err(format!(
            "origin doesn't publish refs/pull/{number}/head (a mirror or proxy may hide it)."
        ));
    }
    Ok(())
}

/// Local branch to check pull request `number` out on: its own branch name
/// when it comes from the same repository, `pr/<number>` from a fork.
pub fn pull_branch_name(number: u64, head_ref: &str, from_fork: bool) -> String {
    if from_fork {
        format!("pr/{number}")
    } else {
        head_ref.to_string()
    }
}

/// The ref on `origin` the local branch of pull request `number` tracks, as
/// `gh pr checkout` sets it: the head branch itself when the pull request
/// comes from the same repository, `refs/pull/<number>/head` from a fork.
pub fn pull_merge_ref(number: u64, head_ref: &str, from_fork: bool) -> String {
    if from_fork {
        format!("refs/pull/{number}/head")
    } else {
        format!("refs/heads/{head_ref}")
    }
}

/// Delete a branch created for a checkout that failed with `err`; when that
/// fails too, say so in the returned message.
fn undo_branch(repo: &git2::Repository, branch: &str, err: String) -> String {
    let removed = repo
        .find_branch(branch, git2::BranchType::Local)
        .and_then(|mut b| b.delete());
    match removed {
        Ok(()) => err,
        Err(e) => format!(
            "{err} (branch {branch} could not be removed: {})",
            e.message()
        ),
    }
}

impl GitStatus {
    /// Check a pull request out on local branch `branch`, created at
    /// `head_sha` unless it exists (then checked out as is). `Ok(true)` when
    /// the branch already existed. A created branch tracks `origin`'s
    /// `merge_ref` (see [`pull_merge_ref`]); an existing one keeps its
    /// configuration. Refused during a merge, rebase or the like. Local
    /// changes the checkout would overwrite refuse it, and a branch created
    /// for it is deleted again.
    pub fn checkout_pull(
        &mut self,
        branch: &str,
        head_sha: &str,
        merge_ref: &str,
    ) -> Result<bool, String> {
        if !git2::Branch::name_is_valid(branch).unwrap_or(false) {
            return Err(format!("'{branch}' is not a valid branch name"));
        }
        let repo = self.open_repo()?;
        if repo.state() != git2::RepositoryState::Clean {
            return Err("Cannot check out: another operation (merge, rebase, revert, bisect…) is in progress. Finish or abort it first.".to_string());
        }
        let existed = repo.find_branch(branch, git2::BranchType::Local).is_ok();
        if !existed {
            let commit = find_commit(&repo, head_sha)?;
            repo.branch(branch, &commit, false)
                .map_err(|e| e.message().to_string())?;
            let tracking = repo.config().and_then(|mut config| {
                config.set_str(&format!("branch.{branch}.remote"), "origin")?;
                config.set_str(&format!("branch.{branch}.merge"), merge_ref)
            });
            if let Err(e) = tracking {
                return Err(undo_branch(&repo, branch, e.message().to_string()));
            }
        }
        // The full name: a tag called like the branch must not win.
        if let Err(e) = self.checkout_branch(&format!("refs/heads/{branch}")) {
            return Err(if existed {
                e
            } else {
                undo_branch(&repo, branch, e)
            });
        }
        Ok(existed)
    }
}

/// A modified text file, for tests.
#[cfg(test)]
pub(crate) fn text_file(path: &str, old: &str, new: &str) -> FileDiff {
    let mut file = binary_file(path);
    file.set_content(DiffContent::Text(TextDiff::new(old, new)));
    file
}

/// A modified binary file, for tests.
#[cfg(test)]
pub(crate) fn binary_file(path: &str) -> FileDiff {
    FileDiff {
        old_path: Some(path.into()),
        new_path: Some(path.into()),
        kind: FileDiffKind::Modified,
        additions: 0,
        deletions: 0,
        old_blob: None,
        new_blob: None,
        new_commit: None,
        content: DiffContent::Binary,
    }
}

#[cfg(test)]
mod tests {
    use super::DiffRow::*;
    use super::*;

    fn rows(old: &str, new: &str) -> Vec<DiffRow> {
        TextDiff::new(old, new).rows
    }

    #[test]
    fn an_insertion_faces_a_filler() {
        assert_eq!(
            rows("a\nb\nc\n", "a\nb\nX\nc\n"),
            vec![
                Context { old: 0, new: 0 },
                Context { old: 1, new: 1 },
                Added { new: 2 },
                Context { old: 2, new: 3 },
            ]
        );
    }

    #[test]
    fn a_deletion_faces_a_filler() {
        assert_eq!(
            rows("a\nb\nc\n", "a\nc\n"),
            vec![
                Context { old: 0, new: 0 },
                Removed { old: 1 },
                Context { old: 2, new: 1 }
            ]
        );
    }

    #[test]
    fn replaced_lines_pair_up_and_extra_ones_are_added() {
        assert_eq!(
            rows("a\nb\nc\n", "a\nB\nB2\nc\n"),
            vec![
                Context { old: 0, new: 0 },
                Modified { old: 1, new: 1 },
                Added { new: 2 },
                Context { old: 2, new: 3 },
            ]
        );
    }

    #[test]
    fn unchanged_runs_fold_beyond_the_context() {
        let old: String = (0..10).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l5\n", "L5\n");
        assert_eq!(
            rows(&old, &new),
            vec![
                Fold {
                    old: 0,
                    new: 0,
                    len: 2
                },
                Context { old: 2, new: 2 },
                Context { old: 3, new: 3 },
                Context { old: 4, new: 4 },
                Modified { old: 5, new: 5 },
                Context { old: 6, new: 6 },
                Context { old: 7, new: 7 },
                Context { old: 8, new: 8 },
                Context { old: 9, new: 9 },
            ]
        );
    }

    #[test]
    fn identical_files_are_one_fold_and_new_files_all_added() {
        assert_eq!(
            rows("a\nb\n", "a\nb\n"),
            vec![Fold {
                old: 0,
                new: 0,
                len: 2
            }]
        );
        assert_eq!(rows("", "a\nb\n"), vec![Added { new: 0 }, Added { new: 1 }]);
    }

    #[test]
    fn unfold_replaces_a_fold_by_its_lines() {
        let mut d = TextDiff::new("a\nb\n", "a\nb\n");
        assert!(d.unfold(0));
        assert_eq!(
            d.rows,
            vec![Context { old: 0, new: 0 }, Context { old: 1, new: 1 }]
        );
        assert!(!d.unfold(0), "not a fold any more");
    }

    #[test]
    fn modified_rows_carry_their_changed_words_and_count_both_ways() {
        let d = TextDiff::new("let x = 1;\n", "let y = 1;\nmore\n");
        assert_eq!(d.words[&0].new, vec![4..5]);
        assert_eq!(d.counts(), (2, 1));
    }

    #[test]
    fn crlf_and_lf_compare_equal_and_tabs_become_spaces() {
        let DiffContent::Text(d) = text_content(b"a\r\nb\r\n", b"a\nb\n", false) else {
            panic!("not text");
        };
        assert_eq!(d.counts(), (0, 0));
        let DiffContent::Text(d) = text_content(b"", b"\tx\n", false) else {
            panic!("not text");
        };
        assert_eq!(d.new, vec!["    x".to_string()]);
    }

    #[test]
    fn nul_bytes_and_invalid_utf8_are_binary() {
        assert_eq!(text_content(b"a\0b", b"a", false), DiffContent::Binary);
        assert_eq!(
            text_content(b"a", &[0xff, 0xfe], false),
            DiffContent::Binary
        );
    }

    #[test]
    fn huge_files_wait_for_show_anyway() {
        let big = "x\n".repeat(crate::editor::LARGE_FILE_LINES + 1);
        assert_eq!(
            text_content(b"", big.as_bytes(), false),
            DiffContent::TooLarge
        );
        assert!(matches!(
            text_content(b"", big.as_bytes(), true),
            DiffContent::Text(_)
        ));
    }

    #[test]
    fn insertions_at_the_top_and_the_end_and_a_missing_final_newline() {
        assert_eq!(
            rows("a\nb\n", "X\na\nb\n"),
            vec![
                Added { new: 0 },
                Context { old: 0, new: 1 },
                Context { old: 1, new: 2 }
            ]
        );
        assert_eq!(
            rows("a\nb\n", "a\nb\nX\n"),
            vec![
                Context { old: 0, new: 0 },
                Context { old: 1, new: 1 },
                Added { new: 2 }
            ]
        );
        assert_eq!(
            rows("a\nb\n", ""),
            vec![Removed { old: 0 }, Removed { old: 1 }]
        );
        assert_eq!(rows("", ""), Vec::<DiffRow>::new());
        // The last line differs by its missing newline only: still one change.
        assert_eq!(
            rows("a\nb", "a\nb\n"),
            vec![Context { old: 0, new: 0 }, Modified { old: 1, new: 1 }]
        );
    }

    fn numbered(count: usize) -> Vec<String> {
        (0..count).map(|i| format!("l{i}\n")).collect()
    }

    fn with_changes(lines: &[String], changed: &[usize]) -> String {
        lines
            .iter()
            .enumerate()
            .map(|(i, l)| {
                if changed.contains(&i) {
                    format!("X{l}")
                } else {
                    l.clone()
                }
            })
            .collect()
    }

    #[test]
    fn hunks_far_apart_fold_what_lies_between() {
        // 8 unchanged lines (1..=8) between the changes: 3 + fold(2) + 3.
        let old = numbered(10);
        let new = with_changes(&old, &[0, 9]);
        let rows = rows(&old.concat(), &new);
        assert_eq!(
            rows,
            vec![
                Modified { old: 0, new: 0 },
                Context { old: 1, new: 1 },
                Context { old: 2, new: 2 },
                Context { old: 3, new: 3 },
                Fold {
                    old: 4,
                    new: 4,
                    len: 2
                },
                Context { old: 6, new: 6 },
                Context { old: 7, new: 7 },
                Context { old: 8, new: 8 },
                Modified { old: 9, new: 9 },
            ]
        );
    }

    #[test]
    fn hunks_close_together_do_not_fold() {
        // 6 and 7 unchanged lines between the changes: all shown.
        for gap in [6, 7] {
            let old = numbered(gap + 2);
            let new = with_changes(&old, &[0, gap + 1]);
            let rows = rows(&old.concat(), &new);
            assert_eq!(rows.len(), gap + 2, "gap {gap}");
            assert!(rows.iter().all(|r| !matches!(r, Fold { .. })), "gap {gap}");
        }
    }

    #[test]
    fn more_removed_than_added_lines_pair_then_remove() {
        assert_eq!(
            rows("a\nb\nc\nd\n", "a\nB\nd\n"),
            vec![
                Context { old: 0, new: 0 },
                Modified { old: 1, new: 1 },
                Removed { old: 2 },
                Context { old: 3, new: 2 },
            ]
        );
    }

    #[test]
    fn a_tab_and_four_spaces_differ_but_show_the_same_words() {
        let d = TextDiff::new("\tx\n", "    x\n");
        assert_eq!(d.rows, vec![Modified { old: 0, new: 0 }]);
        assert_eq!(d.counts(), (1, 1));
        assert_eq!(d.old, d.new);
        assert_eq!(d.words[&0], WordChanges::default());
    }

    #[test]
    fn word_changes_stop_when_the_budget_is_spent() {
        let old: Vec<String> = vec!["let x = 1;".into(); 2];
        let new: Vec<String> = vec!["let y = 1;".into(); 2];
        let rows = [Modified { old: 0, new: 0 }, Modified { old: 1, new: 1 }];
        // Each pair costs 10 * 10.
        assert_eq!(word_changes(&rows, &old, &new, 1_000).len(), 2);
        let words = word_changes(&rows, &old, &new, 150);
        assert_eq!(words.keys().copied().collect::<Vec<_>>(), vec![0]);
        assert!(word_changes(&rows, &old, &new, 50).is_empty());
    }

    #[test]
    fn lines_too_long_to_compare_do_not_spend_the_budget() {
        let long_old = format!("{}x", "a ".repeat(MAX_LINE));
        let long_new = format!("{}y", "a ".repeat(MAX_LINE));
        let old = vec![long_old, "let x = 1;".to_string()];
        let new = vec![long_new, "let y = 1;".to_string()];
        let rows = [Modified { old: 0, new: 0 }, Modified { old: 1, new: 1 }];
        let words = word_changes(&rows, &old, &new, 150);
        assert_eq!(words.keys().copied().collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn invalid_utf8_is_binary_even_when_huge() {
        let mut big = vec![b'x'; LARGE_FILE_BYTES + 1];
        big[0] = 0xff;
        assert_eq!(text_content(&big, b"a", false), DiffContent::Binary);
    }

    #[test]
    fn without_hunks_different_texts_show_as_replaced() {
        assert_eq!(
            rows_of(Vec::new(), 2, 3, true),
            vec![
                Modified { old: 0, new: 0 },
                Modified { old: 1, new: 1 },
                Added { new: 2 }
            ]
        );
        assert_eq!(
            rows_of(Vec::new(), 2, 1, true),
            vec![Modified { old: 0, new: 0 }, Removed { old: 1 }]
        );
        assert_eq!(
            rows_of(Vec::new(), 1, 1, false),
            vec![Context { old: 0, new: 0 }]
        );
    }

    #[test]
    fn hunks_that_do_not_fit_the_files_show_as_replaced() {
        let bad = vec![Hunk {
            old: 5..6,
            new: 0..1,
        }];
        assert_eq!(
            rows_of(bad, 2, 2, true),
            vec![Modified { old: 0, new: 0 }, Modified { old: 1, new: 1 }]
        );
    }

    #[test]
    fn a_lone_carriage_return_stays_visible() {
        let DiffContent::Text(d) = text_content(b"a\r\r\n", b"a\r\n", false) else {
            panic!("not text");
        };
        assert_eq!(d.old, vec!["a\r".to_string()]);
        assert_eq!(d.new, vec!["a".to_string()]);
        assert_eq!(d.counts(), (1, 1));
    }

    use crate::git::tests::TestRepo;

    /// `main` and a `feature` branch made from it; then `main` moves on.
    /// Returns the repo, `main`'s tip and `feature`'s tip.
    fn pull_repo() -> (TestRepo, String, String) {
        let r = TestRepo::new();
        r.write("a.txt", "one\ntwo\nthree\n");
        r.write("b.txt", "gone\n");
        r.write("r.txt", &"same\n".repeat(10));
        r.commit_all("init");
        r.branch("feature");
        r.switch("feature");
        r.write("a.txt", "one\nTWO\nthree\n");
        r.remove("b.txt");
        r.write("c.txt", "new\n");
        r.remove("r.txt");
        r.write("s.txt", &"same\n".repeat(10));
        let head = r.commit_all("feature");
        r.switch("main");
        r.write("d.txt", "base only\n");
        let base = r.commit_all("main moves on");
        (r, base.to_string(), head.to_string())
    }

    #[test]
    fn diff_pull_compares_against_the_merge_base() {
        let (r, base, head) = pull_repo();
        let files = diff_pull(r.path(), &base, &head).unwrap();
        let paths: Vec<&str> = files.iter().map(FileDiff::path).collect();
        let find = |p: &str| {
            files
                .iter()
                .find(|f| f.path() == p)
                .unwrap_or_else(|| panic!("{p} not in {paths:?}"))
        };
        assert!(!paths.contains(&"d.txt"), "a change of the base is shown");
        assert_eq!(files.len(), 4, "{paths:?}");
        let a = find("a.txt");
        assert_eq!(
            (a.kind, a.additions, a.deletions),
            (FileDiffKind::Modified, 1, 1)
        );
        let b = find("b.txt");
        assert_eq!(
            (b.kind, b.new_path.as_deref(), b.deletions),
            (FileDiffKind::Deleted, None, 1)
        );
        let c = find("c.txt");
        assert_eq!(
            (c.kind, c.old_path.as_deref(), c.additions),
            (FileDiffKind::Added, None, 1)
        );
        let s = find("s.txt");
        assert_eq!(
            (s.kind, s.old_path.as_deref(), s.display_path()),
            (
                FileDiffKind::Renamed,
                Some("r.txt"),
                "r.txt → s.txt".to_string()
            )
        );
    }

    #[test]
    fn binary_files_are_flagged() {
        let r = TestRepo::new();
        r.write("x.bin", "a\0b");
        let base = r.commit_all("init");
        r.write("x.bin", "a\0c");
        let head = r.commit_all("change");
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        assert_eq!(files[0].content, DiffContent::Binary);
        assert_eq!((files[0].additions, files[0].deletions), (0, 0));
    }

    #[test]
    fn unknown_commits_ask_for_a_refresh() {
        let r = TestRepo::new();
        r.write("a", "1");
        let c = r.commit_all("init");
        let err = diff_pull(
            r.path(),
            &c.to_string(),
            "0123456789abcdef0123456789abcdef01234567",
        )
        .unwrap_err();
        assert_eq!(err, CHANGED);
    }

    /// Commit `entries` (path, mode, id) added to the index, on top of
    /// `parent`, without touching the working tree.
    fn commit_entries(
        r: &TestRepo,
        entries: &[(&str, u32, git2::Oid)],
        parent: Option<git2::Oid>,
    ) -> git2::Oid {
        let mut index = r.repo.index().unwrap();
        index.clear().unwrap();
        for &(path, mode, id) in entries {
            index
                .add(&git2::IndexEntry {
                    ctime: git2::IndexTime::new(0, 0),
                    mtime: git2::IndexTime::new(0, 0),
                    dev: 0,
                    ino: 0,
                    mode,
                    uid: 0,
                    gid: 0,
                    file_size: 0,
                    id,
                    flags: 0,
                    flags_extended: 0,
                    path: path.as_bytes().to_vec(),
                })
                .unwrap();
        }
        let tree = index.write_tree().unwrap();
        commit_tree(r, tree, parent)
    }

    fn commit_tree(r: &TestRepo, tree: git2::Oid, parent: Option<git2::Oid>) -> git2::Oid {
        let tree = r.repo.find_tree(tree).unwrap();
        let parent = parent.map(|p| r.repo.find_commit(p).unwrap());
        let sig = crate::git::tests::sig("Test");
        r.repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "commit",
                &tree,
                parent.as_ref().as_slice(),
            )
            .unwrap()
    }

    /// A tree object written as is, without any check: `(mode, name, id)`,
    /// entries sorted by the caller. Modes are in git's text form (`40000`
    /// for a directory).
    fn raw_tree(r: &TestRepo, entries: &[(&str, &[u8], git2::Oid)]) -> git2::Oid {
        let mut data = Vec::new();
        for &(mode, name, id) in entries {
            data.extend_from_slice(mode.as_bytes());
            data.push(b' ');
            data.extend_from_slice(name);
            data.push(0);
            data.extend_from_slice(id.as_bytes());
        }
        r.repo
            .odb()
            .unwrap()
            .write(git2::ObjectType::Tree, &data)
            .unwrap()
    }

    /// The id of a blob that isn't written: enough to compare trees.
    fn fake_blob(content: &str) -> git2::Oid {
        git2::Oid::hash_object(git2::ObjectType::Blob, content.as_bytes()).unwrap()
    }

    #[test]
    fn moving_more_files_than_the_rename_limit_is_still_all_renames() {
        let r = TestRepo::new();
        let count = 1100;
        // Names sorted as git sorts them; the blobs aren't written (so the
        // files are `Unavailable`), only their ids matter here.
        let mut names: Vec<String> = (0..count).map(|i| format!("f{i}.txt")).collect();
        names.sort();
        let blob = |name: &str| fake_blob(&format!("content of {name}\n"));
        let flat: Vec<(&str, &[u8], git2::Oid)> = names
            .iter()
            .map(|n| ("100644", n.as_bytes(), blob(n)))
            .collect();
        let base = commit_tree(&r, raw_tree(&r, &flat), None);
        let moved = raw_tree(&r, &flat);
        let head = commit_tree(&r, raw_tree(&r, &[("40000", b"m", moved)]), Some(base));
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        assert_eq!(files.len(), count);
        for file in &files {
            assert_eq!(file.kind, FileDiffKind::Renamed, "{}", file.display_path());
            let name = file.path().strip_prefix("m/").unwrap();
            assert_eq!(file.old_path.as_deref(), Some(name));
        }
    }

    /// `count` files moved from the root into `m/`, the first `edited` of them
    /// with one line changed. Returns the base and the head.
    fn moved_files(r: &TestRepo, count: usize, edited: usize) -> (String, String) {
        let text = |i: usize, edited: bool| -> String {
            let mut lines: Vec<String> = (0..10).map(|k| format!("file {i} line {k}\n")).collect();
            if edited {
                lines[3] = "edited line\n".to_string();
            }
            lines.concat()
        };
        let mut names: Vec<(usize, String)> =
            (0..count).map(|i| (i, format!("f{i}.txt"))).collect();
        names.sort_by(|a, b| a.1.cmp(&b.1));
        let entries = |edit: bool| -> Vec<(String, git2::Oid)> {
            names
                .iter()
                .map(|(i, name)| {
                    let blob = r
                        .repo
                        .blob(text(*i, edit && *i < edited).as_bytes())
                        .unwrap();
                    (name.clone(), blob)
                })
                .collect()
        };
        let tree = |files: &[(String, git2::Oid)]| {
            let entries: Vec<(&str, &[u8], git2::Oid)> = files
                .iter()
                .map(|(name, id)| ("100644", name.as_bytes(), *id))
                .collect();
            raw_tree(r, &entries)
        };
        let base = commit_tree(r, tree(&entries(false)), None);
        let moved = tree(&entries(true));
        let head = commit_tree(r, raw_tree(r, &[("40000", b"m", moved)]), Some(base));
        (base.to_string(), head.to_string())
    }

    #[test]
    fn edited_moves_are_renames_too() {
        let r = TestRepo::new();
        // 3 of 20 files edited: the 17 others are exact renames, the 3 left
        // are within the limit squared (100) and are matched by similarity.
        let (base, head) = moved_files(&r, 20, 3);
        let files = diff_pull_limited(r.path(), &base, &head, 10).unwrap();
        assert_eq!(files.len(), 20);
        assert!(files.iter().all(|f| f.kind == FileDiffKind::Renamed));
        let edited: Vec<_> = files.iter().filter(|f| f.additions > 0).collect();
        assert_eq!(edited.len(), 3);
        for file in edited {
            assert_eq!((file.additions, file.deletions), (1, 1));
            assert_eq!(file.path().strip_prefix("m/"), file.old_path.as_deref());
        }
    }

    #[test]
    fn too_many_edited_moves_stay_added_and_deleted() {
        let r = TestRepo::new();
        // 4 added x 4 deleted = 16 pairs left, over the limit squared (9).
        let (base, head) = moved_files(&r, 8, 4);
        let files = diff_pull_limited(r.path(), &base, &head, 3).unwrap();
        let count = |kind| files.iter().filter(|f| f.kind == kind).count();
        assert_eq!(count(FileDiffKind::Renamed), 4);
        assert_eq!(count(FileDiffKind::Added), 4);
        assert_eq!(count(FileDiffKind::Deleted), 4);
    }

    fn assert_each_path_once(files: &[FileDiff]) {
        let mut paths: Vec<&str> = files.iter().map(FileDiff::path).collect();
        paths.sort();
        let before = paths.len();
        paths.dedup();
        assert_eq!(paths.len(), before, "a path appears twice: {files:#?}");
    }

    #[test]
    fn a_symlink_replaced_by_its_targets_content_has_each_path_once() {
        let r = TestRepo::new();
        let content = r.repo.blob(b"content\n").unwrap();
        let link = r.repo.blob(b"Y").unwrap();
        let base = commit_entries(&r, &[("X", 0o120000, link), ("Y", 0o100644, content)], None);
        let head = commit_entries(&r, &[("X", 0o100644, content)], Some(base));
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        assert_each_path_once(&files);
        let kinds: Vec<_> = files.iter().map(|f| (f.path(), f.kind)).collect();
        assert_eq!(
            kinds,
            vec![("X", FileDiffKind::Modified), ("Y", FileDiffKind::Deleted)]
        );
    }

    #[test]
    fn a_file_moved_with_a_symlink_left_behind_has_each_path_once() {
        let r = TestRepo::new();
        let content = r.repo.blob(b"content\n").unwrap();
        let link = r.repo.blob(b"Z").unwrap();
        let base = commit_entries(&r, &[("X", 0o100644, content)], None);
        let head = commit_entries(
            &r,
            &[("X", 0o120000, link), ("Z", 0o100644, content)],
            Some(base),
        );
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        assert_each_path_once(&files);
        let kinds: Vec<_> = files.iter().map(|f| (f.path(), f.kind)).collect();
        assert_eq!(
            kinds,
            vec![("X", FileDiffKind::Modified), ("Z", FileDiffKind::Added)]
        );
    }

    #[test]
    fn a_submodule_in_a_big_directory_is_found() {
        let r = TestRepo::new();
        let (first, second) = (fake_blob("1\n"), fake_blob("2\n"));
        let names: Vec<String> = (0..300).map(|i| format!("f{i:04}")).collect();
        let tree = |commit: git2::Oid| {
            let mut entries: Vec<(&str, &[u8], git2::Oid)> = names
                .iter()
                .map(|n| ("100644", n.as_bytes(), fake_blob("x\n")))
                .collect();
            entries.push(("160000", b"sub", commit));
            raw_tree(&r, &[("40000", b"dir", raw_tree(&r, &entries))])
        };
        let base = commit_tree(&r, tree(first), None);
        let head = commit_tree(&r, tree(second), Some(base));
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path(), "dir/sub");
        assert_eq!(files[0].version(), Some(second));
    }

    #[test]
    fn a_submodule_with_an_odd_name_is_still_a_submodule() {
        let (first, second) = (fake_blob("1\n"), fake_blob("2\n"));
        // Not UTF-8, and with a backslash.
        for name in [&b"a\xffb"[..], &b"a\\b"[..]] {
            let r = TestRepo::new();
            let base = commit_tree(&r, raw_tree(&r, &[("160000", name, first)]), None);
            let head = commit_tree(&r, raw_tree(&r, &[("160000", name, second)]), Some(base));
            let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].new_blob, None, "{name:?} is a gitlink");
            assert_eq!(files[0].version(), Some(second));
            assert_eq!(files[0].path(), String::from_utf8_lossy(name));
        }
    }

    #[test]
    fn a_path_that_is_not_utf8_does_not_panic() {
        let r = TestRepo::new();
        let blob = r.repo.blob(b"x\n").unwrap();
        let base = commit_tree(&r, raw_tree(&r, &[("100644", b"keep", blob)]), None);
        let head = commit_tree(
            &r,
            raw_tree(&r, &[("100644", b"keep", blob), ("100644", b"x\xff", blob)]),
            Some(base),
        );
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path(), "x\u{fffd}");
    }

    #[test]
    fn a_renamed_file_can_be_modified_too() {
        let r = TestRepo::new();
        let text: String = (0..20).map(|i| format!("line {i}\n")).collect();
        r.write("old.txt", &text);
        let base = r.commit_all("init");
        r.remove("old.txt");
        r.write("new.txt", &text.replace("line 7\n", "changed\n"));
        let head = r.commit_all("rename");
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.kind, FileDiffKind::Renamed);
        assert_eq!((f.additions, f.deletions), (1, 1));
        assert_eq!(f.display_path(), "old.txt → new.txt");
    }

    #[test]
    fn a_pure_rename_is_one_fold() {
        let r = TestRepo::new();
        r.write("a.txt", "1\n2\n3\n");
        let base = r.commit_all("init");
        r.remove("a.txt");
        r.write("b.txt", "1\n2\n3\n");
        let head = r.commit_all("rename");
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        let DiffContent::Text(text) = &files[0].content else {
            panic!("not text");
        };
        assert_eq!(files[0].old_blob, files[0].new_blob);
        assert_eq!(
            text.rows,
            vec![DiffRow::Fold {
                old: 0,
                new: 0,
                len: 3
            }]
        );
    }

    #[test]
    fn a_type_change_is_one_modified_file() {
        let r = TestRepo::new();
        let commit = git2::Oid::from_str("1111111111111111111111111111111111111111").unwrap();
        let blob = r.repo.blob(b"now a file\n").unwrap();
        let base = commit_entries(&r, &[("sub", 0o160000, commit)], None);
        let head = commit_entries(&r, &[("sub", 0o100644, blob)], Some(base));
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        assert_eq!(files.len(), 1, "{files:?}");
        let f = &files[0];
        assert_eq!((f.path(), f.kind), ("sub", FileDiffKind::Modified));
        assert_eq!(
            (f.old_blob, f.new_blob, f.new_commit),
            (None, Some(blob), None)
        );
        let DiffContent::Text(text) = &f.content else {
            panic!("not text: {:?}", f.content);
        };
        assert_eq!(text.old, vec![format!("Subproject commit {commit}")]);
        assert_eq!(text.new, vec!["now a file".to_string()]);
    }

    #[test]
    fn a_missing_blob_only_makes_its_file_unavailable() {
        let r = TestRepo::new();
        r.write("keep.txt", "one\n");
        r.write("gone.txt", "old content\n");
        let base = r.commit_all("init");
        r.write("keep.txt", "two\n");
        r.remove("gone.txt");
        r.write("lost.txt", "brand new content\n");
        let head = r.commit_all("change");
        let lost = r.repo.blob(b"brand new content\n").unwrap().to_string();
        let object = r
            .repo
            .path()
            .join("objects")
            .join(&lost[..2])
            .join(&lost[2..]);
        let mut permissions = std::fs::metadata(&object).unwrap().permissions();
        // Windows only: the object is read-only there.
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&object, permissions).unwrap();
        std::fs::remove_file(&object).unwrap();
        // Reopened by `diff_pull`: no cached object.
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        let find = |p: &str| files.iter().find(|f| f.path() == p).unwrap();
        assert!(matches!(
            find("lost.txt").content,
            DiffContent::Unavailable(_)
        ));
        assert_eq!(find("lost.txt").additions, 0);
        assert!(matches!(find("keep.txt").content, DiffContent::Text(_)));
        assert!(matches!(find("gone.txt").content, DiffContent::Text(_)));
    }

    #[test]
    fn a_backslash_is_part_of_a_path() {
        let r = TestRepo::new();
        let blob = r.repo.blob(b"x\n").unwrap();
        let base = commit_entries(&r, &[("keep", 0o100644, blob)], None);
        let mut tree = r.repo.treebuilder(None).unwrap();
        tree.insert("keep", blob, 0o100644).unwrap();
        tree.insert("a\\b", blob, 0o100644).unwrap();
        let head = commit_tree(&r, tree.write().unwrap(), Some(base));
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path(), "a\\b");
    }

    #[test]
    fn a_non_canonical_mode_does_not_panic() {
        let r = TestRepo::new();
        let blob = r.repo.blob(b"x\n").unwrap();
        let other = r.repo.blob(b"y\n").unwrap();
        // `100775` can't be inserted through the tree builder: write the tree object.
        let raw = |id: git2::Oid| raw_tree(&r, &[("100775", b"f", id)]);
        let base = commit_tree(&r, raw(blob), None);
        let head = commit_tree(&r, raw(other), Some(base));
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(matches!(files[0].content, DiffContent::Text(_)));
    }

    #[test]
    fn a_changed_submodule_is_a_one_line_change() {
        let r = TestRepo::new();
        let (first, second) = (
            git2::Oid::from_str("1111111111111111111111111111111111111111").unwrap(),
            git2::Oid::from_str("2222222222222222222222222222222222222222").unwrap(),
        );
        let base = commit_entries(&r, &[("sub", 0o160000, first)], None);
        let head = commit_entries(&r, &[("sub", 0o160000, second)], Some(base));
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        assert_eq!(files.len(), 1);
        let file = &files[0];
        assert_eq!((file.path(), file.kind), ("sub", FileDiffKind::Modified));
        assert_eq!((file.old_blob, file.new_blob), (None, None));
        assert_eq!(
            file.version(),
            Some(second),
            "a new commit is a new version"
        );
        let DiffContent::Text(text) = &file.content else {
            panic!("not text: {:?}", file.content);
        };
        assert_eq!(text.rows, vec![DiffRow::Modified { old: 0, new: 0 }]);
        assert_eq!(text.old, vec![format!("Subproject commit {first}")]);
        assert_eq!(text.new, vec![format!("Subproject commit {second}")]);
    }

    #[test]
    fn load_text_ignores_the_size_limit() {
        let r = TestRepo::new();
        r.write("a", "1\n");
        let base = r.commit_all("init");
        r.write("a", &"x\n".repeat(crate::editor::LARGE_FILE_LINES + 1));
        let head = r.commit_all("big");
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        assert_eq!(files[0].content, DiffContent::TooLarge);
        let content = load_text(r.path(), files[0].old_blob, files[0].new_blob).unwrap();
        assert!(matches!(content, DiffContent::Text(_)));
    }

    #[test]
    fn fetch_pull_brings_the_pull_ref_and_the_base() {
        let r = TestRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        let (_bare, url) = crate::git::tests::bare_origin();
        r.repo.remote("origin", &url).unwrap();
        // Someone's pull request: a commit only GitHub's `refs/pull/7/head` points at.
        r.branch("feature");
        r.switch("feature");
        r.write("a.txt", "2\n");
        let pr_tip = r.commit_all("change");
        let push = |refspecs: &[&str]| {
            r.repo
                .find_remote("origin")
                .unwrap()
                .push(refspecs, None)
                .unwrap()
        };
        push(&[
            "refs/heads/main:refs/heads/main",
            "refs/heads/feature:refs/pull/7/head",
        ]);
        let dir = tempfile::tempdir().unwrap();
        let clone = git2::Repository::clone(&url, dir.path()).unwrap();
        assert!(clone.find_reference("refs/remotes/origin/pr/7").is_err());
        let fetch_head_before = std::fs::read(clone.path().join("FETCH_HEAD")).ok();
        // The base moves on after the clone: only the fetch brings it.
        r.switch("main");
        r.write("b.txt", "base\n");
        let new_base = r.commit_all("base moves on");
        push(&["refs/heads/main:refs/heads/main"]);

        fetch_pull(dir.path(), 7, "main").unwrap();

        let pr = clone.find_reference("refs/remotes/origin/pr/7").unwrap();
        assert_eq!(pr.target(), Some(pr_tip));
        let base = clone.find_reference("refs/remotes/origin/main").unwrap();
        assert_eq!(base.target(), Some(new_base));
        let fetch_head = || std::fs::read(clone.path().join("FETCH_HEAD")).ok();
        assert_eq!(fetch_head(), fetch_head_before, "FETCH_HEAD is left alone");
    }

    #[test]
    fn fetch_pull_needs_an_origin_and_a_published_pull_ref() {
        let r = TestRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        assert!(fetch_pull(r.path(), 7, "main").is_err());
        let (_bare, url) = crate::git::tests::bare_origin();
        r.repo.remote("origin", &url).unwrap();
        r.repo
            .find_remote("origin")
            .unwrap()
            .push(&["refs/heads/main:refs/heads/main"], None)
            .unwrap();
        // The origin has the base but no `refs/pull/7/head`: libgit2 fetches
        // nothing without an error, so `fetch_pull` reports it.
        let err = fetch_pull(r.path(), 7, "main").unwrap_err();
        assert!(err.contains("refs/pull/7/head"), "{err}");
        assert!(r.repo.find_reference("refs/remotes/origin/pr/7").is_err());
    }

    #[test]
    fn branch_name_is_the_head_branch_or_the_number_for_forks() {
        assert_eq!(pull_branch_name(58, "feat/x", false), "feat/x");
        assert_eq!(pull_branch_name(58, "main", true), "pr/58");
        assert_eq!(pull_merge_ref(58, "feat/x", false), "refs/heads/feat/x");
        assert_eq!(pull_merge_ref(58, "main", true), "refs/pull/58/head");
    }

    /// A repo on `main` with a `work` branch one commit ahead; returns `work`'s tip.
    fn repo_with_work() -> (TestRepo, git2::Oid) {
        let r = TestRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        r.branch("work");
        r.switch("work");
        r.write("a.txt", "2\n");
        let head = r.commit_all("pr");
        r.switch("main");
        (r, head)
    }

    fn config_value(r: &TestRepo, key: &str) -> Option<String> {
        r.repo.config().unwrap().get_string(key).ok()
    }

    #[test]
    fn checkout_pull_creates_the_branch_at_the_head() {
        let (r, head) = repo_with_work();
        let mut s = r.status();
        let merge = "refs/heads/pr/7";
        assert_eq!(s.checkout_pull("pr/7", &head.to_string(), merge), Ok(false));
        assert_eq!(r.head_name(), "pr/7");
        assert_eq!(r.head_oid(), head);
        // `core.autocrlf` may make the checkout write CRLF.
        assert_eq!(r.read("a.txt").replace("\r\n", "\n"), "2\n");
    }

    #[test]
    fn a_created_branch_tracks_the_pull_ref_and_an_existing_one_is_left_alone() {
        let (r, head) = repo_with_work();
        let mut s = r.status();
        let merge = "refs/pull/7/head";
        assert_eq!(s.checkout_pull("pr/7", &head.to_string(), merge), Ok(false));
        assert_eq!(
            config_value(&r, "branch.pr/7.remote").as_deref(),
            Some("origin")
        );
        assert_eq!(
            config_value(&r, "branch.pr/7.merge").as_deref(),
            Some(merge)
        );
        // An existing branch keeps whatever it was configured with.
        s.checkout_pull("main", &head.to_string(), "refs/heads/other")
            .unwrap();
        assert_eq!(config_value(&r, "branch.main.merge"), None);
        assert_eq!(config_value(&r, "branch.main.remote"), None);
    }

    #[test]
    fn checkout_pull_keeps_an_existing_branch_as_is() {
        let r = TestRepo::new();
        r.write("a.txt", "1\n");
        let init = r.commit_all("init");
        r.branch("feat");
        r.write("a.txt", "2\n");
        let newer = r.commit_all("newer on main");
        let mut s = r.status();
        let result = s.checkout_pull("feat", &newer.to_string(), "refs/heads/feat");
        assert_eq!(result, Ok(true));
        assert_eq!((r.head_name(), r.head_oid()), ("feat".to_string(), init));
    }

    #[test]
    fn a_tag_named_like_the_branch_does_not_detach_head() {
        let (r, head) = repo_with_work();
        let main_tip = r.head_oid();
        let target = r.repo.find_object(main_tip, None).unwrap();
        r.repo.tag_lightweight("feat", &target, false).unwrap();
        let mut s = r.status();
        let result = s.checkout_pull("feat", &head.to_string(), "refs/heads/feat");
        assert_eq!(result, Ok(false));
        assert!(!r.repo.head_detached().unwrap());
        assert_eq!((r.head_name(), r.head_oid()), ("feat".to_string(), head));
    }

    #[test]
    fn a_refused_checkout_leaves_no_branch_behind() {
        let (r, head) = repo_with_work();
        r.write("a.txt", "local edit\n");
        let mut s = r.status();
        let result = s.checkout_pull("pr/7", &head.to_string(), "refs/heads/pr/7");
        assert!(result.is_err());
        assert!(r.repo.find_branch("pr/7", git2::BranchType::Local).is_err());
        assert_eq!(config_value(&r, "branch.pr/7.remote"), None);
        assert_eq!(r.head_name(), "main");
        assert_eq!(r.read("a.txt"), "local edit\n");
    }

    #[test]
    fn a_refused_checkout_keeps_an_existing_branch() {
        let (r, head) = repo_with_work();
        r.write("a.txt", "local edit\n");
        let mut s = r.status();
        let result = s.checkout_pull("work", &head.to_string(), "refs/heads/work");
        assert!(result.is_err());
        let work = r.repo.find_branch("work", git2::BranchType::Local).unwrap();
        assert_eq!(work.get().target(), Some(head));
        assert_eq!(r.head_name(), "main");
    }

    #[test]
    fn an_invalid_branch_name_creates_nothing() {
        let (r, head) = repo_with_work();
        let mut s = r.status();
        let result = s.checkout_pull("bad..name", &head.to_string(), "refs/heads/x");
        assert_eq!(
            result,
            Err("'bad..name' is not a valid branch name".to_string())
        );
        assert_eq!(r.head_name(), "main");
    }

    #[test]
    fn checkout_pull_is_refused_during_a_merge() {
        let (r, head) = repo_with_work();
        let main_tip = r.head_oid();
        std::fs::write(r.repo.path().join("MERGE_HEAD"), format!("{head}\n")).unwrap();
        let mut s = r.status();
        let result = s.checkout_pull("pr/7", &head.to_string(), "refs/heads/pr/7");
        assert_eq!(
            result,
            Err("Cannot check out: another operation (merge, rebase, revert, bisect…) is in progress. Finish or abort it first.".to_string())
        );
        assert!(r.repo.find_branch("pr/7", git2::BranchType::Local).is_err());
        assert_eq!(
            (r.head_name(), r.head_oid()),
            ("main".to_string(), main_tip)
        );
    }
}
