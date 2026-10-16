//! Side-by-side diff of a pull request's files: the base on the left, the
//! pull request on the right, review threads under the lines they are
//! about. Only the items on screen are laid out (see [`DiffLayout`]).

use super::diff_layout::{DiffLayout, Item, Placement, HEADER_HEIGHT};
use super::theme::Palette;
use crate::editor::highlight::{Highlighter, Token, TokenKind};
use crate::editor::{MAX_TOKENIZED_LINE_CHARS, VIEWPORT_THRESHOLD};
use crate::git::compare::{DiffContent, DiffRow, FileDiff, FileDiffKind, TextDiff};
use crate::github::{ReviewThread, Side};
use crate::plugin::manager::PluginManager;
use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke, TextFormat, Ui,
    UiBuilder,
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use egui_phosphor::regular;
use std::collections::HashMap;
use std::hash::Hash;
use std::ops::Range;

/// Minimum width of each side's line-number gutter.
const GUTTER: f32 = 44.0;
/// Width of the `+` / `−` marker after the gutter.
const MARKER: f32 = 14.0;
/// How long a change reached with F7 stays outlined, in seconds.
const FLASH_SECONDS: f64 = 0.8;
/// Room a thread may take while it is measured.
const THREAD_MAX_HEIGHT: f32 = 4000.0;
/// How many times a scroll is repeated because threads measured above its
/// target moved the target.
const MAX_SCROLL_RETRIES: u8 = 3;
/// Longest text shown in the tooltip of a clipped line, in chars.
const HOVER_CHARS: usize = 2000;

/// Something the user did in the view, for the pull request tab to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffEvent {
    ToggleViewed(usize),
    /// The chevron of a file header: collapse or expand it, whatever its
    /// "viewed" mark.
    ToggleCollapsed(usize),
    Unfold {
        file: usize,
        row: usize,
    },
    ShowAnyway(usize),
    /// Double-click on 0-based line `line` of the pull request's version of `file`.
    OpenLine {
        file: usize,
        line: usize,
    },
}

/// What the view shows; owned by the pull request tab.
pub struct DiffInput<'a> {
    pub files: &'a [FileDiff],
    pub threads: &'a [ReviewThread],
    /// `viewed[f]`: file `f` is marked viewed (the header's checkbox).
    pub viewed: &'a [bool],
    /// `collapsed[f]`: file `f` is folded to its header (the layout).
    pub collapsed: &'a [bool],
    pub palette: Palette,
    pub plugins: Option<&'a PluginManager>,
}

/// Syntax tokens of one side of a file, by line. Documents of up to
/// [`VIEWPORT_THRESHOLD`] lines are tokenized whole, like the editor does;
/// longer ones line by line, when a line is first drawn. Lines over
/// [`MAX_TOKENIZED_LINE_CHARS`] never go through the plugins.
struct SideTokens {
    highlighter: Highlighter,
    lines: Vec<Option<Vec<Token>>>,
}

impl SideTokens {
    fn new(lines: &[String], filename: &str, plugins: Option<&PluginManager>) -> Self {
        let mut highlighter = Highlighter::new();
        highlighter.set_language_from_filename(filename);
        if lines.len() > VIEWPORT_THRESHOLD {
            return Self {
                highlighter,
                lines: std::iter::repeat_with(|| None).take(lines.len()).collect(),
            };
        }
        let tokenizer = if lines.iter().any(|l| too_long_to_tokenize(l)) {
            None
        } else {
            plugins
        };
        highlighter.highlight_document(&lines.join("\n"), 0, tokenizer);
        let lines = lines
            .iter()
            .enumerate()
            .map(|(i, line)| Some(highlighter.tokens_for_line(i, line, tokenizer)))
            .collect();
        Self { highlighter, lines }
    }

    /// Tokenize line `index` (`text`) unless it already is.
    fn ensure(&mut self, index: usize, text: &str, plugins: Option<&PluginManager>) {
        let Some(slot) = self.lines.get_mut(index) else {
            return;
        };
        if slot.is_none() {
            let tokenizer = if too_long_to_tokenize(text) {
                None
            } else {
                plugins
            };
            *slot = Some(self.highlighter.tokens_for_line(index, text, tokenizer));
        }
    }

    fn get(&self, index: usize) -> &[Token] {
        self.lines
            .get(index)
            .and_then(|l| l.as_deref())
            .unwrap_or(&[])
    }
}

fn too_long_to_tokenize(line: &str) -> bool {
    line.len() > MAX_TOKENIZED_LINE_CHARS && line.chars().count() > MAX_TOKENIZED_LINE_CHARS
}

type TokenCache = HashMap<(usize, Side), SideTokens>;

/// A change-navigation key press waiting for the next frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Nav {
    Next,
    Prev,
}

#[derive(Default)]
pub struct DiffView {
    layout: Option<DiffLayout>,
    /// Measured height of each thread (by index), kept across rebuilds.
    thread_heights: HashMap<usize, f32>,
    /// Computed the first time a file is drawn.
    tokens: TokenCache,
    /// Item to bring into view on the next frame; `true` puts it at the very
    /// top of the viewport, `false` leaves a lead above it.
    scroll_to: Option<(usize, bool)>,
    /// Times `scroll_to` was repeated since it was asked for.
    scroll_retries: u8,
    /// File whose header is brought into view once the layout exists (`true`:
    /// at the very top).
    anchor: Option<(usize, bool)>,
    /// Item outlined after F7, and when the outline started.
    flash: Option<(usize, Option<f64>)>,
    /// F7 stop the reader is on (index into the layout's change starts).
    current: Option<usize>,
    /// Top and bottom of the last viewport, in content coordinates.
    viewport: Option<(f32, f32)>,
    /// Scroll offset at the end of the last frame (wheel and drags included).
    offset: f32,
    /// File of the first item under the top of the last viewport.
    top_file: Option<usize>,
    /// F7 or Shift+F7 waiting for the layout.
    pending: Option<Nav>,
    /// Height gained by threads measured above the viewport, to be added to
    /// the scroll offset so the reader's place doesn't move.
    scroll_shift: f32,
}

/// What a frame learned while painting.
struct Painted {
    viewport: (f32, f32),
    /// Height gained above the viewport by measured threads.
    shift: f32,
    /// An item above the scroll target changed height.
    resized_above: bool,
    top_file: Option<usize>,
}

impl DiffView {
    /// Rebuild the layout on the next frame, keeping the F7 position (folds
    /// changed: a fold never holds a change, so the stops keep their order).
    pub fn invalidate(&mut self) {
        self.layout = None;
        self.flash = None;
    }

    /// Rebuild the layout on the next frame and forget the F7 position
    /// ("viewed" or "show anyway" added or removed stops).
    pub fn invalidate_stops(&mut self) {
        self.invalidate();
        self.current = None;
    }

    /// Forget everything computed from the files (new data loaded).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn layout(&self) -> Option<&DiffLayout> {
        self.layout.as_ref()
    }

    /// F7 stop the reader is on, if any.
    pub fn current_change(&self) -> Option<usize> {
        self.current
    }

    /// Put the header of `file` at the top of the viewport (on the next
    /// frame, once the layout exists).
    pub fn scroll_to_file(&mut self, file: usize) {
        self.anchor = Some((file, true));
    }

    /// Top and bottom of the last viewport, in content coordinates.
    pub fn viewport(&self) -> Option<(f32, f32)> {
        self.viewport
    }

    /// File being read: the one under the top of the last viewport.
    pub fn top_file(&self) -> Option<usize> {
        self.top_file
    }

    /// After `file` was collapsed or expanded: once the layout is rebuilt,
    /// put its header at the top of the viewport so the reader stays on it.
    pub fn collapse_anchor(&mut self, file: usize) {
        self.anchor = Some((file, true));
    }

    /// F7: go to the next change on the next frame.
    pub fn request_next(&mut self) {
        self.pending = Some(Nav::Next);
    }

    /// Shift+F7: go to the previous change on the next frame.
    pub fn request_prev(&mut self) {
        self.pending = Some(Nav::Prev);
    }

    /// Bring F7 stop `index` into view and outline it.
    fn go_to_change(&mut self, index: usize) {
        let Some(&item) = self
            .layout
            .as_ref()
            .and_then(|l| l.change_starts().get(index))
        else {
            return;
        };
        self.current = Some(index);
        self.scroll_to = Some((item, false));
        self.scroll_retries = 0;
        self.flash = Some((item, None));
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        input: &DiffInput<'_>,
        md_cache: &mut CommonMarkCache,
    ) -> Vec<DiffEvent> {
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let (row_height, char_width) =
            ui.fonts(|f| (f.row_height(&font) + 4.0, f.glyph_width(&font, '0')));
        let layout = self.layout.get_or_insert_with(|| {
            DiffLayout::build(
                input.files,
                input.threads,
                input.collapsed,
                row_height,
                &self.thread_heights,
            )
        });
        if let Some((file, at_top)) = self.anchor.take() {
            if let Some(item) = layout.header_item(file) {
                self.scroll_to = Some((item, at_top));
                self.scroll_retries = 0;
            }
        }
        if let Some(nav) = self.pending.take() {
            if let Some(stop) = next_stop(layout, self.current, self.viewport, nav) {
                self.go_to_change(stop);
            }
        }
        let Self {
            layout,
            thread_heights,
            tokens,
            scroll_to,
            flash,
            scroll_shift,
            offset,
            ..
        } = self;
        let Some(layout) = layout.as_mut() else {
            return Vec::new();
        };
        let metrics = Metrics {
            p: &input.palette,
            font: &font,
            char_width,
        };
        let now = ui.input(|i| i.time);
        let mut events = Vec::new();
        let mut area = egui::ScrollArea::vertical()
            .id_salt("pr-diff")
            .auto_shrink([false, false]);
        let applied = scroll_to.take().filter(|&(i, _)| i < layout.items().len());
        if let Some((item, at_top)) = applied {
            let lead = if at_top {
                0.0
            } else {
                ui.available_height() / 3.0
            };
            area = area.vertical_scroll_offset((layout.top(item) - lead).max(0.0));
            *scroll_shift = 0.0;
        } else if *scroll_shift != 0.0 {
            area = area.vertical_scroll_offset((*offset + *scroll_shift).max(0.0));
            *scroll_shift = 0.0;
        }
        let target = applied.map(|(item, _)| item);
        let output = area.show_viewport(ui, |ui, viewport| {
            ui.set_height(layout.total_height());
            let origin = ui.min_rect().min;
            let width = ui.max_rect().width();
            let visible = layout.visible(viewport.min.y, viewport.max.y);
            let mut painted = Painted {
                viewport: (viewport.min.y, viewport.max.y),
                shift: 0.0,
                resized_above: false,
                top_file: visible
                    .clone()
                    .next()
                    .and_then(|first| layout.items()[first].file()),
            };
            for i in visible.clone() {
                let rect = Rect::from_min_size(
                    origin + egui::vec2(0.0, layout.top(i)),
                    egui::vec2(width, layout.height(i)),
                );
                let item = layout.items()[i];
                let thread_cell = match item {
                    Item::Thread {
                        thread, placement, ..
                    } => {
                        let t = &input.threads[thread];
                        let (full_width, label) = match placement {
                            Placement::Line => (false, None),
                            Placement::File => (true, t.line.map(|l| format!("Line {l}"))),
                            Placement::Outdated => (true, Some(outdated_label(t))),
                        };
                        Some((thread, full_width, label))
                    }
                    Item::OrphanThread { thread } => {
                        let label = format!(
                            "On {}, which this diff doesn't show",
                            input.threads[thread].path
                        );
                        Some((thread, true, Some(label)))
                    }
                    _ => None,
                };
                if let Some((thread, full_width, label)) = thread_cell {
                    let cell = ThreadCell {
                        top_left: rect.min,
                        width,
                        full_width,
                        label,
                    };
                    let height = paint_thread(
                        ui,
                        &cell,
                        &input.threads[thread],
                        &input.palette,
                        md_cache,
                        ("diff-thread", i),
                    );
                    let before = layout.height(i);
                    thread_heights.insert(thread, height);
                    if layout.set_height(i, height) {
                        ui.ctx().request_repaint();
                        if layout.top(i) < viewport.min.y {
                            painted.shift += layout.height(i) - before;
                        }
                        if target.is_some_and(|t| i < t) {
                            painted.resized_above = true;
                        }
                    }
                    continue;
                }
                let event = match item {
                    Item::FileHeader { file } => header(ui, rect, file, input, ("diff-header", i)),
                    Item::Notice { file } => notice(
                        ui,
                        rect,
                        file,
                        &input.files[file],
                        &input.palette,
                        ("diff-notice", i),
                    ),
                    Item::OrphanTitle => {
                        group_title(
                            ui,
                            rect,
                            "Comments on files not in this diff",
                            &input.palette,
                        );
                        None
                    }
                    Item::FileCommentsTitle { .. } => {
                        group_title(ui, rect, "Comments on the file", &input.palette);
                        None
                    }
                    Item::OutdatedTitle { .. } => {
                        group_title(ui, rect, "Outdated comments", &input.palette);
                        None
                    }
                    Item::Row { file, row: index } => {
                        let f = &input.files[file];
                        let DiffContent::Text(text) = &f.content else {
                            continue;
                        };
                        ensure_tokens(tokens, file, f, text, index, input.plugins);
                        let flashing = match flash {
                            Some((at, started)) if *at == i => {
                                now - *started.get_or_insert(now) < FLASH_SECONDS
                            }
                            _ => false,
                        };
                        let side_tokens =
                            [&tokens[&(file, Side::Left)], &tokens[&(file, Side::Right)]];
                        let id = ui.id().with(("diff-row", i));
                        let cell = RowCell {
                            rect,
                            file,
                            index,
                            text,
                            flashing,
                            id,
                        };
                        row(ui, &cell, side_tokens, &metrics)
                    }
                    Item::Thread { .. } | Item::OrphanThread { .. } => None,
                };
                events.extend(event);
            }
            // The header of the file being read stays at the top, pushed up
            // by the header of the next file.
            if let Some(first) = visible.clone().next() {
                let item = layout.items()[first];
                if let Some(file) = item.file() {
                    if !matches!(item, Item::FileHeader { .. }) {
                        let next_header = visible
                            .clone()
                            .skip(1)
                            .find(|&i| {
                                matches!(
                                    layout.items()[i],
                                    Item::FileHeader { .. } | Item::OrphanTitle
                                )
                            })
                            .map(|i| layout.top(i));
                        let y = sticky_top(viewport.min.y, next_header);
                        let rect = Rect::from_min_size(
                            origin + egui::vec2(0.0, y),
                            egui::vec2(width, HEADER_HEIGHT),
                        );
                        events.extend(header(ui, rect, file, input, "diff-sticky"));
                    }
                }
            }
            painted
        });
        let painted = output.inner;
        self.viewport = Some(painted.viewport);
        self.top_file = painted.top_file;
        self.offset = output.state.offset.y;
        self.scroll_shift += painted.shift;
        if let Some(again) = applied {
            if painted.resized_above && self.scroll_retries < MAX_SCROLL_RETRIES {
                self.scroll_retries += 1;
                self.scroll_to = Some(again);
                ui.ctx().request_repaint();
            } else {
                self.scroll_retries = 0;
            }
        }
        let expired = matches!(self.flash, Some((_, Some(t0))) if now - t0 >= FLASH_SECONDS);
        if expired {
            self.flash = None;
        } else if self.flash.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        events
    }
}

/// Top of the sticky header: the viewport top, unless the next file's header
/// is closer than a header's height.
fn sticky_top(viewport_top: f32, next_header: Option<f32>) -> f32 {
    next_header.map_or(viewport_top, |next| viewport_top.min(next - HEADER_HEIGHT))
}

/// The F7 stop to go to. A cursor still inside the last viewport steps from
/// itself; one that scrolled out of sight, or none, searches from the top of
/// the viewport (the start before the first frame).
fn next_stop(
    layout: &DiffLayout,
    current: Option<usize>,
    viewport: Option<(f32, f32)>,
    nav: Nav,
) -> Option<usize> {
    let cursor = current.filter(|&c| {
        let (Some(&item), Some((top, bottom))) = (layout.change_starts().get(c), viewport) else {
            return false;
        };
        (top..bottom).contains(&layout.top(item))
    });
    let from = viewport.map_or(0.0, |(top, _)| top);
    match nav {
        Nav::Next => layout.next_change(cursor, from),
        Nav::Prev => layout.prev_change(cursor, from),
    }
}

/// Letter and colour of a file's change, like the Git panel.
pub fn kind_icon(kind: FileDiffKind, p: &Palette) -> (&'static str, Color32) {
    match kind {
        FileDiffKind::Added => ("A", p.git_added),
        FileDiffKind::Modified => ("M", p.git_modified),
        FileDiffKind::Deleted => ("D", p.git_removed),
        FileDiffKind::Renamed => ("R", p.info),
    }
}

fn header(
    ui: &mut Ui,
    rect: Rect,
    file: usize,
    input: &DiffInput<'_>,
    salt: impl Hash,
) -> Option<DiffEvent> {
    let p = &input.palette;
    let f = &input.files[file];
    // Registered before the contents: the header takes the clicks that would
    // otherwise reach the rows under it (the sticky one covers rows), while
    // its own widgets stay on top.
    ui.interact(rect, ui.id().with((&salt, "bar")), Sense::CLICK);
    ui.painter().rect_filled(rect, 0.0, p.surface_raised);
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0_f32, p.border),
    );
    let inner = rect.shrink2(egui::vec2(8.0, 2.0));
    // Counts on the right, so a long path is cut instead of pushing them away.
    let mut right = ui.new_child(
        UiBuilder::new()
            .id_salt((&salt, "counts"))
            .max_rect(inner)
            .layout(Layout::right_to_left(Align::Center)),
    );
    let threads = input
        .threads
        .iter()
        .filter(|t| t.path == f.path() || f.old_path.as_deref() == Some(t.path.as_str()))
        .count();
    if threads > 0 {
        right.label(
            RichText::new(format!("{} {threads}", regular::CHAT_CIRCLE))
                .small()
                .color(p.text_muted),
        );
    }
    match &f.content {
        DiffContent::Text(_) => {
            right.label(
                RichText::new(format!("−{}", f.deletions))
                    .small()
                    .color(p.git_removed),
            );
            right.label(
                RichText::new(format!("+{}", f.additions))
                    .small()
                    .color(p.git_added),
            );
        }
        DiffContent::Binary => {
            right.label(RichText::new("binary").small().color(p.text_muted));
        }
        DiffContent::TooLarge => {
            right.label(RichText::new("large file").small().color(p.text_muted));
        }
        DiffContent::Unavailable(_) => {
            right.label(RichText::new("unavailable").small().color(p.warning));
        }
    }
    let left_edge = (right.min_rect().left() - 8.0).max(inner.left());
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt((&salt, "name"))
            .max_rect(Rect::from_min_max(
                inner.min,
                Pos2::new(left_edge, inner.max.y),
            ))
            .layout(Layout::left_to_right(Align::Center)),
    );
    let collapsed = input.collapsed.get(file).copied().unwrap_or(false);
    let chevron = child
        .add(
            egui::Button::new(
                RichText::new(if collapsed {
                    regular::CARET_RIGHT
                } else {
                    regular::CARET_DOWN
                })
                .color(p.text_muted),
            )
            .frame(false),
        )
        .on_hover_text(if collapsed { "Expand" } else { "Collapse" })
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let mut viewed = input.viewed.get(file).copied().unwrap_or(false);
    let toggled = child
        .checkbox(&mut viewed, "")
        .on_hover_text("Viewed")
        .changed();
    let (icon, color) = kind_icon(f.kind, p);
    child.label(RichText::new(icon).monospace().color(color));
    let path = f.display_path();
    child.add(
        egui::Label::new(RichText::new(&path).monospace().color(p.text))
            .truncate()
            .selectable(false),
    );
    if chevron.clicked() {
        Some(DiffEvent::ToggleCollapsed(file))
    } else {
        toggled.then_some(DiffEvent::ToggleViewed(file))
    }
}

fn notice(
    ui: &mut Ui,
    rect: Rect,
    file: usize,
    f: &FileDiff,
    p: &Palette,
    salt: impl Hash,
) -> Option<DiffEvent> {
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(salt)
            .max_rect(rect.shrink2(egui::vec2(12.0, 2.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    match &f.content {
        DiffContent::Binary => {
            child.label(
                RichText::new("Binary file changed")
                    .italics()
                    .color(p.text_muted),
            );
            None
        }
        DiffContent::TooLarge => {
            child.label(
                RichText::new("Large file, not shown.")
                    .italics()
                    .color(p.text_muted),
            );
            child
                .small_button("Show anyway")
                .clicked()
                .then_some(DiffEvent::ShowAnyway(file))
        }
        DiffContent::Unavailable(reason) => {
            child.label(
                RichText::new(format!("Not available locally: {reason}"))
                    .italics()
                    .color(p.warning),
            );
            None
        }
        DiffContent::Text(_) => None,
    }
}

/// Title above a group of threads listed before a file's rows.
fn group_title(ui: &Ui, rect: Rect, title: &str, p: &Palette) {
    ui.painter().text(
        rect.left_center() + egui::vec2(12.0, 0.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(12.0),
        p.text_muted,
    );
}

/// What an outdated thread says about where it was.
fn outdated_label(thread: &ReviewThread) -> String {
    let version = match thread.side {
        Side::Left => "base",
        Side::Right => "pull request",
    };
    let line = thread
        .original_line
        .map_or_else(|| "?".to_string(), |l| l.to_string());
    format!("Line {line} of the {version} version (outdated)")
}

/// Make sure the tokens of both sides of `file` exist, and those of the lines
/// of row `index`.
fn ensure_tokens(
    tokens: &mut TokenCache,
    file: usize,
    f: &FileDiff,
    text: &TextDiff,
    index: usize,
    plugins: Option<&PluginManager>,
) {
    let sides = [
        (Side::Left, &text.old, f.old_path.as_deref()),
        (Side::Right, &text.new, f.new_path.as_deref()),
    ];
    for (side, lines, path) in sides {
        tokens
            .entry((file, side))
            .or_insert_with(|| SideTokens::new(lines, path.unwrap_or(f.path()), plugins));
    }
    let (old, new) = match text.rows[index] {
        DiffRow::Context { old, new } | DiffRow::Modified { old, new } => (Some(old), Some(new)),
        DiffRow::Removed { old } => (Some(old), None),
        DiffRow::Added { new } => (None, Some(new)),
        DiffRow::Fold { .. } => (None, None),
    };
    for (side, line, lines) in [(Side::Left, old, &text.old), (Side::Right, new, &text.new)] {
        if let (Some(line), Some(tokens)) = (line, tokens.get_mut(&(file, side))) {
            if let Some(content) = lines.get(line) {
                tokens.ensure(line, content, plugins);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum LineKind {
    Context,
    Removed,
    Added,
}

/// What one side of a row shows.
struct SideLine<'a> {
    /// 0-based.
    number: usize,
    text: &'a str,
    tokens: &'a [Token],
    changed: &'a [Range<usize>],
    kind: LineKind,
    /// Digits of the largest line number of the side.
    digits: usize,
}

fn side_line<'a>(
    lines: &'a [String],
    tokens: &'a SideTokens,
    changed: Option<&'a [Range<usize>]>,
    at: Option<(usize, LineKind)>,
) -> Option<SideLine<'a>> {
    let (number, kind) = at?;
    Some(SideLine {
        number,
        text: lines.get(number)?.as_str(),
        tokens: tokens.get(number),
        changed: changed.unwrap_or(&[][..]),
        kind,
        digits: lines.len().max(1).to_string().len(),
    })
}

/// Fonts and colours shared by the cells of a frame.
struct Metrics<'a> {
    p: &'a Palette,
    font: &'a FontId,
    /// Width of a monospace char.
    char_width: f32,
}

/// Where and what row is drawn.
struct RowCell<'a> {
    rect: Rect,
    file: usize,
    /// Index in `text.rows`.
    index: usize,
    text: &'a TextDiff,
    flashing: bool,
    id: egui::Id,
}

fn row(
    ui: &mut Ui,
    cell: &RowCell<'_>,
    tokens: [&SideTokens; 2],
    m: &Metrics<'_>,
) -> Option<DiffEvent> {
    let RowCell {
        rect,
        file,
        index,
        text,
        flashing,
        id,
    } = *cell;
    let p = m.p;
    let r = text.rows[index];
    let (old, new) = match r {
        DiffRow::Fold { len, .. } => {
            let resp = ui.interact(rect, id, Sense::click());
            let bg = if resp.hovered() {
                p.surface_raised
            } else {
                p.surface
            };
            ui.painter().rect_filled(rect, 0.0, bg);
            let s = if len == 1 { "" } else { "s" };
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                format!("⋯ {len} unchanged line{s}"),
                FontId::proportional(12.0),
                p.text_muted,
            );
            return resp
                .on_hover_text("Show these lines")
                .clicked()
                .then_some(DiffEvent::Unfold { file, row: index });
        }
        DiffRow::Context { old, new } => (
            Some((old, LineKind::Context)),
            Some((new, LineKind::Context)),
        ),
        DiffRow::Modified { old, new } => {
            (Some((old, LineKind::Removed)), Some((new, LineKind::Added)))
        }
        DiffRow::Removed { old } => (Some((old, LineKind::Removed)), None),
        DiffRow::Added { new } => (None, Some((new, LineKind::Added))),
    };
    let words = match r {
        DiffRow::Modified { old, .. } => text.words.get(&old),
        _ => None,
    };
    let half = rect.width() / 2.0;
    let left = Rect::from_min_size(rect.min, egui::vec2(half, rect.height()));
    let right = Rect::from_min_size(
        rect.min + egui::vec2(half, 0.0),
        egui::vec2(half, rect.height()),
    );
    let old_line = side_line(&text.old, tokens[0], words.map(|w| w.old.as_slice()), old);
    let new_line = side_line(&text.new, tokens[1], words.map(|w| w.new.as_slice()), new);
    // The base side only shows; the pull request's side takes double-clicks
    // (without joining the keyboard focus order: a row is not a widget).
    side_cell(ui, left, old_line, m, id.with("left"), Sense::HOVER);
    let right_resp = side_cell(ui, right, new_line, m, id.with("right"), Sense::CLICK);
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        Stroke::new(1.0_f32, p.border),
    );
    if flashing {
        ui.painter().rect_stroke(
            rect,
            0.0,
            Stroke::new(1.5_f32, p.accent),
            egui::StrokeKind::Inside,
        );
    }
    match new {
        Some((line, _)) if right_resp.double_clicked() => Some(DiffEvent::OpenLine { file, line }),
        _ => None,
    }
}

/// Width of the line-number gutter for numbers of `digits` digits.
fn gutter_width(digits: usize, char_width: f32) -> f32 {
    (digits as f32 * char_width + 14.0).max(GUTTER)
}

fn side_cell(
    ui: &mut Ui,
    rect: Rect,
    line: Option<SideLine<'_>>,
    m: &Metrics<'_>,
    id: egui::Id,
    sense: Sense,
) -> egui::Response {
    let p = m.p;
    let resp = ui.interact(rect, id, sense);
    let painter = ui.painter_at(rect);
    let Some(line) = line else {
        // Filler facing a line that only exists on the other side.
        painter.rect_filled(rect, 0.0, p.surface);
        return resp;
    };
    let (bg, strong, marker) = match line.kind {
        LineKind::Context => (Color32::TRANSPARENT, Color32::TRANSPARENT, ""),
        LineKind::Removed => (
            p.git_removed.gamma_multiply(0.15),
            p.git_removed.gamma_multiply(0.45),
            "−",
        ),
        LineKind::Added => (
            p.git_added.gamma_multiply(0.15),
            p.git_added.gamma_multiply(0.45),
            "+",
        ),
    };
    painter.rect_filled(rect, 0.0, bg);
    let gutter = gutter_width(line.digits, m.char_width);
    let y = rect.center().y;
    painter.text(
        Pos2::new(rect.left() + gutter - 6.0, y),
        Align2::RIGHT_CENTER,
        (line.number + 1).to_string(),
        m.font.clone(),
        p.text_faint,
    );
    painter.text(
        Pos2::new(rect.left() + gutter + 2.0, y),
        Align2::LEFT_CENTER,
        marker,
        m.font.clone(),
        p.text_muted,
    );
    // Only what fits is laid out: a minified file's line can be megabytes.
    let left = rect.left() + gutter + MARKER;
    let fits = (((rect.right() - left).max(0.0) / m.char_width).ceil() as usize).saturating_add(2);
    let shown = cut_at(line.text, fits);
    let style = LineStyle {
        font: m.font.clone(),
        strong,
        plain: p.text,
        max_bytes: shown,
    };
    let galley = ui.fonts(|f| f.layout_job(line_job(line.text, line.tokens, line.changed, &style)));
    let clipped = shown < line.text.len() || left + galley.size().x > rect.right();
    painter.galley(Pos2::new(left, y - galley.size().y / 2.0), galley, p.text);
    if clipped {
        resp.on_hover_ui(|ui| {
            let end = cut_at(line.text, HOVER_CHARS);
            let ellipsis = if end < line.text.len() { "…" } else { "" };
            ui.label(RichText::new(format!("{}{ellipsis}", &line.text[..end])).monospace());
        })
    } else {
        resp
    }
}

/// Byte index after the first `chars` chars of `text` (all of it if shorter).
fn cut_at(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(i, _)| i)
}

/// How [`line_job`] draws a line.
struct LineStyle {
    font: FontId,
    /// Background of the changed words.
    strong: Color32,
    /// Colour of plain text (`Normal` and `Operator` tokens): the theme's, not
    /// the editor's fixed light grey.
    plain: Color32,
    /// Only the first `max_bytes` of the line are laid out.
    max_bytes: usize,
}

/// The layout of one line: token colours, and a `strong` background on the
/// `changed` byte ranges. Tokens that don't spell the line exactly are ignored.
fn line_job(
    text: &str,
    tokens: &[Token],
    changed: &[Range<usize>],
    style: &LineStyle,
) -> LayoutJob {
    let mut limit = style.max_bytes.min(text.len());
    while !text.is_char_boundary(limit) {
        limit -= 1;
    }
    let plain = || {
        let mut job = LayoutJob::default();
        job.append(&text[..limit], 0.0, format_of(style, style.plain, false));
        job
    };
    // Spans of the part that is laid out; the tokens must spell the line.
    let mut spans = Vec::with_capacity(tokens.len());
    let mut pos = 0;
    for t in tokens {
        if pos >= limit {
            break;
        }
        let end = pos + t.text.len();
        if text.get(pos..end) != Some(t.text.as_str()) {
            return plain();
        }
        let color = match t.kind {
            TokenKind::Normal | TokenKind::Operator => style.plain,
            kind => kind.color(),
        };
        spans.push((pos..end.min(limit), color));
        pos = end;
    }
    if pos < limit {
        return plain();
    }
    let mut job = LayoutJob::default();
    for (range, color) in spans {
        let mut start = range.start;
        while start < range.end {
            let (end, marked) = next_boundary(start, range.end, changed);
            let Some(part) = text.get(start..end) else {
                return plain();
            };
            job.append(part, 0.0, format_of(style, color, marked));
            start = end;
        }
    }
    job
}

fn format_of(style: &LineStyle, color: Color32, marked: bool) -> TextFormat {
    TextFormat {
        font_id: style.font.clone(),
        color,
        background: if marked {
            style.strong
        } else {
            Color32::TRANSPARENT
        },
        ..Default::default()
    }
}

/// End of the run from `start` (at most `end`) that is uniformly inside or
/// outside `changed`, and whether it is inside.
fn next_boundary(start: usize, end: usize, changed: &[Range<usize>]) -> (usize, bool) {
    if let Some(r) = changed.iter().find(|r| r.contains(&start)) {
        return (r.end.min(end), true);
    }
    let next = changed
        .iter()
        .map(|r| r.start)
        .filter(|&s| s > start)
        .min()
        .unwrap_or(end);
    (next.min(end), false)
}

/// Where and how a thread is drawn.
struct ThreadCell {
    top_left: Pos2,
    /// Width of the whole view.
    width: f32,
    /// Across the whole view rather than under its side.
    full_width: bool,
    /// Small line above the first comment.
    label: Option<String>,
}

/// Draw a thread; its height.
fn paint_thread(
    ui: &mut Ui,
    cell: &ThreadCell,
    thread: &ReviewThread,
    p: &Palette,
    md_cache: &mut CommonMarkCache,
    salt: impl Hash,
) -> f32 {
    let ThreadCell {
        top_left,
        width,
        full_width,
        ref label,
    } = *cell;
    let (x, w) = match (full_width, thread.side) {
        (true, _) => (0.0, width),
        (false, Side::Left) => (0.0, width / 2.0),
        (false, Side::Right) => (width / 2.0, width / 2.0),
    };
    let max = Rect::from_min_size(
        top_left + egui::vec2(x + 8.0, 4.0),
        egui::vec2((w - 16.0).max(40.0), THREAD_MAX_HEIGHT),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(salt)
            .max_rect(max)
            .layout(Layout::top_down(Align::Min)),
    );
    egui::Frame::new()
        .fill(p.surface_raised)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::same(8))
        .show(&mut child, |ui| {
            ui.set_width((max.width() - 18.0).max(1.0));
            if let Some(label) = label {
                ui.label(RichText::new(label).small().color(p.text_faint));
            }
            for (k, comment) in thread.comments.iter().enumerate() {
                if k > 0 {
                    ui.add_space(4.0);
                }
                ui.horizontal(|ui| {
                    if k > 0 {
                        ui.add_space(16.0);
                    }
                    ui.label(RichText::new(&comment.author).strong());
                    ui.label(RichText::new(comment.date()).small().color(p.text_muted));
                });
                if k == 0 {
                    CommonMarkViewer::new().show(ui, md_cache, &comment.body);
                } else {
                    ui.indent(("reply", k), |ui| {
                        CommonMarkViewer::new().show(ui, md_cache, &comment.body);
                    });
                }
            }
        });
    child.min_rect().height() + 8.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::compare::{binary_file, text_file};
    use crate::github::api::thread;
    use std::collections::HashMap;

    fn style(strong: Color32, plain: Color32, max_bytes: usize) -> LineStyle {
        LineStyle {
            font: FontId::monospace(12.0),
            strong,
            plain,
            max_bytes,
        }
    }

    fn parts(job: &LayoutJob, strong: Color32) -> Vec<(&str, bool)> {
        job.sections
            .iter()
            .map(|s| {
                (
                    &job.text[s.byte_range.clone()],
                    s.format.background == strong,
                )
            })
            .collect()
    }

    #[test]
    fn changed_words_get_the_strong_background_and_tokens_keep_their_colour() {
        let tokens = vec![
            Token {
                text: "let".into(),
                kind: TokenKind::Keyword,
            },
            Token {
                text: " y = 1;".into(),
                kind: TokenKind::Normal,
            },
        ];
        let job = line_job(
            "let y = 1;",
            &tokens,
            std::slice::from_ref(&(4..5)),
            &style(Color32::RED, Color32::GOLD, usize::MAX),
        );
        assert_eq!(
            parts(&job, Color32::RED),
            vec![("let", false), (" ", false), ("y", true), (" = 1;", false)]
        );
        assert_eq!(job.sections[0].format.color, TokenKind::Keyword.color());
        assert_eq!(
            job.sections[1].format.color,
            Color32::GOLD,
            "normal text takes the theme's colour"
        );
    }

    #[test]
    fn tokens_that_do_not_spell_the_line_fall_back_to_plain_text() {
        let tokens = vec![Token {
            text: "other".into(),
            kind: TokenKind::Keyword,
        }];
        let job = line_job(
            "abc",
            &tokens,
            &[],
            &style(Color32::RED, Color32::GOLD, usize::MAX),
        );
        assert_eq!(parts(&job, Color32::RED), vec![("abc", false)]);
        assert_eq!(job.sections[0].format.color, Color32::GOLD);
    }

    #[test]
    fn operators_and_normal_tokens_use_the_plain_colour() {
        let tokens = vec![
            Token {
                text: "a".into(),
                kind: TokenKind::Operator,
            },
            Token {
                text: "b".into(),
                kind: TokenKind::Normal,
            },
        ];
        let job = line_job(
            "ab",
            &tokens,
            &[],
            &style(Color32::RED, Color32::GOLD, usize::MAX),
        );
        assert!(job.sections.iter().all(|s| s.format.color == Color32::GOLD));
    }

    #[test]
    fn a_changed_range_spanning_two_tokens_is_marked_in_both() {
        let tokens = vec![
            Token {
                text: "ab".into(),
                kind: TokenKind::Keyword,
            },
            Token {
                text: "cd".into(),
                kind: TokenKind::Normal,
            },
        ];
        let job = line_job(
            "abcd",
            &tokens,
            std::slice::from_ref(&(1..3)),
            &style(Color32::RED, Color32::GOLD, usize::MAX),
        );
        assert_eq!(
            parts(&job, Color32::RED),
            vec![("a", false), ("b", true), ("c", true), ("d", false)]
        );
    }

    #[test]
    fn a_long_line_is_cut_on_a_char_boundary_and_the_spans_are_clamped() {
        let tokens = vec![
            Token {
                text: "ab".into(),
                kind: TokenKind::Keyword,
            },
            Token {
                text: "éécd".into(),
                kind: TokenKind::Normal,
            },
        ];
        let text = "abéécd";
        // 3 bytes would cut "é" in two: the cut moves back to the boundary.
        let job = line_job(
            text,
            &tokens,
            std::slice::from_ref(&(1..8)),
            &style(Color32::RED, Color32::GOLD, 3),
        );
        assert_eq!(job.text, "ab");
        let job = line_job(
            text,
            &tokens,
            std::slice::from_ref(&(1..8)),
            &style(Color32::RED, Color32::GOLD, 4),
        );
        assert_eq!(
            parts(&job, Color32::RED),
            vec![("a", false), ("b", true), ("é", true)]
        );
        assert_eq!(cut_at("aéb", 2), 3);
        assert_eq!(cut_at("aéb", 9), 4);
    }

    #[test]
    fn bad_changed_ranges_fall_back_to_plain_text_instead_of_panicking() {
        // 2..3 cuts "é" (bytes 1..3) in two.
        let job = line_job(
            "aé",
            &[],
            std::slice::from_ref(&(2..3)),
            &style(Color32::RED, Color32::GOLD, usize::MAX),
        );
        assert_eq!(job.text, "aé");
        assert_eq!(job.sections[0].format.color, Color32::GOLD);
    }

    #[test]
    fn boundaries_split_runs_inside_and_outside_changed_ranges() {
        let changed = [2..4, 6..7];
        assert_eq!(next_boundary(0, 10, &changed), (2, false));
        assert_eq!(next_boundary(2, 10, &changed), (4, true));
        assert_eq!(next_boundary(3, 10, &changed), (4, true));
        assert_eq!(next_boundary(4, 10, &changed), (6, false));
        assert_eq!(next_boundary(6, 10, &changed), (7, true));
        assert_eq!(next_boundary(7, 10, &changed), (10, false));
        assert_eq!(next_boundary(2, 3, &changed), (3, true), "clamped to end");
        assert_eq!(next_boundary(0, 1, &changed), (1, false));
        assert_eq!(next_boundary(0, 5, &[]), (5, false));
    }

    #[test]
    fn the_sticky_header_is_pushed_up_by_the_next_file() {
        assert_eq!(sticky_top(100.0, None), 100.0);
        assert_eq!(sticky_top(100.0, Some(300.0)), 100.0);
        assert_eq!(sticky_top(100.0, Some(110.0)), 110.0 - HEADER_HEIGHT);
    }

    #[test]
    fn the_gutter_fits_the_largest_line_number() {
        assert_eq!(gutter_width(2, 7.0), GUTTER);
        assert!(gutter_width(7, 7.0) > GUTTER);
    }

    #[test]
    fn big_documents_are_tokenized_line_by_line() {
        let lines: Vec<String> = (0..VIEWPORT_THRESHOLD + 1)
            .map(|i| format!("l{i}"))
            .collect();
        let mut side = SideTokens::new(&lines, "a.txt", None);
        assert!(side.get(5).is_empty(), "not tokenized before it is drawn");
        side.ensure(5, &lines[5], None);
        assert_eq!(side.get(5)[0].text, "l5");
        let small = SideTokens::new(&lines[..10], "a.txt", None);
        assert_eq!(small.get(9)[0].text, "l9", "small documents are tokenized");
    }

    /// Items: header (0..30), 4 rows of 20; the stops are items 1 and 4, at
    /// tops 30 and 90.
    fn two_stops() -> DiffLayout {
        let files = vec![text_file("a", "a\nb\nc\nd\n", "A\nb\nc\n")];
        let l = DiffLayout::build(&files, &[], &[false], 20.0, &HashMap::new());
        assert_eq!(l.change_starts(), [1, 4]);
        l
    }

    #[test]
    fn f7_steps_from_a_cursor_that_is_still_on_screen() {
        let l = two_stops();
        let view = Some((0.0, 100.0));
        assert_eq!(next_stop(&l, Some(0), view, Nav::Next), Some(1));
        assert_eq!(next_stop(&l, Some(1), view, Nav::Prev), Some(0));
        // Stays on the last / first stop.
        assert_eq!(next_stop(&l, Some(1), view, Nav::Next), Some(1));
        assert_eq!(next_stop(&l, Some(0), view, Nav::Prev), Some(0));
    }

    #[test]
    fn f7_searches_from_the_position_when_the_cursor_scrolled_away() {
        let l = two_stops();
        // Cursor on stop 0 (top 30), but the reader is looking at 60..160:
        // the next stop at or below the top of the view is stop 1 (top 90).
        assert_eq!(
            next_stop(&l, Some(0), Some((60.0, 160.0)), Nav::Next),
            Some(1)
        );
        // Cursor on stop 1 (top 90), reader scrolled back to 0..60: Shift+F7
        // goes to the last stop above the view top... none, so the first.
        assert_eq!(
            next_stop(&l, Some(1), Some((0.0, 60.0)), Nav::Prev),
            Some(0)
        );
        // Cursor on stop 0, reader far below: Shift+F7 lands on the last stop
        // above the view, not on stop 0's predecessor.
        assert_eq!(
            next_stop(&l, Some(0), Some((200.0, 300.0)), Nav::Prev),
            Some(1)
        );
        assert_eq!(
            next_stop(&l, Some(0), Some((200.0, 300.0)), Nav::Next),
            Some(1),
            "past the last stop"
        );
    }

    #[test]
    fn f7_without_a_cursor_or_a_viewport_starts_from_the_top() {
        let l = two_stops();
        assert_eq!(next_stop(&l, None, None, Nav::Next), Some(0));
        assert_eq!(
            next_stop(&l, Some(1), None, Nav::Next),
            Some(0),
            "no viewport yet: the cursor can't be trusted"
        );
        assert_eq!(
            next_stop(&l, Some(7), Some((0.0, 100.0)), Nav::Next),
            Some(0)
        );
        assert_eq!(next_stop(&l, None, Some((50.0, 150.0)), Nav::Next), Some(1));
        assert_eq!(next_stop(&l, None, Some((50.0, 150.0)), Nav::Prev), Some(0));
        let empty = DiffLayout::build(&[], &[], &[], 20.0, &HashMap::new());
        assert_eq!(next_stop(&empty, None, None, Nav::Next), None);
    }

    #[test]
    fn invalidate_keeps_the_cursor_and_invalidate_stops_clears_it() {
        let mut view = DiffView {
            current: Some(1),
            ..Default::default()
        };
        view.invalidate();
        assert_eq!(view.current_change(), Some(1));
        view.invalidate_stops();
        assert_eq!(view.current_change(), None);
    }

    // ── Headless tests of `show()` ──────────────────────────────────────

    /// Runs `DiffView::show` in an egui context, a frame at a time.
    struct Harness {
        ctx: egui::Context,
        md: CommonMarkCache,
        /// Text shapes of the last frame: text, bounds and colours.
        texts: Vec<Painted2>,
        time: f64,
        size: egui::Vec2,
    }

    struct Painted2 {
        text: String,
        bounds: Rect,
        colors: Vec<Color32>,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                ctx: egui::Context::default(),
                md: CommonMarkCache::default(),
                texts: Vec::new(),
                time: 0.0,
                size: egui::vec2(800.0, 300.0),
            }
        }

        fn frame(
            &mut self,
            view: &mut DiffView,
            input: &DiffInput<'_>,
            events: Vec<egui::Event>,
        ) -> Vec<DiffEvent> {
            self.time += 1.0 / 60.0;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                time: Some(self.time),
                predicted_dt: 1.0 / 60.0,
                events,
                ..Default::default()
            };
            let mut shown = Vec::new();
            let md = &mut self.md;
            let out = self.ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    shown = view.show(ui, input, md);
                });
            });
            self.texts.clear();
            for clipped in &out.shapes {
                collect(&clipped.shape, &mut self.texts);
            }
            shown
        }

        fn frames(&mut self, view: &mut DiffView, input: &DiffInput<'_>, n: usize) {
            for _ in 0..n {
                self.frame(view, input, vec![]);
            }
        }

        fn text(&self, text: &str) -> Rect {
            self.texts
                .iter()
                .find(|t| t.text == text)
                .unwrap_or_else(|| panic!("{text:?} is not drawn"))
                .bounds
        }

        /// Move to `pos`, then click it `times` times; the events they produce.
        fn click(
            &mut self,
            view: &mut DiffView,
            input: &DiffInput<'_>,
            pos: Pos2,
            times: usize,
        ) -> Vec<DiffEvent> {
            let mut all = self.frame(view, input, vec![egui::Event::PointerMoved(pos)]);
            for _ in 0..times {
                all.extend(self.frame(view, input, vec![button(pos, true)]));
                all.extend(self.frame(view, input, vec![button(pos, false)]));
            }
            all.extend(self.frame(view, input, vec![]));
            all
        }
    }

    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn collect(shape: &egui::Shape, out: &mut Vec<Painted2>) {
        match shape {
            egui::Shape::Text(t) => out.push(Painted2 {
                text: t.galley.text().to_string(),
                bounds: t.visual_bounding_rect(),
                colors: t
                    .galley
                    .job
                    .sections
                    .iter()
                    .map(|s| s.format.color)
                    .collect(),
            }),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }

    fn palette() -> Palette {
        Palette::from_theme(&crate::config::Theme::default())
    }

    fn input<'a>(
        files: &'a [FileDiff],
        threads: &'a [ReviewThread],
        viewed: &'a [bool],
    ) -> DiffInput<'a> {
        DiffInput {
            files,
            threads,
            viewed,
            collapsed: viewed,
            palette: palette(),
            plugins: None,
        }
    }

    /// 200 lines with five changes: five blocks of rows between folds.
    fn long_file(path: &str) -> FileDiff {
        let old: String = (0..200).map(|i| format!("l{i}\n")).collect();
        let mut new = old.clone();
        for i in [0, 50, 100, 150, 199] {
            new = new.replace(&format!("l{i}\n"), &format!("L{i}\n"));
        }
        text_file(path, &old, &new)
    }

    fn is_fold(files: &[FileDiff], item: &Item) -> bool {
        match item {
            Item::Row { file, row } => match &files[*file].content {
                DiffContent::Text(t) => matches!(t.rows[*row], DiffRow::Fold { .. }),
                _ => false,
            },
            _ => false,
        }
    }

    #[test]
    fn a_click_on_the_sticky_header_does_not_reach_the_fold_under_it() {
        let files = vec![long_file("a.rs")];
        let viewed = [false];
        let input = input(&files, &[], &viewed);
        let mut view = DiffView::default();
        let mut h = Harness::new();
        h.frame(&mut view, &input, vec![]);
        let fold = view
            .layout()
            .unwrap()
            .items()
            .iter()
            .position(|i| is_fold(&files, i))
            .unwrap();
        // The fold is the first row under the sticky header.
        view.scroll_to = Some((fold, true));
        h.frames(&mut view, &input, 3);
        let name = h.text("a.rs");
        let pos = Pos2::new(700.0, name.center().y);
        let events = h.click(&mut view, &input, pos, 2);
        assert_eq!(events, vec![], "the header swallowed the clicks");
    }

    #[test]
    fn a_double_click_on_the_right_half_opens_the_line_and_on_a_filler_does_nothing() {
        // Rows: Context a, Context b, Added c (filler on the left).
        let files = vec![text_file("a.rs", "a\nb\n", "a\nb\nc\n")];
        let viewed = [false];
        let input = input(&files, &[], &viewed);
        let mut view = DiffView::default();
        let mut h = Harness::new();
        h.frames(&mut view, &input, 2);
        let c = h.text("c").center();
        let events = h.click(&mut view, &input, Pos2::new(c.x + 100.0, c.y), 2);
        assert_eq!(events, vec![DiffEvent::OpenLine { file: 0, line: 2 }]);
        let events = h.click(&mut view, &input, Pos2::new(100.0, c.y), 2);
        assert_eq!(events, vec![], "left filler");
    }

    #[test]
    fn collapse_anchor_puts_the_header_at_the_top_of_the_viewport() {
        let files = vec![long_file("a.rs"), long_file("b.rs")];
        let viewed = [false, false];
        let input = input(&files, &[], &viewed);
        let mut view = DiffView::default();
        let mut h = Harness::new();
        h.frames(&mut view, &input, 2);
        view.collapse_anchor(1);
        h.frames(&mut view, &input, 3);
        let layout = view.layout().unwrap();
        let top = layout.top(layout.header_item(1).unwrap());
        assert!(top > 300.0, "the second file is below the first screen");
        let (viewport_top, _) = view.viewport.unwrap();
        assert!((viewport_top - top).abs() < 1.0, "{viewport_top} vs {top}");
        // The same through a rebuild, as after a "viewed" toggle.
        view.invalidate_stops();
        view.collapse_anchor(0);
        h.frames(&mut view, &input, 3);
        assert!(view.viewport.unwrap().0 < 1.0);
    }

    #[test]
    fn scroll_to_file_waits_for_the_layout() {
        let files = vec![long_file("a.rs"), long_file("b.rs")];
        let viewed = [false, false];
        let input = input(&files, &[], &viewed);
        let mut view = DiffView::default();
        let mut h = Harness::new();
        view.scroll_to_file(1);
        h.frames(&mut view, &input, 3);
        let layout = view.layout().unwrap();
        let top = layout.top(layout.header_item(1).unwrap());
        assert!(top > 300.0);
        let viewport_top = view.viewport.unwrap().0;
        assert!(
            (viewport_top - top).abs() < 1.0,
            "header at the top: {viewport_top} vs {top}"
        );
        assert_eq!(view.top_file(), Some(1));
    }

    #[test]
    fn scroll_to_file_works_right_after_an_invalidation_and_from_below() {
        let files = vec![long_file("a.rs"), long_file("b.rs"), long_file("c.rs")];
        let viewed = [false, false, false];
        let input = input(&files, &[], &viewed);
        let mut view = DiffView::default();
        let mut h = Harness::new();
        h.frames(&mut view, &input, 2);
        view.scroll_to_file(2);
        h.frames(&mut view, &input, 3);
        view.invalidate();
        view.scroll_to_file(1);
        h.frames(&mut view, &input, 1);
        let layout = view.layout().unwrap();
        let top = layout.top(layout.header_item(1).unwrap());
        let viewport_top = view.viewport.unwrap().0;
        assert!(
            (viewport_top - top).abs() < 1.0,
            "header at the top at once: {viewport_top} vs {top}"
        );
    }

    #[test]
    fn the_chevron_asks_to_toggle_the_collapse_and_the_checkbox_the_mark() {
        let files = vec![long_file("a.rs")];
        let flags = [false];
        let input = input(&files, &[], &flags);
        let mut view = DiffView::default();
        let mut h = Harness::new();
        h.frames(&mut view, &input, 2);
        let name = h.text("a.rs");
        // Chevron: left of the checkbox, the first thing in the header.
        let pos = Pos2::new(10.0 + 8.0, name.center().y);
        let events = h.click(&mut view, &input, pos, 1);
        assert_eq!(events, vec![DiffEvent::ToggleCollapsed(0)]);
    }

    #[test]
    fn f7_ends_inside_the_viewport_below_an_unmeasured_thread() {
        let old: String = (0..100).map(|i| format!("l{i}\n")).collect();
        let new: String = (0..100)
            .map(|i| {
                if i < 40 || i == 60 {
                    format!("L{i}\n")
                } else {
                    format!("l{i}\n")
                }
            })
            .collect();
        let files = vec![text_file("a.rs", &old, &new)];
        let mut t = thread("a.rs", Side::Right, Some(60));
        t.comments[0].body = (0..30).map(|i| format!("Paragraph {i}.\n\n")).collect();
        let threads = vec![t];
        let viewed = [false];
        let input = input(&files, &threads, &viewed);
        let mut view = DiffView::default();
        let mut h = Harness::new();
        h.size = egui::vec2(800.0, 600.0);
        h.frames(&mut view, &input, 2);
        view.request_next();
        h.frames(&mut view, &input, 4);
        view.request_next();
        h.frames(&mut view, &input, 8);
        assert_eq!(view.current_change(), Some(1));
        let layout = view.layout().unwrap();
        let stop = layout.top(layout.change_starts()[1]);
        let (top, bottom) = view.viewport.unwrap();
        assert!(
            (top..bottom).contains(&stop),
            "stop at {stop}, viewport {top}..{bottom}"
        );
        assert!(
            layout
                .items()
                .iter()
                .any(|i| matches!(i, Item::Thread { .. })),
            "the thread sits above the stop"
        );
    }

    #[test]
    fn a_huge_line_is_laid_out_only_as_far_as_it_is_visible() {
        let base = "abc ".repeat(25_000);
        let files = vec![text_file(
            "min.js",
            &format!("{base}\n"),
            &format!("{base}X\n"),
        )];
        let viewed = [false];
        let input = input(&files, &[], &viewed);
        let mut view = DiffView::default();
        let mut h = Harness::new();
        h.frames(&mut view, &input, 3);
        let longest = h.texts.iter().map(|t| t.text.len()).max().unwrap();
        assert!(longest < 500, "{longest} bytes laid out");
        assert!(
            h.texts.iter().any(|t| t.text.starts_with("abc abc")),
            "the start of the line is drawn"
        );
    }

    #[test]
    fn plain_text_takes_the_theme_colour() {
        let light = crate::config::Theme {
            name: "light".into(),
            background: [255, 255, 255],
            foreground: [30, 30, 30],
            accent: [0, 122, 204],
        };
        let files = vec![text_file("notes.txt", "hello\n", "hello world\n")];
        let viewed = [false];
        let mut input = input(&files, &[], &viewed);
        input.palette = Palette::from_theme(&light);
        let mut view = DiffView::default();
        let mut h = Harness::new();
        h.frames(&mut view, &input, 3);
        let line = h
            .texts
            .iter()
            .find(|t| t.text == "hello world")
            .expect("the line is drawn");
        assert!(!line.colors.is_empty());
        assert!(
            line.colors.iter().all(|&c| c == input.palette.text),
            "{:?} vs {:?}",
            line.colors,
            input.palette.text
        );
        assert_ne!(input.palette.text, TokenKind::Normal.color());
    }

    #[test]
    fn a_binary_file_and_a_thread_on_another_file_still_render() {
        let files = vec![binary_file("img.png"), long_file("a.rs")];
        let threads = vec![thread("gone.rs", Side::Right, Some(1))];
        let viewed = [false, false];
        let input = input(&files, &threads, &viewed);
        let mut view = DiffView::default();
        let mut h = Harness::new();
        h.size = egui::vec2(800.0, 700.0);
        h.frames(&mut view, &input, 4);
        h.text("Comments on files not in this diff");
        h.text("Binary file changed");
    }
}
