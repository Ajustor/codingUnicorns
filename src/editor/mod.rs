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

use autocomplete::Autocomplete;
use bracket_match::find_matching_bracket;
use buffer::Buffer;
use cursor::Cursor;
use diff::{compute_line_diff, DIFF_ADDED, DIFF_MODIFIED, DIFF_UNCHANGED};
use folding::compute_fold_regions;
use highlight::Highlighter;
use hover::{inline_markdown_job, parse_hover_sections, HoverSection};
use indent::detect_indent;
use std::path::PathBuf;
use utils::{find_next_occurrence, get_word_at};

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
    // ── Breadcrumbs ─────────────────────────────────────────────────────────
    /// Current symbol name at cursor (populated by app from outline).
    pub current_symbol: Option<String>,
    // ── Cursor blink ────────────────────────────────────────────────────────
    /// Epoch-ms of the last cursor movement / keypress, used to reset blink.
    cursor_blink_epoch: std::time::Instant,
    // ── Selection word highlighting ─────────────────────────────────────
    /// Visible occurrences of the word under cursor: (row, col_start, col_end).
    word_occurrences: Vec<(usize, usize, usize)>,
    /// Content version when word_occurrences was last computed.
    word_occurrences_version: i32,
    /// The word that was highlighted (to avoid recomputing when cursor moves within same word).
    word_occurrences_word: String,
}

impl Editor {
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
            go_to_definition_request: None,
            ctrl_hover_word_bounds: None,
            hover_word: None,
            hover_pos: egui::Pos2::ZERO,
            hover_start: None,
            hover_signature: None,
            workspace_path: None,
            content_version: 0,
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
            current_symbol: None,
            line_diff: Vec::new(),
            line_diff_path: None,
            format_request_pending: false,
            wrap_col: 0,
            cursor_blink_epoch: std::time::Instant::now(),
            word_occurrences: vec![],
            word_occurrences_version: -1,
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
        self.current_symbol = None;
        // Detect indentation style from file content
        let (spaces, size) = detect_indent(&content);
        self.detected_indent_spaces = spaces;
        self.detected_indent_size = size;
        if let Some(name) = lang {
            self.highlighter.set_language_from_filename(&name);
        }
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        if let Some(path) = &self.current_path {
            std::fs::write(path, self.buffer.to_string())?;
            self.is_modified = false;
            self.invalidate_line_diff();
        }
        Ok(())
    }

    /// The word currently under the mouse pointer (used to populate `PluginContext`).
    pub fn hovered_word(&self) -> Option<&str> {
        self.hover_word.as_deref()
    }

    /// Search the current buffer for a definition of `word` and return a short signature string.
    fn lookup_signature_in_buffer(&self, word: &str) -> Option<String> {
        let content = self.buffer.to_string();
        for raw_line in content.lines() {
            let trimmed = raw_line.trim();

            // Function definitions
            let fn_needle_paren = format!("fn {}(", word);
            let fn_needle_space = format!("fn {} (", word);
            if (trimmed.contains(&fn_needle_paren) || trimmed.contains(&fn_needle_space))
                && (trimmed.starts_with("fn ")
                    || trimmed.starts_with("pub fn ")
                    || trimmed.starts_with("async fn ")
                    || trimmed.starts_with("pub async fn ")
                    || trimmed.starts_with("pub(crate) fn ")
                    || trimmed.starts_with("unsafe fn ")
                    || trimmed.starts_with("pub unsafe fn "))
            {
                // Strip trailing `{` to keep the signature clean
                let sig = trimmed.trim_end_matches('{').trim_end();
                return Some(sig.to_string());
            }

            // Struct definitions
            if trimmed.starts_with(&format!("struct {} ", word))
                || trimmed.starts_with(&format!("struct {}{}", word, '{'))
                || trimmed.starts_with(&format!("pub struct {} ", word))
                || trimmed.starts_with(&format!("pub struct {}{}", word, '{'))
                || trimmed.starts_with(&format!("pub(crate) struct {} ", word))
            {
                let sig = trimmed.trim_end_matches('{').trim_end();
                return Some(sig.to_string());
            }

            // Enum definitions
            if trimmed.starts_with(&format!("enum {} ", word))
                || trimmed.starts_with(&format!("enum {}{}", word, '{'))
                || trimmed.starts_with(&format!("pub enum {} ", word))
                || trimmed.starts_with(&format!("pub enum {}{}", word, '{'))
            {
                let sig = trimmed.trim_end_matches('{').trim_end();
                return Some(sig.to_string());
            }

            // Type aliases
            if trimmed.starts_with(&format!("type {} ", word))
                || trimmed.starts_with(&format!("pub type {} ", word))
            {
                let sig = trimmed.trim_end_matches(';').trim_end();
                return Some(sig.to_string());
            }

            // Let bindings (typed or inferred)
            if trimmed.starts_with(&format!("let {}: ", word))
                || trimmed.starts_with(&format!("let mut {}: ", word))
                || trimmed.starts_with(&format!("let {} =", word))
                || trimmed.starts_with(&format!("let mut {} =", word))
            {
                // Return just the declaration part (up to `=` or `;`)
                let end = trimmed
                    .find('=')
                    .or_else(|| trimmed.find(';'))
                    .unwrap_or(trimmed.len());
                return Some(trimmed[..end].trim_end().to_string());
            }
        }
        None
    }

    /// Search all source files in the workspace for a definition of `word`.
    fn lookup_signature_in_workspace(&self, word: &str) -> Option<String> {
        let workspace = self.workspace_path.as_ref()?;

        let patterns = [
            format!("fn {}(", word),
            format!("fn {} (", word),
            format!("pub fn {}(", word),
            format!("struct {} ", word),
            format!("struct {}{}", word, '{'),
            format!("pub struct {}", word),
            format!("enum {} ", word),
            format!("pub enum {}", word),
            format!("type {} ", word),
        ];

        let mut stack = vec![(workspace.to_path_buf(), 0usize)];
        let mut files_checked = 0;

        while let Some((dir, depth)) = stack.pop() {
            if depth > 8 || files_checked > 1000 {
                break;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if !matches!(name, "target" | ".git" | "node_modules" | ".cargo") {
                        stack.push((path, depth + 1));
                    }
                } else if path.is_file() {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if matches!(ext, "rs" | "ts" | "js" | "py" | "go") {
                        files_checked += 1;
                        let Ok(content) = std::fs::read_to_string(&path) else {
                            continue;
                        };
                        for line in content.lines() {
                            let trimmed = line.trim();
                            for pattern in &patterns {
                                if trimmed.contains(pattern.as_str()) {
                                    let sig = trimmed.trim_end_matches('{').trim_end();
                                    if !sig.is_empty() {
                                        return Some(sig.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    pub fn insert_char(&mut self, ch: char, auto_close_enabled: bool) {
        self.buffer.checkpoint();

        // Skip-close: if typing a closing char that's already under the cursor, just move right.
        if auto_close_enabled && auto_close::is_closing(ch) {
            let (row, col) = self.cursor.position();
            let line = self.buffer.line(row);
            let chars: Vec<char> = line.chars().collect();
            if col < chars.len() && chars[col] == ch {
                self.cursor.move_right(&self.buffer);
                self.cursor.clear_selection();
                self.cursor_blink_epoch = std::time::Instant::now();
                return;
            }
        }

        // Surround: if there's a selection and typing an opening char, wrap the selection.
        if auto_close_enabled && self.cursor.has_selection() {
            if let Some(close) = auto_close::closing_pair(ch) {
                if let Some(((sr, sc), (er, ec))) = self.cursor.selection_range() {
                    self.buffer.insert_char(er, ec, close);
                    self.buffer.insert_char(sr, sc, ch);
                    if sr == er {
                        self.cursor.set_position(er, ec + 2);
                    } else {
                        self.cursor.set_position(er, ec + 1);
                    }
                    self.cursor.clear_selection();
                    self.is_modified = true;
                    self.content_version = self.content_version.wrapping_add(1);
                    self.cursor_blink_epoch = std::time::Instant::now();
                    return;
                }
            }
        }

        if self.cursor.has_selection() {
            self.delete_selection();
        }
        let (row, col) = self.cursor.position();
        self.buffer.insert_char(row, col, ch);
        self.cursor.move_right(&self.buffer);
        self.cursor.clear_selection();

        // Auto-insert closing bracket/quote if appropriate.
        if auto_close_enabled {
            if let Some(close) = auto_close::closing_pair(ch) {
                let line = self.buffer.line(row);
                let chars: Vec<char> = line.chars().collect();
                let cursor_col = col + 1; // cursor moved right already
                let next_char = chars.get(cursor_col).copied();
                let prev_char = if col > 0 {
                    chars.get(col.saturating_sub(1)).copied()
                } else {
                    None
                };
                if !auto_close::should_skip_quote_auto_close(ch, prev_char)
                    && auto_close::should_auto_close(ch, next_char)
                {
                    let (cur_row, cur_col) = self.cursor.position();
                    self.buffer.insert_char(cur_row, cur_col, close);
                }
            }
        }

        // Adjust extra cursors on the same row that are after the insertion point.
        for ec in &mut self.extra_cursors {
            if ec.row == row && ec.col > col {
                ec.col += 1;
                ec.desired_col = ec.col;
            }
            if let Some((ar, ac)) = ec.sel_anchor {
                if ar == row && ac > col {
                    ec.sel_anchor = Some((ar, ac + 1));
                }
            }
        }

        // Process each extra cursor in order: delete its selection first, then insert.
        let n = self.extra_cursors.len();
        for i in 0..n {
            if self.extra_cursors[i].has_selection() {
                if let Some(((sr, sc), (er, ec))) = self.extra_cursors[i].selection_range() {
                    if sr == er {
                        // Same-line selection: delete the selected characters.
                        let count = ec - sc;
                        for _ in 0..count {
                            self.buffer.delete_char(sr, sc);
                        }
                        self.extra_cursors[i].set_position(sr, sc);
                        self.extra_cursors[i].clear_selection();

                        // Adjust all subsequent extra cursors on the same row.
                        for j in (i + 1)..n {
                            if self.extra_cursors[j].row == sr && self.extra_cursors[j].col > sc {
                                self.extra_cursors[j].col = self.extra_cursors[j]
                                    .col
                                    .saturating_sub(ec)
                                    .saturating_add(sc);
                                self.extra_cursors[j].desired_col = self.extra_cursors[j].col;
                            }
                            if let Some((ar, ac)) = self.extra_cursors[j].sel_anchor {
                                if ar == sr && ac > sc {
                                    let adj = ac.saturating_sub(ec).saturating_add(sc);
                                    self.extra_cursors[j].sel_anchor = Some((ar, adj));
                                }
                            }
                        }
                    } else {
                        // Multi-line selection: move cursor to start and clear selection.
                        self.extra_cursors[i].set_position(sr, sc);
                        self.extra_cursors[i].clear_selection();
                    }
                }
            }

            // Insert the character at the (possibly adjusted) cursor position.
            let (er, ec) = self.extra_cursors[i].position();
            self.buffer.insert_char(er, ec, ch);
            self.extra_cursors[i].move_right(&self.buffer);

            // Adjust all subsequent extra cursors on the same row.
            for j in (i + 1)..n {
                if self.extra_cursors[j].row == er && self.extra_cursors[j].col >= ec {
                    self.extra_cursors[j].col += 1;
                    self.extra_cursors[j].desired_col = self.extra_cursors[j].col;
                }
                if let Some((ar, ac)) = self.extra_cursors[j].sel_anchor {
                    if ar == er && ac >= ec {
                        self.extra_cursors[j].sel_anchor = Some((ar, ac + 1));
                    }
                }
            }
        }

        self.is_modified = true;
        self.content_version = self.content_version.wrapping_add(1);
        self.cursor_blink_epoch = std::time::Instant::now();
    }

    pub fn delete_char_before(&mut self) {
        self.buffer.checkpoint();
        if self.cursor.has_selection() {
            self.delete_selection();
            return;
        }
        let (row, col) = self.cursor.position();

        // Pair-delete: if the char before cursor and the char under cursor form a pair, delete both.
        if col > 0 {
            let line = self.buffer.line(row);
            let chars: Vec<char> = line.chars().collect();
            let prev = chars[col - 1];
            if let Some(expected_close) = auto_close::closing_pair(prev) {
                if col < chars.len() && chars[col] == expected_close {
                    self.buffer.delete_char(row, col);
                    self.buffer.delete_char(row, col - 1);
                    self.cursor.move_left(&self.buffer);
                    self.is_modified = true;
                    self.content_version = self.content_version.wrapping_add(1);
                    self.cursor_blink_epoch = std::time::Instant::now();
                    return;
                }
            }
        }

        if col > 0 {
            self.buffer.delete_char(row, col - 1);
            self.cursor.move_left(&self.buffer);

            // Adjust extra cursors on the same row that are after the deleted column.
            for ec in &mut self.extra_cursors {
                if ec.row == row && ec.col >= col {
                    ec.col -= 1;
                    ec.desired_col = ec.col;
                }
                if let Some((ar, ac)) = ec.sel_anchor {
                    if ar == row && ac >= col {
                        ec.sel_anchor = Some((ar, ac - 1));
                    }
                }
            }
        } else if row > 0 {
            let prev_len = self.buffer.line_len(row - 1);
            self.buffer.join_lines(row);
            self.cursor.set_position(row - 1, prev_len);
        }

        // Process each extra cursor in order: delete its selection if any, else delete char before.
        let n = self.extra_cursors.len();
        for i in 0..n {
            if self.extra_cursors[i].has_selection() {
                if let Some(((sr, sc), (er, ec))) = self.extra_cursors[i].selection_range() {
                    if sr == er {
                        let count = ec - sc;
                        for _ in 0..count {
                            self.buffer.delete_char(sr, sc);
                        }
                        self.extra_cursors[i].set_position(sr, sc);
                        self.extra_cursors[i].clear_selection();

                        // Adjust all subsequent extra cursors on the same row.
                        for j in (i + 1)..n {
                            if self.extra_cursors[j].row == sr && self.extra_cursors[j].col > sc {
                                self.extra_cursors[j].col = self.extra_cursors[j]
                                    .col
                                    .saturating_sub(ec)
                                    .saturating_add(sc);
                                self.extra_cursors[j].desired_col = self.extra_cursors[j].col;
                            }
                            if let Some((ar, ac)) = self.extra_cursors[j].sel_anchor {
                                if ar == sr && ac > sc {
                                    let adj = ac.saturating_sub(ec).saturating_add(sc);
                                    self.extra_cursors[j].sel_anchor = Some((ar, adj));
                                }
                            }
                        }
                    } else {
                        self.extra_cursors[i].set_position(sr, sc);
                        self.extra_cursors[i].clear_selection();
                    }
                }
            } else {
                let (er, ec) = self.extra_cursors[i].position();
                if ec > 0 {
                    self.buffer.delete_char(er, ec - 1);
                    self.extra_cursors[i].move_left(&self.buffer);

                    // Adjust all subsequent extra cursors on the same row.
                    for j in (i + 1)..n {
                        if self.extra_cursors[j].row == er && self.extra_cursors[j].col >= ec {
                            self.extra_cursors[j].col -= 1;
                            self.extra_cursors[j].desired_col = self.extra_cursors[j].col;
                        }
                        if let Some((ar, ac)) = self.extra_cursors[j].sel_anchor {
                            if ar == er && ac >= ec {
                                self.extra_cursors[j].sel_anchor = Some((ar, ac - 1));
                            }
                        }
                    }
                } else if er > 0 {
                    let prev_len = self.buffer.line_len(er - 1);
                    self.buffer.join_lines(er);
                    self.extra_cursors[i].set_position(er - 1, prev_len);
                }
            }
        }

        self.is_modified = true;
        self.content_version = self.content_version.wrapping_add(1);
    }

    pub fn insert_newline(&mut self) {
        self.buffer.checkpoint();
        if self.cursor.has_selection() {
            self.delete_selection();
        }
        let (row, col) = self.cursor.position();
        self.buffer.split_line(row, col);
        self.cursor.set_position(row + 1, 0);

        // Process each extra cursor in order: delete its selection if any, then insert newline.
        let n = self.extra_cursors.len();
        for i in 0..n {
            if self.extra_cursors[i].has_selection() {
                if let Some(((sr, sc), (er, ec))) = self.extra_cursors[i].selection_range() {
                    if sr == er {
                        let count = ec - sc;
                        for _ in 0..count {
                            self.buffer.delete_char(sr, sc);
                        }
                        self.extra_cursors[i].set_position(sr, sc);
                        self.extra_cursors[i].clear_selection();

                        // Adjust subsequent extra cursors on the same row.
                        for j in (i + 1)..n {
                            if self.extra_cursors[j].row == sr && self.extra_cursors[j].col > sc {
                                self.extra_cursors[j].col = self.extra_cursors[j]
                                    .col
                                    .saturating_sub(ec)
                                    .saturating_add(sc);
                                self.extra_cursors[j].desired_col = self.extra_cursors[j].col;
                            }
                            if let Some((ar, ac)) = self.extra_cursors[j].sel_anchor {
                                if ar == sr && ac > sc {
                                    let adj = ac.saturating_sub(ec).saturating_add(sc);
                                    self.extra_cursors[j].sel_anchor = Some((ar, adj));
                                }
                            }
                        }
                    } else {
                        self.extra_cursors[i].set_position(sr, sc);
                        self.extra_cursors[i].clear_selection();
                    }
                }
            }

            let (er, ec) = self.extra_cursors[i].position();
            self.buffer.split_line(er, ec);
            self.extra_cursors[i].set_position(er + 1, 0);
        }

        self.is_modified = true;
        self.content_version = self.content_version.wrapping_add(1);
    }

    pub fn selected_text(&self) -> Option<String> {
        let ((sr, sc), (er, ec)) = self.cursor.selection_range()?;
        let start = self.buffer.char_index(sr, sc);
        let end = self.buffer.char_index(er, ec).min(self.buffer.rope_len());
        Some(self.buffer.rope_slice(start, end))
    }

    pub fn delete_selection(&mut self) {
        if let Some(((sr, sc), (er, ec))) = self.cursor.selection_range() {
            let start = self.buffer.char_index(sr, sc);
            let end = self.buffer.char_index(er, ec).min(self.buffer.rope_len());
            self.buffer.delete_range(start, end);
            self.cursor.set_position(sr, sc);
            self.cursor.clear_selection();
            self.is_modified = true;
            self.content_version = self.content_version.wrapping_add(1);
        }
    }

    /// Replace the first occurrence of `find_query` on the current match line.
    pub fn replace_current(&mut self) {
        if self.find_query.is_empty() {
            return;
        }
        if self.find_matches.is_empty() {
            return;
        }
        let row = self.find_matches[self.find_current];
        let line = self.buffer.line(row);
        let query_lc = self.find_query.to_lowercase();
        if let Some(col) = line.to_lowercase().find(&query_lc) {
            let q_len = self.find_query.chars().count();
            // Delete the match
            for _ in 0..q_len {
                self.buffer.delete_char(row, col);
            }
            // Insert replacement
            for (i, ch) in self.replace_query.chars().enumerate() {
                self.buffer.insert_char(row, col + i, ch);
            }
            self.is_modified = true;
            self.content_version = self.content_version.wrapping_add(1);
            self.update_find_matches();
        }
    }

    /// Replace all occurrences of `find_query` with `replace_query`.
    pub fn replace_all_matches(&mut self) {
        if self.find_query.is_empty() {
            return;
        }
        let query_lc = self.find_query.to_lowercase();
        let q_len = self.find_query.chars().count();
        let rep = self.replace_query.clone();
        let total = self.buffer.num_lines();
        for row in 0..total {
            loop {
                let line = self.buffer.line(row);
                let line_lc = line.to_lowercase();
                if let Some(col) = line_lc.find(&query_lc) {
                    for _ in 0..q_len {
                        self.buffer.delete_char(row, col);
                    }
                    for (i, ch) in rep.chars().enumerate() {
                        self.buffer.insert_char(row, col + i, ch);
                    }
                } else {
                    break;
                }
            }
        }
        self.is_modified = true;
        self.content_version = self.content_version.wrapping_add(1);
        self.update_find_matches();
    }

    fn update_find_matches(&mut self) {
        self.find_matches.clear();
        // Pick up from nearest match to current cursor row
        let cursor_row = self.cursor.row;
        if self.find_query.is_empty() {
            return;
        }
        if self.find_use_regex {
            let pattern = if self.find_case_sensitive {
                regex::Regex::new(&self.find_query)
            } else {
                regex::Regex::new(&format!("(?i){}", self.find_query))
            };
            if let Ok(re) = pattern {
                for i in 0..self.buffer.num_lines() {
                    if re.is_match(&self.buffer.line(i)) {
                        self.find_matches.push(i);
                    }
                }
            }
        } else if self.find_case_sensitive {
            for i in 0..self.buffer.num_lines() {
                if self.buffer.line(i).contains(&self.find_query) {
                    self.find_matches.push(i);
                }
            }
        } else {
            let query = self.find_query.to_lowercase();
            for i in 0..self.buffer.num_lines() {
                if self.buffer.line(i).to_lowercase().contains(&query) {
                    self.find_matches.push(i);
                }
            }
        }
        // Jump to the first match at or after the cursor row, or wrap to first.
        if !self.find_matches.is_empty() {
            self.find_current = self
                .find_matches
                .iter()
                .position(|&r| r >= cursor_row)
                .unwrap_or(0);
            let row = self.find_matches[self.find_current];
            self.cursor.set_position(row, 0);
            self.scroll_to_cursor = true;
        } else {
            self.find_current = 0;
        }
    }

    /// Recompute visible word occurrences if the word under cursor changed.
    /// Only highlights when there is an active selection (not just cursor on a word).
    fn update_word_occurrences(&mut self, first_visible_line: usize, last_visible_line: usize) {
        let (row, col) = self.cursor.position();

        // Only highlight when there is an active selection of a single word
        let word = if let Some(((sr, sc), (er, ec))) = self.cursor.selection_range() {
            if sr == er && ec > sc {
                let line = self.buffer.line(sr);
                let selected: String = line.chars().skip(sc).take(ec - sc).collect();
                if selected.len() >= 3 && selected.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    selected
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            // No selection = no highlighting
            String::new()
        };

        if word == self.word_occurrences_word && self.content_version == self.word_occurrences_version {
            return;
        }

        self.word_occurrences.clear();
        self.word_occurrences_version = self.content_version;
        self.word_occurrences_word = word.clone();

        if word.len() < 3 {
            return;
        }

        let word_chars: Vec<char> = word.chars().collect();
        let word_len = word_chars.len();
        let is_word_char = |c: char| c.is_alphanumeric() || c == '_';

        for line_idx in first_visible_line..=last_visible_line.min(self.buffer.num_lines().saturating_sub(1)) {
            let line = self.buffer.line(line_idx);
            let chars: Vec<char> = line.chars().collect();
            if chars.len() < word_len {
                continue;
            }
            let mut col_pos = 0;
            while col_pos + word_len <= chars.len() {
                if chars[col_pos..col_pos + word_len] == word_chars[..] {
                    let before_ok = col_pos == 0 || !is_word_char(chars[col_pos - 1]);
                    let after_ok = col_pos + word_len >= chars.len() || !is_word_char(chars[col_pos + word_len]);
                    if before_ok && after_ok {
                        let is_cursor_pos = line_idx == row && col_pos <= col && col <= col_pos + word_len;
                        if !is_cursor_pos {
                            self.word_occurrences.push((line_idx, col_pos, col_pos + word_len));
                        }
                    }
                    col_pos += word_len;
                } else {
                    col_pos += 1;
                }
            }
        }
    }

    fn find_next(&mut self) {
        if self.find_matches.is_empty() {
            return;
        }
        self.find_current = (self.find_current + 1) % self.find_matches.len();
        let row = self.find_matches[self.find_current];
        self.cursor.set_position(row, 0);
        self.scroll_to_cursor = true;
    }

    fn find_prev(&mut self) {
        if self.find_matches.is_empty() {
            return;
        }
        if self.find_current == 0 {
            self.find_current = self.find_matches.len() - 1;
        } else {
            self.find_current -= 1;
        }
        let row = self.find_matches[self.find_current];
        self.cursor.set_position(row, 0);
        self.scroll_to_cursor = true;
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

    /// Recompute git diff indicators for the current file.
    pub fn refresh_line_diff(&mut self) {
        let path = match &self.current_path {
            Some(p) => p.clone(),
            None => {
                self.line_diff.clear();
                return;
            }
        };
        if self.line_diff_path.as_ref() == Some(&path) {
            return; // already current
        }
        self.line_diff_path = Some(path.clone());
        self.line_diff = compute_line_diff(&path, self.buffer.num_lines());
    }

    /// Force a refresh of line diff (called after save).
    pub fn invalidate_line_diff(&mut self) {
        self.line_diff_path = None;
    }

    /// Remove extra cursors that share a position with the primary cursor or with each other.
    fn dedup_cursors(&mut self) {
        let primary_pos = self.cursor.position();
        self.extra_cursors.retain(|c| c.position() != primary_pos);
        let mut seen = std::collections::HashSet::new();
        self.extra_cursors.retain(|c| seen.insert(c.position()));
    }

    /// Public wrapper for `current_word_full` (used by app.rs).
    pub fn current_word_full_pub(&self) -> Option<String> {
        self.current_word_full()
    }

    /// Returns the full word under the primary cursor (extending left and right from cursor),
    /// or the existing selection text if a selection is active.
    fn current_word_full(&self) -> Option<String> {
        if let Some(text) = self.selected_text() {
            if !text.is_empty() && !text.contains('\n') {
                return Some(text);
            }
        }
        let (row, col) = self.cursor.position();
        let line = self.buffer.line(row);
        let chars: Vec<char> = line.chars().collect();
        let col = col.min(chars.len());
        let mut start = col;
        while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
            start -= 1;
        }
        let mut end = col;
        while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
            end += 1;
        }
        if start == end {
            return None;
        }
        Some(chars[start..end].iter().collect())
    }

    /// Returns (word_start_col, word) for the partial word ending at the cursor.
    fn current_word_at_cursor(&self) -> (usize, String) {
        let (row, col) = self.cursor.position();
        let line = self.buffer.line(row);
        let chars: Vec<char> = line.chars().collect();
        let col = col.min(chars.len());
        let mut start = col;
        while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
            start -= 1;
        }
        let word: String = chars[start..col].iter().collect();
        (start, word)
    }

    /// Collect all words of length ≥ 2 present in the buffer (for autocomplete suggestions).
    fn buffer_words(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        for i in 0..self.buffer.num_lines() {
            let line = self.buffer.line(i);
            let mut word = String::new();
            for ch in line.chars() {
                if ch.is_alphanumeric() || ch == '_' {
                    word.push(ch);
                } else {
                    if word.chars().count() >= 2 {
                        seen.insert(word.clone());
                    }
                    word.clear();
                }
            }
            if word.chars().count() >= 2 {
                seen.insert(word);
            }
        }
        let mut result: Vec<String> = seen.into_iter().collect();
        result.sort();
        result
    }

    /// Replace the partial word at the cursor with the confirmed autocomplete suggestion.
    fn confirm_autocomplete(&mut self) {
        if let Some(suggestion) = self.autocomplete.confirm() {
            let suggestion = suggestion.to_owned();
            let (word_start, _) = self.current_word_at_cursor();
            let (row, col) = self.cursor.position();
            let start_idx = self.buffer.char_index(row, word_start);
            let end_idx = self.buffer.char_index(row, col);
            self.buffer.checkpoint();
            self.buffer.delete_range(start_idx, end_idx);
            self.buffer.insert_str(row, word_start, &suggestion);
            self.cursor
                .set_position(row, word_start + suggestion.chars().count());
            self.is_modified = true;
            self.content_version = self.content_version.wrapping_add(1);
        }
        self.autocomplete.visible = false;
    }

    fn trigger_autocomplete_update(&mut self) {
        let (_, word) = self.current_word_at_cursor();
        let buffer_words = self.buffer_words();
        let lang = self.highlighter.language.clone();
        let keywords = highlight::keywords_for_language(&lang);
        self.autocomplete.update(&word, &buffer_words, keywords);
    }

    fn all_cursor_rows(&self) -> Vec<usize> {
        let mut rows = vec![self.cursor.row];
        for ec in &self.extra_cursors {
            rows.push(ec.row);
        }
        rows.sort_unstable();
        rows.dedup();
        rows
    }

    /// Returns the set of lines covered by the current selection, or just the cursor lines.
    fn selected_line_rows(&self) -> Vec<usize> {
        if let Some(((sr, _), (er, _))) = self.cursor.selection_range() {
            (sr..=er).collect()
        } else {
            self.all_cursor_rows()
        }
    }

    /// Returns the comment prefix for the current file based on its extension.
    fn comment_prefix(&self) -> &'static str {
        let ext = self
            .current_path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .unwrap_or("");
        match ext {
            "py" | "sh" | "bash" | "zsh" | "fish" | "toml" | "ini" | "cfg" | "conf" | "yaml"
            | "yml" | "rb" | "r" | "pl" => "# ",
            "sql" | "lua" => "-- ",
            _ => "// ",
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        config: &crate::config::Config,
        plugin_manager: &crate::plugin::manager::PluginManager,
        lsp_hover: Option<String>,
        breakpoint_lines: &std::collections::HashSet<usize>,
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
                            egui::Color32::from_rgb(100, 180, 255)
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
                            egui::Color32::from_rgb(100, 180, 255)
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
                        resp.request_focus();
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            do_goto = true;
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
        let line_num_color =
            egui::Color32::from_rgb(fg_color.r() / 2, fg_color.g() / 2, fg_color.b() / 2);
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

                if response.has_focus() || ui.memory(|m| m.focused().is_none()) {
                    ui.input(|i| {
                        let mut text_typed = false;
                        let mut ac_confirm = false;
                        let mut ac_dismiss = false;
                        let mut ac_nav = false;

                        for event in &i.events {
                            match event {
                                egui::Event::Text(text) => {
                                    // Don't insert text when Ctrl is held (shortcuts)
                                    if !i.modifiers.ctrl && !i.modifiers.command {
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
                                }
                                egui::Event::Paste(text) => {
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

                                        self.buffer.checkpoint();
                                        // Insert in reverse order to preserve positions
                                        for (sorted_idx, &(row, col, _)) in positions.iter().enumerate().rev() {
                                            let line_text = lines[sorted_idx];
                                            for (j, ch) in line_text.chars().enumerate() {
                                                self.buffer.insert_char(row, col + j, ch);
                                            }
                                        }
                                        // Update cursor positions (forward order)
                                        for (sorted_idx, &(row, col, is_main)) in positions.iter().enumerate() {
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
                                    text_typed = true;
                                }
                                egui::Event::Key {
                                    key,
                                    pressed: true,
                                    modifiers,
                                    ..
                                } => {
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
                                        egui::Key::Backspace => {
                                            self.delete_char_before();
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
                                        egui::Key::D if modifiers.ctrl => {
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
                                                    // Subsequent presses: add cursor at next occurrence
                                                    let (cur_row, cur_col) = self.cursor.position();
                                                    if let Some((match_row, match_col)) =
                                                        find_next_occurrence(
                                                            &self.buffer,
                                                            &word,
                                                            cur_row,
                                                            cur_col,
                                                        )
                                                    {
                                                        // Save current main cursor as extra, then advance main to next occurrence
                                                        let saved = self.cursor.clone();
                                                        self.extra_cursors.push(saved);
                                                        self.cursor.row = match_row;
                                                        self.cursor.col = match_col + word_len;
                                                        self.cursor.desired_col = self.cursor.col;
                                                        self.cursor.sel_anchor =
                                                            Some((match_row, match_col));
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
                                                    // Stop if we've wrapped past the starting point
                                                    if mr < search_row
                                                        || (mr == search_row
                                                            && mc < search_col
                                                            && search_row != 0)
                                                    {
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
                    let do_copy =
                        ui.input(|i| {
                            i.events.iter().any(|e| matches!(
                        e,
                        egui::Event::Key { key: egui::Key::C, pressed: true, modifiers, .. }
                        if modifiers.ctrl
                    ))
                        });
                    let do_cut =
                        ui.input(|i| {
                            i.events.iter().any(|e| matches!(
                        e,
                        egui::Event::Key { key: egui::Key::X, pressed: true, modifiers, .. }
                        if modifiers.ctrl
                    ))
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
                    ui.input(|i| {
                        self.scroll_offset.y -= i.smooth_scroll_delta.y;
                        self.scroll_offset.y = self
                            .scroll_offset
                            .y
                            .max(0.0)
                            .min((total_height - rect.height()).max(0.0));
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

                // Rebuild tree-sitter highlight cache when content changes.
                if self.highlighter.needs_update(self.content_version) {
                    let source = self.buffer.to_string();
                    self.highlighter
                        .highlight_document(&source, self.content_version);
                }

                // Fold regions: recompute if dirty (on every content change)
                if self.fold_regions.is_empty() || self.content_version % 30 == 0 {
                    self.fold_regions = compute_fold_regions(&self.buffer);
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
                        let tokens = self.highlighter.tokens_for_line(line_idx, &preview);
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
                                line_num_color,
                            );
                        }
                        line_idx = fold_end + 1;
                        continue;
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
                            line_num_color,
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
                            DIFF_ADDED => Some(egui::Color32::from_rgb(80, 200, 80)),
                            DIFF_MODIFIED => Some(egui::Color32::from_rgb(80, 150, 255)),
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
                                &entry.commit_short,
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
                            painter.text(
                                egui::pos2(
                                    rect.min.x + blame_extra_width + 2.0,
                                    y + line_height * 0.5,
                                ),
                                egui::Align2::LEFT_CENTER,
                                "💡",
                                egui::FontId::proportional(11.0),
                                egui::Color32::from_rgb(255, 220, 50),
                            );
                        }
                    }

                    let line = self.buffer.line(line_idx);
                    let x_start = rect.min.x + gutter_width;

                    // Find bar match highlight — precise character-level boxes
                    if !self.find_query.is_empty() && self.find_matches.contains(&line_idx) {
                        let haystack_raw = self.buffer.line(line_idx);
                        let (haystack, needle) = if self.find_case_sensitive {
                            (haystack_raw.clone(), self.find_query.clone())
                        } else {
                            (haystack_raw.to_lowercase(), self.find_query.to_lowercase())
                        };
                        // Collect all match start byte-offsets in this line.
                        let mut search_pos = 0usize;
                        while search_pos <= haystack.len() {
                            let found = if self.find_use_regex {
                                regex::Regex::new(&if self.find_case_sensitive {
                                    needle.clone()
                                } else {
                                    format!("(?i){}", self.find_query)
                                })
                                .ok()
                                .and_then(|re| re.find(&haystack[search_pos..]))
                                .map(|m| (m.start(), m.end()))
                            } else {
                                haystack[search_pos..]
                                    .find(&needle)
                                    .map(|s| (s, s + needle.len()))
                            };
                            match found {
                                None => break,
                                Some((rel_start, rel_end)) => {
                                    let abs_start = search_pos + rel_start;
                                    let abs_end = search_pos + rel_end;
                                    // Convert byte offset → char count for pixel measurement
                                    let pre_chars = haystack_raw[..abs_start].chars().count();
                                    let span_chars =
                                        haystack_raw[abs_start..abs_end].chars().count();
                                    let pre_text: String =
                                        haystack_raw.chars().take(pre_chars).collect();
                                    let span_text: String = haystack_raw
                                        .chars()
                                        .skip(pre_chars)
                                        .take(span_chars)
                                        .collect();
                                    let pre_w = ui.fonts(|f| {
                                        f.layout_no_wrap(
                                            pre_text,
                                            font_id.clone(),
                                            egui::Color32::WHITE,
                                        )
                                        .size()
                                        .x
                                    });
                                    let span_w = ui.fonts(|f| {
                                        f.layout_no_wrap(
                                            span_text,
                                            font_id.clone(),
                                            egui::Color32::WHITE,
                                        )
                                        .size()
                                        .x
                                    });
                                    let hx = x_start + pre_w - self.scroll_offset.x;
                                    if hx < rect.max.x && hx + span_w > x_start {
                                        // Active match: bright yellow; other matches: dim
                                        let is_active = self.find_matches.get(self.find_current)
                                            == Some(&line_idx);
                                        let color = if is_active {
                                            find_highlight_active
                                        } else {
                                            find_highlight
                                        };
                                        painter.rect_filled(
                                            egui::Rect::from_min_size(
                                                egui::pos2(hx, y + 1.0),
                                                egui::vec2(span_w.max(4.0), line_height - 2.0),
                                            ),
                                            2.0,
                                            color,
                                        );
                                    }
                                    search_pos = abs_end.max(abs_start + 1);
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
                                    egui::Color32::from_rgba_premultiplied(
                                        accent_color.r(),
                                        accent_color.g(),
                                        accent_color.b(),
                                        15,
                                    ),
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
                                    egui::Color32::from_rgba_premultiplied(
                                        accent_color.r(),
                                        accent_color.g(),
                                        accent_color.b(),
                                        60,
                                    ),
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
                                        egui::Color32::from_rgba_premultiplied(
                                            accent_color.r(),
                                            accent_color.g(),
                                            accent_color.b(),
                                            60,
                                        ),
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
                                egui::Stroke::new(2.0, cursor_color),
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
                                egui::Stroke::new(2.0, accent_color),
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
                            let gx =
                                x_start + (g * ind_size) as f32 * char_width - self.scroll_offset.x;
                            // clamp to visible text area
                            if gx < x_start || gx > rect.max.x {
                                continue;
                            }
                            painter.line_segment(
                                [egui::pos2(gx, y), egui::pos2(gx, y + line_height)],
                                egui::Stroke::new(
                                    1.0,
                                    egui::Color32::from_rgba_premultiplied(80, 80, 90, 50),
                                ),
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
                                    egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 160, 255)),
                                    egui::StrokeKind::Inside,
                                );
                            }
                        }
                    }

                    // Syntax-highlighted text
                    let tokens = self.highlighter.tokens_for_line(line_idx, &line);
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
                    painter.galley(
                        egui::pos2(x_start, y + line_height * 0.15),
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
                                egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 160, 255)),
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
                                crate::lsp::client::DiagSeverity::Error => {
                                    egui::Color32::from_rgb(255, 80, 80)
                                }
                                crate::lsp::client::DiagSeverity::Warning => {
                                    egui::Color32::from_rgb(255, 200, 0)
                                }
                                _ => egui::Color32::from_rgb(100, 150, 255),
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
                                    egui::Stroke::new(1.0, color),
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
                    let max_line_chars = (0..self.buffer.num_lines())
                        .map(|i| self.buffer.line_len(i))
                        .max()
                        .unwrap_or(0);
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

                // Only grab keyboard focus for the editor when no overlay/popup is open.
                // If show_find, show_goto_line or autocomplete is active, those widgets
                // manage their own focus and we must not steal it every frame.
                if !self.show_find && !self.show_goto_line && !self.autocomplete.visible {
                    ui.memory_mut(|m| {
                        // Only grab focus if nothing else currently has it.
                        if m.focused().is_none() {
                            m.request_focus(response.id);
                        }
                    });
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
                                    .fill(egui::Color32::from_rgb(30, 30, 30))
                                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(75)))
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
                                                                1.0,
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
                                    .fill(egui::Color32::from_rgb(36, 26, 26))
                                    .stroke(egui::Stroke::new(1.5, border_color))
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
                                    .fill(egui::Color32::from_rgb(25, 40, 60))
                                    .stroke(egui::Stroke::new(
                                        1.0,
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
                self.autocomplete.show(ui.ctx());
            });
    }
}
