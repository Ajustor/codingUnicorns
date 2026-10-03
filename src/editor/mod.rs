pub mod auto_close;
pub mod autocomplete;
pub mod bracket_match;
pub mod buffer;
pub mod cursor;
pub mod diff;
pub mod folding;
pub mod highlight;
pub mod hover;
pub mod indent;
pub mod utils;

mod input;
mod line_diff;
mod multi_cursor;
mod search;
mod word_analysis;

use autocomplete::Autocomplete;
use bracket_match::find_matching_bracket;
use buffer::Buffer;
use cursor::Cursor;
use diff::{DIFF_ADDED, DIFF_MODIFIED, DIFF_UNCHANGED};
use folding::compute_fold_regions;
use highlight::Highlighter;
use hover::{inline_markdown_job, parse_hover_sections, HoverSection};
use indent::detect_indent;
use std::path::PathBuf;
use utils::{find_next_occurrence, get_word_at};

/// Editor state right after the last typed char: primary cursor position, whether
/// it has a selection, extra cursor count and content version. Typing continues
/// the current undo burst only while this is unchanged.
type TypingBurstState = ((usize, usize), bool, usize, i32);

pub struct Editor {
    pub buffer: Buffer,
    pub cursor: Cursor,
    pub extra_cursors: Vec<Cursor>,
    pub highlighter: Highlighter,
    pub autocomplete: Autocomplete,
    pub scroll_offset: egui::Vec2,
    pub current_path: Option<PathBuf>,
    pub is_modified: bool,
    pub line_height: f32,
    pub char_width: f32,
    pub show_find: bool,
    pub find_query: String,
    find_matches: Vec<usize>,
    find_current: usize,
    pub show_goto_line: bool,
    pub goto_line_input: String,
    /// Set to true to scroll the viewport so the cursor is visible on the next frame.
    pub scroll_to_cursor: bool,
    /// Set to true to request keyboard focus on the next frame (e.g. after opening a file).
    pub focus_requested: bool,
    /// Populated by Ctrl+click; consumed by the app to navigate to definition.
    pub go_to_definition_request: Option<String>,
    /// Word under the mouse when Ctrl is held: (row, start_col, end_col).
    pub ctrl_hover_word_bounds: Option<(usize, usize, usize)>,
    /// Word currently under the mouse pointer (for hover tooltip).
    hover_word: Option<String>,
    /// Screen position of the mouse (updated every frame the mouse is over the editor).
    hover_pos: egui::Pos2,
    /// When the current `hover_word` was first detected.
    hover_start: Option<std::time::Instant>,
    /// Resolved signature string to display in the tooltip (empty string = looked up, nothing found).
    hover_signature: Option<String>,
    /// Root path of the open workspace, used to search for definitions across files.
    pub workspace_path: Option<std::path::PathBuf>,
    /// Monotonically increasing counter bumped on every edit; used to detect changes for LSP didChange.
    pub content_version: i32,
    /// Debounce state for full-document re-highlighting: the content_version we're
    /// waiting to tokenize, and when it last changed. Avoids re-tokenizing huge
    /// files on every keystroke (which lags the UI).
    pub hl_pending_version: i32,
    pub hl_pending_at: Option<std::time::Instant>,
    /// Set while a run of typed characters is being coalesced into a single undo
    /// step (see `insert_char`). Cleared by any non-text key, and implicitly broken
    /// by cursor movement or any other edit, so each typing burst is one undo unit.
    typing_burst: Option<TypingBurstState>,
    /// Cached longest line length (chars) + the content_version it was computed for.
    /// Avoids an O(file) max-width scan every frame for the horizontal scrollbar.
    max_line_chars: usize,
    max_line_chars_version: i32,
    /// Set when an LSP hover request has been fired; cleared when the response arrives.
    pub hover_lsp_request_pending: bool,
    /// Cursor row when the LSP hover request was triggered.
    pub hover_row: u32,
    /// Cursor column when the LSP hover request was triggered.
    pub hover_col: u32,
    /// Fixed screen position where the tooltip is anchored (set once when hover fires).
    hover_tooltip_anchor: Option<egui::Pos2>,
    /// Diagnostics for the current file, updated by the app each frame from the LSP client.
    pub diagnostics: Vec<crate::lsp::client::Diagnostic>,
    /// Set by the keyboard handler when Ctrl+Space is pressed; consumed by the app.
    pub completion_request_pending: bool,
    /// Cursor row when the completion request was triggered.
    pub completion_trigger_row: usize,
    /// Cursor column when the completion request was triggered.
    pub completion_trigger_col: usize,
    /// Diagnostic hover tooltip message.
    pub diag_hover_msg: Option<String>,
    /// Severity of the hovered diagnostic.
    pub diag_hover_severity: crate::lsp::client::DiagSeverity,
    /// Git blame data for the current file.
    pub blame_data: Vec<crate::git::BlameEntry>,
    /// Path that blame_data was loaded for.
    pub blame_path: Option<std::path::PathBuf>,
    /// Whether to show git blame in the gutter.
    pub show_blame: bool,
    /// Set when a signature help request should be sent.
    pub signature_help_request_pending: bool,
    /// The signature help text to display.
    pub signature_help_text: Option<String>,
    /// Cursor row when signature help was triggered.
    pub signature_help_row: u32,
    /// Cursor column when signature help was triggered.
    pub signature_help_col: u32,
    /// Rect of the hover popup last frame — used to keep popup open when mouse enters it.
    hover_popup_rect: Option<egui::Rect>,
    /// When the mouse left the hovered word / editor — used for the dismissal grace period.
    hover_leave_instant: Option<std::time::Instant>,
    // ── Indent detection ────────────────────────────────────────────────────
    /// Whether the open file uses spaces (true) or tabs (false) for indentation.
    pub detected_indent_spaces: bool,
    /// Detected indent unit size (e.g. 2 or 4).
    pub detected_indent_size: usize,
    // ── Find & Replace ───────────────────────────────────────────────────────
    pub show_replace: bool,
    pub replace_query: String,
    pub find_case_sensitive: bool,
    pub find_use_regex: bool,
    // ── Git diff gutter ────────────────────────────────────────────────────
    /// Per-line diff status: 0=unchanged, 1=added, 2=modified, 3=deleted(marker)
    pub line_diff: Vec<u8>,
    /// Path for which line_diff was computed.
    pub line_diff_path: Option<PathBuf>,
    // ── LSP formatting ─────────────────────────────────────────────────────
    pub format_request_pending: bool,
    // ── Word wrap ───────────────────────────────────────────────────────────
    /// Cached word-wrap column (0 = no wrap).  Set from config each frame.
    pub wrap_col: usize,
    // ── Bracket matching ────────────────────────────────────────────────────
    /// (open_row, open_col, close_row, close_col) of the matching bracket pair.
    bracket_match: Option<(usize, usize, usize, usize)>,
    // ── Code folding ────────────────────────────────────────────────────────
    /// Lines that are the *start* of folded regions.
    pub folded_lines: std::collections::HashSet<usize>,
    /// Cached (start, end) foldable region list.
    fold_regions: Vec<(usize, usize)>,
    /// content_version for which `fold_regions` was last computed (avoids an
    /// O(file) recompute every frame on files that have no foldable regions).
    fold_regions_version: i32,
    // ── Breadcrumbs ─────────────────────────────────────────────────────────
    /// Set true for one frame after an explicit Ctrl+S save, so the app can toast.
    pub just_saved: bool,
    // ── Cursor blink ────────────────────────────────────────────────────────
    /// Epoch-ms of the last cursor movement / keypress, used to reset blink.
    cursor_blink_epoch: std::time::Instant,
    // ── Selection word highlighting ─────────────────────────────────────
    /// Visible occurrences of the word under cursor: (row, col_start, col_end).
    word_occurrences: Vec<(usize, usize, usize)>,
    /// Content version when word_occurrences was last computed.
    word_occurrences_version: i32,
    /// Cached per-line shape data for the minimap (indent + length).
    minimap_lines: Vec<crate::ui::minimap::LineShape>,
    /// Content version when minimap_lines was last computed.
    minimap_lines_version: i32,
    /// The word that was highlighted (to avoid recomputing when cursor moves within same word).
    word_occurrences_word: String,
}

impl Editor {
    /// The currently selected text, if any.
    pub fn selected_text_pub(&self) -> Option<String> {
        let ((sr, sc), (er, ec)) = self.cursor.selection_range()?;
        let start = self.buffer.char_index(sr, sc);
        let end = self.buffer.char_index(er, ec);
        Some(self.buffer.rope_slice(start, end))
    }

    /// The 1-based inclusive line range of the selection, if any.
    pub fn selection_line_range_pub(&self) -> Option<(usize, usize)> {
        let ((sr, _), (er, _)) = self.cursor.selection_range()?;
        Some((sr + 1, er + 1))
    }

    pub fn new() -> Self {
        Self {
            buffer: Buffer::new(),
            cursor: Cursor::new(),
            extra_cursors: vec![],
            highlighter: Highlighter::new(),
            autocomplete: Autocomplete::new(),
            scroll_offset: egui::Vec2::ZERO,
            current_path: None,
            is_modified: false,
            line_height: 20.0,
            char_width: 8.5,
            show_find: false,
            find_query: String::new(),
            find_matches: vec![],
            find_current: 0,
            show_goto_line: false,
            goto_line_input: String::new(),
            scroll_to_cursor: false,
            focus_requested: false,
            go_to_definition_request: None,
            ctrl_hover_word_bounds: None,
            hover_word: None,
            hover_pos: egui::Pos2::ZERO,
            hover_start: None,
            hover_signature: None,
            workspace_path: None,
            content_version: 0,
            hl_pending_version: -1,
            hl_pending_at: None,
            typing_burst: None,
            max_line_chars: 0,
            max_line_chars_version: -1,
            hover_lsp_request_pending: false,
            hover_row: 0,
            hover_col: 0,
            hover_tooltip_anchor: None,
            diagnostics: vec![],
            completion_request_pending: false,
            completion_trigger_row: 0,
            completion_trigger_col: 0,
            diag_hover_msg: None,
            diag_hover_severity: crate::lsp::client::DiagSeverity::Info,
            blame_data: vec![],
            blame_path: None,
            show_blame: false,
            signature_help_request_pending: false,
            signature_help_text: None,
            signature_help_row: 0,
            signature_help_col: 0,
            hover_popup_rect: None,
            hover_leave_instant: None,
            detected_indent_spaces: true,
            detected_indent_size: 4,
            show_replace: false,
            replace_query: String::new(),
            find_case_sensitive: false,
            find_use_regex: false,
            bracket_match: None,
            folded_lines: std::collections::HashSet::new(),
            fold_regions: Vec::new(),
            fold_regions_version: -1,
            just_saved: false,
            line_diff: Vec::new(),
            line_diff_path: None,
            format_request_pending: false,
            wrap_col: 0,
            cursor_blink_epoch: std::time::Instant::now(),
            word_occurrences: vec![],
            word_occurrences_version: -1,
            minimap_lines: vec![],
            minimap_lines_version: -1,
            word_occurrences_word: String::new(),
        }
    }

    pub fn set_content(&mut self, content: String, path: Option<PathBuf>) {
        let lang = path.as_ref().and_then(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.to_string())
        });
        self.buffer = Buffer::from_str(&content);
        self.cursor = Cursor::new();
        self.extra_cursors.clear();
        self.autocomplete = Autocomplete::new();
        self.current_path = path;
        self.is_modified = false;
        self.content_version = 0;
        self.hover_lsp_request_pending = false;
        self.hover_signature = None;
        self.hover_word = None;
        self.hover_start = None;
        self.diagnostics.clear();
        self.scroll_offset = egui::Vec2::ZERO;
        self.show_find = false;
        self.find_query.clear();
        self.find_matches.clear();
        self.diag_hover_msg = None;
        self.blame_data.clear();
        self.signature_help_text = None;
        self.signature_help_request_pending = false;
        self.folded_lines.clear();
        self.fold_regions.clear();
        self.fold_regions_version = -1;
        self.hover_tooltip_anchor = None;
        self.hover_popup_rect = None;
        self.word_occurrences.clear();
        self.word_occurrences_version = -1;
        // Invalidate the minimap cache too — content_version resets to 0 on every
        // load, so without this the new file can collide with the previous file's
        // cached version and the minimap shows a stale ("ghost") overview.
        self.minimap_lines.clear();
        self.minimap_lines_version = -1;
        self.typing_burst = None;
        self.max_line_chars_version = -1;
        // Detect indentation style from file content
        let (spaces, size) = detect_indent(&content);
        self.detected_indent_spaces = spaces;
        self.detected_indent_size = size;
        // Always invalidate the highlight cache — even when the language
        // hasn't changed, the tokens from the previous file must be discarded.
        self.highlighter.invalidate();
        if let Some(name) = lang {
            self.highlighter.set_language_from_filename(&name);
        }
    }

    /// Longest line length (chars), cached and recomputed only when the content
    /// changes — used to size the horizontal scrollbar and clamp horizontal scroll.
    /// Avoids an allocating O(file) scan every frame.
    fn cached_max_line_chars(&mut self) -> usize {
        if self.max_line_chars_version != self.content_version {
            self.max_line_chars = (0..self.buffer.num_lines())
                .map(|i| self.buffer.line_char_len_fast(i))
                .max()
                .unwrap_or(0);
            self.max_line_chars_version = self.content_version;
        }
        self.max_line_chars
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        if let Some(path) = &self.current_path {
            std::fs::write(path, self.buffer.to_string())?;
            self.is_modified = false;
            self.invalidate_line_diff();
        }
        Ok(())
    }

    fn typing_burst_state(&self) -> TypingBurstState {
        (
            self.cursor.position(),
            self.cursor.has_selection(),
            self.extra_cursors.len(),
            self.content_version,
        )
    }

    /// Duplicate current line(s) below.
    fn duplicate_line(&mut self) {
        self.buffer.checkpoint();
        let row = self.cursor.row;
        let line = self.buffer.line(row);
        self.buffer.insert_line(row + 1, &line);
        self.cursor.row += 1;
        self.is_modified = true;
        self.content_version = self.content_version.wrapping_add(1);
    }

    // Render entry point: the parameter list is wide because it threads shared
    // services + the theme palette/spacing into one draw call.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        config: &crate::config::Config,
        plugin_manager: &crate::plugin::manager::PluginManager,
        lsp_hover: Option<String>,
        breakpoint_lines: &std::collections::HashSet<usize>,
        palette: crate::ui::theme::Palette,
        spacing: crate::ui::theme::Spacing,
    ) {
        // Find / Replace bar (floating overlay)
        if self.show_find {
            let ctx = ui.ctx().clone();
            let mut close_find = false;
            let mut do_find_next = false;
            let mut do_find_prev = false;
            let mut query_changed = false;
            let mut do_replace = false;
            let mut do_replace_all = false;

            egui::Window::new("Find")
                .collapsible(false)
                .resizable(false)
                .default_size(egui::vec2(430.0, 30.0))
                .show(&ctx, |ui| {
                    ui.horizontal(|ui| {
                        // Toggle replace row
                        let rep_icon = if self.show_replace { "▴" } else { "▾" };
                        if ui
                            .small_button(rep_icon)
                            .on_hover_text("Toggle replace")
                            .clicked()
                        {
                            self.show_replace = !self.show_replace;
                        }
                        if ui.button("✕").clicked() {
                            close_find = true;
                        }
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.find_query)
                                .hint_text("Find…")
                                .desired_width(160.0),
                        );
                        if resp.changed() {
                            query_changed = true;
                        }
                        if resp.lost_focus() {
                            if ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift) {
                                do_find_next = true;
                            } else if ui
                                .input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.shift)
                            {
                                do_find_prev = true;
                            }
                        }
                        // Case-sensitive toggle (Aa)
                        let cs_color = if self.find_case_sensitive {
                            palette.accent
                        } else {
                            egui::Color32::GRAY
                        };
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new("Aa").color(cs_color))
                                    .frame(false),
                            )
                            .on_hover_text("Case sensitive")
                            .clicked()
                        {
                            self.find_case_sensitive = !self.find_case_sensitive;
                            query_changed = true;
                        }
                        // Regex toggle (.*)
                        let re_color = if self.find_use_regex {
                            palette.accent
                        } else {
                            egui::Color32::GRAY
                        };
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new(".*").color(re_color))
                                    .frame(false),
                            )
                            .on_hover_text("Use regular expression")
                            .clicked()
                        {
                            self.find_use_regex = !self.find_use_regex;
                            query_changed = true;
                        }
                        if ui
                            .button("▲")
                            .on_hover_text("Previous match (Shift+Enter)")
                            .clicked()
                        {
                            do_find_prev = true;
                        }
                        if ui.button("▼").on_hover_text("Next match (Enter)").clicked() {
                            do_find_next = true;
                        }
                        let total = self.find_matches.len();
                        let label = if total > 0 {
                            format!("{}/{}", self.find_current + 1, total)
                        } else if !self.find_query.is_empty() {
                            "No results".to_string()
                        } else {
                            String::new()
                        };
                        ui.label(
                            egui::RichText::new(label)
                                .size(11.0)
                                .color(egui::Color32::GRAY),
                        );
                    });
                    if self.show_replace {
                        ui.horizontal(|ui| {
                            ui.add_space(28.0); // align with find field
                            ui.add(
                                egui::TextEdit::singleline(&mut self.replace_query)
                                    .hint_text("Replace…")
                                    .desired_width(160.0),
                            );
                            if ui.small_button("Replace").clicked() {
                                do_replace = true;
                            }
                            if ui.small_button("All").clicked() {
                                do_replace_all = true;
                            }
                        });
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close_find = true;
                    }
                });

            if close_find {
                self.show_find = false;
                self.show_replace = false;
                self.find_query.clear();
                self.find_matches.clear();
            }
            if query_changed {
                self.update_find_matches();
            }
            if do_find_next {
                self.find_next();
            }
            if do_find_prev {
                self.find_prev();
            }
            if do_replace {
                self.replace_current();
            }
            if do_replace_all {
                self.replace_all_matches();
            }
        }

        // Go to line dialog
        if self.show_goto_line {
            let ctx = ui.ctx().clone();
            let mut close_goto = false;
            let mut do_goto = false;

            egui::Window::new("Go to Line")
                .collapsible(false)
                .resizable(false)
                .default_size(egui::vec2(200.0, 30.0))
                .show(&ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("✕").clicked() {
                            close_goto = true;
                        }
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.goto_line_input)
                                .hint_text("Line number…")
                                .desired_width(120.0),
                        );
                        // Check for Enter (which makes the single-line edit drop focus)
                        // BEFORE re-requesting focus, or lost_focus() would never be true.
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            do_goto = true;
                            // Consume Enter so the editor (which regains focus this
                            // frame) doesn't also insert a newline at the target line.
                            ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                            });
                        } else {
                            resp.request_focus();
                        }
                        if ui.button("Go").clicked() {
                            do_goto = true;
                        }
                    });
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close_goto = true;
                    }
                });

            if do_goto {
                if let Ok(n) = self.goto_line_input.trim().parse::<usize>() {
                    let row = n.saturating_sub(1);
                    let row = row.min(self.buffer.num_lines().saturating_sub(1));
                    self.cursor.set_position(row, 0);
                    self.extra_cursors.clear();
                    // Scroll to the line
                    let target_y = row as f32 * self.line_height;
                    self.scroll_offset.y = target_y;
                }
                close_goto = true;
            }
            if close_goto {
                self.show_goto_line = false;
                self.goto_line_input.clear();
            }
        }

        let bg_color = egui::Color32::from_rgb(
            config.theme.background[0],
            config.theme.background[1],
            config.theme.background[2],
        );
        let fg_color = egui::Color32::from_rgb(
            config.theme.foreground[0],
            config.theme.foreground[1],
            config.theme.foreground[2],
        );
        let accent_color = egui::Color32::from_rgb(
            config.theme.accent[0],
            config.theme.accent[1],
            config.theme.accent[2],
        );
        let line_num_color = palette.text_muted;
        let line_num_color_active = palette.text;
        let (cur_row_for_gutter, _) = self.cursor.position();
        let cursor_color = accent_color;
        let find_highlight = egui::Color32::from_rgba_premultiplied(255, 200, 0, 35);
        let find_highlight_active = egui::Color32::from_rgba_premultiplied(255, 200, 0, 80);
        let blame_extra_width = if self.show_blame { 110.0f32 } else { 0.0f32 };
        let gutter_width = if config.editor.line_numbers {
            50.0
        } else {
            8.0
        } + blame_extra_width;

        let line_height = self.line_height;
        let font_id = egui::FontId::monospace(config.font.size);

        // Measure actual monospace character width from the font (replaces hardcoded 8.5)
        let char_width = ui.fonts(|f| {
            f.layout_no_wrap("M".into(), font_id.clone(), egui::Color32::WHITE)
                .size()
                .x
        });
        self.char_width = char_width;

        let total_lines = self.buffer.num_lines();
        let total_height = total_lines as f32 * line_height + line_height;

        egui::Frame::new()
            .fill(bg_color)
            .inner_margin(egui::Margin::ZERO)
            .show(ui, |ui| {
                // Remove default spacing between elements inside the editor frame
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                let available = ui.available_size();
                let (rect, response) =
                    ui.allocate_exact_size(available, egui::Sense::click_and_drag());

                // Show text cursor when hovering over the editor area; switch to pointer when Ctrl is held.
                if response.hovered() {
                    if ui.input(|i| i.modifiers.ctrl) {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        // Compute the word bounds under the mouse for the underline.
                        if let Some(hover_pos) = ui.input(|i| i.pointer.hover_pos()) {
                            let local = hover_pos - rect.min;
                            let row = ((local.y + self.scroll_offset.y) / line_height) as usize;
                            let row = row.min(self.buffer.num_lines().saturating_sub(1));
                            let x_in_text =
                                (local.x - gutter_width + self.scroll_offset.x).max(0.0);
                            let col = (x_in_text / char_width).round() as usize;
                            let col = col.min(self.buffer.line_len(row));

                            let line_chars: Vec<char> = self.buffer.line(row).chars().collect();
                            let is_sym = |ch: char| ch.is_alphanumeric() || ch == '_';
                            if col < line_chars.len() && is_sym(line_chars[col]) {
                                let mut start = col;
                                while start > 0 && is_sym(line_chars[start - 1]) {
                                    start -= 1;
                                }
                                let mut end = col + 1;
                                while end < line_chars.len() && is_sym(line_chars[end]) {
                                    end += 1;
                                }
                                self.ctrl_hover_word_bounds = Some((row, start, end));
                            } else {
                                self.ctrl_hover_word_bounds = None;
                            }
                        }
                    } else {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                        self.ctrl_hover_word_bounds = None;
                    }
                } else {
                    self.ctrl_hover_word_bounds = None;
                }

                // Hover-word detection for signature tooltip (only when Ctrl is NOT held).
                if response.hovered() && !ui.input(|i| i.modifiers.ctrl) {
                    if let Some(mouse_pos) = ui.input(|i| i.pointer.hover_pos()) {
                        let local = mouse_pos - rect.min;
                        let row = ((local.y + self.scroll_offset.y) / line_height) as usize;
                        let row = row.min(self.buffer.num_lines().saturating_sub(1));
                        let x_in_text = (local.x - gutter_width + self.scroll_offset.x).max(0.0);
                        let col = (x_in_text / char_width).round() as usize;
                        let col = col.min(self.buffer.line_len(row));

                        let word_now = get_word_at(&self.buffer, row, col);
                        let word_changed = word_now.as_deref() != self.hover_word.as_deref();
                        if word_changed {
                            if word_now.is_some() {
                                // Moved to a new word — immediately switch (cancel grace period).
                                self.hover_leave_instant = None;
                                self.hover_word = word_now;
                                self.hover_start = Some(std::time::Instant::now());
                                self.hover_signature = None;
                                self.hover_tooltip_anchor = None;
                                self.hover_lsp_request_pending = false;
                            } else {
                                // Moved to empty space — start grace period, keep popup visible.
                                if self.hover_leave_instant.is_none()
                                    && self.hover_signature.is_some()
                                {
                                    self.hover_leave_instant = Some(std::time::Instant::now());
                                } else if self.hover_signature.is_none() {
                                    self.hover_word = None;
                                    self.hover_start = None;
                                    self.hover_lsp_request_pending = false;
                                }
                            }
                        } else {
                            // Still on same word — cancel any pending dismissal.
                            self.hover_leave_instant = None;
                        }
                        self.hover_pos = mouse_pos;
                    }
                } else if !response.hovered() {
                    // Mouse left the editor — but keep tooltip if mouse is inside the popup.
                    let over_popup = ui.input(|i| {
                        i.pointer
                            .hover_pos()
                            .and_then(|p| self.hover_popup_rect.map(|r| r.contains(p)))
                            .unwrap_or(false)
                    });
                    if over_popup {
                        // Mouse is inside the popup — cancel grace period.
                        self.hover_leave_instant = None;
                    } else if self.hover_word.is_some() {
                        // Start grace period before clearing.
                        if self.hover_leave_instant.is_none() && self.hover_signature.is_some() {
                            self.hover_leave_instant = Some(std::time::Instant::now());
                        } else if self.hover_signature.is_none() {
                            self.hover_word = None;
                            self.hover_start = None;
                            self.hover_lsp_request_pending = false;
                            self.hover_popup_rect = None;
                        }
                    }
                }

                // Apply grace-period dismissal after 700 ms.
                const HOVER_DISMISS_MS: u128 = 700;
                if let Some(leave_t) = self.hover_leave_instant {
                    if leave_t.elapsed().as_millis() > HOVER_DISMISS_MS {
                        self.hover_word = None;
                        self.hover_start = None;
                        self.hover_signature = None;
                        self.hover_tooltip_anchor = None;
                        self.hover_lsp_request_pending = false;
                        self.hover_popup_rect = None;
                        self.hover_leave_instant = None;
                        // Request a repaint so the popup disappears promptly.
                        ui.ctx().request_repaint();
                    } else {
                        // Still in grace period — keep repainting.
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(
                                HOVER_DISMISS_MS as u64 + 16,
                            ));
                    }
                }

                // Diagnostic hover detection: check if mouse is over a diagnostic span.
                if response.hovered() && !ui.input(|i| i.modifiers.ctrl) {
                    if let Some(mouse_pos) = ui.input(|i| i.pointer.hover_pos()) {
                        let local = mouse_pos - rect.min;
                        let row = ((local.y + self.scroll_offset.y) / line_height) as usize;
                        let row = row.min(self.buffer.num_lines().saturating_sub(1));
                        let x_in_text = (local.x - gutter_width + self.scroll_offset.x).max(0.0);
                        let col = (x_in_text / char_width).round() as usize;
                        let mut found_diag = false;
                        for diag in &self.diagnostics {
                            if diag.line as usize == row {
                                let start = diag.col as usize;
                                let end = if diag.end_col > diag.col {
                                    diag.end_col as usize
                                } else {
                                    start + 1
                                };
                                if col >= start && col <= end {
                                    self.diag_hover_msg = Some(diag.message.clone());
                                    self.diag_hover_severity = diag.severity.clone();
                                    found_diag = true;
                                    break;
                                }
                            }
                        }
                        if !found_diag {
                            self.diag_hover_msg = None;
                        }
                    }
                } else if !response.hovered() {
                    self.diag_hover_msg = None;
                }

                // If an LSP hover response has arrived, apply it (overrides regex result).
                if let Some(lsp_sig) = lsp_hover {
                    if !lsp_sig.is_empty() {
                        self.hover_signature = Some(lsp_sig);
                    }
                    self.hover_lsp_request_pending = false;
                }

                // Resolve signature once the hover timer has fired.
                if self.hover_signature.is_none() && !self.hover_lsp_request_pending {
                    if let (Some(word), Some(start)) = (self.hover_word.clone(), self.hover_start) {
                        if start.elapsed() > std::time::Duration::from_millis(500) {
                            // Mark pending so the app sends an LSP hover request.
                            // Use mouse hover position, not text cursor position.
                            self.hover_lsp_request_pending = true;
                            let local = self.hover_pos - rect.min;
                            let h_row = ((local.y + self.scroll_offset.y) / line_height) as usize;
                            let h_row = h_row.min(self.buffer.num_lines().saturating_sub(1));
                            let x_in_text =
                                (local.x - gutter_width + self.scroll_offset.x).max(0.0);
                            let h_col = (x_in_text / char_width).round() as usize;
                            self.hover_row = h_row as u32;
                            self.hover_col = h_col.min(self.buffer.line_len(h_row)) as u32;
                            // Anchor the tooltip below the hovered word — fixed for this hover session.
                            let anchor_x =
                                rect.min.x + gutter_width + self.hover_col as f32 * char_width
                                    - self.scroll_offset.x;
                            let anchor_y = rect.min.y + (h_row + 1) as f32 * line_height
                                - self.scroll_offset.y
                                + 4.0;
                            self.hover_tooltip_anchor = Some(egui::pos2(anchor_x, anchor_y));
                            // Regex fallback: show something immediately while LSP responds.
                            let lang = self.highlighter.language.clone();
                            let content = self.buffer.to_string();
                            let sig = plugin_manager
                                .hover_info(&lang, &word, &content)
                                .or_else(|| self.lookup_signature_in_buffer(&word))
                                .or_else(|| self.lookup_signature_in_workspace(&word))
                                .unwrap_or_default();
                            if !sig.is_empty() {
                                self.hover_signature = Some(sig);
                            }
                        }
                    }
                }

                // Manage keyboard focus. The editor is the primary keyboard target
                // unless a focus-grabbing modal (find bar, goto-line) is open.
                // The autocomplete popup is NOT excluded: it's a non-focusable hover
                // Area and the editor itself handles its arrow/Enter navigation, so
                // the editor must KEEP focus while it's open (otherwise arrow keys
                // can leak focus away and break completion navigation).
                // We only REQUEST focus when needed — never re-request when we
                // already have it, to avoid disrupting egui's key event delivery.
                let has_focus = response.has_focus();
                if !has_focus && !self.show_find && !self.show_goto_line {
                    let explicit = std::mem::take(&mut self.focus_requested);
                    if response.clicked() || explicit || ui.memory(|m| m.focused().is_none()) {
                        ui.memory_mut(|m| m.request_focus(response.id));
                    }
                }

                // Capture Tab + arrow keys so egui's directional focus navigation
                // doesn't move focus out of the editor (into the file tree / terminal)
                // when the user presses an arrow key.
                if has_focus || response.has_focus() {
                    ui.memory_mut(|m| {
                        m.set_focus_lock_filter(
                            response.id,
                            egui::EventFilter {
                                tab: true,
                                horizontal_arrows: true,
                                vertical_arrows: true,
                                escape: false,
                            },
                        );
                    });
                }

                if has_focus || response.has_focus() {
                    ui.input(|i| {
                        let mut text_typed = false;
                        let mut ac_confirm = false;
                        let mut ac_dismiss = false;
                        let mut ac_nav = false;

                        for event in &i.events {
                            match event {
                                // Don't insert text when Ctrl is held (shortcuts)
                                egui::Event::Text(text)
                                    if !i.modifiers.ctrl && !i.modifiers.command =>
                                {
                                    // insert_char checkpoints once per typing run, so
                                    // Ctrl+Z undoes the whole run (not per char).
                                    let auto_close = config.editor.auto_close_brackets;
                                    for ch in text.chars() {
                                        self.insert_char(ch, auto_close);
                                        // Signature help: trigger on '(' or ','
                                        if ch == '(' || ch == ',' {
                                            let (row, col) = self.cursor.position();
                                            self.signature_help_request_pending = true;
                                            self.signature_help_row = row as u32;
                                            self.signature_help_col = col as u32;
                                        }
                                        // Clear signature help on ')'
                                        if ch == ')' {
                                            self.signature_help_text = None;
                                        }
                                        // LSP completion: auto-trigger on '.' (member access)
                                        if ch == '.' {
                                            let (row, col) = self.cursor.position();
                                            self.completion_request_pending = true;
                                            self.completion_trigger_row = row;
                                            self.completion_trigger_col = col;
                                        }
                                    }
                                    text_typed = true;
                                }
                                egui::Event::Paste(text) => {
                                    self.buffer.checkpoint();
                                    // The paste is its own undo step: let its chars
                                    // join the checkpoint above, not open a new one.
                                    self.typing_burst = Some(self.typing_burst_state());
                                    let cursor_count = 1 + self.extra_cursors.len();
                                    let lines: Vec<&str> = text.lines().collect();

                                    if cursor_count > 1 && lines.len() == cursor_count {
                                        // Collect all cursor positions sorted top-to-bottom
                                        let mut positions: Vec<(usize, usize, bool)> = vec![];
                                        let (mr, mc) = self.cursor.position();
                                        positions.push((mr, mc, true));
                                        for ec in &self.extra_cursors {
                                            let (er, ec_col) = ec.position();
                                            positions.push((er, ec_col, false));
                                        }
                                        positions.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

                                        // Insert in reverse order to preserve positions
                                        for (sorted_idx, &(row, col, _)) in
                                            positions.iter().enumerate().rev()
                                        {
                                            let line_text = lines[sorted_idx];
                                            for (j, ch) in line_text.chars().enumerate() {
                                                self.buffer.insert_char(row, col + j, ch);
                                            }
                                        }
                                        // Update cursor positions (forward order)
                                        for (sorted_idx, &(row, col, is_main)) in
                                            positions.iter().enumerate()
                                        {
                                            let new_col = col + lines[sorted_idx].chars().count();
                                            if is_main {
                                                self.cursor.set_position(row, new_col);
                                            } else {
                                                for ec in &mut self.extra_cursors {
                                                    let (er, ec_col) = ec.position();
                                                    if er == row && ec_col == col {
                                                        ec.set_position(row, new_col);
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                        self.is_modified = true;
                                        self.content_version = self.content_version.wrapping_add(1);
                                    } else {
                                        // Default: paste full text at each cursor
                                        for ch in text.chars() {
                                            if ch == '\n' {
                                                self.insert_newline();
                                            } else {
                                                self.insert_char(ch, false);
                                            }
                                        }
                                    }
                                    // Typing after a paste starts a new undo step.
                                    self.typing_burst = None;
                                    text_typed = true;
                                }
                                egui::Event::Key {
                                    key,
                                    pressed: true,
                                    modifiers,
                                    ..
                                } => {
                                    // Any non-text key ends the current typing run, so
                                    // the next typed run becomes its own undo step.
                                    self.typing_burst = None;
                                    match key {
                                        egui::Key::Enter if modifiers.ctrl && modifiers.shift => {
                                            self.buffer.checkpoint();
                                            let row = self.cursor.row;
                                            self.buffer.split_line(row, 0);
                                            self.cursor.col = 0;
                                            self.cursor.clear_selection();
                                            self.extra_cursors.clear();
                                            self.is_modified = true;
                                            self.content_version =
                                                self.content_version.wrapping_add(1);
                                        }
                                        egui::Key::Enter if modifiers.ctrl => {
                                            self.buffer.checkpoint();
                                            let row = self.cursor.row;
                                            let line_len = self.buffer.line_len(row);
                                            self.buffer.split_line(row, line_len);
                                            self.cursor.row += 1;
                                            self.cursor.col = 0;
                                            self.cursor.clear_selection();
                                            self.extra_cursors.clear();
                                            self.is_modified = true;
                                            self.content_version =
                                                self.content_version.wrapping_add(1);
                                        }
                                        egui::Key::Enter => {
                                            if self.autocomplete.visible {
                                                ac_confirm = true;
                                                ac_nav = true;
                                            } else {
                                                self.insert_newline();
                                            }
                                        }
                                        // delete_char_before/after push their own checkpoint.
                                        egui::Key::Backspace => {
                                            self.delete_char_before();
                                            text_typed = true;
                                        }
                                        egui::Key::Delete => {
                                            self.delete_char_after();
                                            text_typed = true;
                                        }

                                        egui::Key::ArrowUp if modifiers.ctrl && modifiers.shift => {
                                            let row = self.cursor.row;
                                            if row > 0 {
                                                self.buffer.checkpoint();
                                                let current = self.buffer.line(row);
                                                let above = self.buffer.line(row - 1);
                                                self.buffer.replace_line(row, &above);
                                                self.buffer.replace_line(row - 1, &current);
                                                self.cursor.row -= 1;
                                                self.is_modified = true;
                                                self.content_version =
                                                    self.content_version.wrapping_add(1);
                                            }
                                        }
                                        egui::Key::ArrowDown
                                            if modifiers.ctrl && modifiers.shift =>
                                        {
                                            let row = self.cursor.row;
                                            if row + 1 < self.buffer.num_lines() {
                                                self.buffer.checkpoint();
                                                let current = self.buffer.line(row);
                                                let below = self.buffer.line(row + 1);
                                                self.buffer.replace_line(row, &below);
                                                self.buffer.replace_line(row + 1, &current);
                                                self.cursor.row += 1;
                                                self.is_modified = true;
                                                self.content_version =
                                                    self.content_version.wrapping_add(1);
                                            }
                                        }
                                        // Alt+Up — move current line up (VSCode: Alt+Up)
                                        egui::Key::ArrowUp
                                            if modifiers.alt
                                                && !modifiers.shift
                                                && !modifiers.ctrl =>
                                        {
                                            let row = self.cursor.row;
                                            if row > 0 {
                                                self.buffer.checkpoint();
                                                let current = self.buffer.line(row);
                                                let above = self.buffer.line(row - 1);
                                                self.buffer.replace_line(row, &above);
                                                self.buffer.replace_line(row - 1, &current);
                                                self.cursor.row -= 1;
                                                self.is_modified = true;
                                                self.content_version =
                                                    self.content_version.wrapping_add(1);
                                            }
                                        }
                                        // Alt+Down — move current line down (VSCode: Alt+Down)
                                        egui::Key::ArrowDown
                                            if modifiers.alt
                                                && !modifiers.shift
                                                && !modifiers.ctrl =>
                                        {
                                            let row = self.cursor.row;
                                            if row + 1 < self.buffer.num_lines() {
                                                self.buffer.checkpoint();
                                                let current = self.buffer.line(row);
                                                let below = self.buffer.line(row + 1);
                                                self.buffer.replace_line(row, &below);
                                                self.buffer.replace_line(row + 1, &current);
                                                self.cursor.row += 1;
                                                self.is_modified = true;
                                                self.content_version =
                                                    self.content_version.wrapping_add(1);
                                            }
                                        }
                                        // Shift+Alt+Up — duplicate line above (VSCode: Shift+Alt+Up)
                                        egui::Key::ArrowUp
                                            if modifiers.alt
                                                && modifiers.shift
                                                && !modifiers.ctrl =>
                                        {
                                            let row = self.cursor.row;
                                            self.buffer.checkpoint();
                                            let line = self.buffer.line(row);
                                            self.buffer.insert_line(row, &line);
                                            // cursor stays on original (now row+1), but we want it on the duplicate above
                                            self.is_modified = true;
                                            self.content_version =
                                                self.content_version.wrapping_add(1);
                                        }
                                        // Shift+Alt+Down — duplicate line below (VSCode: Shift+Alt+Down)
                                        egui::Key::ArrowDown
                                            if modifiers.alt
                                                && modifiers.shift
                                                && !modifiers.ctrl =>
                                        {
                                            self.duplicate_line();
                                        }
                                        // Ctrl+Alt+Up — add cursor above (VSCode: Ctrl+Alt+Up)
                                        egui::Key::ArrowUp if modifiers.alt && modifiers.ctrl => {
                                            let mut new_extras: Vec<Cursor> = vec![];
                                            let (pr, pc) = self.cursor.position();
                                            if pr > 0 {
                                                let mut c = Cursor::new();
                                                c.set_position(
                                                    pr - 1,
                                                    pc.min(self.buffer.line_len(pr - 1)),
                                                );
                                                new_extras.push(c);
                                            }
                                            for ec in &self.extra_cursors {
                                                let (er, ec_col) = ec.position();
                                                if er > 0 {
                                                    let mut c = Cursor::new();
                                                    c.set_position(
                                                        er - 1,
                                                        ec_col.min(self.buffer.line_len(er - 1)),
                                                    );
                                                    new_extras.push(c);
                                                }
                                            }
                                            self.extra_cursors.extend(new_extras);
                                            self.dedup_cursors();
                                        }
                                        // Ctrl+Alt+Down — add cursor below (VSCode: Ctrl+Alt+Down)
                                        egui::Key::ArrowDown if modifiers.alt && modifiers.ctrl => {
                                            let mut new_extras: Vec<Cursor> = vec![];
                                            let (pr, pc) = self.cursor.position();
                                            if pr + 1 < self.buffer.num_lines() {
                                                let mut c = Cursor::new();
                                                c.set_position(
                                                    pr + 1,
                                                    pc.min(self.buffer.line_len(pr + 1)),
                                                );
                                                new_extras.push(c);
                                            }
                                            for ec in &self.extra_cursors {
                                                let (er, ec_col) = ec.position();
                                                if er + 1 < self.buffer.num_lines() {
                                                    let mut c = Cursor::new();
                                                    c.set_position(
                                                        er + 1,
                                                        ec_col.min(self.buffer.line_len(er + 1)),
                                                    );
                                                    new_extras.push(c);
                                                }
                                            }
                                            self.extra_cursors.extend(new_extras);
                                            self.dedup_cursors();
                                        }

                                        // Autocomplete navigation
                                        egui::Key::ArrowUp if self.autocomplete.visible => {
                                            self.autocomplete.move_up();
                                            ac_nav = true;
                                        }
                                        egui::Key::ArrowDown if self.autocomplete.visible => {
                                            self.autocomplete.move_down();
                                            ac_nav = true;
                                        }

                                        egui::Key::ArrowLeft if modifiers.shift => {
                                            self.cursor.move_left_select(&self.buffer);
                                            for ec in &mut self.extra_cursors {
                                                ec.move_left_select(&self.buffer);
                                            }
                                        }
                                        egui::Key::ArrowRight if modifiers.shift => {
                                            self.cursor.move_right_select(&self.buffer);
                                            for ec in &mut self.extra_cursors {
                                                ec.move_right_select(&self.buffer);
                                            }
                                        }
                                        egui::Key::ArrowUp if modifiers.shift => {
                                            self.cursor.move_up_select(&self.buffer);
                                            for ec in &mut self.extra_cursors {
                                                ec.move_up_select(&self.buffer);
                                            }
                                        }
                                        egui::Key::ArrowDown if modifiers.shift => {
                                            self.cursor.move_down_select(&self.buffer);
                                            for ec in &mut self.extra_cursors {
                                                ec.move_down_select(&self.buffer);
                                            }
                                        }

                                        egui::Key::ArrowLeft => {
                                            self.cursor.move_left(&self.buffer);
                                            for ec in &mut self.extra_cursors {
                                                ec.move_left(&self.buffer);
                                            }
                                            self.dedup_cursors();
                                            ac_dismiss = true;
                                        }
                                        egui::Key::ArrowRight => {
                                            self.cursor.move_right(&self.buffer);
                                            for ec in &mut self.extra_cursors {
                                                ec.move_right(&self.buffer);
                                            }
                                            self.dedup_cursors();
                                            ac_dismiss = true;
                                        }
                                        egui::Key::ArrowUp => {
                                            self.cursor.move_up(&self.buffer);
                                            for ec in &mut self.extra_cursors {
                                                ec.move_up(&self.buffer);
                                            }
                                            self.dedup_cursors();
                                            ac_dismiss = true;
                                        }
                                        egui::Key::ArrowDown => {
                                            self.cursor.move_down(&self.buffer);
                                            for ec in &mut self.extra_cursors {
                                                ec.move_down(&self.buffer);
                                            }
                                            self.dedup_cursors();
                                            ac_dismiss = true;
                                        }

                                        egui::Key::Home if modifiers.ctrl && modifiers.shift => {
                                            self.cursor.start_selection();
                                            self.cursor.set_position(0, 0);
                                            self.scroll_offset = egui::Vec2::ZERO;
                                            for ec in &mut self.extra_cursors {
                                                ec.start_selection();
                                                ec.set_position(0, 0);
                                            }
                                            self.dedup_cursors();
                                            ac_dismiss = true;
                                        }
                                        egui::Key::End if modifiers.ctrl && modifiers.shift => {
                                            self.cursor.start_selection();
                                            let last = self.buffer.num_lines().saturating_sub(1);
                                            self.cursor
                                                .set_position(last, self.buffer.line_len(last));
                                            for ec in &mut self.extra_cursors {
                                                ec.start_selection();
                                                ec.set_position(last, self.buffer.line_len(last));
                                            }
                                            self.dedup_cursors();
                                            ac_dismiss = true;
                                        }
                                        egui::Key::Home if modifiers.ctrl => {
                                            self.cursor.clear_selection();
                                            self.cursor.set_position(0, 0);
                                            self.scroll_offset = egui::Vec2::ZERO;
                                            for ec in &mut self.extra_cursors {
                                                ec.clear_selection();
                                                ec.set_position(0, 0);
                                            }
                                            self.dedup_cursors();
                                            ac_dismiss = true;
                                        }
                                        egui::Key::End if modifiers.ctrl => {
                                            self.cursor.clear_selection();
                                            let last = self.buffer.num_lines().saturating_sub(1);
                                            self.cursor
                                                .set_position(last, self.buffer.line_len(last));
                                            for ec in &mut self.extra_cursors {
                                                ec.clear_selection();
                                                ec.set_position(last, self.buffer.line_len(last));
                                            }
                                            self.dedup_cursors();
                                            ac_dismiss = true;
                                        }
                                        egui::Key::Home if modifiers.shift => {
                                            let (row, _) = self.cursor.position();
                                            self.cursor.start_selection();
                                            self.cursor.set_position(row, 0);
                                            for ec in &mut self.extra_cursors {
                                                let (er, _) = ec.position();
                                                ec.start_selection();
                                                ec.set_position(er, 0);
                                            }
                                            self.dedup_cursors();
                                            ac_dismiss = true;
                                        }
                                        egui::Key::End if modifiers.shift => {
                                            let (row, _) = self.cursor.position();
                                            self.cursor.start_selection();
                                            self.cursor
                                                .set_position(row, self.buffer.line_len(row));
                                            for ec in &mut self.extra_cursors {
                                                let (er, _) = ec.position();
                                                ec.start_selection();
                                                ec.set_position(er, self.buffer.line_len(er));
                                            }
                                            self.dedup_cursors();
                                            ac_dismiss = true;
                                        }
                                        egui::Key::Home => {
                                            let (row, _) = self.cursor.position();
                                            self.cursor.clear_selection();
                                            self.cursor.set_position(row, 0);
                                            for ec in &mut self.extra_cursors {
                                                let (er, _) = ec.position();
                                                ec.clear_selection();
                                                ec.set_position(er, 0);
                                            }
                                            self.dedup_cursors();
                                            ac_dismiss = true;
                                        }
                                        egui::Key::End => {
                                            let (row, _) = self.cursor.position();
                                            self.cursor.clear_selection();
                                            self.cursor
                                                .set_position(row, self.buffer.line_len(row));
                                            for ec in &mut self.extra_cursors {
                                                let (er, _) = ec.position();
                                                ec.clear_selection();
                                                ec.set_position(er, self.buffer.line_len(er));
                                            }
                                            self.dedup_cursors();
                                            ac_dismiss = true;
                                        }
                                        egui::Key::Tab => {
                                            if self.autocomplete.visible {
                                                ac_confirm = true;
                                                ac_nav = true;
                                            } else if !modifiers.shift {
                                                if self.detected_indent_spaces {
                                                    for _ in 0..self.detected_indent_size {
                                                        self.insert_char(' ', false);
                                                    }
                                                } else {
                                                    self.insert_char('\t', false);
                                                }
                                                text_typed = true;
                                            }
                                        }

                                        // Save
                                        egui::Key::S if modifiers.ctrl => {
                                            let _ = self.save();
                                            self.just_saved = true;
                                        }

                                        // Undo
                                        egui::Key::Z if modifiers.ctrl && !modifiers.shift => {
                                            self.buffer.undo();
                                            self.is_modified = true;
                                            self.content_version =
                                                self.content_version.wrapping_add(1);
                                        }
                                        // Redo (Ctrl+Shift+Z or Ctrl+Y)
                                        egui::Key::Z if modifiers.ctrl && modifiers.shift => {
                                            self.buffer.redo();
                                            self.is_modified = true;
                                            self.content_version =
                                                self.content_version.wrapping_add(1);
                                        }
                                        egui::Key::Y if modifiers.ctrl => {
                                            self.buffer.redo();
                                            self.is_modified = true;
                                            self.content_version =
                                                self.content_version.wrapping_add(1);
                                        }

                                        // Select All
                                        egui::Key::A if modifiers.ctrl => {
                                            let last_row =
                                                self.buffer.num_lines().saturating_sub(1);
                                            let last_col = self.buffer.line_len(last_row);
                                            self.cursor.sel_anchor = Some((0, 0));
                                            self.cursor.set_position(last_row, last_col);
                                            // Restore anchor (set_position clears desired_col but not anchor)
                                            self.cursor.sel_anchor = Some((0, 0));
                                        }

                                        // Ctrl+D — select word then find next occurrence (VSCode style)
                                        egui::Key::D if modifiers.ctrl && !modifiers.shift => {
                                            if let Some(word) = self.current_word_full() {
                                                let word_len = word.chars().count();
                                                if !self.cursor.has_selection() {
                                                    // First press: select the word under cursor
                                                    let (row, col) = self.cursor.position();
                                                    let line = self.buffer.line(row);
                                                    let chars: Vec<char> = line.chars().collect();
                                                    let c = col.min(chars.len());
                                                    let mut start = c;
                                                    while start > 0
                                                        && (chars[start - 1].is_alphanumeric()
                                                            || chars[start - 1] == '_')
                                                    {
                                                        start -= 1;
                                                    }
                                                    self.cursor.set_position(row, start + word_len);
                                                    self.cursor.sel_anchor = Some((row, start));
                                                } else {
                                                    // Subsequent presses: keep main cursor at original
                                                    // occurrence, add extra cursor at next match.
                                                    // Search from the last extra cursor (or main if none).
                                                    let (search_row, search_col) =
                                                        if let Some(last) =
                                                            self.extra_cursors.last()
                                                        {
                                                            last.position()
                                                        } else {
                                                            self.cursor.position()
                                                        };
                                                    if let Some((match_row, match_col)) =
                                                        find_next_occurrence(
                                                            &self.buffer,
                                                            &word,
                                                            search_row,
                                                            search_col,
                                                        )
                                                    {
                                                        let mut extra = Cursor::new();
                                                        extra.row = match_row;
                                                        extra.col = match_col + word_len;
                                                        extra.desired_col = extra.col;
                                                        extra.sel_anchor =
                                                            Some((match_row, match_col));
                                                        self.extra_cursors.push(extra);
                                                        self.dedup_cursors();
                                                    }
                                                }
                                            }
                                        }

                                        // Ctrl+Shift+L — select ALL occurrences of current word/selection
                                        egui::Key::L if modifiers.ctrl && modifiers.shift => {
                                            if let Some(word) = self.current_word_full() {
                                                let word_len = word.chars().count();
                                                self.extra_cursors.clear();
                                                // Select the word with the primary cursor first
                                                let (row, col) = self.cursor.position();
                                                let line = self.buffer.line(row);
                                                let chars: Vec<char> = line.chars().collect();
                                                let c = col.min(chars.len());
                                                let mut start = c;
                                                while start > 0
                                                    && (chars[start - 1].is_alphanumeric()
                                                        || chars[start - 1] == '_')
                                                {
                                                    start -= 1;
                                                }
                                                self.cursor.set_position(row, start + word_len);
                                                self.cursor.sel_anchor = Some((row, start));
                                                // Add extra cursors for every other occurrence
                                                let mut search_row = 0;
                                                let mut search_col = 0;
                                                while let Some((mr, mc)) = find_next_occurrence(
                                                    &self.buffer,
                                                    &word,
                                                    search_row,
                                                    search_col,
                                                ) {
                                                    // Stop once the search wraps around (the match
                                                    // lies before the search position).
                                                    if (mr, mc) < (search_row, search_col) {
                                                        break;
                                                    }
                                                    // Skip the primary cursor's occurrence
                                                    if !(mr == row && mc == start) {
                                                        let mut extra = Cursor::new();
                                                        extra.row = mr;
                                                        extra.col = mc + word_len;
                                                        extra.desired_col = extra.col;
                                                        extra.sel_anchor = Some((mr, mc));
                                                        self.extra_cursors.push(extra);
                                                    }
                                                    // Advance past this match
                                                    search_col = mc + word_len;
                                                    search_row = mr;
                                                    if search_col > self.buffer.line_len(search_row)
                                                    {
                                                        search_row += 1;
                                                        search_col = 0;
                                                        if search_row >= self.buffer.num_lines() {
                                                            break;
                                                        }
                                                    }
                                                }
                                                self.dedup_cursors();
                                            }
                                        }

                                        // Copy (Ctrl+C)
                                        egui::Key::C if modifiers.ctrl => {
                                            // handled below via output_mut – skip here since we need ui
                                        }
                                        // Cut (Ctrl+X)
                                        egui::Key::X if modifiers.ctrl => {
                                            // handled below
                                        }

                                        // Trigger LSP completions (Ctrl+Space)
                                        egui::Key::Space if modifiers.ctrl => {
                                            let (row, col) = self.cursor.position();
                                            self.completion_request_pending = true;
                                            self.completion_trigger_row = row;
                                            self.completion_trigger_col = col;
                                        }

                                        // F2 — rename symbol (handled by app)
                                        egui::Key::F2 => {
                                            // Signal to app; app handles the dialog
                                        }

                                        // Format document (Ctrl+Shift+F → LSP)
                                        egui::Key::F if modifiers.ctrl && modifiers.shift => {
                                            self.format_request_pending = true;
                                        }

                                        // Find
                                        egui::Key::F if modifiers.ctrl => {
                                            self.show_find = true;
                                        }

                                        // Find & Replace (Ctrl+H)
                                        egui::Key::H if modifiers.ctrl => {
                                            self.show_find = true;
                                            self.show_replace = true;
                                        }

                                        // Go to line (Ctrl+G)
                                        egui::Key::G if modifiers.ctrl => {
                                            self.show_goto_line = true;
                                            self.goto_line_input.clear();
                                        }

                                        // Toggle line comment (Ctrl+/)
                                        egui::Key::Slash if modifiers.ctrl => {
                                            self.buffer.checkpoint();
                                            let prefix = self.comment_prefix();
                                            let prefix_len = prefix.chars().count();
                                            let prefix_trim = prefix.trim_end();
                                            let rows = self.selected_line_rows();
                                            let all_commented = rows.iter().all(|&row| {
                                                let line = self.buffer.line(row);
                                                let leading = line
                                                    .chars()
                                                    .take_while(|c| c.is_whitespace())
                                                    .count();
                                                let byte_offset = line
                                                    .char_indices()
                                                    .nth(leading)
                                                    .map(|(i, _)| i)
                                                    .unwrap_or(line.len());
                                                line[byte_offset..].starts_with(prefix_trim)
                                            });
                                            for row in rows.iter().rev() {
                                                let line = self.buffer.line(*row);
                                                let leading = line
                                                    .chars()
                                                    .take_while(|c| c.is_whitespace())
                                                    .count();
                                                let byte_offset = line
                                                    .char_indices()
                                                    .nth(leading)
                                                    .map(|(i, _)| i)
                                                    .unwrap_or(line.len());
                                                if all_commented {
                                                    let n = if line[byte_offset..]
                                                        .starts_with(prefix)
                                                    {
                                                        prefix_len
                                                    } else {
                                                        prefix_trim.chars().count()
                                                    };
                                                    for _ in 0..n {
                                                        self.buffer.delete_char(*row, leading);
                                                    }
                                                } else {
                                                    let pfx_chars: Vec<char> =
                                                        prefix.chars().collect();
                                                    for (i, &ch) in pfx_chars.iter().enumerate() {
                                                        self.buffer.insert_char(*row, i, ch);
                                                    }
                                                }
                                            }
                                            self.is_modified = true;
                                            self.content_version =
                                                self.content_version.wrapping_add(1);
                                        }

                                        // Duplicate current line (Ctrl+Shift+D)
                                        egui::Key::D if modifiers.ctrl && modifiers.shift => {
                                            self.duplicate_line();
                                        }

                                        // Delete current line(s) (Ctrl+Shift+K)
                                        egui::Key::K if modifiers.ctrl && modifiers.shift => {
                                            self.buffer.checkpoint();
                                            let mut rows = self.all_cursor_rows();
                                            rows.sort_unstable();
                                            rows.dedup();
                                            for row in rows.iter().rev() {
                                                self.buffer.delete_line(*row);
                                            }
                                            let max_row = self.buffer.num_lines().saturating_sub(1);
                                            self.cursor.row = self.cursor.row.min(max_row);
                                            self.cursor.col = 0;
                                            self.extra_cursors.clear();
                                            self.is_modified = true;
                                            self.content_version =
                                                self.content_version.wrapping_add(1);
                                        }

                                        // Indent selected/current lines (Ctrl+])
                                        egui::Key::CloseBracket if modifiers.ctrl => {
                                            self.buffer.checkpoint();
                                            let rows = self.selected_line_rows();
                                            for row in &rows {
                                                for i in 0..4 {
                                                    self.buffer.insert_char(*row, i, ' ');
                                                }
                                            }
                                            self.cursor.col = self.cursor.col.saturating_add(4);
                                            self.cursor.desired_col = self.cursor.col;
                                            self.is_modified = true;
                                            self.content_version =
                                                self.content_version.wrapping_add(1);
                                        }

                                        // Unindent selected/current lines (Ctrl+[)
                                        egui::Key::OpenBracket if modifiers.ctrl => {
                                            self.buffer.checkpoint();
                                            let rows = self.selected_line_rows();
                                            for row in &rows {
                                                let line = self.buffer.line(*row);
                                                let spaces: usize = line
                                                    .chars()
                                                    .take(4)
                                                    .take_while(|&c| c == ' ')
                                                    .count();
                                                for _ in 0..spaces {
                                                    self.buffer.delete_char(*row, 0);
                                                }
                                                if *row == self.cursor.row {
                                                    self.cursor.col =
                                                        self.cursor.col.saturating_sub(spaces);
                                                    self.cursor.desired_col = self.cursor.col;
                                                }
                                            }
                                            self.is_modified = true;
                                            self.content_version =
                                                self.content_version.wrapping_add(1);
                                        }

                                        egui::Key::Escape => {
                                            if self.autocomplete.visible {
                                                ac_dismiss = true;
                                                ac_nav = true;
                                            } else {
                                                if self.show_find {
                                                    self.show_find = false;
                                                    self.find_query.clear();
                                                    self.find_matches.clear();
                                                }
                                                if self.show_goto_line {
                                                    self.show_goto_line = false;
                                                    self.goto_line_input.clear();
                                                }
                                                self.extra_cursors.clear();
                                                self.cursor.clear_selection();
                                            }
                                        }

                                        _ => {}
                                    }
                                }
                                _ => {}
                            }
                        }

                        // Apply autocomplete state changes.
                        if ac_confirm {
                            self.confirm_autocomplete();
                        } else if ac_dismiss {
                            self.autocomplete.visible = false;
                        } else if text_typed {
                            self.trigger_autocomplete_update();
                        }
                        // ac_nav without confirm/dismiss means navigation — keep visible.
                        let _ = ac_nav;

                        // Handle copy/cut here so we have access to the full event list
                        // (these need separate ui.output_mut calls)
                    });

                    // Copy / Cut (need ui for output_mut)
                    // egui-winit turns Ctrl+C / Ctrl+X into Event::Copy / Event::Cut
                    // and emits no Key event for them; the Key arms cover other backends.
                    let do_copy = ui.input(|i| {
                        i.events.iter().any(|e| {
                            matches!(
                                e,
                                egui::Event::Copy
                                    | egui::Event::Key {
                                        key: egui::Key::C,
                                        pressed: true,
                                        modifiers: egui::Modifiers { ctrl: true, .. },
                                        ..
                                    }
                            )
                        })
                    });
                    let do_cut = ui.input(|i| {
                        i.events.iter().any(|e| {
                            matches!(
                                e,
                                egui::Event::Cut
                                    | egui::Event::Key {
                                        key: egui::Key::X,
                                        pressed: true,
                                        modifiers: egui::Modifiers { ctrl: true, .. },
                                        ..
                                    }
                            )
                        })
                    });

                    if do_copy {
                        if !self.extra_cursors.is_empty() {
                            let mut parts: Vec<String> = vec![];
                            // Main cursor
                            if let Some(((sr, sc), (er, ec))) = self.cursor.selection_range() {
                                if sr == er {
                                    let line = self.buffer.line(sr);
                                    parts.push(line.chars().skip(sc).take(ec - sc).collect());
                                } else {
                                    // Multi-line selection: collect all lines
                                    let mut text = String::new();
                                    for r in sr..=er {
                                        let l = self.buffer.line(r);
                                        if r == sr {
                                            text.push_str(&l.chars().skip(sc).collect::<String>());
                                        } else if r == er {
                                            text.push('\n');
                                            text.push_str(&l.chars().take(ec).collect::<String>());
                                        } else {
                                            text.push('\n');
                                            text.push_str(&l);
                                        }
                                    }
                                    parts.push(text);
                                }
                            } else {
                                parts.push(self.buffer.line(self.cursor.row).to_string());
                            }
                            // Extra cursors
                            for extra in &self.extra_cursors {
                                if let Some(((sr, sc), (er, ec))) = extra.selection_range() {
                                    if sr == er {
                                        let line = self.buffer.line(sr);
                                        parts.push(line.chars().skip(sc).take(ec - sc).collect());
                                    } else {
                                        parts.push(self.buffer.line(sr).to_string());
                                    }
                                } else {
                                    parts.push(self.buffer.line(extra.row).to_string());
                                }
                            }
                            ui.ctx().copy_text(parts.join("\n"));
                        } else {
                            let text = self.selected_text().unwrap_or_else(|| {
                                let (row, _) = self.cursor.position();
                                self.buffer.line(row) + "\n"
                            });
                            ui.ctx().copy_text(text);
                        }
                    }
                    if do_cut {
                        if let Some(text) = self.selected_text() {
                            ui.ctx().copy_text(text);
                            self.buffer.checkpoint();
                            self.delete_selection();
                        }
                    }
                }

                let mut double_click_handled = false;

                if response.double_clicked() {
                    self.extra_cursors.clear();
                    if let Some(pos) = response.interact_pointer_pos() {
                        let (row, col) = {
                            let local = pos - rect.min;
                            let r = ((local.y + self.scroll_offset.y) / line_height) as usize;
                            let r = r.min(self.buffer.num_lines().saturating_sub(1));
                            let x_in_text =
                                (local.x - gutter_width + self.scroll_offset.x).max(0.0);
                            let c = (x_in_text / char_width).round() as usize;
                            let c = c.min(self.buffer.line_len(r));
                            (r, c)
                        };
                        let line_chars: Vec<char> = self.buffer.line(row).chars().collect();
                        let c = col.min(line_chars.len());
                        let is_word = |ch: char| ch.is_alphanumeric() || ch == '_';
                        if c < line_chars.len() && is_word(line_chars[c]) {
                            let mut start = c;
                            while start > 0 && is_word(line_chars[start - 1]) {
                                start -= 1;
                            }
                            let mut end = c;
                            while end < line_chars.len() && is_word(line_chars[end]) {
                                end += 1;
                            }
                            self.cursor.sel_anchor = Some((row, start));
                            self.cursor.set_position(row, end);
                        }
                    }
                    double_click_handled = true;
                }

                // Triple-click selects the whole line (incl. trailing newline when
                // there's a line below), like most editors.
                if response.triple_clicked() {
                    self.extra_cursors.clear();
                    if let Some(pos) = response.interact_pointer_pos() {
                        let local = pos - rect.min;
                        let r = ((local.y + self.scroll_offset.y) / line_height) as usize;
                        let r = r.min(self.buffer.num_lines().saturating_sub(1));
                        self.cursor.sel_anchor = Some((r, 0));
                        if r + 1 < self.buffer.num_lines() {
                            self.cursor.set_position(r + 1, 0);
                        } else {
                            self.cursor.set_position(r, self.buffer.line_len(r));
                        }
                    }
                    double_click_handled = true;
                }

                if response.drag_started() && !double_click_handled {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let (row, col) = {
                            let local = pos - rect.min;
                            let r = ((local.y + self.scroll_offset.y) / line_height) as usize;
                            let r = r.min(self.buffer.num_lines().saturating_sub(1));
                            let x_in_text =
                                (local.x - gutter_width + self.scroll_offset.x).max(0.0);
                            let c = (x_in_text / char_width).round() as usize;
                            let c = c.min(self.buffer.line_len(r));
                            (r, c)
                        };
                        if ui.input(|i| i.modifiers.ctrl) {
                            let mut extra = Cursor::new();
                            extra.set_position(row, col);
                            self.extra_cursors.push(extra);
                        } else if ui.input(|i| i.modifiers.shift) {
                            if self.cursor.sel_anchor.is_none() {
                                self.cursor.sel_anchor = Some(self.cursor.position());
                            }
                            self.cursor.set_position(row, col);
                        } else {
                            self.extra_cursors.clear();
                            self.cursor.clear_selection();
                            self.cursor.set_position(row, col);
                            self.cursor.sel_anchor = Some((row, col));
                        }
                        self.autocomplete.visible = false;
                    }
                }

                if response.dragged() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let (row, col) = {
                            let local = pos - rect.min;
                            let r = ((local.y + self.scroll_offset.y) / line_height) as usize;
                            let r = r.min(self.buffer.num_lines().saturating_sub(1));
                            let x_in_text =
                                (local.x - gutter_width + self.scroll_offset.x).max(0.0);
                            let c = (x_in_text / char_width).round() as usize;
                            let c = c.min(self.buffer.line_len(r));
                            (r, c)
                        };
                        self.cursor.set_position(row, col);
                        if self.cursor.sel_anchor.is_none() {
                            self.cursor.sel_anchor = Some((row, col));
                        }
                        // Auto-scroll when dragging near edges
                        let margin = line_height * 2.0;
                        let local_y = pos.y - rect.min.y;
                        if local_y < margin {
                            self.scroll_offset.y = (self.scroll_offset.y - line_height).max(0.0);
                        } else if local_y > rect.height() - margin {
                            self.scroll_offset.y = (self.scroll_offset.y + line_height)
                                .min((total_height - rect.height()).max(0.0));
                        }
                    }
                }

                if response.drag_stopped() && !double_click_handled {
                    if let Some(anchor) = self.cursor.sel_anchor {
                        if anchor == self.cursor.position() {
                            self.cursor.clear_selection();
                        }
                    }
                }

                if response.clicked() && !response.dragged() && !double_click_handled {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let (row, col) = {
                            let local = pos - rect.min;
                            let r = ((local.y + self.scroll_offset.y) / line_height) as usize;
                            let r = r.min(self.buffer.num_lines().saturating_sub(1));
                            let x_in_text =
                                (local.x - gutter_width + self.scroll_offset.x).max(0.0);
                            let c = (x_in_text / char_width).round() as usize;
                            let c = c.min(self.buffer.line_len(r));
                            (r, c)
                        };
                        if ui.input(|i| i.modifiers.ctrl) {
                            // Ctrl+click: navigate to definition of the word under the pointer.
                            // Move cursor to the clicked position first so LSP uses the right location.
                            if let Some(word) = get_word_at(&self.buffer, row, col) {
                                self.cursor.set_position(row, col);
                                self.go_to_definition_request = Some(word);
                            }
                        } else if ui.input(|i| i.modifiers.shift) {
                            if self.cursor.sel_anchor.is_none() {
                                self.cursor.sel_anchor = Some(self.cursor.position());
                            }
                            self.cursor.set_position(row, col);
                        } else {
                            self.extra_cursors.clear();
                            self.cursor.clear_selection();
                            self.cursor.set_position(row, col);
                        }
                        self.autocomplete.visible = false;
                    }
                }

                if response.hovered() {
                    // Horizontal scroll bound = longest line width minus the viewport.
                    let max_x = {
                        let content_w =
                            self.cached_max_line_chars() as f32 * char_width + gutter_width + 40.0;
                        (content_w - rect.width()).max(0.0)
                    };
                    ui.input(|i| {
                        let mut dx = i.smooth_scroll_delta.x;
                        let mut dy = i.smooth_scroll_delta.y;
                        // Shift+wheel scrolls horizontally — on Windows the wheel
                        // delta arrives on the Y axis even with Shift held, so remap.
                        if i.modifiers.shift && dx == 0.0 {
                            dx = dy;
                            dy = 0.0;
                        }
                        self.scroll_offset.y -= dy;
                        self.scroll_offset.y = self
                            .scroll_offset
                            .y
                            .max(0.0)
                            .min((total_height - rect.height()).max(0.0));
                        self.scroll_offset.x = (self.scroll_offset.x - dx).clamp(0.0, max_x);
                    });
                }

                // ── Right-click context menu ──────────────────────────────────
                response.context_menu(|ui| {
                    let has_sel = self.cursor.has_selection();

                    if ui.add_enabled(has_sel, egui::Button::new("Cut")).clicked() {
                        if let Some(text) = self.selected_text() {
                            ui.ctx().copy_text(text);
                            self.buffer.checkpoint();
                            self.delete_selection();
                            self.is_modified = true;
                            self.content_version += 1;
                        }
                        ui.close_menu();
                    }

                    let copy_label = if has_sel { "Copy" } else { "Copy Line" };
                    if ui.button(copy_label).clicked() {
                        let text = self.selected_text().unwrap_or_else(|| {
                            let (row, _) = self.cursor.position();
                            self.buffer.line(row) + "\n"
                        });
                        ui.ctx().copy_text(text);
                        ui.close_menu();
                    }

                    ui.separator();

                    if ui.button("Select All").clicked() {
                        let last_row = self.buffer.num_lines().saturating_sub(1);
                        let last_col = self.buffer.line_len(last_row);
                        self.cursor.sel_anchor = Some((0, 0));
                        self.cursor.set_position(last_row, last_col);
                        self.cursor.sel_anchor = Some((0, 0));
                        ui.close_menu();
                    }

                    ui.separator();

                    if ui.button("Go to Definition    Ctrl+Click").clicked() {
                        let (row, col) = self.cursor.position();
                        if let Some(word) = get_word_at(&self.buffer, row, col) {
                            self.go_to_definition_request = Some(word);
                        }
                        ui.close_menu();
                    }

                    if ui.button("Find    Ctrl+F").clicked() {
                        self.show_find = true;
                        ui.close_menu();
                    }
                });

                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 0.0, bg_color);

                // Scroll viewport to make the cursor visible when requested.
                if self.scroll_to_cursor {
                    let (cur_row, _) = self.cursor.position();
                    let target_y = cur_row as f32 * line_height;
                    if target_y < self.scroll_offset.y {
                        self.scroll_offset.y = target_y;
                    } else if target_y + line_height > self.scroll_offset.y + rect.height() {
                        self.scroll_offset.y = (target_y + line_height - rect.height()).max(0.0);
                    }
                    self.scroll_to_cursor = false;
                }

                let first_visible = (self.scroll_offset.y / line_height) as usize;
                let visible_count = (rect.height() / line_height) as usize + 2;

                // Bracket match: recompute every frame based on cursor position
                {
                    let (cur_row, cur_col) = self.cursor.position();
                    self.bracket_match = find_matching_bracket(&self.buffer, cur_row, cur_col);
                }

                // Rebuild the highlight cache when content changes.
                //
                // Very large files (> VIEWPORT_THRESHOLD lines): tokenizing the whole
                // document each edit/frame lags badly, so only tokenize the visible
                // window (± MARGIN lines) and re-tokenize when scrolling out of it.
                // Smaller files tokenize the whole document (accurate), debounced so
                // typing in a moderately large file stays smooth.
                const VIEWPORT_THRESHOLD: usize = 4000;
                let total_lines_hl = self.buffer.num_lines();
                if total_lines_hl > VIEWPORT_THRESHOLD {
                    let vis_start = first_visible;
                    let vis_end = (first_visible + visible_count).min(total_lines_hl);
                    if self
                        .highlighter
                        .viewport_stale(self.content_version, vis_start, vis_end)
                    {
                        const MARGIN: usize = 100;
                        let start = vis_start.saturating_sub(MARGIN);
                        let end = (vis_end + MARGIN).min(total_lines_hl);
                        let window: String = (start..end)
                            .map(|r| self.buffer.line(r))
                            .collect::<Vec<_>>()
                            .join("\n");
                        self.highlighter.highlight_viewport(
                            &window,
                            start,
                            total_lines_hl,
                            self.content_version,
                            (start, end),
                            Some(plugin_manager),
                        );
                    }
                } else if self.highlighter.needs_update(self.content_version) {
                    // Debounce full re-tokenization for moderately large files.
                    let ready = if total_lines_hl > 2000 {
                        if self.hl_pending_version != self.content_version {
                            self.hl_pending_version = self.content_version;
                            self.hl_pending_at = Some(std::time::Instant::now());
                        }
                        let elapsed_ok = self
                            .hl_pending_at
                            .map(|t| t.elapsed() >= std::time::Duration::from_millis(150))
                            .unwrap_or(true);
                        if !elapsed_ok {
                            response
                                .ctx
                                .request_repaint_after(std::time::Duration::from_millis(150));
                        }
                        elapsed_ok
                    } else {
                        true
                    };
                    if ready {
                        let source = self.buffer.to_string();
                        self.highlighter.highlight_document(
                            &source,
                            self.content_version,
                            Some(plugin_manager),
                        );
                    }
                }

                // Fold regions: recompute only when the content changed — NOT every
                // frame. (The old `is_empty()` check recomputed on every frame for
                // files with no foldable regions, an O(file) per-frame cost.)
                if self.fold_regions_version != self.content_version {
                    self.fold_regions = compute_fold_regions(&self.buffer);
                    self.fold_regions_version = self.content_version;
                }

                // Build fold map: start_line → end_line for O(1) lookup
                let fold_map: std::collections::HashMap<usize, usize> = self
                    .folded_lines
                    .iter()
                    .filter_map(|&start| {
                        self.fold_regions
                            .iter()
                            .find(|(s, _)| *s == start)
                            .map(|&(s, e)| (s, e))
                    })
                    .collect();

                // Handle gutter click to toggle fold
                if response.clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let local = pos - rect.min;
                        if local.x < gutter_width && local.x > gutter_width - 14.0 {
                            let row = ((local.y + self.scroll_offset.y) / line_height) as usize;
                            if self.folded_lines.contains(&row) {
                                self.folded_lines.remove(&row);
                            } else if self.fold_regions.iter().any(|(s, _)| *s == row) {
                                self.folded_lines.insert(row);
                            }
                        }
                    }
                }

                // Determine selection range for highlight
                let sel_range = self.cursor.selection_range();

                // Update word occurrence highlights
                let last_visible = first_visible + visible_count;
                self.update_word_occurrences(first_visible, last_visible);

                // Compute the active indent block (for guide highlighting)
                let active_block = {
                    let (cr, _) = self.cursor.position();
                    active_indent_block(
                        self.buffer.num_lines(),
                        cr,
                        self.detected_indent_size.max(1),
                        self.detected_indent_spaces,
                        |i| self.buffer.line(i),
                    )
                };

                // Iterate visible lines, skipping folded content
                let mut line_idx = first_visible;
                while line_idx < total_lines
                    && line_idx < first_visible + visible_count + fold_map.len()
                {
                    let y = rect.min.y + line_idx as f32 * line_height - self.scroll_offset.y;
                    // Stop drawing if off-screen bottom
                    if y > rect.max.y + line_height {
                        break;
                    }

                    // Code folding: draw placeholder and skip folded lines
                    if let Some(&fold_end) = fold_map.get(&line_idx) {
                        let line = self.buffer.line(line_idx);
                        let x_start = rect.min.x + gutter_width;
                        // Draw fold marker in gutter
                        painter.text(
                            egui::pos2(rect.min.x + gutter_width - 12.0, y + line_height * 0.5),
                            egui::Align2::RIGHT_CENTER,
                            "›",
                            font_id.clone(),
                            egui::Color32::from_rgb(100, 160, 255),
                        );
                        // Draw the first (header) line normally, then "⋯" placeholder
                        let preview: String = line.chars().take(60).collect();
                        let folded_count = fold_end - line_idx;
                        let tokens = self.highlighter.tokens_for_line(
                            line_idx,
                            &preview,
                            Some(plugin_manager),
                        );
                        let mut job = egui::text::LayoutJob::default();
                        for tok in &tokens {
                            job.append(
                                &tok.text,
                                0.0,
                                egui::TextFormat {
                                    font_id: font_id.clone(),
                                    color: tok.kind.color(),
                                    ..Default::default()
                                },
                            );
                        }
                        job.append(
                            &format!("  ⋯  ({} lines)", folded_count),
                            0.0,
                            egui::TextFormat {
                                font_id: font_id.clone(),
                                color: egui::Color32::from_gray(100),
                                ..Default::default()
                            },
                        );
                        painter.add(egui::epaint::TextShape::new(
                            egui::pos2(x_start - self.scroll_offset.x, y),
                            ui.fonts(|f| f.layout_job(job)),
                            egui::Color32::WHITE,
                        ));
                        if config.editor.line_numbers {
                            painter.text(
                                egui::pos2(
                                    rect.min.x + blame_extra_width + 50.0 - 8.0,
                                    y + line_height * 0.5,
                                ),
                                egui::Align2::RIGHT_CENTER,
                                (line_idx + 1).to_string(),
                                font_id.clone(),
                                if line_idx == cur_row_for_gutter {
                                    line_num_color_active
                                } else {
                                    line_num_color
                                },
                            );
                        }
                        line_idx = fold_end + 1;
                        continue;
                    }

                    // Current-line highlight (suppressed while a selection is active, to avoid noise).
                    let (hl_row, _) = self.cursor.position();
                    let selection_active = sel_range.is_some()
                        || self
                            .extra_cursors
                            .iter()
                            .any(|c| c.selection_range().is_some());
                    if config.editor.highlight_current_line
                        && line_idx == hl_row
                        && !selection_active
                    {
                        painter.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(rect.min.x, y),
                                egui::pos2(rect.max.x, y + line_height),
                            ),
                            0.0,
                            palette.line_highlight,
                        );
                    }

                    if config.editor.line_numbers {
                        painter.text(
                            egui::pos2(
                                rect.min.x + blame_extra_width + 50.0 - 8.0,
                                y + line_height * 0.5,
                            ),
                            egui::Align2::RIGHT_CENTER,
                            (line_idx + 1).to_string(),
                            font_id.clone(),
                            if line_idx == cur_row_for_gutter {
                                line_num_color_active
                            } else {
                                line_num_color
                            },
                        );
                    }

                    // Git diff bar in gutter (left edge of line number area)
                    if config.editor.line_numbers && !self.line_diff.is_empty() {
                        let diff_status = self
                            .line_diff
                            .get(line_idx)
                            .copied()
                            .unwrap_or(DIFF_UNCHANGED);
                        let bar_color = match diff_status {
                            DIFF_ADDED => Some(palette.git_added),
                            DIFF_MODIFIED => Some(palette.git_modified),
                            _ => None,
                        };
                        if let Some(color) = bar_color {
                            let bar_x = rect.min.x + blame_extra_width;
                            painter.rect_filled(
                                egui::Rect::from_min_size(
                                    egui::pos2(bar_x, y + 1.0),
                                    egui::vec2(3.0, line_height - 2.0),
                                ),
                                0.0,
                                color,
                            );
                        }
                    }

                    // Git blame in gutter
                    if self.show_blame {
                        if let Some(entry) = self.blame_data.get(line_idx) {
                            let blame_text = format!(
                                "{} {}",
                                entry.commit_short,
                                if entry.author.len() > 8 {
                                    &entry.author[..8]
                                } else {
                                    &entry.author
                                }
                            );
                            painter.text(
                                egui::pos2(rect.min.x + 2.0, y + line_height * 0.5),
                                egui::Align2::LEFT_CENTER,
                                blame_text,
                                egui::FontId::monospace(config.font.size * 0.8),
                                egui::Color32::from_gray(100),
                            );
                        }
                    }

                    // Breakpoint dot in gutter (red circle, left side)
                    if breakpoint_lines.contains(&line_idx) {
                        let bp_x = rect.min.x + blame_extra_width + 5.0;
                        let bp_y = y + line_height * 0.5;
                        painter.circle_filled(
                            egui::pos2(bp_x, bp_y),
                            5.0,
                            egui::Color32::from_rgb(220, 50, 50),
                        );
                    }

                    // Lightbulb icon in gutter when cursor line has diagnostics (Code Actions)
                    let (cur_row, _) = self.cursor.position();
                    if line_idx == cur_row {
                        let has_diag = self.diagnostics.iter().any(|d| d.line as usize == line_idx);
                        if has_diag && config.editor.line_numbers {
                            // Pick the most severe diagnostic on the line (Error > Warning > Info/Hint).
                            let sev_color = self
                                .diagnostics
                                .iter()
                                .filter(|d| d.line as usize == line_idx)
                                .min_by_key(|d| match d.severity {
                                    crate::lsp::client::DiagSeverity::Error => 0,
                                    crate::lsp::client::DiagSeverity::Warning => 1,
                                    crate::lsp::client::DiagSeverity::Info => 2,
                                    _ => 3,
                                })
                                .map(|d| match d.severity {
                                    crate::lsp::client::DiagSeverity::Error => palette.error,
                                    crate::lsp::client::DiagSeverity::Warning => palette.warning,
                                    _ => palette.info,
                                })
                                .unwrap_or(palette.warning);
                            painter.text(
                                egui::pos2(
                                    rect.min.x + blame_extra_width + 2.0,
                                    y + line_height * 0.5,
                                ),
                                egui::Align2::LEFT_CENTER,
                                "💡",
                                egui::FontId::proportional(11.0),
                                sev_color,
                            );
                        }
                    }

                    let line = self.buffer.line(line_idx);
                    let x_start = rect.min.x + gutter_width;

                    // Find bar match highlight — precise character-level boxes.
                    // Uses the same matcher as find/replace, on the original line,
                    // so offsets are always valid char boundaries.
                    if self.find_matches.contains(&line_idx) {
                        if let Some(re) = self.find_regex() {
                            let haystack = self.buffer.line(line_idx);
                            let is_active =
                                self.find_matches.get(self.find_current) == Some(&line_idx);
                            let color = if is_active {
                                find_highlight_active
                            } else {
                                find_highlight
                            };
                            for m in re.find_iter(&haystack).filter(|m| !m.is_empty()) {
                                let measure = |text: &str| {
                                    ui.fonts(|f| {
                                        f.layout_no_wrap(
                                            text.to_owned(),
                                            font_id.clone(),
                                            egui::Color32::WHITE,
                                        )
                                        .size()
                                        .x
                                    })
                                };
                                let pre_w = measure(&haystack[..m.start()]);
                                let span_w = measure(m.as_str());
                                let hx = x_start + pre_w - self.scroll_offset.x;
                                if hx < rect.max.x && hx + span_w > x_start {
                                    painter.rect_filled(
                                        egui::Rect::from_min_size(
                                            egui::pos2(hx, y + 1.0),
                                            egui::vec2(span_w.max(4.0), line_height - 2.0),
                                        ),
                                        2.0,
                                        color,
                                    );
                                }
                            }
                        }
                    }

                    // Word occurrence highlighting
                    for &(occ_row, occ_start, occ_end) in &self.word_occurrences {
                        if occ_row == line_idx {
                            let occ_sx = x_start
                                + ui.fonts(|f| {
                                    let text: String = line.chars().take(occ_start).collect();
                                    f.layout_no_wrap(text, font_id.clone(), egui::Color32::WHITE)
                                        .size()
                                        .x
                                });
                            let occ_ex = x_start
                                + ui.fonts(|f| {
                                    let text: String = line.chars().take(occ_end).collect();
                                    f.layout_no_wrap(text, font_id.clone(), egui::Color32::WHITE)
                                        .size()
                                        .x
                                });
                            if occ_ex > occ_sx {
                                painter.rect_filled(
                                    egui::Rect::from_min_max(
                                        egui::pos2(occ_sx - self.scroll_offset.x, y),
                                        egui::pos2(occ_ex - self.scroll_offset.x, y + line_height),
                                    ),
                                    2.0,
                                    palette.accent_muted,
                                );
                            }
                        }
                    }

                    // Selection highlight (per-line)
                    if let Some(((sr, sc), (er, ec))) = sel_range {
                        if line_idx >= sr && line_idx <= er {
                            let sel_start_col = if line_idx == sr { sc } else { 0 };
                            let sel_end_col = if line_idx == er {
                                ec
                            } else {
                                self.buffer.line_len(line_idx)
                            };
                            // Measure pixel positions using font layout for accuracy
                            let sx = x_start
                                + ui.fonts(|f| {
                                    let text: String = line.chars().take(sel_start_col).collect();
                                    f.layout_no_wrap(text, font_id.clone(), egui::Color32::WHITE)
                                        .size()
                                        .x
                                });
                            let ex = x_start
                                + ui.fonts(|f| {
                                    let text: String = line.chars().take(sel_end_col).collect();
                                    f.layout_no_wrap(text, font_id.clone(), egui::Color32::WHITE)
                                        .size()
                                        .x
                                });
                            if ex > sx {
                                painter.rect_filled(
                                    egui::Rect::from_min_max(
                                        egui::pos2(sx, y),
                                        egui::pos2(ex, y + line_height),
                                    ),
                                    0.0,
                                    palette.selection,
                                );
                            }
                        }
                    }

                    // Selection highlight for extra cursors (Ctrl+D multi-selection)
                    for extra_cur in &self.extra_cursors {
                        if let Some(((esr, esc), (eer, eec))) = extra_cur.selection_range() {
                            if line_idx >= esr && line_idx <= eer {
                                let sel_start_col = if line_idx == esr { esc } else { 0 };
                                let sel_end_col = if line_idx == eer {
                                    eec
                                } else {
                                    self.buffer.line_len(line_idx)
                                };
                                let sx = x_start
                                    + ui.fonts(|f| {
                                        let text: String =
                                            line.chars().take(sel_start_col).collect();
                                        f.layout_no_wrap(
                                            text,
                                            font_id.clone(),
                                            egui::Color32::WHITE,
                                        )
                                        .size()
                                        .x
                                    });
                                let ex = x_start
                                    + ui.fonts(|f| {
                                        let text: String = line.chars().take(sel_end_col).collect();
                                        f.layout_no_wrap(
                                            text,
                                            font_id.clone(),
                                            egui::Color32::WHITE,
                                        )
                                        .size()
                                        .x
                                    });
                                if ex > sx {
                                    painter.rect_filled(
                                        egui::Rect::from_min_max(
                                            egui::pos2(sx, y),
                                            egui::pos2(ex, y + line_height),
                                        ),
                                        0.0,
                                        palette.selection,
                                    );
                                }
                            }
                        }
                    }

                    let (cur_row, cur_col) = self.cursor.position();
                    if line_idx == cur_row {
                        // Cursor: measure actual pixel offset of char col in the line
                        let text_to_cursor: String = line.chars().take(cur_col).collect();
                        let cx = x_start
                            + ui.fonts(|f| {
                                f.layout_no_wrap(
                                    text_to_cursor,
                                    font_id.clone(),
                                    egui::Color32::WHITE,
                                )
                                .size()
                                .x
                            });
                        // Blink: 530ms on / 530ms off, reset on any input event.
                        let blink_ms = self.cursor_blink_epoch.elapsed().as_millis() % 1060;
                        let cursor_visible = blink_ms < 530;
                        if cursor_visible {
                            painter.line_segment(
                                [egui::pos2(cx, y), egui::pos2(cx, y + line_height)],
                                egui::Stroke::new(2.0_f32, cursor_color),
                            );
                        }
                        // Request repaint at the next blink transition.
                        let next_transition = if cursor_visible {
                            530 - blink_ms
                        } else {
                            1060 - blink_ms
                        };
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(
                                next_transition as u64 + 1,
                            ));
                        // Update autocomplete popup anchor for this frame.
                        self.autocomplete.cursor_screen_pos = egui::pos2(cx, y + line_height);
                    }

                    // Render extra cursors
                    for extra_cur in &self.extra_cursors {
                        let (ecr, ecc) = extra_cur.position();
                        if line_idx == ecr {
                            let text_to_cur: String = line.chars().take(ecc).collect();
                            let ecx = x_start
                                + ui.fonts(|f| {
                                    f.layout_no_wrap(
                                        text_to_cur,
                                        font_id.clone(),
                                        egui::Color32::WHITE,
                                    )
                                    .size()
                                    .x
                                });
                            painter.line_segment(
                                [egui::pos2(ecx, y), egui::pos2(ecx, y + line_height)],
                                egui::Stroke::new(2.0_f32, accent_color),
                            );
                        }
                    }

                    // Indent guides
                    {
                        let ind_size = self.detected_indent_size.max(1);
                        let leading = if self.detected_indent_spaces {
                            line.chars().take_while(|&c| c == ' ').count()
                        } else {
                            line.chars().take_while(|&c| c == '\t').count() * ind_size
                        };
                        let guides = leading / ind_size;
                        for g in 1..=guides {
                            // Nudge guides half a character to the left so they sit at
                            // the indent boundary rather than under the first glyph.
                            let gx = x_start + (g * ind_size) as f32 * char_width
                                - self.scroll_offset.x
                                - char_width * 0.5
                                - 2.0;
                            // clamp to visible text area
                            if gx < x_start || gx > rect.max.x {
                                continue;
                            }
                            let guide_color = match active_block {
                                Some((lvl, s, e)) if g == lvl && line_idx >= s && line_idx <= e => {
                                    palette.accent_muted
                                }
                                _ => egui::Color32::from_rgba_unmultiplied(130, 130, 145, 50),
                            };
                            painter.line_segment(
                                [egui::pos2(gx, y), egui::pos2(gx, y + line_height)],
                                egui::Stroke::new(1.0_f32, guide_color),
                            );
                        }
                    }

                    // Bracket match highlight
                    if let Some((or, oc, cr, cc)) = self.bracket_match {
                        for (br, bc) in [(or, oc), (cr, cc)] {
                            if line_idx == br {
                                let bx = x_start
                                    + ui.fonts(|f| {
                                        let text: String = line.chars().take(bc).collect();
                                        f.layout_no_wrap(
                                            text,
                                            font_id.clone(),
                                            egui::Color32::WHITE,
                                        )
                                        .size()
                                        .x
                                    })
                                    - self.scroll_offset.x;
                                painter.rect_filled(
                                    egui::Rect::from_min_size(
                                        egui::pos2(bx, y),
                                        egui::vec2(char_width, line_height),
                                    ),
                                    2.0,
                                    egui::Color32::from_rgba_premultiplied(100, 160, 255, 50),
                                );
                                painter.rect_stroke(
                                    egui::Rect::from_min_size(
                                        egui::pos2(bx, y),
                                        egui::vec2(char_width, line_height),
                                    ),
                                    2.0,
                                    egui::Stroke::new(
                                        1.0_f32,
                                        egui::Color32::from_rgb(100, 160, 255),
                                    ),
                                    egui::StrokeKind::Inside,
                                );
                            }
                        }
                    }

                    // Syntax-highlighted text
                    let tokens =
                        self.highlighter
                            .tokens_for_line(line_idx, &line, Some(plugin_manager));
                    let mut job = egui::text::LayoutJob::default();
                    for tok in &tokens {
                        job.append(
                            &tok.text,
                            0.0,
                            egui::TextFormat {
                                font_id: font_id.clone(),
                                color: tok.kind.color(),
                                ..Default::default()
                            },
                        );
                    }
                    let galley = ui.fonts(|f| f.layout_job(job));
                    // Clip to the content area (right of the gutter) so horizontally
                    // scrolled text doesn't draw over the line-number gutter.
                    let text_clip = egui::Rect::from_min_max(
                        egui::pos2(x_start, rect.min.y),
                        egui::pos2(rect.max.x, rect.max.y),
                    );
                    painter.with_clip_rect(text_clip).galley(
                        egui::pos2(x_start - self.scroll_offset.x, y + line_height * 0.15),
                        galley,
                        fg_color,
                    );

                    // Ctrl+hover underline (VSCode-style go-to-definition hint)
                    if let Some((hover_row, hover_start, hover_end)) = self.ctrl_hover_word_bounds {
                        if line_idx == hover_row {
                            let line = self.buffer.line(line_idx);
                            let underline_x_start = x_start
                                + ui.fonts(|f| {
                                    let text: String = line.chars().take(hover_start).collect();
                                    f.layout_no_wrap(text, font_id.clone(), egui::Color32::WHITE)
                                        .size()
                                        .x
                                });
                            let underline_x_end = x_start
                                + ui.fonts(|f| {
                                    let text: String = line.chars().take(hover_end).collect();
                                    f.layout_no_wrap(text, font_id.clone(), egui::Color32::WHITE)
                                        .size()
                                        .x
                                });
                            let underline_y = y + line_height - 2.0;
                            painter.line_segment(
                                [
                                    egui::pos2(underline_x_start, underline_y),
                                    egui::pos2(underline_x_end, underline_y),
                                ],
                                egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(100, 160, 255)),
                            );
                        }
                    }

                    // Diagnostic squiggly underlines
                    for diag in &self.diagnostics {
                        if diag.line as usize == line_idx {
                            let underline_y = y + line_height - 2.0;
                            let x_diag_start = x_start + diag.col as f32 * char_width;
                            let diag_width = if diag.end_col > diag.col {
                                (diag.end_col - diag.col) as f32 * char_width
                            } else {
                                100.0
                            };
                            let x_diag_end = (x_diag_start + diag_width).min(rect.right());
                            let color = match diag.severity {
                                crate::lsp::client::DiagSeverity::Error => palette.error,
                                crate::lsp::client::DiagSeverity::Warning => palette.warning,
                                _ => palette.info,
                            };
                            let amp = 1.5_f32;
                            let period = 4.0_f32;
                            let mut x = x_diag_start;
                            while x < x_diag_end {
                                let x1 = x;
                                let y1 =
                                    underline_y + amp * ((x / period * std::f32::consts::PI).sin());
                                let x2 = (x + period / 2.0).min(x_diag_end);
                                let y2 = underline_y - amp;
                                painter.line_segment(
                                    [egui::pos2(x1, y1), egui::pos2(x2, y2)],
                                    egui::Stroke::new(1.0_f32, color),
                                );
                                x = x2;
                            }
                        }
                    }

                    // Fold marker in gutter for foldable (non-folded) lines
                    if self.fold_regions.iter().any(|(s, _)| *s == line_idx) {
                        painter.text(
                            egui::pos2(rect.min.x + gutter_width - 12.0, y + line_height * 0.5),
                            egui::Align2::RIGHT_CENTER,
                            "⌄",
                            egui::FontId::monospace(10.0),
                            egui::Color32::from_gray(100),
                        );
                    }

                    line_idx += 1;
                } // end while

                // ── Horizontal scrollbar ─────────────────────────────────────────
                {
                    let scrollbar_h = 8.0_f32;
                    let max_line_chars = self.cached_max_line_chars();
                    let content_w = max_line_chars as f32 * char_width + gutter_width + 40.0;
                    let view_w = rect.width();
                    if content_w > view_w {
                        let track_x = rect.min.x + gutter_width;
                        let track_w = view_w - gutter_width;
                        let track_y = rect.max.y - scrollbar_h;
                        // Thumb proportional size and position
                        let thumb_ratio = (view_w / content_w).min(1.0);
                        let thumb_w = (track_w * thumb_ratio).max(20.0);
                        let scroll_ratio = self.scroll_offset.x / (content_w - view_w);
                        let thumb_x = track_x + scroll_ratio * (track_w - thumb_w);
                        // Track background
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(track_x, track_y),
                                egui::vec2(track_w, scrollbar_h),
                            ),
                            0.0,
                            egui::Color32::from_rgba_premultiplied(0, 0, 0, 60),
                        );
                        // Thumb
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(thumb_x, track_y + 1.0),
                                egui::vec2(thumb_w, scrollbar_h - 2.0),
                            ),
                            3.0,
                            egui::Color32::from_rgba_premultiplied(150, 150, 150, 120),
                        );
                        // Drag scrollbar thumb
                        let thumb_rect = egui::Rect::from_min_size(
                            egui::pos2(thumb_x, track_y),
                            egui::vec2(thumb_w, scrollbar_h),
                        );
                        let sb_resp = ui.interact(
                            thumb_rect,
                            response.id.with("hscroll"),
                            egui::Sense::drag(),
                        );
                        if sb_resp.dragged() {
                            let delta = sb_resp.drag_delta().x;
                            let scroll_range = content_w - view_w;
                            self.scroll_offset.x = (self.scroll_offset.x
                                + delta * scroll_range / (track_w - thumb_w))
                                .clamp(0.0, scroll_range);
                        }
                    } else {
                        // Content fits: reset horizontal scroll
                        self.scroll_offset.x = 0.0;
                    }
                }

                // ── Vertical scrollbar ────────────────────────────────────────
                {
                    let scrollbar_w = 8.0_f32;
                    let num_lines = self.buffer.num_lines() as f32;
                    let content_h = num_lines * line_height;
                    let view_h = rect.height();

                    if content_h > view_h {
                        let track_x = rect.max.x - scrollbar_w;
                        let track_h = view_h;

                        let thumb_ratio = (view_h / content_h).min(1.0);
                        let thumb_h = (track_h * thumb_ratio).max(20.0);
                        let scroll_range = content_h - view_h;
                        let scroll_ratio = if scroll_range > 0.0 {
                            self.scroll_offset.y / scroll_range
                        } else {
                            0.0
                        };
                        let thumb_y = rect.min.y + scroll_ratio * (track_h - thumb_h);

                        // Track background
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(track_x, rect.min.y),
                                egui::vec2(scrollbar_w, track_h),
                            ),
                            0.0,
                            egui::Color32::from_rgba_premultiplied(0, 0, 0, 60),
                        );
                        // Thumb
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(track_x + 1.0, thumb_y),
                                egui::vec2(scrollbar_w - 2.0, thumb_h),
                            ),
                            3.0,
                            egui::Color32::from_rgba_premultiplied(150, 150, 150, 120),
                        );
                        // Drag scrollbar thumb
                        let thumb_rect = egui::Rect::from_min_size(
                            egui::pos2(track_x, thumb_y),
                            egui::vec2(scrollbar_w, thumb_h),
                        );
                        let sb_resp = ui.interact(
                            thumb_rect,
                            response.id.with("vscroll"),
                            egui::Sense::drag(),
                        );
                        if sb_resp.dragged() {
                            let delta = sb_resp.drag_delta().y;
                            self.scroll_offset.y = (self.scroll_offset.y
                                + delta * scroll_range / (track_h - thumb_h))
                                .clamp(0.0, scroll_range);
                        }
                        // Click on track to jump
                        let track_rect = egui::Rect::from_min_size(
                            egui::pos2(track_x, rect.min.y),
                            egui::vec2(scrollbar_w, track_h),
                        );
                        let track_resp = ui.interact(
                            track_rect,
                            response.id.with("vscroll_track"),
                            egui::Sense::click(),
                        );
                        if track_resp.clicked() {
                            if let Some(pos) = track_resp.interact_pointer_pos() {
                                let click_ratio = (pos.y - rect.min.y) / track_h;
                                self.scroll_offset.y = (click_ratio * content_h - view_h / 2.0)
                                    .clamp(0.0, scroll_range);
                            }
                        }
                    } else {
                        self.scroll_offset.y = 0.0;
                    }
                }

                // ── Minimap ──────────────────────────────────────────────────────
                if config.editor.show_minimap {
                    if self.minimap_lines_version != self.content_version {
                        self.minimap_lines = (0..total_lines)
                            .map(|i| {
                                let line = self.buffer.line(i);
                                let indent = line.chars().take_while(|c| c.is_whitespace()).count();
                                let trimmed_len = line.trim().len();
                                crate::ui::minimap::LineShape {
                                    indent,
                                    content_len: trimmed_len,
                                    blank: trimmed_len == 0,
                                }
                            })
                            .collect();
                        self.minimap_lines_version = self.content_version;
                    }
                    let minimap_data = crate::ui::minimap::MinimapData {
                        total_lines,
                        first_visible,
                        visible_count,
                        diagnostics: &self.diagnostics,
                        line_diff: &self.line_diff,
                        find_matches: &self.find_matches,
                        cursor_row: self.cursor.row,
                        extra_cursor_rows: self.extra_cursors.iter().map(|c| c.row).collect(),
                        line_height,
                        bg_color,
                        accent_color: egui::Color32::from_rgb(
                            config.theme.accent[0],
                            config.theme.accent[1],
                            config.theme.accent[2],
                        ),
                        lines: &self.minimap_lines,
                        fold_regions: &self.fold_regions,
                    };
                    if let Some(new_scroll_y) =
                        crate::ui::minimap::render(ui, &painter, rect, &minimap_data)
                    {
                        self.scroll_offset.y = new_scroll_y;
                    }
                }

                // ── VSCode-style hover popup ──────────────────────────────────────
                if let Some(sig) = self.hover_signature.clone() {
                    if !sig.is_empty() {
                        // Parse markdown into typed sections.
                        let sections = parse_hover_sections(&sig);

                        // Pre-compute per-section rendering data (before entering closures).
                        struct RenderedSection {
                            is_code: bool,
                            is_separator: bool,
                            jobs: Vec<egui::text::LayoutJob>,
                        }
                        let rendered: Vec<RenderedSection> = sections
                            .iter()
                            .map(|sec| match sec {
                                HoverSection::CodeBlock { code, .. } => {
                                    let jobs = code
                                        .lines()
                                        .map(|line| {
                                            let tokens = self.highlighter.tokenize_line(line);
                                            let mut job = egui::text::LayoutJob {
                                                wrap: egui::text::TextWrapping {
                                                    max_width: 500.0,
                                                    ..Default::default()
                                                },
                                                ..Default::default()
                                            };
                                            for tok in &tokens {
                                                if !tok.text.is_empty() {
                                                    job.append(
                                                        &tok.text,
                                                        0.0,
                                                        egui::TextFormat {
                                                            font_id: egui::FontId::monospace(13.0),
                                                            color: tok.kind.color(),
                                                            ..Default::default()
                                                        },
                                                    );
                                                }
                                            }
                                            if job.sections.is_empty() {
                                                job.append(
                                                    line,
                                                    0.0,
                                                    egui::TextFormat {
                                                        font_id: egui::FontId::monospace(13.0),
                                                        color: egui::Color32::from_rgb(
                                                            212, 212, 212,
                                                        ),
                                                        ..Default::default()
                                                    },
                                                );
                                            }
                                            job
                                        })
                                        .collect();
                                    RenderedSection {
                                        is_code: true,
                                        is_separator: false,
                                        jobs,
                                    }
                                }
                                HoverSection::Text(text) => {
                                    let jobs = text
                                        .lines()
                                        .map(|line| {
                                            if line.trim().is_empty() {
                                                egui::text::LayoutJob::default()
                                            } else {
                                                inline_markdown_job(line, 13.0)
                                            }
                                        })
                                        .collect();
                                    RenderedSection {
                                        is_code: false,
                                        is_separator: false,
                                        jobs,
                                    }
                                }
                                HoverSection::Separator => RenderedSection {
                                    is_code: false,
                                    is_separator: true,
                                    jobs: vec![],
                                },
                            })
                            .collect();

                        // Determine anchor: below the hovered word, or above if near bottom.
                        let screen_rect = ui.ctx().screen_rect();
                        let raw_anchor = self
                            .hover_tooltip_anchor
                            .unwrap_or(self.hover_pos + egui::vec2(0.0, line_height + 4.0));
                        // Estimate content height to decide above/below.
                        let est_lines: usize = rendered.iter().map(|s| s.jobs.len().max(1)).sum();
                        let est_height = est_lines as f32 * 18.0 + 60.0;
                        let anchor = if raw_anchor.y + est_height > screen_rect.bottom() - 8.0 {
                            // Not enough room below — go above the word.
                            let above_y = raw_anchor.y - line_height - est_height - 8.0;
                            egui::pos2(raw_anchor.x, above_y.max(screen_rect.top() + 4.0))
                        } else {
                            raw_anchor
                        };

                        let hover_word_for_goto = self.hover_word.clone();
                        let area_resp = egui::Area::new(egui::Id::new("hover_sig_tooltip"))
                            .fixed_pos(anchor)
                            .order(egui::Order::Tooltip)
                            .constrain(true)
                            .show(ui.ctx(), |ui| {
                                egui::Frame::new()
                                    .fill(palette.surface_raised)
                                    .stroke(egui::Stroke::new(
                                        1.0_f32,
                                        egui::Color32::from_gray(75),
                                    ))
                                    .corner_radius(egui::CornerRadius::same(4))
                                    .inner_margin(egui::Margin::same(10))
                                    .show(ui, |ui| {
                                        ui.set_max_width(540.0);
                                        egui::ScrollArea::vertical()
                                            .max_height(320.0)
                                            .id_salt("hover_scroll")
                                            .show(ui, |ui| {
                                                for sec in &rendered {
                                                    if sec.is_separator {
                                                        ui.add_space(4.0);
                                                        let sep_rect =
                                                            ui.available_rect_before_wrap();
                                                        let y = sep_rect.min.y + 1.0;
                                                        ui.painter().line_segment(
                                                            [
                                                                egui::pos2(sep_rect.min.x, y),
                                                                egui::pos2(
                                                                    sep_rect.min.x + 500.0,
                                                                    y,
                                                                ),
                                                            ],
                                                            egui::Stroke::new(
                                                                1.0_f32,
                                                                egui::Color32::from_gray(60),
                                                            ),
                                                        );
                                                        ui.add_space(6.0);
                                                    } else if sec.is_code {
                                                        egui::Frame::new()
                                                            .fill(egui::Color32::from_rgb(
                                                                20, 20, 20,
                                                            ))
                                                            .corner_radius(
                                                                egui::CornerRadius::same(3),
                                                            )
                                                            .inner_margin(egui::Margin::symmetric(
                                                                8, 4,
                                                            ))
                                                            .show(ui, |ui| {
                                                                ui.set_max_width(520.0);
                                                                for job in &sec.jobs {
                                                                    ui.label(
                                                                        egui::WidgetText::LayoutJob(
                                                                            job.clone(),
                                                                        ),
                                                                    );
                                                                }
                                                            });
                                                    } else {
                                                        for job in &sec.jobs {
                                                            if job.sections.is_empty() {
                                                                ui.add_space(4.0);
                                                            } else {
                                                                ui.label(
                                                                    egui::WidgetText::LayoutJob(
                                                                        job.clone(),
                                                                    ),
                                                                );
                                                            }
                                                        }
                                                    }
                                                }
                                            });
                                        // Separator before actions row.
                                        ui.add_space(6.0);
                                        ui.separator();
                                        ui.add_space(2.0);
                                        ui.horizontal(|ui| {
                                            if hover_word_for_goto.is_some()
                                                && ui.small_button("Go to Definition").clicked()
                                            {
                                                ui.ctx().data_mut(|d| {
                                                    d.insert_temp(
                                                        egui::Id::new("hover_goto_clicked"),
                                                        true,
                                                    );
                                                });
                                            }
                                        });
                                    });
                            });

                        // Store popup rect so mouse-enter keeps it open.
                        self.hover_popup_rect = Some(area_resp.response.rect);

                        // Handle "Go to Definition" click (re-read hover_word here since closures moved it).
                        // The button click was already stored via egui's response — we detect it indirectly
                        // by checking if the area was clicked on the button region.
                        // Simpler: render a second pass check is complex; use a shared flag via id-based memory.
                        let goto_clicked = ui.ctx().data(|d| {
                            d.get_temp::<bool>(egui::Id::new("hover_goto_clicked"))
                                .unwrap_or(false)
                        });
                        if goto_clicked {
                            ui.ctx().data_mut(|d| {
                                d.remove::<bool>(egui::Id::new("hover_goto_clicked"));
                            });
                            if let Some(ref word) = self.hover_word {
                                self.go_to_definition_request = Some(word.clone());
                            }
                        }
                    }
                }

                // ── Diagnostic hover tooltip (styled by severity) ─────────────────
                if let Some(ref diag_msg) = self.diag_hover_msg.clone() {
                    if !diag_msg.is_empty() {
                        let border_color = match self.diag_hover_severity {
                            crate::lsp::client::DiagSeverity::Error => {
                                egui::Color32::from_rgb(230, 70, 70)
                            }
                            crate::lsp::client::DiagSeverity::Warning => {
                                egui::Color32::from_rgb(230, 185, 30)
                            }
                            _ => egui::Color32::from_rgb(80, 130, 220),
                        };
                        let diag_anchor = self.hover_pos + egui::vec2(0.0, line_height + 4.0);
                        egui::Area::new(egui::Id::new("diag_hover_tooltip"))
                            .fixed_pos(diag_anchor)
                            .order(egui::Order::Tooltip)
                            .constrain(true)
                            .show(ui.ctx(), |ui| {
                                egui::Frame::new()
                                    .fill(palette.surface_raised)
                                    .stroke(egui::Stroke::new(1.5_f32, border_color))
                                    .corner_radius(egui::CornerRadius::same(4))
                                    .inner_margin(egui::Margin::symmetric(10, 6))
                                    .show(ui, |ui| {
                                        ui.set_max_width(400.0);
                                        // Severity badge
                                        let badge = match self.diag_hover_severity {
                                            crate::lsp::client::DiagSeverity::Error => "⛔ Error",
                                            crate::lsp::client::DiagSeverity::Warning => {
                                                "⚠ Warning"
                                            }
                                            _ => "ℹ Info",
                                        };
                                        ui.label(
                                            egui::RichText::new(badge)
                                                .color(border_color)
                                                .size(11.0),
                                        );
                                        ui.add_space(2.0);
                                        ui.label(
                                            egui::RichText::new(diag_msg.as_str())
                                                .color(egui::Color32::from_rgb(220, 220, 220))
                                                .size(13.0),
                                        );
                                    });
                            });
                    }
                }

                // ── Signature help tooltip (shows while typing function args) ─────
                if let Some(ref sig_text) = self.signature_help_text.clone() {
                    if !sig_text.is_empty() {
                        let (cur_row, cur_col) = self.cursor.position();
                        let cursor_x = rect.min.x + gutter_width + cur_col as f32 * char_width;
                        let cursor_y =
                            rect.min.y + (cur_row + 1) as f32 * line_height - self.scroll_offset.y;
                        let screen_rect = ui.ctx().screen_rect();
                        // Prefer below cursor; go above if needed.
                        let mut tl = egui::pos2(cursor_x, cursor_y + 4.0);
                        let est_h = 36.0;
                        if tl.y + est_h > screen_rect.bottom() - 4.0 {
                            tl.y = cursor_y - line_height - est_h;
                        }
                        egui::Area::new(egui::Id::new("sig_help_tooltip"))
                            .fixed_pos(tl)
                            .order(egui::Order::Tooltip)
                            .constrain(true)
                            .show(ui.ctx(), |ui| {
                                egui::Frame::new()
                                    .fill(palette.surface_raised)
                                    .stroke(egui::Stroke::new(
                                        1.0_f32,
                                        egui::Color32::from_rgb(80, 130, 200),
                                    ))
                                    .corner_radius(egui::CornerRadius::same(4))
                                    .inner_margin(egui::Margin::symmetric(10, 6))
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new(sig_text.as_str())
                                                .font(egui::FontId::monospace(13.0))
                                                .color(egui::Color32::from_rgb(200, 230, 255)),
                                        );
                                    });
                            });
                    }
                }

                // Render autocomplete popup on top of editor content.
                self.autocomplete.show(ui.ctx(), palette, spacing);
            });
    }
}

/// Returns `(level, start_row, end_row)` of the indentation block enclosing `cursor_row`,
/// where `level` is the 1-based indent depth of the cursor line. `None` when the cursor
/// line has no indentation (level 0).
pub(crate) fn active_indent_block(
    num_lines: usize,
    cursor_row: usize,
    indent_size: usize,
    spaces: bool,
    line_at: impl Fn(usize) -> String,
) -> Option<(usize, usize, usize)> {
    let ind = |s: &str| -> usize {
        let size = indent_size.max(1);
        if spaces {
            s.chars().take_while(|&c| c == ' ').count() / size
        } else {
            s.chars().take_while(|&c| c == '\t').count()
        }
    };
    if cursor_row >= num_lines {
        return None;
    }
    let level = ind(&line_at(cursor_row));
    if level == 0 {
        return None;
    }
    // Scan outward from the cursor row only as far as the block extends — cost is
    // proportional to block size, not file size (no full-buffer materialization).
    let mut start = cursor_row;
    while start > 0 && ind(&line_at(start - 1)) >= level {
        start -= 1;
    }
    let mut end = cursor_row;
    while end + 1 < num_lines && ind(&line_at(end + 1)) >= level {
        end += 1;
    }
    Some((level, start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_indent_block_spans_the_enclosing_block() {
        let lines = [
            "fn x() {".to_string(),
            "    a();".to_string(),
            "    b();".to_string(),
            "}".to_string(),
        ];
        let got = active_indent_block(lines.len(), 1, 4, true, |i| lines[i].clone());
        assert_eq!(got, Some((1, 1, 2)));
    }

    #[test]
    fn active_indent_block_none_for_unindented_or_out_of_range_rows() {
        let lines = ["a".to_string(), "  b".to_string()];
        assert_eq!(
            active_indent_block(lines.len(), 0, 2, true, |i| lines[i].clone()),
            None
        );
        assert_eq!(
            active_indent_block(lines.len(), 5, 2, true, |i| lines[i].clone()),
            None
        );
    }

    #[test]
    fn active_indent_block_counts_tabs_and_nested_levels() {
        let lines = [
            "fn x() {".to_string(),
            "\tif y {".to_string(),
            "\t\tz();".to_string(),
            "\t\tw();".to_string(),
            "\t}".to_string(),
            "}".to_string(),
        ];
        let at = |i: usize| lines[i].clone();
        assert_eq!(
            active_indent_block(lines.len(), 2, 4, false, at),
            Some((2, 2, 3))
        );
        assert_eq!(
            active_indent_block(lines.len(), 1, 4, false, at),
            Some((1, 1, 4))
        );
        // indent_size 0 is clamped to 1 rather than dividing by zero.
        let sp = ["  a".to_string()];
        assert_eq!(
            active_indent_block(1, 0, 0, true, |i| sp[i].clone()),
            Some((2, 0, 0))
        );
    }

    // ── Non-UI editor state ─────────────────────────────────────────────────

    fn editor_with(content: &str) -> Editor {
        let mut ed = Editor::new();
        ed.set_content(content.to_string(), None);
        ed
    }

    #[test]
    fn set_content_resets_state_and_detects_indent_and_language() {
        let mut ed = editor_with("old");
        ed.cursor.set_position(0, 2);
        ed.extra_cursors.push(Cursor::new());
        ed.is_modified = true;
        ed.content_version = 42;
        ed.show_find = true;
        ed.find_query = "x".into();
        ed.folded_lines.insert(3);
        ed.diagnostics.push(crate::lsp::client::Diagnostic {
            message: "m".into(),
            line: 0,
            col: 0,
            end_col: 1,
            severity: crate::lsp::client::DiagSeverity::Error,
        });
        ed.signature_help_text = Some("sig".into());

        ed.set_content(
            "def f():\n\tpass\n".to_string(),
            Some(PathBuf::from("script.py")),
        );

        assert_eq!(ed.buffer.to_string(), "def f():\n\tpass\n");
        assert_eq!(ed.cursor.position(), (0, 0));
        assert!(ed.extra_cursors.is_empty());
        assert!(!ed.is_modified);
        assert_eq!(ed.content_version, 0);
        assert!(!ed.show_find);
        assert!(ed.find_query.is_empty());
        assert!(ed.folded_lines.is_empty());
        assert!(ed.diagnostics.is_empty());
        assert!(ed.signature_help_text.is_none());
        assert_eq!(ed.current_path, Some(PathBuf::from("script.py")));
        assert!(!ed.detected_indent_spaces, "tab-indented file");
        assert_eq!(ed.highlighter.language, "py");

        ed.set_content("a\n  b\n  c\n".to_string(), None);
        assert!(ed.detected_indent_spaces);
        assert_eq!(ed.detected_indent_size, 2);
        assert_eq!(ed.current_path, None);
    }

    #[test]
    fn selection_pub_helpers() {
        let mut ed = editor_with("one\ntwo\nthree");
        assert_eq!(ed.selected_text_pub(), None);
        assert_eq!(ed.selection_line_range_pub(), None);
        ed.cursor.set_position(2, 2);
        ed.cursor.sel_anchor = Some((0, 1));
        assert_eq!(ed.selected_text_pub().as_deref(), Some("ne\ntwo\nth"));
        assert_eq!(ed.selection_line_range_pub(), Some((1, 3)));
    }

    #[test]
    fn duplicate_line_copies_current_line_below_and_moves_cursor() {
        let mut ed = editor_with("a\nb\nc");
        ed.cursor.set_position(1, 1);
        ed.duplicate_line();
        assert_eq!(ed.buffer.to_string(), "a\nb\nb\nc");
        assert_eq!(ed.cursor.position(), (2, 1));
        assert!(ed.is_modified);
        assert!(ed.buffer.undo());
        assert_eq!(ed.buffer.to_string(), "a\nb\nc");
    }

    #[test]
    fn cached_max_line_chars_recomputes_only_on_version_change() {
        let mut ed = editor_with("ab\nabcdef\nx");
        // line_char_len_fast includes the trailing newline.
        assert_eq!(ed.cached_max_line_chars(), 7);
        ed.buffer = Buffer::from_str("a very much longer line");
        assert_eq!(ed.cached_max_line_chars(), 7, "cached until version bumps");
        ed.content_version += 1;
        assert_eq!(ed.cached_max_line_chars(), 23);
    }

    #[test]
    fn save_writes_buffer_and_clears_modified_flag() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.rs");
        let mut ed = editor_with("");
        ed.set_content("fn main() {}\n".to_string(), Some(path.clone()));
        ed.cursor.set_position(0, 0);
        ed.insert_char('x', false);
        ed.line_diff_path = Some(path.clone());
        assert!(ed.is_modified);
        ed.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "xfn main() {}\n");
        assert!(!ed.is_modified);
        assert!(ed.line_diff_path.is_none(), "line diff invalidated");
    }

    #[test]
    fn save_without_path_is_a_noop() {
        let mut ed = editor_with("abc");
        ed.is_modified = true;
        ed.save().unwrap();
        assert!(ed.is_modified);
    }

    #[test]
    fn save_to_unwritable_path_errors_and_keeps_modified() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing_dir").join("f.txt");
        let mut ed = editor_with("");
        ed.set_content("abc".to_string(), Some(path));
        ed.is_modified = true;
        assert!(ed.save().is_err());
        assert!(ed.is_modified);
    }

    // ── Headless egui harness for `show` ────────────────────────────────────

    use egui::{Event, Key, Modifiers, PointerButton};

    const CTRL: Modifiers = Modifiers::CTRL;
    const SHIFT: Modifiers = Modifiers::SHIFT;
    const ALT: Modifiers = Modifiers::ALT;
    const NONE: Modifiers = Modifiers::NONE;

    fn mods(list: &[Modifiers]) -> Modifiers {
        list.iter().fold(NONE, |a, &b| a | b)
    }

    struct Harness {
        ctx: egui::Context,
        config: crate::config::Config,
        plugins: crate::plugin::manager::PluginManager,
        breakpoints: std::collections::HashSet<usize>,
        lsp_hover: Option<String>,
        time: f64,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                ctx: egui::Context::default(),
                config: crate::config::Config::default(),
                plugins: crate::plugin::manager::PluginManager::new(),
                breakpoints: Default::default(),
                lsp_hover: None,
                time: 0.0,
            }
        }

        fn frame(&mut self, ed: &mut Editor, events: Vec<Event>, m: Modifiers) -> egui::FullOutput {
            self.time += 1.0 / 60.0;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                time: Some(self.time),
                modifiers: m,
                events,
                focused: true,
                ..Default::default()
            };
            let palette = crate::ui::theme::Palette::from_theme(&self.config.theme);
            let lsp = self.lsp_hover.take();
            let (config, plugins, bps) = (&self.config, &self.plugins, &self.breakpoints);
            self.ctx.run(raw, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        ed.show(
                            ui,
                            config,
                            plugins,
                            lsp.clone(),
                            bps,
                            palette,
                            crate::ui::theme::Spacing::default(),
                        );
                    });
            })
        }

        fn idle(&mut self, ed: &mut Editor) -> egui::FullOutput {
            self.frame(ed, vec![], NONE)
        }

        fn press(&mut self, ed: &mut Editor, key: Key, m: Modifiers) -> egui::FullOutput {
            let ev = Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: m,
            };
            self.frame(ed, vec![ev], m)
        }

        fn type_text(&mut self, ed: &mut Editor, s: &str) {
            self.frame(ed, vec![Event::Text(s.to_string())], NONE);
        }

        fn pos(ed: &Editor, row: usize, col: usize) -> egui::Pos2 {
            // Gutter is 50px wide with line numbers on; editor rect starts at (0, 0).
            egui::pos2(
                50.0 + col as f32 * ed.char_width + ed.char_width * 0.2,
                row as f32 * ed.line_height + ed.line_height * 0.5,
            )
        }

        fn button(pos: egui::Pos2, pressed: bool, m: Modifiers) -> Event {
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: m,
            }
        }

        /// Move the pointer off the editor (outside the screen rect).
        fn leave(&mut self, ed: &mut Editor) {
            self.frame(
                ed,
                vec![Event::PointerMoved(egui::pos2(900.0, 900.0))],
                NONE,
            );
        }

        /// A single click, spaced in time so it is never merged into a double click.
        fn click_at(&mut self, ed: &mut Editor, pos: egui::Pos2, m: Modifiers) {
            self.time += 1.0;
            self.fast_click_at(ed, pos, m);
        }

        /// A click with no extra delay (consecutive calls form double/triple clicks).
        fn fast_click_at(&mut self, ed: &mut Editor, pos: egui::Pos2, m: Modifiers) {
            self.frame(
                ed,
                vec![Event::PointerMoved(pos), Self::button(pos, true, m)],
                m,
            );
            self.frame(ed, vec![Self::button(pos, false, m)], m);
        }

        fn click(&mut self, ed: &mut Editor, row: usize, col: usize, m: Modifiers) {
            let p = Self::pos(ed, row, col);
            self.click_at(ed, p, m);
        }

        fn hover(&mut self, ed: &mut Editor, pos: egui::Pos2, m: Modifiers) {
            self.frame(ed, vec![Event::PointerMoved(pos)], m);
        }

        fn hover_at(&mut self, ed: &mut Editor, row: usize, col: usize, m: Modifiers) {
            let p = Self::pos(ed, row, col);
            self.hover(ed, p, m);
        }
    }

    /// Editor loaded with `content`, already focused by a warm-up frame.
    fn setup(content: &str) -> (Harness, Editor) {
        setup_path(content, None)
    }

    fn setup_path(content: &str, path: Option<PathBuf>) -> (Harness, Editor) {
        let mut h = Harness::new();
        let mut ed = Editor::new();
        ed.set_content(content.to_string(), path);
        h.idle(&mut ed);
        (h, ed)
    }

    fn text(ed: &Editor) -> String {
        ed.buffer.to_string()
    }

    fn copied(out: &egui::FullOutput) -> Option<String> {
        out.platform_output.commands.iter().find_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
    }

    fn extra(row: usize, col: usize) -> Cursor {
        let mut c = Cursor::new();
        c.set_position(row, col);
        c
    }

    // ── Typing & text events ────────────────────────────────────────────────

    #[test]
    fn typing_inserts_text_with_auto_close() {
        let (mut h, mut ed) = setup("");
        h.type_text(&mut ed, "f(");
        assert_eq!(text(&ed), "f()");
        assert_eq!(ed.cursor.position(), (0, 2));
        assert!(ed.is_modified);
        assert!(ed.signature_help_request_pending);
        assert_eq!((ed.signature_help_row, ed.signature_help_col), (0, 2));
    }

    #[test]
    fn typing_respects_auto_close_config() {
        let (mut h, mut ed) = setup("");
        h.config.editor.auto_close_brackets = false;
        h.type_text(&mut ed, "[");
        assert_eq!(text(&ed), "[");
    }

    #[test]
    fn typing_close_paren_clears_signature_help_and_dot_requests_completion() {
        let (mut h, mut ed) = setup("");
        ed.signature_help_text = Some("fn f(a)".into());
        h.type_text(&mut ed, "a)");
        assert!(ed.signature_help_text.is_none());
        h.type_text(&mut ed, ".");
        assert!(ed.completion_request_pending);
        assert_eq!(
            (ed.completion_trigger_row, ed.completion_trigger_col),
            (0, 3)
        );
    }

    #[test]
    fn text_events_are_ignored_while_ctrl_is_held() {
        let (mut h, mut ed) = setup("");
        h.frame(&mut ed, vec![Event::Text("s".into())], CTRL);
        assert_eq!(text(&ed), "");
    }

    #[test]
    fn typing_a_word_prefix_opens_local_autocomplete() {
        let (mut h, mut ed) = setup("hello_world\n");
        ed.cursor.set_position(1, 0);
        h.type_text(&mut ed, "hel");
        assert!(ed.autocomplete.visible);
        assert_eq!(ed.autocomplete.confirm(), Some("hello_world"));
    }

    #[test]
    fn events_are_ignored_without_focus_while_find_is_open() {
        let mut h = Harness::new();
        let mut ed = Editor::new();
        ed.set_content("abc".into(), None);
        ed.show_find = true;
        h.idle(&mut ed);
        h.type_text(&mut ed, "zzz");
        assert_eq!(text(&ed), "abc");
    }

    // ── Undo / redo ─────────────────────────────────────────────────────────

    #[test]
    fn ctrl_z_undoes_and_ctrl_y_or_ctrl_shift_z_redoes() {
        let (mut h, mut ed) = setup("");
        h.type_text(&mut ed, "a");
        assert_eq!(text(&ed), "a");
        h.press(&mut ed, Key::Z, CTRL);
        assert_eq!(text(&ed), "");
        h.press(&mut ed, Key::Y, CTRL);
        assert_eq!(text(&ed), "a");
        h.press(&mut ed, Key::Z, CTRL);
        assert_eq!(text(&ed), "");
        h.press(&mut ed, Key::Z, mods(&[CTRL, SHIFT]));
        assert_eq!(text(&ed), "a");
        assert!(ed.is_modified);
    }

    #[test]
    fn ctrl_z_undoes_a_whole_typing_run() {
        let (mut h, mut ed) = setup("x");
        ed.cursor.set_position(0, 1);
        h.type_text(&mut ed, "abc");
        assert_eq!(text(&ed), "xabc");
        h.press(&mut ed, Key::Z, CTRL);
        assert_eq!(text(&ed), "x");
    }

    #[test]
    fn each_backspace_is_one_undo_step() {
        let (mut h, mut ed) = setup("ab");
        ed.cursor.set_position(0, 2);
        h.press(&mut ed, Key::Backspace, NONE);
        h.press(&mut ed, Key::Backspace, NONE);
        assert_eq!(text(&ed), "");
        h.press(&mut ed, Key::Z, CTRL);
        assert_eq!(text(&ed), "a");
        h.press(&mut ed, Key::Z, CTRL);
        assert_eq!(text(&ed), "ab");
    }

    #[test]
    fn each_delete_is_one_undo_step() {
        let (mut h, mut ed) = setup("ab");
        ed.cursor.set_position(0, 0);
        h.press(&mut ed, Key::Delete, NONE);
        h.press(&mut ed, Key::Delete, NONE);
        assert_eq!(text(&ed), "");
        h.press(&mut ed, Key::Z, CTRL);
        assert_eq!(text(&ed), "b");
        h.press(&mut ed, Key::Z, CTRL);
        assert_eq!(text(&ed), "ab");
    }

    #[test]
    fn typing_bursts_split_on_cursor_moves_and_other_edits() {
        let (mut h, mut ed) = setup("");
        h.type_text(&mut ed, "ab");
        h.type_text(&mut ed, "cd");
        // Moving the cursor (even without a key event, e.g. a click) ends the burst.
        ed.cursor.set_position(0, 1);
        h.type_text(&mut ed, "X");
        assert_eq!(text(&ed), "aXbcd");
        h.press(&mut ed, Key::Z, CTRL);
        assert_eq!(text(&ed), "abcd");
        h.press(&mut ed, Key::Z, CTRL);
        assert_eq!(text(&ed), "");

        // A non-typing edit (Backspace) between runs ends the burst too.
        h.type_text(&mut ed, "xy");
        h.press(&mut ed, Key::Backspace, NONE);
        h.type_text(&mut ed, "z");
        assert_eq!(text(&ed), "xz");
        h.press(&mut ed, Key::Z, CTRL);
        assert_eq!(text(&ed), "x");
        h.press(&mut ed, Key::Z, CTRL);
        assert_eq!(text(&ed), "xy");
        h.press(&mut ed, Key::Z, CTRL);
        assert_eq!(text(&ed), "");
    }

    // ── Editing keys ────────────────────────────────────────────────────────

    #[test]
    fn enter_backspace_and_delete_keys_edit_text() {
        let (mut h, mut ed) = setup("abcd");
        ed.cursor.set_position(0, 2);
        h.press(&mut ed, Key::Enter, NONE);
        assert_eq!(text(&ed), "ab\ncd");
        assert_eq!(ed.cursor.position(), (1, 0));
        h.press(&mut ed, Key::Backspace, NONE);
        assert_eq!(text(&ed), "abcd");
        assert_eq!(ed.cursor.position(), (0, 2));
        h.press(&mut ed, Key::Delete, NONE);
        assert_eq!(text(&ed), "abd");
    }

    #[test]
    fn ctrl_enter_inserts_line_below_and_ctrl_shift_enter_above() {
        let (mut h, mut ed) = setup("abc\ndef");
        ed.cursor.set_position(0, 1);
        ed.extra_cursors.push(extra(1, 1));
        h.press(&mut ed, Key::Enter, CTRL);
        assert_eq!(text(&ed), "abc\n\ndef");
        assert_eq!(ed.cursor.position(), (1, 0));
        assert!(ed.extra_cursors.is_empty());

        ed.cursor.set_position(2, 2);
        h.press(&mut ed, Key::Enter, mods(&[CTRL, SHIFT]));
        assert_eq!(text(&ed), "abc\n\n\ndef");
        assert_eq!(ed.cursor.position(), (2, 0));
    }

    #[test]
    fn tab_inserts_detected_indent_and_shift_tab_does_nothing() {
        let (mut h, mut ed) = setup("x");
        h.press(&mut ed, Key::Tab, NONE);
        assert_eq!(text(&ed), "    x");
        assert_eq!(ed.cursor.position(), (0, 4));
        h.press(&mut ed, Key::Tab, SHIFT);
        assert_eq!(text(&ed), "    x");

        let (mut h, mut ed) = setup("\tfoo\n");
        ed.cursor.set_position(1, 0);
        h.press(&mut ed, Key::Tab, NONE);
        assert_eq!(text(&ed), "\tfoo\n\t");
    }

    #[test]
    fn paste_inserts_text_including_newlines() {
        let (mut h, mut ed) = setup("[]");
        ed.cursor.set_position(0, 1);
        h.frame(&mut ed, vec![Event::Paste("a\nb(".into())], NONE);
        // Paste never auto-closes brackets.
        assert_eq!(text(&ed), "[a\nb(]");
        assert_eq!(ed.cursor.position(), (1, 2));
    }

    #[test]
    fn paste_distributes_lines_across_matching_cursor_count() {
        let (mut h, mut ed) = setup("x\ny\nz");
        ed.cursor.set_position(1, 1);
        ed.extra_cursors.push(extra(0, 1));
        ed.extra_cursors.push(extra(2, 1));
        h.frame(&mut ed, vec![Event::Paste("1\n2\n3".into())], NONE);
        assert_eq!(text(&ed), "x1\ny2\nz3");
        assert_eq!(ed.cursor.position(), (1, 2));
        assert_eq!(ed.extra_cursors[0].position(), (0, 2));
        assert_eq!(ed.extra_cursors[1].position(), (2, 2));
    }

    #[test]
    fn paste_with_mismatched_line_count_pastes_everything_at_each_cursor() {
        let (mut h, mut ed) = setup("a\nb");
        ed.cursor.set_position(0, 1);
        ed.extra_cursors.push(extra(1, 1));
        h.frame(&mut ed, vec![Event::Paste("XY".into())], NONE);
        assert_eq!(text(&ed), "aXY\nbXY");
    }

    // ── Line operations ─────────────────────────────────────────────────────

    #[test]
    fn move_line_up_and_down_with_ctrl_shift_and_alt() {
        let (mut h, mut ed) = setup("a\nb\nc");
        ed.cursor.set_position(1, 0);
        h.press(&mut ed, Key::ArrowUp, mods(&[CTRL, SHIFT]));
        assert_eq!(text(&ed), "b\na\nc");
        assert_eq!(ed.cursor.row, 0);
        // At the top: no-op.
        h.press(&mut ed, Key::ArrowUp, mods(&[CTRL, SHIFT]));
        assert_eq!(text(&ed), "b\na\nc");
        h.press(&mut ed, Key::ArrowDown, mods(&[CTRL, SHIFT]));
        assert_eq!(text(&ed), "a\nb\nc");
        assert_eq!(ed.cursor.row, 1);

        h.press(&mut ed, Key::ArrowDown, ALT);
        assert_eq!(text(&ed), "a\nc\nb");
        assert_eq!(ed.cursor.row, 2);
        // At the bottom: no-op for both bindings.
        h.press(&mut ed, Key::ArrowDown, ALT);
        h.press(&mut ed, Key::ArrowDown, mods(&[CTRL, SHIFT]));
        assert_eq!(text(&ed), "a\nc\nb");
        h.press(&mut ed, Key::ArrowUp, ALT);
        assert_eq!(text(&ed), "a\nb\nc");
        assert_eq!(ed.cursor.row, 1);
        ed.cursor.set_position(0, 0);
        h.press(&mut ed, Key::ArrowUp, ALT);
        assert_eq!(text(&ed), "a\nb\nc");
    }

    #[test]
    fn duplicate_line_with_shift_alt_and_ctrl_shift_d() {
        let (mut h, mut ed) = setup("a\nb");
        ed.cursor.set_position(0, 0);
        h.press(&mut ed, Key::ArrowDown, mods(&[SHIFT, ALT]));
        assert_eq!(text(&ed), "a\na\nb");
        assert_eq!(ed.cursor.row, 1);
        h.press(&mut ed, Key::ArrowUp, mods(&[SHIFT, ALT]));
        assert_eq!(text(&ed), "a\na\na\nb");
        assert_eq!(ed.cursor.row, 1);
        ed.cursor.set_position(2, 0);
        h.press(&mut ed, Key::D, mods(&[CTRL, SHIFT]));
        assert_eq!(text(&ed), "a\na\na\na\nb");
        assert_eq!(ed.cursor.row, 3);
    }

    #[test]
    fn duplicate_last_line_without_trailing_newline() {
        let (mut h, mut ed) = setup("a\nb");
        ed.cursor.set_position(1, 0);
        h.press(&mut ed, Key::D, mods(&[CTRL, SHIFT]));
        // Currently produces "a\nbb\n".
        assert_eq!(text(&ed), "a\nb\nb");
        assert_eq!(ed.cursor.row, 2);
    }

    #[test]
    fn ctrl_shift_k_deletes_lines_of_all_cursors() {
        let (mut h, mut ed) = setup("a\nb\nc\nd");
        ed.cursor.set_position(1, 1);
        h.press(&mut ed, Key::K, mods(&[CTRL, SHIFT]));
        assert_eq!(text(&ed), "a\nc\nd");
        assert_eq!(ed.cursor.position(), (1, 0));

        ed.cursor.set_position(2, 0);
        ed.extra_cursors.push(extra(0, 0));
        h.press(&mut ed, Key::K, mods(&[CTRL, SHIFT]));
        assert_eq!(text(&ed), "c\n");
        assert!(ed.extra_cursors.is_empty());
        assert_eq!(ed.cursor.position(), (1, 0));
    }

    #[test]
    fn ctrl_brackets_indent_and_unindent_lines() {
        let (mut h, mut ed) = setup("ab\ncd");
        ed.cursor.set_position(0, 1);
        h.press(&mut ed, Key::CloseBracket, CTRL);
        assert_eq!(text(&ed), "    ab\ncd");
        assert_eq!(ed.cursor.col, 5);
        h.press(&mut ed, Key::OpenBracket, CTRL);
        assert_eq!(text(&ed), "ab\ncd");
        assert_eq!(ed.cursor.col, 1);

        // Selection covering both lines indents both.
        ed.cursor.set_position(1, 1);
        ed.cursor.sel_anchor = Some((0, 0));
        h.press(&mut ed, Key::CloseBracket, CTRL);
        assert_eq!(text(&ed), "    ab\n    cd");
        // Unindent only strips up to 4 leading spaces.
        ed.buffer = Buffer::from_str("      x\n y");
        ed.cursor.set_position(1, 1);
        ed.cursor.sel_anchor = Some((0, 0));
        h.press(&mut ed, Key::OpenBracket, CTRL);
        assert_eq!(text(&ed), "  x\ny");
        assert_eq!(ed.cursor.col, 0);
    }

    #[test]
    fn ctrl_slash_toggles_line_comment() {
        let (mut h, mut ed) = setup("fn a() {\n    let x = 1;\n}");
        ed.cursor.set_position(1, 4);
        h.press(&mut ed, Key::Slash, CTRL);
        assert_eq!(ed.buffer.line(1), "//     let x = 1;");
        h.press(&mut ed, Key::Slash, CTRL);
        assert_eq!(ed.buffer.line(1), "    let x = 1;");

        // Mixed selection: everything gets commented.
        ed.buffer = Buffer::from_str("// a\nb");
        ed.cursor.set_position(1, 1);
        ed.cursor.sel_anchor = Some((0, 0));
        h.press(&mut ed, Key::Slash, CTRL);
        assert_eq!(text(&ed), "// // a\n// b");

        // All commented, one without the trailing space: both get uncommented.
        ed.buffer = Buffer::from_str("  //a\n// b");
        h.press(&mut ed, Key::Slash, CTRL);
        assert_eq!(text(&ed), "  a\nb");
    }

    #[test]
    fn comment_prefix_depends_on_file_extension() {
        let (mut h, mut ed) = setup_path("x = 1", Some(PathBuf::from("a.py")));
        h.press(&mut ed, Key::Slash, CTRL);
        assert_eq!(text(&ed), "# x = 1");
        ed.current_path = Some(PathBuf::from("q.sql"));
        assert_eq!(ed.comment_prefix(), "-- ");
        ed.current_path = Some(PathBuf::from("Makefile"));
        assert_eq!(ed.comment_prefix(), "// ");
    }

    // ── Cursor movement & selection ─────────────────────────────────────────

    #[test]
    fn arrow_keys_move_all_cursors_and_dismiss_autocomplete() {
        let (mut h, mut ed) = setup("abc\ndefgh\nij");
        ed.cursor.set_position(0, 1);
        ed.extra_cursors.push(extra(1, 3));
        ed.autocomplete.visible = true;
        ed.autocomplete.suggestions.clear();
        h.press(&mut ed, Key::ArrowRight, NONE);
        assert_eq!(ed.cursor.position(), (0, 2));
        assert_eq!(ed.extra_cursors[0].position(), (1, 4));
        assert!(!ed.autocomplete.visible);
        h.press(&mut ed, Key::ArrowDown, NONE);
        assert_eq!(ed.cursor.position(), (1, 2));
        assert_eq!(ed.extra_cursors[0].position(), (2, 2));
        h.press(&mut ed, Key::ArrowLeft, NONE);
        assert_eq!(ed.cursor.position(), (1, 1));
        h.press(&mut ed, Key::ArrowUp, NONE);
        assert_eq!(ed.cursor.position(), (0, 1));
        assert_eq!(ed.extra_cursors[0].position(), (1, 1));
        // Moving cursors onto the same spot merges them.
        ed.extra_cursors = vec![extra(0, 0)];
        ed.cursor.set_position(0, 1);
        h.press(&mut ed, Key::ArrowLeft, NONE);
        // primary (0,0), extra stays (0,0) -> deduped.
        assert!(ed.extra_cursors.is_empty());
    }

    #[test]
    fn shift_arrows_extend_selection() {
        let (mut h, mut ed) = setup("abc\ndef");
        ed.cursor.set_position(0, 1);
        ed.extra_cursors.push(extra(1, 1));
        h.press(&mut ed, Key::ArrowRight, SHIFT);
        assert_eq!(ed.cursor.selection_range(), Some(((0, 1), (0, 2))));
        assert_eq!(
            ed.extra_cursors[0].selection_range(),
            Some(((1, 1), (1, 2)))
        );
        h.press(&mut ed, Key::ArrowDown, SHIFT);
        assert_eq!(ed.cursor.selection_range(), Some(((0, 1), (1, 2))));
        h.press(&mut ed, Key::ArrowUp, SHIFT);
        h.press(&mut ed, Key::ArrowLeft, SHIFT);
        h.press(&mut ed, Key::ArrowLeft, SHIFT);
        assert_eq!(ed.cursor.selection_range(), Some(((0, 0), (0, 1))));
        assert_eq!(ed.selected_text().as_deref(), Some("a"));
    }

    #[test]
    fn home_end_variants() {
        let (mut h, mut ed) = setup("  abc\nxy\nlast line");
        ed.cursor.set_position(0, 3);
        ed.extra_cursors.push(extra(1, 1));
        h.press(&mut ed, Key::End, NONE);
        assert_eq!(ed.cursor.position(), (0, 5));
        assert_eq!(ed.extra_cursors[0].position(), (1, 2));
        h.press(&mut ed, Key::Home, NONE);
        assert_eq!(ed.cursor.position(), (0, 0));
        assert_eq!(ed.extra_cursors[0].position(), (1, 0));

        h.press(&mut ed, Key::End, SHIFT);
        assert_eq!(ed.cursor.selection_range(), Some(((0, 0), (0, 5))));
        assert_eq!(
            ed.extra_cursors[0].selection_range(),
            Some(((1, 0), (1, 2)))
        );
        ed.cursor.clear_selection();
        ed.cursor.set_position(0, 4);
        ed.extra_cursors.clear();
        h.press(&mut ed, Key::Home, SHIFT);
        assert_eq!(ed.cursor.selection_range(), Some(((0, 0), (0, 4))));

        ed.extra_cursors.push(extra(1, 1));
        ed.scroll_offset = egui::vec2(0.0, 30.0);
        h.press(&mut ed, Key::End, CTRL);
        assert_eq!(ed.cursor.position(), (2, 9));
        assert!(!ed.cursor.has_selection());
        // The extra cursor lands on the same spot and is merged.
        assert!(ed.extra_cursors.is_empty());
        h.press(&mut ed, Key::Home, CTRL);
        assert_eq!(ed.cursor.position(), (0, 0));
        assert_eq!(ed.scroll_offset, egui::Vec2::ZERO);

        ed.extra_cursors.push(extra(1, 1));
        h.press(&mut ed, Key::End, mods(&[CTRL, SHIFT]));
        assert_eq!(ed.cursor.selection_range(), Some(((0, 0), (2, 9))));
        ed.cursor.clear_selection();
        ed.cursor.set_position(1, 1);
        ed.extra_cursors.push(extra(2, 2));
        h.press(&mut ed, Key::Home, mods(&[CTRL, SHIFT]));
        assert_eq!(ed.cursor.selection_range(), Some(((0, 0), (1, 1))));
        assert!(ed.extra_cursors.is_empty());
    }

    #[test]
    fn ctrl_a_selects_everything() {
        let (mut h, mut ed) = setup("ab\ncde");
        h.press(&mut ed, Key::A, CTRL);
        assert_eq!(ed.cursor.selection_range(), Some(((0, 0), (1, 3))));
        assert_eq!(ed.selected_text().as_deref(), Some("ab\ncde"));
    }

    #[test]
    fn ctrl_alt_arrows_add_cursors_above_and_below() {
        let (mut h, mut ed) = setup("long line\nab\nlong line");
        ed.cursor.set_position(1, 2);
        h.press(&mut ed, Key::ArrowDown, mods(&[CTRL, ALT]));
        assert_eq!(ed.extra_cursors.len(), 1);
        assert_eq!(ed.extra_cursors[0].position(), (2, 2));
        h.press(&mut ed, Key::ArrowUp, mods(&[CTRL, ALT]));
        let mut rows: Vec<_> = ed.extra_cursors.iter().map(|c| c.position()).collect();
        rows.sort();
        // Above the primary and above the extra (which is the primary row -> deduped).
        assert_eq!(rows, vec![(0, 2), (2, 2)]);
        // At document edges nothing more is added.
        h.press(&mut ed, Key::ArrowUp, mods(&[CTRL, ALT]));
        h.press(&mut ed, Key::ArrowDown, mods(&[CTRL, ALT]));
        let mut rows: Vec<_> = ed.extra_cursors.iter().map(|c| c.position()).collect();
        rows.sort();
        assert_eq!(rows, vec![(0, 2), (2, 2)]);
    }

    #[test]
    fn ctrl_d_selects_word_then_adds_next_occurrences() {
        let (mut h, mut ed) = setup("foo bar foo baz foo");
        ed.cursor.set_position(0, 1);
        h.press(&mut ed, Key::D, CTRL);
        assert_eq!(ed.cursor.selection_range(), Some(((0, 0), (0, 3))));
        assert!(ed.extra_cursors.is_empty());
        h.press(&mut ed, Key::D, CTRL);
        assert_eq!(ed.extra_cursors.len(), 1);
        assert_eq!(
            ed.extra_cursors[0].selection_range(),
            Some(((0, 8), (0, 11)))
        );
        h.press(&mut ed, Key::D, CTRL);
        assert_eq!(
            ed.extra_cursors[1].selection_range(),
            Some(((0, 16), (0, 19)))
        );
        // Wrapping back onto the primary occurrence adds nothing new.
        h.press(&mut ed, Key::D, CTRL);
        assert_eq!(ed.extra_cursors.len(), 2);

        // Typing now replaces every selected occurrence.
        h.type_text(&mut ed, "X");
        assert_eq!(text(&ed), "X bar X baz X");
    }

    #[test]
    fn ctrl_d_on_non_word_does_nothing() {
        let (mut h, mut ed) = setup("  ");
        ed.cursor.set_position(0, 1);
        h.press(&mut ed, Key::D, CTRL);
        assert!(!ed.cursor.has_selection());
    }

    #[test]
    fn ctrl_shift_l_selects_all_occurrences() {
        let (mut h, mut ed) = setup("foo x foo\nfoo");
        ed.cursor.set_position(0, 0);
        h.press(&mut ed, Key::L, mods(&[CTRL, SHIFT]));
        assert_eq!(ed.cursor.selection_range(), Some(((0, 0), (0, 3))));
        let sels: Vec<_> = ed
            .extra_cursors
            .iter()
            .map(|c| c.selection_range().unwrap())
            .collect();
        assert_eq!(sels, vec![((0, 6), (0, 9)), ((1, 0), (1, 3))]);

        // Multi-cursor copy joins each cursor's selection with newlines.
        let out = h.press(&mut ed, Key::C, CTRL);
        assert_eq!(copied(&out).as_deref(), Some("foo\nfoo\nfoo"));
    }

    #[test]
    fn ctrl_shift_l_on_single_line_selects_each_occurrence_once() {
        // find_next_occurrence wraps within the start row; the loop must still stop.
        let (mut h, mut ed) = setup("foo foo foo");
        ed.cursor.set_position(0, 5);
        h.press(&mut ed, Key::L, mods(&[CTRL, SHIFT]));
        assert_eq!(ed.cursor.selection_range(), Some(((0, 4), (0, 7))));
        let sels: Vec<_> = ed
            .extra_cursors
            .iter()
            .map(|c| c.selection_range().unwrap())
            .collect();
        assert_eq!(sels, vec![((0, 0), (0, 3)), ((0, 8), (0, 11))]);
    }

    // ── Clipboard ───────────────────────────────────────────────────────────

    #[test]
    fn ctrl_c_copies_selection_or_current_line() {
        let (mut h, mut ed) = setup("hello\nworld");
        ed.cursor.set_position(0, 4);
        ed.cursor.sel_anchor = Some((0, 1));
        let out = h.press(&mut ed, Key::C, CTRL);
        assert_eq!(copied(&out).as_deref(), Some("ell"));
        assert_eq!(text(&ed), "hello\nworld", "copy must not modify");

        ed.cursor.clear_selection();
        ed.cursor.set_position(1, 2);
        let out = h.press(&mut ed, Key::C, CTRL);
        assert_eq!(copied(&out).as_deref(), Some("world\n"));
    }

    #[test]
    fn ctrl_c_with_extra_cursors_copies_each_cursor() {
        let (mut h, mut ed) = setup("abc\ndef\nghi\njkl");
        // Primary: multi-line selection; extras: one without selection, one multi-line.
        ed.cursor.set_position(2, 1);
        ed.cursor.sel_anchor = Some((0, 1));
        ed.extra_cursors.push(extra(3, 0));
        let mut multi = extra(3, 2);
        multi.sel_anchor = Some((2, 0));
        ed.extra_cursors.push(multi);
        let out = h.press(&mut ed, Key::C, CTRL);
        assert_eq!(copied(&out).as_deref(), Some("bc\ndef\ng\njkl\nghi"));

        // Primary without selection copies its whole line.
        ed.cursor.clear_selection();
        ed.cursor.set_position(1, 0);
        ed.extra_cursors = vec![extra(0, 0)];
        let out = h.press(&mut ed, Key::C, CTRL);
        assert_eq!(copied(&out).as_deref(), Some("def\nabc"));
    }

    #[test]
    fn copy_and_cut_events_from_winit_hit_the_clipboard() {
        // egui-winit sends Event::Copy / Event::Cut (no Key event) for Ctrl+C / Ctrl+X.
        let (mut h, mut ed) = setup("hello world");
        ed.cursor.set_position(0, 5);
        ed.cursor.sel_anchor = Some((0, 0));
        let out = h.frame(&mut ed, vec![Event::Copy], NONE);
        assert_eq!(copied(&out).as_deref(), Some("hello"));
        assert_eq!(text(&ed), "hello world");

        let out = h.frame(&mut ed, vec![Event::Cut], NONE);
        assert_eq!(copied(&out).as_deref(), Some("hello"));
        assert_eq!(text(&ed), " world");
    }

    #[test]
    fn ctrl_x_cuts_selection_and_is_undoable() {
        let (mut h, mut ed) = setup("hello world");
        ed.cursor.set_position(0, 5);
        ed.cursor.sel_anchor = Some((0, 0));
        let out = h.press(&mut ed, Key::X, CTRL);
        assert_eq!(copied(&out).as_deref(), Some("hello"));
        assert_eq!(text(&ed), " world");
        h.press(&mut ed, Key::Z, CTRL);
        assert_eq!(text(&ed), "hello world");
    }

    #[test]
    fn ctrl_x_without_selection_does_nothing() {
        let (mut h, mut ed) = setup("hello");
        let out = h.press(&mut ed, Key::X, CTRL);
        assert_eq!(copied(&out), None);
        assert_eq!(text(&ed), "hello");
    }

    // ── Save & request flags ────────────────────────────────────────────────

    #[test]
    fn ctrl_s_saves_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.rs");
        std::fs::write(&path, "").unwrap();
        let (mut h, mut ed) = setup_path("", Some(path.clone()));
        h.type_text(&mut ed, "x");
        assert!(ed.is_modified);
        h.press(&mut ed, Key::S, CTRL);
        assert!(ed.just_saved);
        assert!(!ed.is_modified);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "x");
    }

    #[test]
    fn shortcut_keys_set_request_flags() {
        let (mut h, mut ed) = setup("abc");
        ed.cursor.set_position(0, 2);
        h.press(&mut ed, Key::Space, CTRL);
        assert!(ed.completion_request_pending);
        assert_eq!(
            (ed.completion_trigger_row, ed.completion_trigger_col),
            (0, 2)
        );
        h.press(&mut ed, Key::F, mods(&[CTRL, SHIFT]));
        assert!(ed.format_request_pending);
        assert!(!ed.show_find);
        h.press(&mut ed, Key::F2, NONE);
        assert_eq!(text(&ed), "abc");
        h.press(&mut ed, Key::F, CTRL);
        assert!(ed.show_find);
        assert!(!ed.show_replace);
    }

    #[test]
    fn ctrl_h_opens_find_and_replace() {
        let (mut h, mut ed) = setup("abc");
        h.press(&mut ed, Key::H, CTRL);
        assert!(ed.show_find && ed.show_replace);
        // Render the find bar with the replace row.
        h.idle(&mut ed);
        h.idle(&mut ed);
        assert!(ed.show_find);
    }

    #[test]
    fn escape_clears_extra_cursors_and_selection() {
        let (mut h, mut ed) = setup("abc\ndef");
        ed.cursor.set_position(0, 2);
        ed.cursor.sel_anchor = Some((0, 0));
        ed.extra_cursors.push(extra(1, 1));
        h.press(&mut ed, Key::Escape, NONE);
        assert!(ed.extra_cursors.is_empty());
        assert!(!ed.cursor.has_selection());
    }

    // ── Autocomplete interaction ────────────────────────────────────────────

    fn with_suggestions(ed: &mut Editor, labels: &[&str]) {
        ed.autocomplete.set_lsp_suggestions(
            labels
                .iter()
                .map(|l| autocomplete::Suggestion {
                    label: l.to_string(),
                    kind: None,
                    match_indices: vec![],
                })
                .collect(),
        );
    }

    #[test]
    fn enter_confirms_visible_autocomplete() {
        let (mut h, mut ed) = setup("x wo");
        ed.cursor.set_position(0, 4);
        with_suggestions(&mut ed, &["world", "wombat"]);
        h.press(&mut ed, Key::ArrowDown, NONE);
        assert_eq!(ed.autocomplete.selected, 1);
        assert_eq!(ed.cursor.position(), (0, 4), "arrows navigate the popup");
        h.press(&mut ed, Key::ArrowUp, NONE);
        assert_eq!(ed.autocomplete.selected, 0);
        h.press(&mut ed, Key::Enter, NONE);
        assert_eq!(text(&ed), "x world");
        assert_eq!(ed.cursor.position(), (0, 7));
        assert!(!ed.autocomplete.visible);
    }

    #[test]
    fn tab_confirms_visible_autocomplete() {
        let (mut h, mut ed) = setup("wo");
        ed.cursor.set_position(0, 2);
        with_suggestions(&mut ed, &["wombat"]);
        h.press(&mut ed, Key::Tab, NONE);
        assert_eq!(text(&ed), "wombat");
    }

    #[test]
    fn escape_dismisses_autocomplete_but_keeps_cursors() {
        let (mut h, mut ed) = setup("wo\nwo");
        ed.cursor.set_position(0, 2);
        ed.extra_cursors.push(extra(1, 2));
        with_suggestions(&mut ed, &["wombat"]);
        h.press(&mut ed, Key::Escape, NONE);
        assert!(!ed.autocomplete.visible);
        assert_eq!(ed.extra_cursors.len(), 1);
        assert_eq!(text(&ed), "wo\nwo");
    }

    // ── Find / replace & goto-line dialogs ──────────────────────────────────

    #[test]
    fn find_highlight_survives_case_folding_that_changes_byte_length() {
        // 'İ' (2 bytes) lowercases to "i̇" (3 bytes); the old highlighter sliced the
        // original line with offsets from the lowercased copy and panicked.
        let (mut h, mut ed) = setup("İİİ foo\né foo é foo");
        ed.show_find = true;
        ed.find_query = "FOO".into();
        ed.update_find_matches();
        assert_eq!(ed.find_matches, vec![0, 1]);
        h.idle(&mut ed);
        ed.find_use_regex = true;
        ed.find_query = "f.o|^".into(); // empty matches must be skipped, not loop
        ed.update_find_matches();
        h.idle(&mut ed);
    }

    #[test]
    fn find_bar_renders_matches_and_escape_closes_it() {
        let (mut h, mut ed) = setup("Foo bar\nfoo foo\nnone");
        ed.show_find = true;
        ed.find_query = "foo".into();
        ed.update_find_matches();
        assert_eq!(ed.find_matches, vec![0, 1]);
        h.idle(&mut ed);
        ed.find_use_regex = true;
        ed.find_query = "f.o".into();
        ed.update_find_matches();
        h.idle(&mut ed);
        ed.find_case_sensitive = true;
        ed.update_find_matches();
        assert_eq!(ed.find_matches, vec![1]);
        h.idle(&mut ed);
        ed.find_query = "zzz".into();
        ed.update_find_matches();
        h.idle(&mut ed);

        h.press(&mut ed, Key::Escape, NONE);
        assert!(!ed.show_find);
        assert!(ed.find_query.is_empty());
        assert!(ed.find_matches.is_empty());
    }

    #[test]
    fn goto_line_dialog_captures_typed_input() {
        let content = (0..50)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (mut h, mut ed) = setup(&content);
        ed.extra_cursors.push(extra(5, 0));
        h.press(&mut ed, Key::G, CTRL);
        assert!(ed.show_goto_line);
        h.idle(&mut ed);
        h.idle(&mut ed);
        h.type_text(&mut ed, "40");
        assert_eq!(ed.goto_line_input, "40");
        assert_eq!(
            text(&ed),
            content,
            "typing goes to the dialog, not the buffer"
        );
        assert!(ed.show_goto_line);
    }

    #[test]
    fn goto_line_enter_moves_cursor() {
        let content = (0..50)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (mut h, mut ed) = setup(&content);
        ed.extra_cursors.push(extra(5, 0));
        h.press(&mut ed, Key::G, CTRL);
        h.idle(&mut ed);
        h.idle(&mut ed);
        h.type_text(&mut ed, "40");
        h.press(&mut ed, Key::Enter, NONE);
        assert!(!ed.show_goto_line);
        assert_eq!(ed.cursor.position(), (39, 0));
        assert!(ed.extra_cursors.is_empty());
        assert_eq!(ed.scroll_offset.y, 39.0 * ed.line_height);
    }

    #[test]
    fn goto_line_escape_closes_without_moving() {
        let (mut h, mut ed) = setup("a\nb\nc");
        ed.cursor.set_position(1, 0);
        ed.show_goto_line = true;
        h.idle(&mut ed);
        h.press(&mut ed, Key::Escape, NONE);
        assert!(!ed.show_goto_line);
        assert_eq!(ed.cursor.position(), (1, 0));
    }

    // ── Mouse interaction ───────────────────────────────────────────────────

    #[test]
    fn click_places_cursor_and_clears_extras() {
        let (mut h, mut ed) = setup("hello world\nsecond line");
        ed.extra_cursors.push(extra(0, 0));
        ed.autocomplete.visible = true;
        h.click(&mut ed, 1, 3, NONE);
        assert_eq!(ed.cursor.position(), (1, 3));
        assert!(ed.extra_cursors.is_empty());
        assert!(!ed.autocomplete.visible);
        // Clicking past the end of a line clamps to the line length.
        h.click(&mut ed, 0, 40, NONE);
        assert_eq!(ed.cursor.position(), (0, 11));
    }

    #[test]
    fn shift_click_extends_selection() {
        let (mut h, mut ed) = setup("hello world");
        ed.cursor.set_position(0, 2);
        h.click(&mut ed, 0, 8, SHIFT);
        assert_eq!(ed.cursor.selection_range(), Some(((0, 2), (0, 8))));
    }

    #[test]
    fn ctrl_click_requests_go_to_definition() {
        let (mut h, mut ed) = setup("let value = other;");
        h.click(&mut ed, 0, 14, CTRL);
        assert_eq!(ed.go_to_definition_request.as_deref(), Some("other"));
        assert_eq!(ed.cursor.position(), (0, 14));
    }

    #[test]
    fn double_click_selects_word_and_triple_click_selects_line() {
        let (mut h, mut ed) = setup("alpha beta_gamma delta\nnext");
        let p = Harness::pos(&ed, 0, 8);
        h.click_at(&mut ed, p, NONE);
        h.fast_click_at(&mut ed, p, NONE);
        assert_eq!(ed.cursor.selection_range(), Some(((0, 6), (0, 16))));
        assert_eq!(ed.selected_text().as_deref(), Some("beta_gamma"));
        h.fast_click_at(&mut ed, p, NONE);
        assert_eq!(ed.cursor.selection_range(), Some(((0, 0), (1, 0))));

        // Triple click on the last line selects to its end.
        let p = Harness::pos(&ed, 1, 1);
        h.click_at(&mut ed, p, NONE);
        h.fast_click_at(&mut ed, p, NONE);
        h.fast_click_at(&mut ed, p, NONE);
        assert_eq!(ed.cursor.selection_range(), Some(((1, 0), (1, 4))));
    }

    #[test]
    fn drag_selects_text() {
        let (mut h, mut ed) = setup("hello world\nsecond line");
        let p1 = Harness::pos(&ed, 0, 2);
        // A small vertical wiggle within the same cell starts the drag.
        let mid = p1 + egui::vec2(0.0, 8.0);
        let p2 = Harness::pos(&ed, 1, 4);
        h.frame(
            &mut ed,
            vec![Event::PointerMoved(p1), Harness::button(p1, true, NONE)],
            NONE,
        );
        h.frame(&mut ed, vec![Event::PointerMoved(mid)], NONE);
        h.frame(&mut ed, vec![Event::PointerMoved(p2)], NONE);
        h.frame(&mut ed, vec![Harness::button(p2, false, NONE)], NONE);
        assert_eq!(ed.cursor.selection_range(), Some(((0, 2), (1, 4))));
        assert_eq!(ed.selected_text().as_deref(), Some("llo world\nseco"));
    }

    #[test]
    fn ctrl_drag_adds_cursor_and_shift_drag_extends() {
        let (mut h, mut ed) = setup("hello world\nsecond line");
        let p1 = Harness::pos(&ed, 1, 2);
        let p2 = Harness::pos(&ed, 1, 6);
        h.frame(
            &mut ed,
            vec![Event::PointerMoved(p1), Harness::button(p1, true, CTRL)],
            CTRL,
        );
        h.frame(
            &mut ed,
            vec![Event::PointerMoved(p1 + egui::vec2(0.0, 8.0))],
            CTRL,
        );
        h.frame(&mut ed, vec![Event::PointerMoved(p2)], CTRL);
        h.frame(&mut ed, vec![Harness::button(p2, false, CTRL)], CTRL);
        assert_eq!(ed.extra_cursors.len(), 1);
        assert_eq!(ed.extra_cursors[0].position(), (1, 2));

        let (mut h, mut ed) = setup("hello world\nsecond line");
        ed.cursor.set_position(0, 1);
        let p1 = Harness::pos(&ed, 0, 5);
        let p2 = Harness::pos(&ed, 0, 9);
        h.frame(
            &mut ed,
            vec![Event::PointerMoved(p1), Harness::button(p1, true, SHIFT)],
            SHIFT,
        );
        h.frame(&mut ed, vec![Event::PointerMoved(p2)], SHIFT);
        h.frame(&mut ed, vec![Harness::button(p2, false, SHIFT)], SHIFT);
        assert_eq!(ed.cursor.selection_range(), Some(((0, 1), (0, 9))));
    }

    #[test]
    fn drag_near_bottom_edge_auto_scrolls() {
        let content = (0..200)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (mut h, mut ed) = setup(&content);
        let p1 = Harness::pos(&ed, 2, 0);
        let p2 = egui::pos2(60.0, 595.0);
        h.frame(
            &mut ed,
            vec![Event::PointerMoved(p1), Harness::button(p1, true, NONE)],
            NONE,
        );
        h.frame(&mut ed, vec![Event::PointerMoved(p2)], NONE);
        h.frame(&mut ed, vec![Event::PointerMoved(p2)], NONE);
        assert!(ed.scroll_offset.y > 0.0);
        let p3 = egui::pos2(60.0, 2.0);
        for _ in 0..5 {
            h.frame(&mut ed, vec![Event::PointerMoved(p3)], NONE);
        }
        h.frame(&mut ed, vec![Harness::button(p3, false, NONE)], NONE);
        assert_eq!(ed.scroll_offset.y, 0.0);
    }

    #[test]
    fn gutter_click_toggles_fold() {
        let (mut h, mut ed) = setup("fn a() {\n    x;\n    y;\n}\nend");
        h.idle(&mut ed);
        assert_eq!(ed.fold_regions, vec![(0, 2)]);
        let gutter = egui::pos2(43.0, 10.0);
        h.click_at(&mut ed, gutter, NONE);
        assert!(ed.folded_lines.contains(&0));
        // Rendering with a folded region still works.
        h.idle(&mut ed);
        h.click_at(&mut ed, gutter, NONE);
        assert!(!ed.folded_lines.contains(&0));
        // Non-foldable rows can't be folded.
        h.click_at(&mut ed, egui::pos2(43.0, 70.0), NONE);
        assert!(ed.folded_lines.is_empty());
    }

    #[test]
    fn mouse_wheel_scrolls_vertically_and_shift_wheel_horizontally() {
        let long_line = "x".repeat(400);
        let mut content = vec![long_line];
        content.extend((0..200).map(|i| format!("l{i}")));
        let (mut h, mut ed) = setup(&content.join("\n"));
        let p = egui::pos2(300.0, 300.0);
        h.hover(&mut ed, p, NONE);
        let wheel = |m: Modifiers| Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -200.0),
            modifiers: m,
        };
        h.frame(&mut ed, vec![Event::PointerMoved(p), wheel(NONE)], NONE);
        for _ in 0..30 {
            h.frame(&mut ed, vec![Event::PointerMoved(p)], NONE);
        }
        assert!(ed.scroll_offset.y > 0.0);
        h.frame(&mut ed, vec![Event::PointerMoved(p), wheel(SHIFT)], SHIFT);
        for _ in 0..30 {
            h.frame(&mut ed, vec![Event::PointerMoved(p)], SHIFT);
        }
        assert!(ed.scroll_offset.x > 0.0);
    }

    // ── Hover, diagnostics & rendering state ────────────────────────────────

    #[test]
    fn ctrl_hover_computes_word_bounds() {
        let (mut h, mut ed) = setup("let some_name = 1;");
        let p = Harness::pos(&ed, 0, 6);
        h.hover(&mut ed, p, CTRL);
        assert_eq!(ed.ctrl_hover_word_bounds, Some((0, 4, 13)));
        h.hover_at(&mut ed, 0, 3, CTRL);
        assert_eq!(ed.ctrl_hover_word_bounds, None);
        h.hover(&mut ed, p, NONE);
        assert_eq!(ed.ctrl_hover_word_bounds, None);
    }

    #[test]
    fn hovering_a_word_resolves_signature_from_buffer() {
        let (mut h, mut ed) = setup("fn foo(a: i32) -> i32 {}\nfoo(1);");
        let p = Harness::pos(&ed, 1, 1);
        h.hover(&mut ed, p, NONE);
        assert_eq!(ed.hovered_word(), Some("foo"));
        assert!(!ed.hover_lsp_request_pending, "timer has not fired yet");
        // Pretend the hover timer elapsed.
        ed.hover_start = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        h.hover(&mut ed, p, NONE);
        assert!(ed.hover_lsp_request_pending);
        assert_eq!((ed.hover_row, ed.hover_col), (1, 1));
        let sig = ed
            .hover_signature
            .clone()
            .expect("regex fallback signature");
        assert!(sig.contains("fn foo(a: i32)"), "{sig}");
        // Render the popup.
        h.hover(&mut ed, p, NONE);

        // LSP response overrides the regex result.
        h.lsp_hover = Some("lsp signature".into());
        h.hover(&mut ed, p, NONE);
        assert_eq!(ed.hover_signature.as_deref(), Some("lsp signature"));
        assert!(!ed.hover_lsp_request_pending);
    }

    #[test]
    fn hover_moves_to_empty_space_and_leaves_editor() {
        let (mut h, mut ed) = setup("word   \nx");
        h.hover_at(&mut ed, 0, 1, NONE);
        assert_eq!(ed.hovered_word(), Some("word"));
        // Empty space without a resolved signature clears the hover immediately.
        h.hover_at(&mut ed, 0, 6, NONE);
        assert_eq!(ed.hovered_word(), None);

        // With a signature shown, leaving starts a grace period instead.
        h.hover_at(&mut ed, 0, 1, NONE);
        ed.hover_signature = Some("sig".into());
        h.hover_at(&mut ed, 0, 6, NONE);
        assert!(ed.hover_leave_instant.is_some());
        assert_eq!(ed.hovered_word(), Some("word"));
        // Back on the same word cancels the dismissal.
        h.hover_at(&mut ed, 0, 1, NONE);
        assert!(ed.hover_leave_instant.is_none());

        // Leaving the editor entirely with no signature clears it.
        ed.hover_signature = None;
        h.leave(&mut ed);
        assert_eq!(ed.hovered_word(), None);
        // ... and with a signature starts the grace period.
        h.hover_at(&mut ed, 0, 1, NONE);
        ed.hover_signature = Some("sig".into());
        h.leave(&mut ed);
        assert!(ed.hover_leave_instant.is_some());
    }

    #[test]
    fn hover_grace_period_expiry_clears_popup() {
        let (mut h, mut ed) = setup("word");
        ed.hover_word = Some("word".into());
        ed.hover_signature = Some("sig".into());
        ed.hover_leave_instant =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(2));
        h.idle(&mut ed);
        assert!(ed.hover_word.is_none());
        assert!(ed.hover_signature.is_none());
        assert!(ed.hover_leave_instant.is_none());
    }

    #[test]
    fn hovering_a_diagnostic_sets_its_message() {
        let (mut h, mut ed) = setup("let bad = 1;\nok");
        ed.diagnostics = vec![
            crate::lsp::client::Diagnostic {
                message: "unused variable".into(),
                line: 0,
                col: 4,
                end_col: 7,
                severity: crate::lsp::client::DiagSeverity::Warning,
            },
            crate::lsp::client::Diagnostic {
                message: "point".into(),
                line: 1,
                col: 0,
                end_col: 0,
                severity: crate::lsp::client::DiagSeverity::Error,
            },
        ];
        h.hover_at(&mut ed, 0, 5, NONE);
        assert_eq!(ed.diag_hover_msg.as_deref(), Some("unused variable"));
        assert!(matches!(
            ed.diag_hover_severity,
            crate::lsp::client::DiagSeverity::Warning
        ));
        // Render the tooltip.
        h.hover_at(&mut ed, 0, 5, NONE);
        h.hover_at(&mut ed, 0, 10, NONE);
        assert_eq!(ed.diag_hover_msg, None);
        h.hover_at(&mut ed, 0, 5, NONE);
        h.leave(&mut ed);
        assert_eq!(ed.diag_hover_msg, None);
    }

    #[test]
    fn scroll_to_cursor_brings_cursor_into_view() {
        let content = (0..200)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (mut h, mut ed) = setup(&content);
        ed.cursor.set_position(150, 0);
        ed.scroll_to_cursor = true;
        h.idle(&mut ed);
        assert!(!ed.scroll_to_cursor);
        assert_eq!(ed.scroll_offset.y, 151.0 * ed.line_height - 600.0);
        ed.cursor.set_position(10, 0);
        ed.scroll_to_cursor = true;
        h.idle(&mut ed);
        assert_eq!(ed.scroll_offset.y, 10.0 * ed.line_height);
    }

    #[test]
    fn frame_updates_bracket_match_folds_word_occurrences_and_minimap() {
        let (mut h, mut ed) = setup_path(
            "fn main() {\n    let foo = 1;\n    foo + foo;\n}\n",
            Some(PathBuf::from("main.rs")),
        );
        ed.cursor.set_position(0, 7); // just after '('
        h.idle(&mut ed);
        assert!(ed.bracket_match.is_some());
        assert_eq!(ed.fold_regions, vec![(0, 2)]);
        assert_eq!(ed.minimap_lines_version, ed.content_version);
        assert!(!ed.minimap_lines.is_empty());

        // Selecting a word highlights its other occurrences.
        ed.cursor.set_position(1, 11);
        ed.cursor.sel_anchor = Some((1, 8));
        h.idle(&mut ed);
        assert_eq!(ed.word_occurrences, vec![(2, 4, 7), (2, 10, 13)]);
    }

    #[test]
    fn renders_gutter_decorations_and_tooltips() {
        let (mut h, mut ed) = setup_path(
            "fn main() {\n    call(a, b);\n\tx\n}\n",
            Some(PathBuf::from("main.rs")),
        );
        ed.show_blame = true;
        ed.blame_data = vec![
            crate::git::BlameEntry {
                commit_short: "abc1234".into(),
                author: "A very long author name".into(),
                line: 0,
            },
            crate::git::BlameEntry {
                commit_short: "def5678".into(),
                author: "Bo".into(),
                line: 1,
            },
        ];
        ed.line_diff = vec![diff::DIFF_ADDED, diff::DIFF_MODIFIED, DIFF_UNCHANGED];
        h.breakpoints.insert(1);
        let sevs = [
            crate::lsp::client::DiagSeverity::Error,
            crate::lsp::client::DiagSeverity::Warning,
            crate::lsp::client::DiagSeverity::Info,
            crate::lsp::client::DiagSeverity::Hint,
        ];
        ed.diagnostics = sevs
            .iter()
            .map(|s| crate::lsp::client::Diagnostic {
                message: "msg".into(),
                line: 1,
                col: 4,
                end_col: 8,
                severity: s.clone(),
            })
            .collect();
        ed.cursor.set_position(1, 5);
        ed.extra_cursors.push(extra(2, 1));
        let mut sel = extra(3, 1);
        sel.sel_anchor = Some((2, 0));
        ed.extra_cursors.push(sel);
        ed.signature_help_text = Some("fn call(a: i32, b: i32)".into());
        ed.diag_hover_msg = Some("an error".into());
        ed.hover_signature =
            Some("```rust\nfn call(a: i32)\n```\n---\nSome **docs** with `code`.\n\nMore.".into());
        ed.hover_word = Some("call".into());
        ed.hover_tooltip_anchor = Some(egui::pos2(100.0, 590.0));
        ed.ctrl_hover_word_bounds = Some((1, 4, 8));
        h.idle(&mut ed);
        h.idle(&mut ed);
        // Line numbers off + highlight off + no minimap also render.
        h.config.editor.line_numbers = false;
        h.config.editor.highlight_current_line = false;
        h.config.editor.show_minimap = false;
        ed.folded_lines.insert(0);
        h.idle(&mut ed);
        assert_eq!(
            ed.signature_help_text.as_deref(),
            Some("fn call(a: i32, b: i32)")
        );
        assert_eq!(ed.diag_hover_msg, None, "cleared while the pointer is away");
    }

    #[test]
    fn large_files_use_debounced_or_viewport_highlighting() {
        let mid = (0..2500)
            .map(|i| format!("let v{i} = {i};"))
            .collect::<Vec<_>>()
            .join("\n");
        let (mut h, mut ed) = setup(&mid);
        // First frame records the pending version and defers tokenization.
        assert_eq!(ed.hl_pending_version, ed.content_version);
        assert!(ed.hl_pending_at.is_some());
        ed.hl_pending_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        h.idle(&mut ed);
        assert!(!ed.highlighter.needs_update(ed.content_version));

        let big = (0..4100)
            .map(|i| format!("x{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (mut h, mut ed) = setup(&big);
        assert!(!ed.highlighter.viewport_stale(ed.content_version, 0, 30));
        ed.cursor.set_position(4000, 0);
        ed.scroll_to_cursor = true;
        h.idle(&mut ed);
        h.idle(&mut ed);
        assert!(!ed
            .highlighter
            .viewport_stale(ed.content_version, 3980, 4001));
    }

    #[test]
    fn secondary_click_opens_context_menu_without_editing() {
        let (mut h, mut ed) = setup("hello");
        let p = Harness::pos(&ed, 0, 2);
        let btn = |pressed| Event::PointerButton {
            pos: p,
            button: PointerButton::Secondary,
            pressed,
            modifiers: NONE,
        };
        h.frame(&mut ed, vec![Event::PointerMoved(p), btn(true)], NONE);
        h.frame(&mut ed, vec![btn(false)], NONE);
        h.idle(&mut ed);
        assert_eq!(text(&ed), "hello");
    }
}
