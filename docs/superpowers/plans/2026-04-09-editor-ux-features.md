# Editor UX Features Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 7 editor UX features (selection highlighting, auto-close brackets, multi-cursor paste, navigation history + F12, git stage/commit/push, 3-panel merge tool, split editor) to bring Coding Unicorns closer to a VS Code experience.

**Architecture:** Each feature is isolated and can be implemented independently. Features 1-3 are small, self-contained changes. Feature 4 adds a navigation history module. Feature 5 extends the existing git module. Feature 6 adds a new merge tool UI. Feature 7 (split editor) is the most invasive, replacing the single editor/tab_manager with a pane system.

**Tech Stack:** Rust, egui 0.31, eframe 0.31, ropey, git2, tree-sitter

---

## File Structure

| File | Responsibility |
|------|---------------|
| `src/editor/mod.rs` (modify) | Selection highlighting, auto-close brackets, multi-cursor paste, F12 keybinding |
| `src/editor/auto_close.rs` (create) | Auto-close pair definitions and logic |
| `src/nav_history.rs` (create) | Navigation history stack (back/forward) |
| `src/git/mod.rs` (modify) | Stage, unstage, commit, push, pull, ahead/behind |
| `src/git/merge.rs` (create) | Conflict file parser, MergeView data structures |
| `src/ui/git_panel.rs` (create) | Interactive git sidebar panel with stage/commit/push |
| `src/ui/merge_panel.rs` (create) | 3-panel merge conflict resolution UI |
| `src/ui/layout.rs` (modify) | Split editor layout, merge tool rendering, git panel integration |
| `src/app.rs` (modify) | Navigation history integration, F12/Alt+Left/Right, split pane system, merge view state |
| `src/config/mod.rs` (modify) | `auto_close_brackets` setting |
| `src/tabs/mod.rs` (modify) | Per-pane tab management for split editor |
| `src/ui/settings.rs` (modify) | Auto-close brackets toggle |
| `src/ui/statusbar.rs` (modify) | Active pane info for split editor |

---

## Task 1: Selection Highlighting

**Files:**
- Modify: `src/editor/mod.rs` (struct fields ~line 131, render loop ~line 2716)

- [ ] **Step 1.1: Add `word_occurrences` field to Editor struct**

In `src/editor/mod.rs`, add after the `cursor_blink_epoch` field (line 130):

```rust
    // ── Selection word highlighting ─────────────────────────────────────
    /// Visible occurrences of the word under cursor: (row, col_start, col_end).
    word_occurrences: Vec<(usize, usize, usize)>,
    /// Content version when word_occurrences was last computed.
    word_occurrences_version: i32,
    /// The word that was highlighted (to avoid recomputing when cursor moves within same word).
    word_occurrences_word: String,
```

And initialize them in `Editor::new()` (find where all fields are initialized):

```rust
    word_occurrences: vec![],
    word_occurrences_version: -1,
    word_occurrences_word: String::new(),
```

- [ ] **Step 1.2: Add occurrence computation method**

In `src/editor/mod.rs`, add a new method to the `impl Editor` block:

```rust
    /// Recompute visible word occurrences if the word under cursor changed.
    fn update_word_occurrences(&mut self, first_visible_line: usize, last_visible_line: usize) {
        let (row, col) = self.cursor.position();

        // Get the selected text or word under cursor
        let word = if let Some(((sr, sc), (er, ec))) = self.cursor.selection_range() {
            // Only highlight if selection is on a single line and looks like a word
            if sr == er {
                let line = self.buffer.line(sr);
                let selected: String = line.chars().skip(sc).take(ec - sc).collect();
                if selected.len() >= 2 && selected.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    selected
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            get_word_at(&self.buffer, row, col).unwrap_or_default()
        };

        // Skip if nothing changed
        if word == self.word_occurrences_word && self.content_version == self.word_occurrences_version {
            return;
        }

        self.word_occurrences.clear();
        self.word_occurrences_version = self.content_version;
        self.word_occurrences_word = word.clone();

        if word.len() < 2 {
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
                    // Check word boundaries
                    let before_ok = col_pos == 0 || !is_word_char(chars[col_pos - 1]);
                    let after_ok = col_pos + word_len >= chars.len() || !is_word_char(chars[col_pos + word_len]);
                    if before_ok && after_ok {
                        // Skip the occurrence at the cursor position itself
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
```

- [ ] **Step 1.3: Call `update_word_occurrences` and render highlights**

In the editor `ui()` method, find the line that computes `first_visible_line` and `last_visible_line` (used to determine which lines to render). Just before the main line rendering loop, add:

```rust
self.update_word_occurrences(first_visible_line, last_visible_line);
```

Then in the per-line rendering loop, after the find-match highlighting block (after line ~2714, before the "Selection highlight" comment at line 2716), add:

```rust
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
                                        40,
                                    ),
                                );
                            }
                        }
                    }
```

- [ ] **Step 1.4: Build and verify**

Run: `rtk cargo build`

Expected: Compiles without errors. When a cursor is on a word, all other visible occurrences of that word should be highlighted with a semi-transparent accent background.

---

## Task 2: Auto-close Brackets/Quotes

**Files:**
- Create: `src/editor/auto_close.rs`
- Modify: `src/editor/mod.rs` (insert_char ~line 377, delete_char_before ~line 462, struct fields)
- Modify: `src/config/mod.rs` (EditorConfig ~line 179)
- Modify: `src/ui/settings.rs` (settings panel)

- [ ] **Step 2.1: Create `src/editor/auto_close.rs`**

```rust
/// Returns the closing character for an opening bracket/quote, if any.
pub fn closing_pair(ch: char) -> Option<char> {
    match ch {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        '\'' => Some('\''),
        '`' => Some('`'),
        _ => None,
    }
}

/// Returns true if the character is a closing bracket/quote.
pub fn is_closing(ch: char) -> bool {
    matches!(ch, ')' | ']' | '}' | '"' | '\'' | '`')
}

/// Returns true if we should auto-close after inserting `ch` given the character
/// that follows the cursor (`next_char`).
pub fn should_auto_close(ch: char, next_char: Option<char>) -> bool {
    match next_char {
        None => true, // end of line
        Some(c) if c.is_whitespace() => true,
        Some(c) if is_closing(c) => true,
        _ => false,
    }
}

/// For quote characters, check if the previous character suggests we should NOT auto-close.
/// E.g. don't close an apostrophe mid-word: `don't`.
pub fn should_skip_quote_auto_close(ch: char, prev_char: Option<char>) -> bool {
    if ch == '\'' || ch == '"' || ch == '`' {
        if let Some(prev) = prev_char {
            return prev.is_alphanumeric();
        }
    }
    false
}
```

- [ ] **Step 2.2: Add `auto_close_brackets` to config**

In `src/config/mod.rs`, add to `EditorConfig` struct (after `auto_save` field at line 184):

```rust
    #[serde(default = "default_true")]
    pub auto_close_brackets: bool,
```

Add the helper function near the `default_terminal_height` function:

```rust
fn default_true() -> bool {
    true
}
```

In the `Default for Config` impl, add to the `editor` block:

```rust
                auto_close_brackets: true,
```

- [ ] **Step 2.3: Wire auto_close module into editor**

In `src/editor/mod.rs`, add to the module declarations at the top:

```rust
pub mod auto_close;
```

- [ ] **Step 2.4: Modify `insert_char()` for auto-close and skip-close**

In `src/editor/mod.rs`, modify the `insert_char()` method. Replace the current method body (lines 377-460) with logic that handles:
1. Skip-close: if typing a closing char and it's already under the cursor, just move right
2. Surround: if there's a selection and typing an opening char, wrap the selection
3. Auto-insert: insert the closing char after the opening char

Replace the beginning of `insert_char` (after `pub fn insert_char(&mut self, ch: char) {`):

```rust
    pub fn insert_char(&mut self, ch: char, auto_close_enabled: bool) {
        self.buffer.checkpoint();

        // ── Skip-close: typing a closing char that's already under the cursor ──
        if auto_close_enabled && auto_close::is_closing(ch) {
            let (row, col) = self.cursor.position();
            let line = self.buffer.line(row);
            let chars: Vec<char> = line.chars().collect();
            if col < chars.len() && chars[col] == ch {
                // Just move cursor past the existing closing char
                self.cursor.move_right(&self.buffer);
                self.cursor.clear_selection();
                self.cursor_blink_epoch = std::time::Instant::now();
                return;
            }
        }

        // ── Surround selection ──────────────────────────────────────────────
        if auto_close_enabled && self.cursor.has_selection() {
            if let Some(close) = auto_close::closing_pair(ch) {
                if let Some(((sr, sc), (er, ec))) = self.cursor.selection_range() {
                    // Insert closing char at end first (so start positions stay valid)
                    self.buffer.insert_char(er, ec, close);
                    self.buffer.insert_char(sr, sc, ch);
                    // Position cursor after the closing bracket
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

        // ── Auto-close: insert closing pair ─────────────────────────────────
        if auto_close_enabled {
            if let Some(close) = auto_close::closing_pair(ch) {
                let line = self.buffer.line(row);
                let chars: Vec<char> = line.chars().collect();
                let next_char = chars.get(col + 1).copied();
                let prev_char = if col > 0 { chars.get(col - 1).copied() } else { None };
                if !auto_close::should_skip_quote_auto_close(ch, prev_char)
                    && auto_close::should_auto_close(ch, next_char)
                {
                    let (cur_row, cur_col) = self.cursor.position();
                    self.buffer.insert_char(cur_row, cur_col, close);
                    // Don't move cursor — it stays between the pair
                }
            }
        }
```

Then keep the rest of the existing extra-cursor adjustment code unchanged (the `for ec in &mut self.extra_cursors` block and everything after).

**Important:** Every call site of `insert_char` must now pass the `auto_close_enabled` parameter. Find all call sites:
- In the text input event loop (~line 1447 area): `self.insert_char(ch)` becomes `self.insert_char(ch, auto_close)`
- The `auto_close` boolean should be read from config at the start of the `ui()` method

- [ ] **Step 2.5: Modify `delete_char_before()` for pair-delete**

In `src/editor/mod.rs`, at the beginning of `delete_char_before()` (after the selection check at line 466), add pair-delete logic:

After `let (row, col) = self.cursor.position();` and before `if col > 0 {`:

```rust
        // Pair-delete: if cursor is between a matching pair like (|), delete both
        if col > 0 {
            let line = self.buffer.line(row);
            let chars: Vec<char> = line.chars().collect();
            let prev = chars[col - 1];
            if let Some(expected_close) = auto_close::closing_pair(prev) {
                if col < chars.len() && chars[col] == expected_close {
                    // Delete the closing char first (col stays valid), then the opening char
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
```

- [ ] **Step 2.6: Add settings toggle**

In `src/ui/settings.rs`, find the auto_save checkbox and add after it:

```rust
                let mut auto_close = config.editor.auto_close_brackets;
                if ui.checkbox(&mut auto_close, "Auto-close brackets").changed() {
                    config.editor.auto_close_brackets = auto_close;
                    changed = true;
                }
```

- [ ] **Step 2.7: Build and verify**

Run: `rtk cargo build`

Expected: Compiles. Typing `(` inserts `()` with cursor between. Typing `)` when cursor is before `)` skips. Backspace between `()` deletes both. Selecting text and typing `(` wraps it.

---

## Task 3: Multi-cursor Paste

**Files:**
- Modify: `src/editor/mod.rs` (paste handler ~line 1450)

- [ ] **Step 3.1: Replace the paste handler with multi-cursor-aware logic**

In `src/editor/mod.rs`, replace the paste handler (lines 1450-1459):

```rust
                                egui::Event::Paste(text) => {
                                    for ch in text.chars() {
                                        if ch == '\n' {
                                            self.insert_newline();
                                        } else {
                                            self.insert_char(ch);
                                        }
                                    }
                                    text_typed = true;
                                }
```

With:

```rust
                                egui::Event::Paste(text) => {
                                    let cursor_count = 1 + self.extra_cursors.len();
                                    let lines: Vec<&str> = text.lines().collect();
                                    // If clipboard has trailing newline, lines() drops the empty last element.
                                    // We count actual lines, not the trailing newline.

                                    if cursor_count > 1 && lines.len() == cursor_count {
                                        // Distribute one line per cursor, sorted by position (top to bottom)
                                        let mut all_cursors: Vec<(usize, usize, usize)> = vec![];
                                        // (row, col, index): index 0 = main cursor, 1+ = extra_cursors[i-1]
                                        let (mr, mc) = self.cursor.position();
                                        all_cursors.push((mr, mc, 0));
                                        for (i, ec) in self.extra_cursors.iter().enumerate() {
                                            let (er, ec_col) = ec.position();
                                            all_cursors.push((er, ec_col, i + 1));
                                        }
                                        all_cursors.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

                                        // Map sorted position to line index
                                        let mut assignments: Vec<(usize, &str)> = vec![]; // (cursor_index, text)
                                        for (sorted_idx, &(_, _, cursor_idx)) in all_cursors.iter().enumerate() {
                                            assignments.push((cursor_idx, lines[sorted_idx]));
                                        }

                                        // Insert in reverse position order to keep earlier positions valid
                                        for &(_, _, cursor_idx) in all_cursors.iter().rev() {
                                            let line_text = assignments.iter().find(|(ci, _)| *ci == cursor_idx).unwrap().1;
                                            if cursor_idx == 0 {
                                                // Main cursor
                                                if self.cursor.has_selection() {
                                                    self.delete_selection();
                                                }
                                                let (row, col) = self.cursor.position();
                                                for ch in line_text.chars() {
                                                    self.buffer.insert_char(row, col + line_text.chars().take_while(|_| true).count(), ch);
                                                }
                                                // Simpler: insert string at position
                                                let (row, col) = self.cursor.position();
                                                for (i, ch) in line_text.chars().enumerate() {
                                                    self.buffer.insert_char(row, col + i, ch);
                                                }
                                                self.cursor.set_position(row, col + line_text.chars().count());
                                            } else {
                                                let ec = &mut self.extra_cursors[cursor_idx - 1];
                                                let (row, col) = ec.position();
                                                for (i, ch) in line_text.chars().enumerate() {
                                                    self.buffer.insert_char(row, col + i, ch);
                                                }
                                                ec.set_position(row, col + line_text.chars().count());
                                            }
                                        }
                                    } else {
                                        // Default: paste full text at each cursor
                                        for ch in text.chars() {
                                            if ch == '\n' {
                                                self.insert_newline();
                                            } else {
                                                self.insert_char(ch, auto_close);
                                            }
                                        }
                                    }
                                    self.is_modified = true;
                                    self.content_version = self.content_version.wrapping_add(1);
                                    text_typed = true;
                                }
```

**Note:** The above is a first pass. The simpler approach that avoids double-insert:

```rust
                                egui::Event::Paste(text) => {
                                    let cursor_count = 1 + self.extra_cursors.len();
                                    let lines: Vec<&str> = text.lines().collect();

                                    if cursor_count > 1 && lines.len() == cursor_count {
                                        // Collect all cursor positions sorted top-to-bottom
                                        let mut positions: Vec<(usize, usize, bool)> = vec![]; // (row, col, is_main)
                                        let (mr, mc) = self.cursor.position();
                                        positions.push((mr, mc, true));
                                        for ec in &self.extra_cursors {
                                            let (er, ec_col) = ec.position();
                                            positions.push((er, ec_col, false));
                                        }
                                        positions.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

                                        self.buffer.checkpoint();
                                        // Insert in reverse order to preserve positions
                                        for (i, &(row, col, _is_main)) in positions.iter().enumerate().rev() {
                                            let line_text = lines[i];
                                            for (j, ch) in line_text.chars().enumerate() {
                                                self.buffer.insert_char(row, col + j, ch);
                                            }
                                        }
                                        // Update cursor positions
                                        for (i, &(row, col, is_main)) in positions.iter().enumerate() {
                                            let new_col = col + lines[i].chars().count();
                                            if is_main {
                                                self.cursor.set_position(row, new_col);
                                            } else {
                                                // Find the matching extra cursor
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
                                        for ch in text.chars() {
                                            if ch == '\n' {
                                                self.insert_newline();
                                            } else {
                                                self.insert_char(ch, auto_close);
                                            }
                                        }
                                    }
                                    text_typed = true;
                                }
```

- [ ] **Step 3.2: Add multi-cursor copy**

Find the Copy/Cut event handler in `src/editor/mod.rs` (search for `Event::Copy`). If multi-cursors are active, collect text from each cursor (selected text or current line) and join with newlines:

```rust
                                egui::Event::Copy => {
                                    if !self.extra_cursors.is_empty() {
                                        // Multi-cursor copy: collect each cursor's selection
                                        let mut parts: Vec<String> = vec![];
                                        // Main cursor
                                        if let Some(((sr, sc), (er, ec))) = self.cursor.selection_range() {
                                            if sr == er {
                                                let line = self.buffer.line(sr);
                                                parts.push(line.chars().skip(sc).take(ec - sc).collect());
                                            } else {
                                                parts.push(self.buffer.line(sr).to_string());
                                            }
                                        } else {
                                            parts.push(self.buffer.line(self.cursor.row).to_string());
                                        }
                                        // Extra cursors
                                        for ec in &self.extra_cursors {
                                            if let Some(((sr, sc), (er, ecc))) = ec.selection_range() {
                                                if sr == er {
                                                    let line = self.buffer.line(sr);
                                                    parts.push(line.chars().skip(sc).take(ecc - sc).collect());
                                                } else {
                                                    parts.push(self.buffer.line(sr).to_string());
                                                }
                                            } else {
                                                parts.push(self.buffer.line(ec.row).to_string());
                                            }
                                        }
                                        ui.ctx().copy_text(parts.join("\n"));
                                    } else {
                                        // existing single-cursor copy logic
                                    }
                                }
```

- [ ] **Step 3.3: Build and verify**

Run: `rtk cargo build`

Expected: With 3 cursors and clipboard containing 3 lines, paste distributes one line per cursor.

---

## Task 4: Navigation History + F12

**Files:**
- Create: `src/nav_history.rs`
- Modify: `src/app.rs` (struct fields, open_file_at_line, keyboard handling)
- Modify: `src/main.rs` (add module declaration)

- [ ] **Step 4.1: Create `src/nav_history.rs`**

```rust
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct NavigationEntry {
    pub path: PathBuf,
    pub row: usize,
    pub col: usize,
}

pub struct NavigationHistory {
    stack: Vec<NavigationEntry>,
    index: usize,
    max_size: usize,
}

impl NavigationHistory {
    pub fn new() -> Self {
        Self {
            stack: vec![],
            index: 0,
            max_size: 50,
        }
    }

    /// Push the current position before navigating away.
    pub fn push(&mut self, path: PathBuf, row: usize, col: usize) {
        // Truncate forward history if we're in the middle of the stack
        if self.index < self.stack.len() {
            self.stack.truncate(self.index);
        }

        // Don't push duplicates of the same position
        if let Some(last) = self.stack.last() {
            if last.path == path && last.row == row {
                return;
            }
        }

        self.stack.push(NavigationEntry { path, row, col });

        // Trim if over max
        if self.stack.len() > self.max_size {
            self.stack.remove(0);
        }

        self.index = self.stack.len();
    }

    /// Go back. Returns the entry to navigate to, or None.
    pub fn go_back(&mut self) -> Option<NavigationEntry> {
        if self.index > 0 {
            self.index -= 1;
            Some(self.stack[self.index].clone())
        } else {
            None
        }
    }

    /// Go forward. Returns the entry to navigate to, or None.
    pub fn go_forward(&mut self) -> Option<NavigationEntry> {
        if self.index + 1 < self.stack.len() {
            self.index += 1;
            Some(self.stack[self.index].clone())
        } else {
            None
        }
    }

    pub fn can_go_back(&self) -> bool {
        self.index > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.index + 1 < self.stack.len()
    }
}
```

- [ ] **Step 4.2: Register the module**

In `src/main.rs`, add:

```rust
mod nav_history;
```

- [ ] **Step 4.3: Add NavigationHistory to app**

In `src/app.rs`, add import:

```rust
use crate::nav_history::NavigationHistory;
```

Add field to `CodingUnicorns` struct (after `debugger_panel` at line 101):

```rust
    pub nav_history: NavigationHistory,
```

Initialize in `CodingUnicorns::new()` (in the `Self { ... }` block):

```rust
            nav_history: NavigationHistory::new(),
```

- [ ] **Step 4.4: Create `push_nav_and_goto` helper**

In `src/app.rs`, add method to `impl CodingUnicorns`:

```rust
    /// Push current position to navigation history, then navigate to target.
    pub fn push_nav_and_goto(&mut self, target_path: PathBuf, target_line: usize) {
        // Push current position
        if let Some(current_path) = self.editor.current_path.clone() {
            let (row, col) = self.editor.cursor.position();
            self.nav_history.push(current_path, row, col);
        }
        self.open_file_at_line(target_path, target_line);
    }
```

- [ ] **Step 4.5: Replace direct `open_file_at_line` calls with `push_nav_and_goto`**

In `src/app.rs`, update `handle_go_to_definition_regex` to use `push_nav_and_goto` instead of `open_file_at_line`:

- Line where `self.open_file_at_line(path, line)` is called in strategy 2 workspace search (~lines 507, 513): replace with `self.push_nav_and_goto(path, line)`
- In the strategy 0 current-file case (~line 442): push nav history before jumping within the same file

In `src/ui/layout.rs`, replace navigation calls from:
- Workspace search results (line ~603): `app.open_file_at_line(path, line)` → `app.push_nav_and_goto(path, line)`
- Outline symbol navigation (line ~704): `app.open_file_at_line(path, line)` → `app.push_nav_and_goto(path, line)`
- References panel (line ~746): `app.open_file_at_line(path, line)` → `app.push_nav_and_goto(path, line)`
- Debugger navigate_to (line ~674): `app.open_file_at_line(path, line)` → `app.push_nav_and_goto(path, line)`

- [ ] **Step 4.6: Add F12 and Alt+Left/Right keybindings**

In `src/app.rs`, in the keyboard handling block (~line 907), add to the `ctx.input()` tuple:

```rust
            // F12 = go to definition
            i.key_pressed(egui::Key::F12),
            // Alt+Left = navigate back
            i.key_pressed(egui::Key::ArrowLeft) && i.modifiers.alt && !i.modifiers.ctrl && !i.modifiers.shift,
            // Alt+Right = navigate forward
            i.key_pressed(egui::Key::ArrowRight) && i.modifiers.alt && !i.modifiers.ctrl && !i.modifiers.shift,
```

And add the corresponding variables to the destructuring: `want_goto_def`, `want_nav_back`, `want_nav_forward`.

Then add the handlers after the existing keybinding handlers:

```rust
        if want_goto_def {
            if let Some(word) = self.editor.current_word_full_pub() {
                self.handle_go_to_definition(&word);
            }
        }
        if want_nav_back {
            if let Some(entry) = self.nav_history.go_back() {
                self.open_file_at_line(entry.path, entry.row);
                // Note: using open_file_at_line directly, not push_nav_and_goto, to avoid pushing during back/forward
            }
        }
        if want_nav_forward {
            if let Some(entry) = self.nav_history.go_forward() {
                self.open_file_at_line(entry.path, entry.row);
            }
        }
```

- [ ] **Step 4.7: Also push nav history when LSP definition response arrives**

Find where the LSP definition response is processed in `src/app.rs` (search for `pending_definition_id`). Before calling `open_file_at_line` there, push the current position:

```rust
        if let Some(current_path) = self.editor.current_path.clone() {
            let (row, col) = self.editor.cursor.position();
            self.nav_history.push(current_path, row, col);
        }
```

- [ ] **Step 4.8: Build and verify**

Run: `rtk cargo build`

Expected: F12 triggers go-to-definition. Alt+Left goes back. Alt+Right goes forward.

---

## Task 5: Git Stage/Commit/Push UI

**Files:**
- Modify: `src/git/mod.rs` (add stage/unstage/commit/push/pull/ahead-behind methods)
- Create: `src/ui/git_panel.rs` (interactive panel)
- Modify: `src/ui/mod.rs` (add module)
- Modify: `src/ui/layout.rs` (replace `app.git_status.show(ui)` with new panel)

- [ ] **Step 5.1: Extend GitStatus with staging info**

In `src/git/mod.rs`, update `FileStatus` to distinguish staged vs unstaged:

```rust
#[derive(Debug, Clone, Default)]
pub struct FileStatus {
    pub path: String,
    pub index_status: FileChangeKind,  // staged status
    pub wt_status: FileChangeKind,     // working tree status
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum FileChangeKind {
    #[default]
    None,
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
}
```

Remove the old `GitFileStatus` enum and update the `load()` method:

```rust
    pub fn load(&mut self, path: PathBuf) {
        self.repo_path = Some(path.clone());
        if let Ok(repo) = git2::Repository::discover(&path) {
            if let Ok(head) = repo.head() {
                if let Some(name) = head.shorthand() {
                    self.branch = name.to_string();
                }
            }
            // Compute ahead/behind
            self.compute_ahead_behind(&repo);

            let mut opts = git2::StatusOptions::new();
            opts.include_untracked(true);
            if let Ok(statuses) = repo.statuses(Some(&mut opts)) {
                self.files = statuses
                    .iter()
                    .filter_map(|s| {
                        let path = s.path()?.to_string();
                        let st = s.status();
                        if st.contains(git2::Status::IGNORED) {
                            return None;
                        }
                        let index_status = if st.contains(git2::Status::INDEX_MODIFIED) {
                            FileChangeKind::Modified
                        } else if st.contains(git2::Status::INDEX_NEW) {
                            FileChangeKind::Added
                        } else if st.contains(git2::Status::INDEX_DELETED) {
                            FileChangeKind::Deleted
                        } else if st.contains(git2::Status::INDEX_RENAMED) {
                            FileChangeKind::Renamed
                        } else {
                            FileChangeKind::None
                        };
                        let wt_status = if st.contains(git2::Status::WT_MODIFIED) {
                            FileChangeKind::Modified
                        } else if st.contains(git2::Status::WT_NEW) || st.contains(git2::Status::WT_NEW) {
                            if st.contains(git2::Status::INDEX_NEW) {
                                FileChangeKind::None // already fully staged
                            } else {
                                FileChangeKind::Untracked
                            }
                        } else if st.contains(git2::Status::WT_DELETED) {
                            FileChangeKind::Deleted
                        } else if st.contains(git2::Status::WT_RENAMED) {
                            FileChangeKind::Renamed
                        } else {
                            FileChangeKind::None
                        };
                        if index_status == FileChangeKind::None && wt_status == FileChangeKind::None {
                            return None;
                        }
                        Some(FileStatus { path, index_status, wt_status })
                    })
                    .collect();
            }
        }
    }
```

- [ ] **Step 5.2: Add ahead/behind, stage, unstage, commit, push, pull methods**

Add fields to `GitStatus`:

```rust
pub struct GitStatus {
    pub branch: String,
    pub files: Vec<FileStatus>,
    pub repo_path: Option<PathBuf>,
    pub ahead: usize,
    pub behind: usize,
    pub last_error: Option<String>,
}
```

Update `new()` to initialize them. Add methods:

```rust
    fn compute_ahead_behind(&mut self, repo: &git2::Repository) {
        self.ahead = 0;
        self.behind = 0;
        if let Ok(head) = repo.head() {
            if let Some(local_oid) = head.target() {
                let branch_name = head.shorthand().unwrap_or("HEAD");
                let upstream_name = format!("refs/remotes/origin/{}", branch_name);
                if let Ok(upstream_ref) = repo.find_reference(&upstream_name) {
                    if let Some(upstream_oid) = upstream_ref.target() {
                        if let Ok((ahead, behind)) = repo.graph_ahead_behind(local_oid, upstream_oid) {
                            self.ahead = ahead;
                            self.behind = behind;
                        }
                    }
                }
            }
        }
    }

    pub fn stage_file(&mut self, file_path: &str) {
        self.last_error = None;
        if let Some(ref repo_path) = self.repo_path {
            match git2::Repository::discover(repo_path) {
                Ok(repo) => {
                    match repo.index() {
                        Ok(mut index) => {
                            let path = std::path::Path::new(file_path);
                            if path.exists() || repo_path.join(file_path).exists() {
                                let _ = index.add_path(std::path::Path::new(file_path));
                            } else {
                                let _ = index.remove_path(std::path::Path::new(file_path));
                            }
                            let _ = index.write();
                        }
                        Err(e) => self.last_error = Some(e.message().to_string()),
                    }
                }
                Err(e) => self.last_error = Some(e.message().to_string()),
            }
        }
        self.refresh();
    }

    pub fn unstage_file(&mut self, file_path: &str) {
        self.last_error = None;
        if let Some(ref repo_path) = self.repo_path {
            if let Ok(repo) = git2::Repository::discover(repo_path) {
                if let Ok(head) = repo.head() {
                    if let Ok(commit) = head.peel_to_commit() {
                        if let Ok(tree) = commit.tree() {
                            let _ = repo.reset_default(Some(tree.as_object()), [file_path]);
                        }
                    }
                }
            }
        }
        self.refresh();
    }

    pub fn stage_all(&mut self) {
        let paths: Vec<String> = self.files.iter()
            .filter(|f| f.wt_status != FileChangeKind::None)
            .map(|f| f.path.clone())
            .collect();
        for path in paths {
            self.stage_file(&path);
        }
    }

    pub fn unstage_all(&mut self) {
        let paths: Vec<String> = self.files.iter()
            .filter(|f| f.index_status != FileChangeKind::None)
            .map(|f| f.path.clone())
            .collect();
        for path in paths {
            self.unstage_file(&path);
        }
    }

    pub fn commit(&mut self, message: &str) -> Result<(), String> {
        self.last_error = None;
        let repo_path = self.repo_path.as_ref().ok_or("No repository")?;
        let repo = git2::Repository::discover(repo_path).map_err(|e| e.message().to_string())?;
        let mut index = repo.index().map_err(|e| e.message().to_string())?;
        let tree_oid = index.write_tree().map_err(|e| e.message().to_string())?;
        let tree = repo.find_tree(tree_oid).map_err(|e| e.message().to_string())?;
        let sig = repo.signature().map_err(|e| e.message().to_string())?;
        let parent = repo.head().ok()
            .and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .map_err(|e| e.message().to_string())?;
        self.refresh();
        Ok(())
    }

    pub fn push(&mut self) -> Result<(), String> {
        self.last_error = None;
        let repo_path = self.repo_path.as_ref().ok_or("No repository")?;
        let repo = git2::Repository::discover(repo_path).map_err(|e| e.message().to_string())?;
        let mut remote = repo.find_remote("origin").map_err(|e| e.message().to_string())?;
        let branch = self.branch.clone();
        let refspec = format!("refs/heads/{}:refs/heads/{}", branch, branch);
        remote.push(&[&refspec], None).map_err(|e| e.message().to_string())?;
        self.refresh();
        Ok(())
    }

    pub fn pull(&mut self) -> Result<(), String> {
        self.last_error = None;
        let repo_path = self.repo_path.as_ref().ok_or("No repository")?;
        let repo = git2::Repository::discover(repo_path).map_err(|e| e.message().to_string())?;
        let mut remote = repo.find_remote("origin").map_err(|e| e.message().to_string())?;
        let branch = self.branch.clone();
        remote.fetch(&[&branch], None, None).map_err(|e| e.message().to_string())?;
        // Fast-forward merge
        let fetch_head = repo.find_reference("FETCH_HEAD").map_err(|e| e.message().to_string())?;
        let fetch_commit = repo.reference_to_annotated_commit(&fetch_head).map_err(|e| e.message().to_string())?;
        let (analysis, _) = repo.merge_analysis(&[&fetch_commit]).map_err(|e| e.message().to_string())?;
        if analysis.is_fast_forward() {
            let mut reference = repo.find_reference(&format!("refs/heads/{}", branch)).map_err(|e| e.message().to_string())?;
            reference.set_target(fetch_commit.id(), "fast-forward").map_err(|e| e.message().to_string())?;
            repo.set_head(&format!("refs/heads/{}", branch)).map_err(|e| e.message().to_string())?;
            repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force())).map_err(|e| e.message().to_string())?;
        } else if analysis.is_normal() {
            return Err("Pull requires a merge — please resolve manually.".to_string());
        }
        self.refresh();
        Ok(())
    }

    pub fn has_staged_files(&self) -> bool {
        self.files.iter().any(|f| f.index_status != FileChangeKind::None)
    }

    pub fn has_conflicts(&self) -> bool {
        // Check if any file has conflict markers
        if let Some(ref repo_path) = self.repo_path {
            if let Ok(repo) = git2::Repository::discover(repo_path) {
                if let Ok(index) = repo.index() {
                    return index.has_conflicts();
                }
            }
        }
        false
    }
```

- [ ] **Step 5.3: Create `src/ui/git_panel.rs`**

```rust
use crate::git::{FileChangeKind, GitStatus};

pub struct GitPanel {
    pub commit_message: String,
}

impl GitPanel {
    pub fn new() -> Self {
        Self {
            commit_message: String::new(),
        }
    }

    /// Returns Some(file_path) if a conflict file was clicked for merge resolution.
    pub fn show(&mut self, ui: &mut egui::Ui, git: &mut GitStatus) -> Option<String> {
        let mut merge_file: Option<String> = None;

        // Branch + ahead/behind
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("⎇ {}", git.branch)).strong());
            if git.ahead > 0 {
                ui.label(egui::RichText::new(format!("↑{}", git.ahead)).color(egui::Color32::GREEN).small());
            }
            if git.behind > 0 {
                ui.label(egui::RichText::new(format!("↓{}", git.behind)).color(egui::Color32::YELLOW).small());
            }
        });
        ui.separator();

        // Commit message input
        ui.label(egui::RichText::new("Message").size(11.0).color(egui::Color32::from_gray(150)));
        ui.add(
            egui::TextEdit::multiline(&mut self.commit_message)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text("Commit message…")
        );

        // Commit + Push/Pull buttons
        ui.horizontal(|ui| {
            let can_commit = !self.commit_message.trim().is_empty() && git.has_staged_files();
            if ui.add_enabled(can_commit, egui::Button::new("Commit")).clicked() {
                let msg = self.commit_message.clone();
                match git.commit(&msg) {
                    Ok(()) => self.commit_message.clear(),
                    Err(e) => git.last_error = Some(e),
                }
            }
            if ui.button("Push").clicked() {
                if let Err(e) = git.push() {
                    git.last_error = Some(e);
                }
            }
            if ui.button("Pull").clicked() {
                if let Err(e) = git.pull() {
                    git.last_error = Some(e);
                }
            }
        });

        // Error display
        if let Some(ref err) = git.last_error {
            ui.label(egui::RichText::new(err).color(egui::Color32::RED).small());
        }

        ui.separator();

        // Staged changes
        let staged: Vec<_> = git.files.iter()
            .filter(|f| f.index_status != FileChangeKind::None)
            .cloned()
            .collect();
        if !staged.is_empty() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("STAGED CHANGES").size(11.0).color(egui::Color32::from_gray(150)));
                if ui.small_button("−").on_hover_text("Unstage All").clicked() {
                    git.unstage_all();
                }
            });
            let mut unstage_path: Option<String> = None;
            egui::ScrollArea::vertical().id_salt("staged").max_height(150.0).show(ui, |ui| {
                for f in &staged {
                    let (icon, color) = change_kind_display(&f.index_status);
                    ui.horizontal(|ui| {
                        if ui.small_button("−").on_hover_text("Unstage").clicked() {
                            unstage_path = Some(f.path.clone());
                        }
                        ui.label(egui::RichText::new(icon).color(color).monospace());
                        ui.label(&f.path);
                    });
                }
            });
            if let Some(path) = unstage_path {
                git.unstage_file(&path);
            }
        }

        ui.separator();

        // Unstaged changes
        let unstaged: Vec<_> = git.files.iter()
            .filter(|f| f.wt_status != FileChangeKind::None)
            .cloned()
            .collect();
        if !unstaged.is_empty() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("CHANGES").size(11.0).color(egui::Color32::from_gray(150)));
                if ui.small_button("+").on_hover_text("Stage All").clicked() {
                    git.stage_all();
                }
            });
            let mut stage_path: Option<String> = None;
            egui::ScrollArea::vertical().id_salt("unstaged").show(ui, |ui| {
                for f in &unstaged {
                    let (icon, color) = change_kind_display(&f.wt_status);
                    ui.horizontal(|ui| {
                        if ui.small_button("+").on_hover_text("Stage").clicked() {
                            stage_path = Some(f.path.clone());
                        }
                        ui.label(egui::RichText::new(icon).color(color).monospace());
                        ui.label(&f.path);
                    });
                }
            });
            if let Some(path) = stage_path {
                git.stage_file(&path);
            }
        }

        if staged.is_empty() && unstaged.is_empty() {
            ui.label(egui::RichText::new("No changes").color(egui::Color32::GRAY));
        }

        merge_file
    }
}

fn change_kind_display(kind: &FileChangeKind) -> (&str, egui::Color32) {
    match kind {
        FileChangeKind::Modified => ("M", egui::Color32::YELLOW),
        FileChangeKind::Added => ("A", egui::Color32::GREEN),
        FileChangeKind::Untracked => ("U", egui::Color32::GREEN),
        FileChangeKind::Deleted => ("D", egui::Color32::RED),
        FileChangeKind::Renamed => ("R", egui::Color32::from_rgb(100, 150, 255)),
        FileChangeKind::None => (" ", egui::Color32::GRAY),
    }
}
```

- [ ] **Step 5.4: Register module and integrate**

In `src/ui/mod.rs`, add:

```rust
pub mod git_panel;
```

In `src/app.rs`, add field:

```rust
    pub git_panel: crate::ui::git_panel::GitPanel,
```

Initialize in `new()`:

```rust
            git_panel: crate::ui::git_panel::GitPanel::new(),
```

In `src/ui/layout.rs`, replace line 606-608:

```rust
                    SidebarTab::Git => {
                        app.git_status.show(ui);
                    }
```

With:

```rust
                    SidebarTab::Git => {
                        let _merge_file = app.git_panel.show(ui, &mut app.git_status);
                        // merge_file handling will be added in Task 6
                    }
```

- [ ] **Step 5.5: Remove old `show()` method from GitStatus**

Delete the `pub fn show(&self, ui: &mut egui::Ui)` method from `src/git/mod.rs` (lines 82-104) since the new `git_panel.rs` replaces it.

- [ ] **Step 5.6: Fix all compilation errors from FileStatus change**

The `FileStatus` struct changed (removed `status: GitFileStatus`, added `index_status` + `wt_status`). Find and fix all references:
- `src/ui/statusbar.rs` if it references `GitFileStatus`
- `src/editor/diff.rs` if it references file status
- Any other file referencing the old enum

- [ ] **Step 5.7: Build and verify**

Run: `rtk cargo build`

Expected: Git sidebar panel shows staged/unstaged files with +/- buttons, commit message input, and commit/push/pull buttons.

---

## Task 6: 3-Panel Merge Tool

**Files:**
- Create: `src/git/merge.rs`
- Create: `src/ui/merge_panel.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/app.rs`
- Modify: `src/ui/layout.rs`

- [ ] **Step 6.1: Create `src/git/merge.rs`**

```rust
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ConflictHunk {
    /// Line range in the result buffer where this conflict sits.
    pub result_line_start: usize,
    pub result_line_end: usize,
    /// "Ours" content lines.
    pub ours: Vec<String>,
    /// "Theirs" content lines.
    pub theirs: Vec<String>,
    /// Current resolution.
    pub resolution: HunkResolution,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HunkResolution {
    Unresolved,
    AcceptOurs,
    AcceptTheirs,
    Manual,
}

#[derive(Debug)]
pub struct ParsedConflict {
    /// Full "ours" version of the file (non-conflict + ours hunks).
    pub ours_content: String,
    /// Full "theirs" version of the file (non-conflict + theirs hunks).
    pub theirs_content: String,
    /// Initial result content (non-conflict lines + ours version of conflicts as default).
    pub result_content: String,
    /// Conflict hunks with line positions in the result.
    pub hunks: Vec<ConflictHunk>,
}

/// Parse a file with conflict markers into ours/theirs/result versions.
pub fn parse_conflict_file(content: &str) -> Option<ParsedConflict> {
    let lines: Vec<&str> = content.lines().collect();
    let mut ours_lines: Vec<String> = vec![];
    let mut theirs_lines: Vec<String> = vec![];
    let mut result_lines: Vec<String> = vec![];
    let mut hunks: Vec<ConflictHunk> = vec![];

    let mut i = 0;
    let mut found_conflict = false;

    while i < lines.len() {
        if lines[i].starts_with("<<<<<<<") {
            found_conflict = true;
            let mut ours: Vec<String> = vec![];
            let mut theirs: Vec<String> = vec![];
            i += 1;

            // Read ours section
            while i < lines.len() && !lines[i].starts_with("=======") {
                ours.push(lines[i].to_string());
                i += 1;
            }
            i += 1; // skip =======

            // Read theirs section
            while i < lines.len() && !lines[i].starts_with(">>>>>>>") {
                theirs.push(lines[i].to_string());
                i += 1;
            }
            i += 1; // skip >>>>>>>

            let result_start = result_lines.len();
            // Default: put ours in result
            for line in &ours {
                result_lines.push(line.clone());
            }
            let result_end = result_lines.len();

            ours_lines.extend(ours.clone());
            theirs_lines.extend(theirs.clone());

            hunks.push(ConflictHunk {
                result_line_start: result_start,
                result_line_end: result_end,
                ours,
                theirs,
                resolution: HunkResolution::Unresolved,
            });
        } else {
            ours_lines.push(lines[i].to_string());
            theirs_lines.push(lines[i].to_string());
            result_lines.push(lines[i].to_string());
            i += 1;
        }
    }

    if !found_conflict {
        return None;
    }

    Some(ParsedConflict {
        ours_content: ours_lines.join("\n"),
        theirs_content: theirs_lines.join("\n"),
        result_content: result_lines.join("\n"),
        hunks,
    })
}
```

- [ ] **Step 6.2: Register git/merge module**

In `src/git/mod.rs`, add:

```rust
pub mod merge;
```

- [ ] **Step 6.3: Create `src/ui/merge_panel.rs`**

```rust
use crate::editor::buffer::Buffer;
use crate::git::merge::{parse_conflict_file, ConflictHunk, HunkResolution, ParsedConflict};
use std::path::PathBuf;

pub struct MergeView {
    pub file_path: PathBuf,
    pub ours_text: String,
    pub theirs_text: String,
    pub result_text: String,
    pub hunks: Vec<ConflictHunk>,
    pub scroll_offset: f32,
    pub is_active: bool,
}

pub enum MergeAction {
    None,
    SaveAndResolve,
    Cancel,
}

impl MergeView {
    pub fn open(file_path: PathBuf) -> Option<Self> {
        let content = std::fs::read_to_string(&file_path).ok()?;
        let parsed = parse_conflict_file(&content)?;
        Some(Self {
            file_path,
            ours_text: parsed.ours_content,
            theirs_text: parsed.theirs_content,
            result_text: parsed.result_content,
            hunks: parsed.hunks,
            scroll_offset: 0.0,
            is_active: true,
        })
    }

    pub fn accept_ours(&mut self, hunk_idx: usize) {
        if let Some(hunk) = self.hunks.get_mut(hunk_idx) {
            hunk.resolution = HunkResolution::AcceptOurs;
        }
        self.rebuild_result();
    }

    pub fn accept_theirs(&mut self, hunk_idx: usize) {
        if let Some(hunk) = self.hunks.get_mut(hunk_idx) {
            hunk.resolution = HunkResolution::AcceptTheirs;
        }
        self.rebuild_result();
    }

    pub fn accept_all_ours(&mut self) {
        for hunk in &mut self.hunks {
            hunk.resolution = HunkResolution::AcceptOurs;
        }
        self.rebuild_result();
    }

    pub fn accept_all_theirs(&mut self) {
        for hunk in &mut self.hunks {
            hunk.resolution = HunkResolution::AcceptTheirs;
        }
        self.rebuild_result();
    }

    fn rebuild_result(&mut self) {
        // Re-parse from original file content and apply resolutions
        if let Ok(content) = std::fs::read_to_string(&self.file_path) {
            let lines: Vec<&str> = content.lines().collect();
            let mut result: Vec<String> = vec![];
            let mut i = 0;
            let mut hunk_idx = 0;

            while i < lines.len() {
                if lines[i].starts_with("<<<<<<<") {
                    let mut ours: Vec<String> = vec![];
                    let mut theirs: Vec<String> = vec![];
                    i += 1;
                    while i < lines.len() && !lines[i].starts_with("=======") {
                        ours.push(lines[i].to_string());
                        i += 1;
                    }
                    i += 1;
                    while i < lines.len() && !lines[i].starts_with(">>>>>>>") {
                        theirs.push(lines[i].to_string());
                        i += 1;
                    }
                    i += 1;

                    if let Some(hunk) = self.hunks.get(hunk_idx) {
                        match hunk.resolution {
                            HunkResolution::AcceptTheirs => result.extend(theirs),
                            _ => result.extend(ours), // Unresolved defaults to ours
                        }
                    }
                    hunk_idx += 1;
                } else {
                    result.push(lines[i].to_string());
                    i += 1;
                }
            }
            self.result_text = result.join("\n");
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> MergeAction {
        let mut action = MergeAction::None;

        // Toolbar
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("MERGE CONFLICT").strong());
            ui.label(
                egui::RichText::new(self.file_path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default())
                    .color(egui::Color32::YELLOW),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    action = MergeAction::Cancel;
                }
                if ui.button("Save & Resolve").clicked() {
                    action = MergeAction::SaveAndResolve;
                }
                if ui.button("Accept All Right").clicked() {
                    self.accept_all_theirs();
                }
                if ui.button("Accept All Left").clicked() {
                    self.accept_all_ours();
                }
            });
        });
        ui.separator();

        // Three panels side by side
        let available = ui.available_size();
        let panel_width = (available.x - 8.0) / 3.0; // 4px gap between panels

        ui.horizontal(|ui| {
            // Left panel: Ours (read-only)
            ui.vertical(|ui| {
                ui.set_width(panel_width);
                ui.label(egui::RichText::new("OURS (current branch)").size(11.0).color(egui::Color32::from_gray(150)));
                egui::ScrollArea::vertical().id_salt("merge_ours").show(ui, |ui| {
                    let mut text = self.ours_text.clone();
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .interactive(false)
                    );
                });
            });

            ui.separator();

            // Center panel: Result (editable)
            ui.vertical(|ui| {
                ui.set_width(panel_width);
                ui.label(egui::RichText::new("RESULT").size(11.0).color(egui::Color32::from_gray(150)));

                // Per-hunk accept buttons
                for (idx, hunk) in self.hunks.iter().enumerate() {
                    let label = match hunk.resolution {
                        HunkResolution::Unresolved => format!("Conflict #{} — ", idx + 1),
                        HunkResolution::AcceptOurs => format!("Conflict #{} ← Ours", idx + 1),
                        HunkResolution::AcceptTheirs => format!("Conflict #{} → Theirs", idx + 1),
                        HunkResolution::Manual => format!("Conflict #{} (manual)", idx + 1),
                    };
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&label).small().color(egui::Color32::YELLOW));
                        if ui.small_button("← Left").clicked() {
                            // Will be applied after the loop to avoid borrow issues
                        }
                        if ui.small_button("Right →").clicked() {
                        }
                    });
                }

                egui::ScrollArea::vertical().id_salt("merge_result").show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.result_text)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                    );
                });
            });

            ui.separator();

            // Right panel: Theirs (read-only)
            ui.vertical(|ui| {
                ui.set_width(panel_width);
                ui.label(egui::RichText::new("THEIRS (incoming)").size(11.0).color(egui::Color32::from_gray(150)));
                egui::ScrollArea::vertical().id_salt("merge_theirs").show(ui, |ui| {
                    let mut text = self.theirs_text.clone();
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .interactive(false)
                    );
                });
            });
        });

        action
    }
}
```

- [ ] **Step 6.4: Register and integrate merge panel**

In `src/ui/mod.rs`, add:

```rust
pub mod merge_panel;
```

In `src/app.rs`, add field:

```rust
    pub merge_view: Option<crate::ui::merge_panel::MergeView>,
```

Initialize as `None` in `new()`.

In `src/ui/layout.rs`, in the `CentralPanel` section (around line 763), add a check before the normal editor rendering:

```rust
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;

            // ── Merge tool takes over the editor area ───────────────────────
            if let Some(ref mut merge_view) = app.merge_view {
                let action = merge_view.show(ui);
                match action {
                    crate::ui::merge_panel::MergeAction::SaveAndResolve => {
                        let path = merge_view.file_path.clone();
                        let content = merge_view.result_text.clone();
                        let _ = std::fs::write(&path, &content);
                        // Stage the resolved file
                        if let Some(rel_path) = app.workspace_path.as_ref()
                            .and_then(|ws| path.strip_prefix(ws).ok())
                            .map(|p| p.to_string_lossy().to_string()) {
                            app.git_status.stage_file(&rel_path);
                        }
                        app.merge_view = None;
                    }
                    crate::ui::merge_panel::MergeAction::Cancel => {
                        app.merge_view = None;
                    }
                    crate::ui::merge_panel::MergeAction::None => {}
                }
                return;
            }

            // Normal editor rendering continues below...
```

- [ ] **Step 6.5: Wire conflict file click in git panel**

Back in `src/ui/layout.rs`, update the Git sidebar to handle merge file clicks:

```rust
                    SidebarTab::Git => {
                        let merge_file = app.git_panel.show(ui, &mut app.git_status);
                        if let Some(file_path) = merge_file {
                            if let Some(ws) = &app.workspace_path {
                                let full_path = ws.join(&file_path);
                                app.merge_view = crate::ui::merge_panel::MergeView::open(full_path);
                            }
                        }
                    }
```

And in `git_panel.rs`, add conflict detection and click handling to the file listing (add a "CONFLICTS" section between STAGED and CHANGES that shows files with conflict markers, returning the path when clicked).

- [ ] **Step 6.6: Build and verify**

Run: `rtk cargo build`

Expected: When a conflicted file exists, clicking it in the git panel opens the 3-panel merge tool. Accept Left/Right buttons resolve hunks. Save & Resolve writes the file and stages it.

---

## Task 7: Split Editor

**Files:**
- Modify: `src/app.rs` (replace single editor/tab_manager with pane system)
- Modify: `src/ui/layout.rs` (dual-pane rendering)
- Modify: `src/tabs/mod.rs` (no structural change, but used per-pane)

This is the most invasive change. It touches nearly every file that references `app.editor` or `app.tab_manager`.

- [ ] **Step 7.1: Add pane fields to app struct**

In `src/app.rs`, add fields to `CodingUnicorns` (keep the existing `editor` and `tab_manager` for now, add split state):

```rust
    /// Second editor pane (None = no split).
    pub editor2: Option<Editor>,
    pub tab_manager2: Option<TabManager>,
    /// Which pane is active: 0 = left/main, 1 = right/split.
    pub active_pane: usize,
    /// Split ratio (0.0 - 1.0), default 0.5.
    pub split_ratio: f32,
```

Initialize in `new()`:

```rust
            editor2: None,
            tab_manager2: None,
            active_pane: 0,
            split_ratio: 0.5,
```

- [ ] **Step 7.2: Add helper methods for active pane access**

In `src/app.rs`:

```rust
    /// Get a reference to the active editor.
    pub fn active_editor(&self) -> &Editor {
        if self.active_pane == 1 {
            self.editor2.as_ref().unwrap_or(&self.editor)
        } else {
            &self.editor
        }
    }

    /// Get a mutable reference to the active editor.
    pub fn active_editor_mut(&mut self) -> &mut Editor {
        if self.active_pane == 1 {
            self.editor2.as_mut().unwrap_or(&mut self.editor)
        } else {
            &mut self.editor
        }
    }

    /// Get a reference to the active tab manager.
    pub fn active_tab_manager(&self) -> &TabManager {
        if self.active_pane == 1 {
            self.tab_manager2.as_ref().unwrap_or(&self.tab_manager)
        } else {
            &self.tab_manager
        }
    }

    /// Get a mutable reference to the active tab manager.
    pub fn active_tab_manager_mut(&mut self) -> &mut TabManager {
        if self.active_pane == 1 {
            self.tab_manager2.as_mut().unwrap_or(&mut self.tab_manager)
        } else {
            &mut self.tab_manager
        }
    }

    /// Split the editor: duplicate current file into a new pane.
    pub fn toggle_split(&mut self) {
        if self.editor2.is_some() {
            // Close split
            self.editor2 = None;
            self.tab_manager2 = None;
            self.active_pane = 0;
        } else {
            // Open split with current file
            let mut editor2 = Editor::new();
            if let Some(ref path) = self.editor.current_path {
                if let Ok(content) = std::fs::read_to_string(path) {
                    editor2.set_content(content.clone(), Some(path.clone()));
                }
            }
            let mut tab_manager2 = TabManager::new();
            if let Some(ref path) = self.editor.current_path {
                if let Ok(content) = std::fs::read_to_string(path) {
                    tab_manager2.open(path.clone(), content);
                }
            }
            self.editor2 = Some(editor2);
            self.tab_manager2 = Some(tab_manager2);
            self.active_pane = 1;
        }
    }
```

- [ ] **Step 7.3: Add Ctrl+\ keybinding**

In `src/app.rs`, in the keyboard handling block, add:

```rust
            // Ctrl+\ = toggle split editor
            i.key_pressed(egui::Key::Backslash) && i.modifiers.ctrl && !i.modifiers.shift && !i.modifiers.alt,
```

And the handler:

```rust
        if want_split {
            self.toggle_split();
        }
```

Add `Backslash` to the `parse_key` method in `src/config/mod.rs` if not already there:

```rust
            "Backslash" | "\\" => Some(egui::Key::Backslash),
```

- [ ] **Step 7.4: Modify layout to render dual panes**

In `src/ui/layout.rs`, in the `CentralPanel` section, after the merge tool check and tab bar, modify the editor rendering to support split:

```rust
            // Determine if we're in split mode
            let is_split = app.editor2.is_some();

            if is_split {
                let available_width = ui.available_width();
                let left_width = available_width * app.split_ratio;
                let right_width = available_width * (1.0 - app.split_ratio);

                ui.horizontal(|ui| {
                    // Left pane
                    ui.vertical(|ui| {
                        ui.set_width(left_width - 2.0);
                        // Detect clicks for focus
                        let pane_rect = ui.available_rect_before_wrap();
                        if ui.input(|i| i.pointer.any_click())
                            && pane_rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default()))
                        {
                            app.active_pane = 0;
                        }

                        // Tab bar for left pane
                        if let Some(path) = app.tab_manager.show(ui) {
                            // open file in left pane
                            app.active_pane = 0;
                            app.open_file(path);
                        }

                        // Editor for left pane
                        // ... render app.editor here (existing editor rendering code)
                    });

                    // Divider (4px draggable)
                    ui.separator();

                    // Right pane
                    ui.vertical(|ui| {
                        ui.set_width(right_width - 2.0);
                        let pane_rect = ui.available_rect_before_wrap();
                        if ui.input(|i| i.pointer.any_click())
                            && pane_rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default()))
                        {
                            app.active_pane = 1;
                        }

                        // Tab bar + editor for right pane
                        if let Some(ref mut tm2) = app.tab_manager2 {
                            if let Some(path) = tm2.show(ui) {
                                app.active_pane = 1;
                                // Open in pane 2
                                if let Ok(content) = std::fs::read_to_string(&path) {
                                    if let Some(ref mut e2) = app.editor2 {
                                        e2.set_content(content, Some(path));
                                    }
                                }
                            }
                        }

                        if let Some(ref mut e2) = app.editor2 {
                            // ... render e2 here
                        }
                    });
                });
            } else {
                // Single pane: existing rendering code
            }
```

**Note:** The actual editor rendering code (the `app.editor.ui()` call with all the LSP/hover/diagnostics logic) is large. For the split pane, extract it into a helper function that takes `&mut Editor` and the shared state, so it can be called for either pane.

- [ ] **Step 7.5: Extract editor rendering into a reusable function**

Create a helper function in `src/ui/layout.rs` or a new file `src/ui/editor_area.rs`:

```rust
pub fn render_editor_pane(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    config: &Config,
    lsp_hover: Option<String>,
    breakpoint_lines: &std::collections::HashSet<usize>,
    is_active: bool,
) {
    // Move the existing editor.ui() call and surrounding logic here
    // The active pane gets a thin accent border to show focus
    if is_active {
        // Draw thin accent border on left side
    }
    editor.workspace_path = /* ... */;
    editor.ui(ui, config, lsp_hover, breakpoint_lines);
}
```

- [ ] **Step 7.6: Handle file opening in correct pane**

Modify `open_file` and `open_file_at_line` to respect `active_pane`:

```rust
    pub fn open_file(&mut self, path: PathBuf) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if self.active_pane == 1 {
                if let Some(ref mut tm2) = self.tab_manager2 {
                    tm2.open(path.clone(), content.clone());
                }
                if let Some(ref mut e2) = self.editor2 {
                    e2.set_content(content.clone(), Some(path.clone()));
                }
            } else {
                self.tab_manager.open(path.clone(), content.clone());
                self.editor.set_content(content.clone(), Some(path.clone()));
            }
            // Common: config save, LSP, etc.
            self.config.last_file = Some(path.to_string_lossy().to_string());
            self.config.save();
            self.ensure_lsp_for_file(&path);
            // LSP didOpen notification...
        }
    }
```

- [ ] **Step 7.7: Close split when last tab of pane 2 is closed**

In the tab close handler, after closing a tab in `tab_manager2`, check if it's empty:

```rust
        if let Some(ref tm2) = self.tab_manager2 {
            if tm2.tabs.is_empty() {
                self.editor2 = None;
                self.tab_manager2 = None;
                self.active_pane = 0;
            }
        }
```

- [ ] **Step 7.8: Build and verify**

Run: `rtk cargo build`

Expected: Ctrl+\ opens a split. Each pane has its own tabs. Clicking in a pane focuses it. Closing all tabs in the right pane collapses back to single view.

---

## Implementation Order Summary

| Order | Task | Risk | Estimated Steps |
|-------|------|------|----------------|
| 1 | Selection Highlighting | Low | 4 |
| 2 | Auto-close Brackets | Low | 7 |
| 3 | Multi-cursor Paste | Low | 3 |
| 4 | Navigation History + F12 | Medium | 8 |
| 5 | Git Stage/Commit/Push | Medium | 7 |
| 6 | 3-Panel Merge Tool | High | 6 |
| 7 | Split Editor | High | 8 |

Tasks 1-3 are independent and can be implemented in parallel.
Task 6 depends on Task 5 (git infrastructure).
Task 7 is independent but should be last due to its scope.
