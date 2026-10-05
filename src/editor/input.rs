use super::auto_close;
use super::Editor;

impl Editor {
    /// Insert a typed char at every cursor. Consecutive calls form one typing burst
    /// sharing a single undo checkpoint; the burst ends as soon as anything else
    /// moves a cursor, changes the selection or edits the buffer.
    pub fn insert_char(&mut self, ch: char, auto_close_enabled: bool) {
        self.insert_char_with(ch, auto_close_enabled, true);
    }

    /// Insert a pasted char: like `insert_char` without auto-close or the
    /// closing-bracket dedent, since pasted text carries its own indentation.
    pub(super) fn insert_pasted_char(&mut self, ch: char) {
        self.insert_char_with(ch, false, false);
    }

    fn insert_char_with(&mut self, ch: char, auto_close_enabled: bool, smart_indent: bool) {
        if self.typing_burst != Some(self.typing_burst_state()) {
            self.buffer.checkpoint();
        }
        self.insert_char_inner(ch, auto_close_enabled, smart_indent);
        self.typing_burst = Some(self.typing_burst_state());
    }

    fn insert_char_inner(&mut self, ch: char, auto_close_enabled: bool, smart_indent: bool) {
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

        // Dedent: a closing bracket typed on a whitespace-only line drops one indent
        // level (single cursor only — other cursors' columns would need shifting).
        if smart_indent
            && matches!(ch, '}' | ']' | ')')
            && !self.cursor.has_selection()
            && self.extra_cursors.is_empty()
        {
            self.dedent_before_cursor();
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
            if let Some(((sr, sc), (er, ec))) = self.cursor.selection_range() {
                // Adjust extra cursors BEFORE deleting, since deletion shifts positions.
                if sr == er {
                    let deleted = ec - sc;
                    for ec_cursor in &mut self.extra_cursors {
                        if ec_cursor.row == sr && ec_cursor.col > sc {
                            ec_cursor.col = ec_cursor.col.saturating_sub(deleted);
                            ec_cursor.desired_col = ec_cursor.col;
                        }
                        if let Some((ar, ac)) = ec_cursor.sel_anchor {
                            if ar == sr && ac > sc {
                                ec_cursor.sel_anchor = Some((ar, ac.saturating_sub(deleted)));
                            }
                        }
                    }
                }
            }
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

    /// When only whitespace precedes the primary cursor, remove one indent level
    /// from it: a trailing tab, or the spaces back to the previous indent stop.
    fn dedent_before_cursor(&mut self) {
        let (row, col) = self.cursor.position();
        let chars: Vec<char> = self.buffer.line(row).chars().collect();
        let col = col.min(chars.len());
        let before = &chars[..col];
        if before.is_empty() || !before.iter().all(|c| *c == ' ' || *c == '\t') {
            return;
        }
        let remove = if before.last() == Some(&'\t') {
            1
        } else {
            let spaces = before.iter().rev().take_while(|c| **c == ' ').count();
            let size = self.detected_indent_size.max(1);
            (spaces - 1) % size + 1
        };
        let start = self.buffer.char_index(row, col - remove);
        self.buffer.delete_range(start, start + remove);
        self.cursor.set_position(row, col - remove);
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

    pub fn delete_char_after(&mut self) {
        self.buffer.checkpoint();
        if self.cursor.has_selection() {
            self.delete_selection();
            return;
        }
        let (row, col) = self.cursor.position();
        let line_len = self.buffer.line_len(row);

        if col < line_len {
            self.buffer.delete_char(row, col);

            // Adjust extra cursors on the same row that are after the deleted column.
            for ec in &mut self.extra_cursors {
                if ec.row == row && ec.col > col {
                    ec.col -= 1;
                    ec.desired_col = ec.col;
                }
                if let Some((ar, ac)) = ec.sel_anchor {
                    if ar == row && ac > col {
                        ec.sel_anchor = Some((ar, ac - 1));
                    }
                }
            }
        } else if row + 1 < self.buffer.num_lines() {
            // At end of line: join with next line
            self.buffer.join_lines(row + 1);
        }

        // Process each extra cursor.
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
                let (er, ec_col) = self.extra_cursors[i].position();
                let el = self.buffer.line_len(er);
                if ec_col < el {
                    self.buffer.delete_char(er, ec_col);
                    for j in (i + 1)..n {
                        if self.extra_cursors[j].row == er && self.extra_cursors[j].col > ec_col {
                            self.extra_cursors[j].col -= 1;
                            self.extra_cursors[j].desired_col = self.extra_cursors[j].col;
                        }
                        if let Some((ar, ac)) = self.extra_cursors[j].sel_anchor {
                            if ar == er && ac > ec_col {
                                self.extra_cursors[j].sel_anchor = Some((ar, ac - 1));
                            }
                        }
                    }
                } else if er + 1 < self.buffer.num_lines() {
                    self.buffer.join_lines(er + 1);
                }
            }
        }

        self.is_modified = true;
        self.content_version = self.content_version.wrapping_add(1);
        self.cursor_blink_epoch = std::time::Instant::now();
    }

    /// Enter: split the line at every cursor and auto-indent the new line (see
    /// `newline_text`).
    pub fn insert_newline(&mut self) {
        self.insert_newline_impl(true);
    }

    /// Split the line at every cursor without any auto-indent — used by paste,
    /// whose text already carries its own indentation.
    pub(super) fn insert_newline_plain(&mut self) {
        self.insert_newline_impl(false);
    }

    /// One indent level in the file's detected style.
    pub(super) fn indent_unit(&self) -> String {
        if self.detected_indent_spaces {
            " ".repeat(self.detected_indent_size.max(1))
        } else {
            "\t".to_string()
        }
    }

    /// Text to insert for a newline at (row, col), and the caret offset inside it
    /// as (lines down, column). With `auto_indent` the new line copies the leading
    /// whitespace before the cursor, gains one level after an opening bracket, and
    /// a cursor between a bracket pair (`{|}`) pushes the closer onto its own line.
    fn newline_text(&self, row: usize, col: usize, auto_indent: bool) -> (String, (usize, usize)) {
        if !auto_indent {
            return ("\n".to_string(), (1, 0));
        }
        let chars: Vec<char> = self.buffer.line(row).chars().collect();
        let col = col.min(chars.len());
        let indent: String = chars[..col]
            .iter()
            .take_while(|c| **c == ' ' || **c == '\t')
            .collect();
        let opener = chars[..col]
            .iter()
            .rev()
            .find(|c| !c.is_whitespace())
            .copied()
            .filter(|c| matches!(c, '{' | '[' | '('));
        let Some(opener) = opener else {
            let n = indent.chars().count();
            return (format!("\n{indent}"), (1, n));
        };
        let inner = format!("{indent}{}", self.indent_unit());
        let caret = (1, inner.chars().count());
        let closer = auto_close::closing_pair(opener);
        let next = chars[col..].iter().find(|c| !c.is_whitespace()).copied();
        if next.is_some() && next == closer {
            (format!("\n{inner}\n{indent}"), caret)
        } else {
            (format!("\n{inner}"), caret)
        }
    }

    /// Shift every cursor (and selection anchor) except the skipped ones to account
    /// for `text` inserted at (row, col).
    fn shift_cursors_for_insert(
        &mut self,
        skip_primary: bool,
        skip_extra: Option<usize>,
        (row, col): (usize, usize),
        text: &str,
    ) {
        let added_lines = text.matches('\n').count();
        let last_len = text.rsplit('\n').next().unwrap_or("").chars().count();
        let shift = |(r, c): (usize, usize)| -> (usize, usize) {
            if r == row && c >= col {
                if added_lines == 0 {
                    (r, c + last_len)
                } else {
                    (r + added_lines, c - col + last_len)
                }
            } else if r > row {
                (r + added_lines, c)
            } else {
                (r, c)
            }
        };
        let apply = |cur: &mut super::cursor::Cursor| {
            let (r, c) = shift(cur.position());
            cur.row = r;
            cur.col = c;
            cur.desired_col = c;
            cur.sel_anchor = cur.sel_anchor.map(shift);
        };
        if !skip_primary {
            apply(&mut self.cursor);
        }
        for (i, ec) in self.extra_cursors.iter_mut().enumerate() {
            if skip_extra != Some(i) {
                apply(ec);
            }
        }
    }

    fn insert_newline_impl(&mut self, auto_indent: bool) {
        self.buffer.checkpoint();
        if self.cursor.has_selection() {
            self.delete_selection();
        }
        let (row, col) = self.cursor.position();
        let (text, (dr, dc)) = self.newline_text(row, col, auto_indent);
        self.buffer.insert_str(row, col, &text);
        self.shift_cursors_for_insert(true, None, (row, col), &text);
        self.cursor.set_position(row + dr, dc);

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
            let (text, (dr, dc)) = self.newline_text(er, ec, auto_indent);
            self.buffer.insert_str(er, ec, &text);
            self.shift_cursors_for_insert(false, Some(i), (er, ec), &text);
            self.extra_cursors[i].set_position(er + dr, dc);
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
}

#[cfg(test)]
mod tests {
    use super::super::buffer::Buffer;
    use super::super::cursor::Cursor;
    use super::*;

    fn editor_with(content: &str, row: usize, col: usize) -> Editor {
        let mut ed = Editor::new();
        ed.set_content(content.to_string(), None);
        ed.cursor.set_position(row, col);
        ed
    }

    fn cursor_at(row: usize, col: usize) -> Cursor {
        let mut c = Cursor::new();
        c.set_position(row, col);
        c
    }

    fn selecting(anchor: (usize, usize), row: usize, col: usize) -> Cursor {
        let mut c = cursor_at(row, col);
        c.sel_anchor = Some(anchor);
        c
    }

    fn text(ed: &Editor) -> String {
        ed.buffer.to_string()
    }

    // ── insert_char ──────────────────────────────────────────────────────────

    #[test]
    fn insert_char_inserts_at_cursor_and_advances() {
        let mut ed = editor_with("ac", 0, 1);
        let v = ed.content_version;
        ed.insert_char('b', true);
        assert_eq!(text(&ed), "abc");
        assert_eq!(ed.cursor.position(), (0, 2));
        assert!(ed.is_modified);
        assert_eq!(ed.content_version, v + 1);
    }

    #[test]
    fn insert_char_auto_closes_brackets_and_quotes() {
        let mut ed = editor_with("", 0, 0);
        ed.insert_char('(', true);
        assert_eq!(text(&ed), "()");
        assert_eq!(ed.cursor.position(), (0, 1));

        let mut ed = editor_with("x = ", 0, 4);
        ed.insert_char('"', true);
        assert_eq!(text(&ed), "x = \"\"");
        assert_eq!(ed.cursor.position(), (0, 5));
    }

    #[test]
    fn insert_char_without_auto_close_inserts_only_the_char() {
        let mut ed = editor_with("", 0, 0);
        ed.insert_char('[', false);
        assert_eq!(text(&ed), "[");
    }

    #[test]
    fn insert_char_does_not_auto_close_before_a_word_char() {
        let mut ed = editor_with("foo", 0, 0);
        ed.insert_char('(', true);
        assert_eq!(text(&ed), "(foo");
    }

    #[test]
    fn insert_char_does_not_auto_close_quote_after_alphanumeric() {
        // e.g. typing an apostrophe in "don't"
        let mut ed = editor_with("don", 0, 3);
        ed.insert_char('\'', true);
        assert_eq!(text(&ed), "don'");
        assert_eq!(ed.cursor.position(), (0, 4));
    }

    #[test]
    fn insert_char_skips_over_existing_closing_char() {
        let mut ed = editor_with("()", 0, 1);
        ed.insert_char(')', true);
        assert_eq!(text(&ed), "()");
        assert_eq!(ed.cursor.position(), (0, 2));
    }

    #[test]
    fn insert_char_closing_char_is_inserted_when_not_under_cursor() {
        let mut ed = editor_with("(a", 0, 2);
        ed.insert_char(')', true);
        assert_eq!(text(&ed), "(a)");
        assert_eq!(ed.cursor.position(), (0, 3));
    }

    #[test]
    fn insert_char_surrounds_single_line_selection() {
        let mut ed = editor_with("let foo = 1;", 0, 7);
        ed.cursor.sel_anchor = Some((0, 4));
        ed.insert_char('(', true);
        assert_eq!(text(&ed), "let (foo) = 1;");
        assert_eq!(ed.cursor.position(), (0, 9));
        assert!(!ed.cursor.has_selection());
        assert!(ed.is_modified);
    }

    #[test]
    fn insert_char_surrounds_multi_line_selection() {
        let mut ed = editor_with("ab\ncd", 1, 1);
        ed.cursor.sel_anchor = Some((0, 1));
        ed.insert_char('[', true);
        assert_eq!(text(&ed), "a[b\nc]d");
        assert_eq!(ed.cursor.position(), (1, 2));
    }

    #[test]
    fn insert_char_replaces_selection_when_not_an_opening_char() {
        let mut ed = editor_with("hello world", 0, 5);
        ed.cursor.sel_anchor = Some((0, 0));
        ed.insert_char('X', true);
        assert_eq!(text(&ed), "X world");
        assert_eq!(ed.cursor.position(), (0, 1));
    }

    #[test]
    fn insert_char_with_selection_shifts_extra_cursors_on_same_line() {
        // Primary selection "bc" (cols 1..3); extra cursor selecting "e" (cols 4..5).
        let mut ed = editor_with("abcdefg", 0, 3);
        ed.cursor.sel_anchor = Some((0, 1));
        ed.extra_cursors.push(selecting((0, 4), 0, 5));
        ed.insert_char('X', false);
        assert_eq!(text(&ed), "aXdXfg");
        assert_eq!(ed.cursor.position(), (0, 2));
        assert_eq!(ed.extra_cursors[0].position(), (0, 4));
        assert!(!ed.extra_cursors[0].has_selection());
    }

    #[test]
    fn insert_char_types_at_every_extra_cursor() {
        let mut ed = editor_with("ab\ncd\nef", 0, 1);
        ed.extra_cursors.push(cursor_at(1, 1));
        ed.extra_cursors.push(cursor_at(2, 1));
        ed.insert_char('-', false);
        assert_eq!(text(&ed), "a-b\nc-d\ne-f");
        assert_eq!(ed.cursor.position(), (0, 2));
        assert_eq!(ed.extra_cursors[0].position(), (1, 2));
        assert_eq!(ed.extra_cursors[1].position(), (2, 2));
    }

    #[test]
    fn insert_char_with_multiple_cursors_on_same_line_keeps_offsets() {
        let mut ed = editor_with("a b c", 0, 1);
        ed.extra_cursors.push(cursor_at(0, 3));
        ed.extra_cursors.push(cursor_at(0, 5));
        ed.insert_char('!', false);
        assert_eq!(text(&ed), "a! b! c!");
        assert_eq!(ed.cursor.position(), (0, 2));
        assert_eq!(ed.extra_cursors[0].position(), (0, 5));
        assert_eq!(ed.extra_cursors[1].position(), (0, 8));
    }

    #[test]
    fn insert_char_shifts_extra_cursor_anchors_after_primary_insert() {
        let mut ed = editor_with("ab cd", 0, 0);
        // Extra cursor selecting "cd" (anchor 3, head 5).
        ed.extra_cursors.push(selecting((0, 3), 0, 5));
        ed.insert_char('Z', false);
        // Primary inserts Z at 0 -> extra selection shifts to 4..6 and is replaced.
        assert_eq!(text(&ed), "Zab Z");
        assert_eq!(ed.extra_cursors[0].position(), (0, 5));
    }

    #[test]
    fn insert_char_extra_cursor_same_line_selections_adjust_later_cursors() {
        // Two extra cursors, each selecting two chars on the same line.
        let mut ed = editor_with("0123456789\n", 1, 0);
        ed.extra_cursors.push(selecting((0, 1), 0, 3)); // "12"
        ed.extra_cursors.push(selecting((0, 6), 0, 8)); // "67"
        ed.insert_char('_', false);
        assert_eq!(ed.buffer.line(0), "0_345_89");
        assert_eq!(ed.extra_cursors[0].position(), (0, 2));
        assert_eq!(ed.extra_cursors[1].position(), (0, 6));
        assert_eq!(ed.buffer.line(1), "_");
    }

    #[test]
    fn insert_char_extra_cursor_multi_line_selection_collapses_to_start() {
        let mut ed = editor_with("aa\nbb\ncc", 2, 2);
        ed.extra_cursors.push(selecting((0, 1), 1, 1));
        ed.insert_char('X', false);
        // Multi-line extra selections are collapsed (not deleted), then typed into.
        assert_eq!(text(&ed), "aXa\nbb\nccX");
        assert_eq!(ed.extra_cursors[0].position(), (0, 2));
    }

    #[test]
    fn insert_char_pushes_an_undo_checkpoint() {
        let mut ed = editor_with("", 0, 0);
        ed.insert_char('a', false);
        assert!(ed.buffer.undo());
        assert_eq!(text(&ed), "");
    }

    // ── delete_char_before ──────────────────────────────────────────────────

    #[test]
    fn backspace_deletes_previous_char() {
        let mut ed = editor_with("abc", 0, 2);
        ed.delete_char_before();
        assert_eq!(text(&ed), "ac");
        assert_eq!(ed.cursor.position(), (0, 1));
        assert!(ed.is_modified);
    }

    #[test]
    fn backspace_deletes_matching_pair() {
        let mut ed = editor_with("f()", 0, 2);
        ed.delete_char_before();
        assert_eq!(text(&ed), "f");
        assert_eq!(ed.cursor.position(), (0, 1));
    }

    #[test]
    fn backspace_does_not_pair_delete_mismatched_chars() {
        let mut ed = editor_with("(]", 0, 1);
        ed.delete_char_before();
        assert_eq!(text(&ed), "]");
    }

    #[test]
    fn backspace_at_line_start_joins_with_previous_line() {
        let mut ed = editor_with("ab\ncd", 1, 0);
        ed.delete_char_before();
        assert_eq!(text(&ed), "abcd");
        assert_eq!(ed.cursor.position(), (0, 2));
    }

    #[test]
    fn backspace_at_document_start_keeps_text() {
        let mut ed = editor_with("ab", 0, 0);
        ed.delete_char_before();
        assert_eq!(text(&ed), "ab");
        assert_eq!(ed.cursor.position(), (0, 0));
    }

    #[test]
    fn backspace_with_selection_deletes_only_the_selection() {
        let mut ed = editor_with("hello\nworld", 1, 2);
        ed.cursor.sel_anchor = Some((0, 3));
        ed.delete_char_before();
        assert_eq!(text(&ed), "helrld");
        assert_eq!(ed.cursor.position(), (0, 3));
        assert!(!ed.cursor.has_selection());
    }

    #[test]
    fn backspace_with_extra_cursors_on_same_line() {
        let mut ed = editor_with("ab cd ef", 0, 2);
        ed.extra_cursors.push(cursor_at(0, 5));
        ed.extra_cursors.push(cursor_at(0, 8));
        ed.delete_char_before();
        assert_eq!(text(&ed), "a c e");
        assert_eq!(ed.cursor.position(), (0, 1));
        assert_eq!(ed.extra_cursors[0].position(), (0, 3));
        assert_eq!(ed.extra_cursors[1].position(), (0, 5));
    }

    #[test]
    fn backspace_extra_cursor_anchor_is_shifted() {
        let mut ed = editor_with("abcdef\nx", 0, 1);
        // Extra cursor selecting "de" (anchor after the deletion point).
        ed.extra_cursors.push(selecting((0, 3), 0, 5));
        ed.delete_char_before();
        // Primary deletes 'a' -> "bcdef"; extra selection shifts to 2..4 ("de") and is deleted.
        assert_eq!(ed.buffer.line(0), "bcf");
        assert_eq!(ed.extra_cursors[0].position(), (0, 2));
    }

    #[test]
    fn backspace_extra_cursor_same_line_selections_adjust_later_cursors() {
        let mut ed = editor_with("0123456789\n", 1, 0);
        ed.extra_cursors.push(selecting((0, 1), 0, 3));
        ed.extra_cursors.push(selecting((0, 6), 0, 8));
        ed.delete_char_before();
        assert_eq!(ed.buffer.line(0), "034589");
        assert_eq!(ed.extra_cursors[0].position(), (0, 1));
        assert_eq!(ed.extra_cursors[1].position(), (0, 4));
    }

    #[test]
    fn backspace_extra_cursor_multi_line_selection_collapses() {
        let mut ed = editor_with("ab\ncd\nef", 2, 0);
        ed.extra_cursors.push(selecting((0, 1), 1, 1));
        ed.delete_char_before();
        // Primary at (2,0) joins rows 1 and 2; extra selection collapses to its start.
        assert_eq!(text(&ed), "ab\ncdef");
        assert_eq!(ed.extra_cursors[0].position(), (0, 1));
    }

    #[test]
    fn backspace_extra_cursor_at_line_start_joins_lines() {
        let mut ed = editor_with("ab\ncd\nef", 0, 2);
        ed.extra_cursors.push(cursor_at(2, 0));
        ed.delete_char_before();
        assert_eq!(text(&ed), "a\ncdef");
        assert_eq!(ed.extra_cursors[0].position(), (1, 2));
    }

    #[test]
    fn backspace_extra_cursors_on_same_line_shift_each_other() {
        let mut ed = editor_with("abcdef\n", 1, 0);
        ed.extra_cursors.push(cursor_at(0, 2));
        ed.extra_cursors.push(selecting((0, 3), 0, 5));
        ed.delete_char_before();
        // Primary joins row 1 into row 0 (no-op on text as row 1 is empty); first extra
        // deletes 'b'; second extra's selection shifts to 2..4 ("de") and is deleted.
        assert_eq!(ed.buffer.line(0), "acf");
        assert_eq!(ed.extra_cursors[0].position(), (0, 1));
        assert_eq!(ed.extra_cursors[1].position(), (0, 2));
    }

    // ── delete_char_after ───────────────────────────────────────────────────

    #[test]
    fn delete_removes_char_under_cursor() {
        let mut ed = editor_with("abc", 0, 1);
        ed.delete_char_after();
        assert_eq!(text(&ed), "ac");
        assert_eq!(ed.cursor.position(), (0, 1));
        assert!(ed.is_modified);
    }

    #[test]
    fn delete_at_line_end_joins_next_line() {
        let mut ed = editor_with("ab\ncd", 0, 2);
        ed.delete_char_after();
        assert_eq!(text(&ed), "abcd");
    }

    #[test]
    fn delete_at_document_end_keeps_text() {
        let mut ed = editor_with("ab", 0, 2);
        ed.delete_char_after();
        assert_eq!(text(&ed), "ab");
    }

    #[test]
    fn delete_with_selection_deletes_selection() {
        let mut ed = editor_with("abcdef", 0, 1);
        ed.cursor.sel_anchor = Some((0, 4));
        ed.delete_char_after();
        assert_eq!(text(&ed), "aef");
        assert_eq!(ed.cursor.position(), (0, 1));
    }

    #[test]
    fn delete_with_extra_cursors() {
        let mut ed = editor_with("ab cd ef", 0, 0);
        ed.extra_cursors.push(cursor_at(0, 3));
        ed.extra_cursors.push(cursor_at(0, 6));
        ed.delete_char_after();
        assert_eq!(text(&ed), "b d f");
        assert_eq!(ed.extra_cursors[0].position(), (0, 2));
        assert_eq!(ed.extra_cursors[1].position(), (0, 4));
    }

    #[test]
    fn delete_extra_cursor_anchor_shift_and_selection_delete() {
        let mut ed = editor_with("abcdef\n", 0, 0);
        ed.extra_cursors.push(selecting((0, 2), 0, 4)); // "cd"
        ed.extra_cursors.push(selecting((0, 5), 0, 6)); // "f"
        ed.delete_char_after();
        // Primary deletes 'a' -> "bcdef"; extras shift to 1..3 and 4..5 and are deleted.
        assert_eq!(ed.buffer.line(0), "be");
        assert_eq!(ed.extra_cursors[0].position(), (0, 1));
    }

    #[test]
    fn delete_extra_cursor_char_delete_shifts_later_anchor() {
        let mut ed = editor_with("abcdef\nx", 1, 1);
        ed.extra_cursors.push(cursor_at(0, 0)); // deletes 'a'
        ed.extra_cursors.push(selecting((0, 3), 0, 5)); // "de" -> shifts to 2..4
        ed.delete_char_after();
        assert_eq!(ed.buffer.line(0), "bcf");
        assert_eq!(ed.extra_cursors[1].position(), (0, 2));
    }

    #[test]
    fn delete_extra_cursor_multi_line_selection_collapses() {
        let mut ed = editor_with("ab\ncd\nef", 2, 2);
        ed.extra_cursors.push(selecting((0, 1), 1, 1));
        ed.delete_char_after();
        assert_eq!(text(&ed), "ab\ncd\nef");
        assert_eq!(ed.extra_cursors[0].position(), (0, 1));
        assert!(!ed.extra_cursors[0].has_selection());
    }

    #[test]
    fn delete_extra_cursor_at_line_end_joins_lines() {
        let mut ed = editor_with("ab\ncd\nef", 2, 0);
        ed.extra_cursors.push(cursor_at(0, 2));
        ed.delete_char_after();
        assert_eq!(text(&ed), "abcd\nf");
    }

    // ── insert_newline ──────────────────────────────────────────────────────

    #[test]
    fn newline_splits_line_at_cursor() {
        let mut ed = editor_with("abcd", 0, 2);
        ed.insert_newline();
        assert_eq!(text(&ed), "ab\ncd");
        assert_eq!(ed.cursor.position(), (1, 0));
        assert!(ed.is_modified);
    }

    #[test]
    fn newline_replaces_selection() {
        let mut ed = editor_with("abcd", 0, 3);
        ed.cursor.sel_anchor = Some((0, 1));
        ed.insert_newline();
        assert_eq!(text(&ed), "a\nd");
        assert_eq!(ed.cursor.position(), (1, 0));
    }

    #[test]
    fn newline_at_extra_cursors() {
        let mut ed = editor_with("ab\ncd", 1, 1);
        ed.extra_cursors.push(cursor_at(0, 1));
        ed.insert_newline();
        // Primary splits row 1 first, then the extra splits row 0.
        assert_eq!(text(&ed), "a\nb\nc\nd");
        assert_eq!(ed.extra_cursors[0].position(), (1, 0));
    }

    #[test]
    fn newline_extra_cursor_selections() {
        let mut ed = editor_with("0123456789\nzz\nyy", 2, 0);
        ed.extra_cursors.push(selecting((0, 1), 0, 3));
        ed.extra_cursors.push(selecting((0, 6), 0, 8));
        ed.insert_newline();
        assert_eq!(ed.buffer.line(0), "0");
        assert_eq!(ed.extra_cursors[0].position(), (1, 0));

        let mut ed = editor_with("ab\ncd\nef", 2, 2);
        ed.extra_cursors.push(selecting((0, 1), 1, 1));
        ed.insert_newline();
        assert_eq!(text(&ed), "a\nb\ncd\nef\n");
        assert_eq!(ed.extra_cursors[0].position(), (1, 0));
    }

    // ── auto-indent ─────────────────────────────────────────────────────────

    #[test]
    fn newline_copies_leading_whitespace() {
        let mut ed = editor_with("fn a() {\n    let x = 1;\n}", 1, 14);
        ed.insert_newline();
        assert_eq!(ed.buffer.line(2), "    ");
        assert_eq!(ed.cursor.position(), (2, 4));

        // Tabs are copied verbatim.
        let mut ed = editor_with("\t\tfoo", 0, 5);
        ed.insert_newline();
        assert_eq!(text(&ed), "\t\tfoo\n\t\t");
        assert_eq!(ed.cursor.position(), (1, 2));
    }

    #[test]
    fn newline_inside_leading_whitespace_keeps_only_whitespace_before_cursor() {
        let mut ed = editor_with("    foo", 0, 2);
        ed.insert_newline();
        assert_eq!(text(&ed), "  \n    foo");
        assert_eq!(ed.cursor.position(), (1, 2));
    }

    #[test]
    fn newline_after_opener_adds_one_detected_indent_level() {
        // Two-space file: the new level is two spaces.
        let mut ed = editor_with("a = [\n  1,\n]\nb = (", 3, 5);
        ed.insert_newline();
        assert_eq!(ed.buffer.line(4), "  ");
        assert_eq!(ed.cursor.position(), (4, 2));

        // Trailing whitespace after the opener still counts.
        let mut ed = editor_with("if x {  ", 0, 8);
        ed.insert_newline();
        assert_eq!(ed.buffer.line(1), "    ");

        // Tab-indented file gets a tab.
        let mut ed = editor_with("\tfoo(\n\t\tbar\n\t\tbaz", 0, 5);
        ed.insert_newline();
        assert_eq!(ed.buffer.line(1), "\t\t");
        assert_eq!(ed.cursor.position(), (1, 2));
    }

    #[test]
    fn newline_between_bracket_pair_puts_closer_on_its_own_line() {
        let mut ed = editor_with("    fn a() {}", 0, 12);
        ed.insert_newline();
        assert_eq!(text(&ed), "    fn a() {\n        \n    }");
        assert_eq!(ed.cursor.position(), (1, 8));

        // Mismatched closer: no extra line.
        let mut ed = editor_with("(]", 0, 1);
        ed.insert_newline();
        assert_eq!(text(&ed), "(\n    ]");
    }

    #[test]
    fn newline_auto_indents_at_every_cursor_and_shifts_others() {
        let mut ed = editor_with("  a{}\n  b", 0, 4);
        ed.extra_cursors.push(cursor_at(1, 3));
        ed.insert_newline();
        // Two-space file, so one level is two spaces.
        assert_eq!(text(&ed), "  a{\n    \n  }\n  b\n  ");
        assert_eq!(ed.cursor.position(), (1, 4));
        // The primary's two new lines pushed the extra cursor from row 1 to row 3
        // before it split that line itself.
        assert_eq!(ed.extra_cursors[0].position(), (4, 2));

        // An extra cursor above the primary shifts the primary down.
        let mut ed = editor_with("x\ny", 1, 1);
        ed.extra_cursors.push(cursor_at(0, 1));
        ed.insert_newline();
        assert_eq!(text(&ed), "x\n\ny\n");
        assert_eq!(ed.cursor.position(), (3, 0));
        assert_eq!(ed.extra_cursors[0].position(), (1, 0));
    }

    #[test]
    fn plain_newline_does_not_indent() {
        let mut ed = editor_with("    {", 0, 5);
        ed.insert_newline_plain();
        assert_eq!(text(&ed), "    {\n");
        assert_eq!(ed.cursor.position(), (1, 0));
    }

    #[test]
    fn closing_bracket_on_whitespace_line_dedents_one_level() {
        let mut ed = editor_with("{\n    a\n        ", 2, 8);
        ed.insert_char('}', true);
        assert_eq!(text(&ed), "{\n    a\n    }");
        assert_eq!(ed.cursor.position(), (2, 5));

        // Off-grid spaces snap back to the previous indent stop.
        let mut ed = editor_with("      ", 0, 6);
        ed.insert_char(']', false);
        assert_eq!(text(&ed), "    ]");

        let mut ed = editor_with("\t\t", 0, 2);
        ed.insert_char(')', false);
        assert_eq!(text(&ed), "\t)");
    }

    #[test]
    fn closing_bracket_after_code_or_at_col_zero_does_not_dedent() {
        let mut ed = editor_with("    x", 0, 5);
        ed.insert_char('}', false);
        assert_eq!(text(&ed), "    x}");

        let mut ed = editor_with("", 0, 0);
        ed.insert_char('}', false);
        assert_eq!(text(&ed), "}");

        // Pasted brackets keep the pasted indentation.
        let mut ed = editor_with("    ", 0, 4);
        ed.insert_pasted_char('}');
        assert_eq!(text(&ed), "    }");
    }

    #[test]
    fn enter_then_close_round_trips_a_block() {
        let mut ed = editor_with("fn a() {", 0, 8);
        ed.insert_newline();
        for ch in "x;".chars() {
            ed.insert_char(ch, true);
        }
        ed.insert_newline();
        ed.insert_char('}', true);
        assert_eq!(text(&ed), "fn a() {\n    x;\n}");
    }

    // ── selection helpers ───────────────────────────────────────────────────

    #[test]
    fn selected_text_returns_none_without_selection() {
        let ed = editor_with("abc", 0, 1);
        assert_eq!(ed.selected_text(), None);
    }

    #[test]
    fn selected_text_spans_lines_regardless_of_direction() {
        let mut ed = editor_with("hello\nworld", 0, 3);
        ed.cursor.sel_anchor = Some((1, 2));
        assert_eq!(ed.selected_text().as_deref(), Some("lo\nwo"));
        ed.cursor.set_position(1, 2);
        ed.cursor.sel_anchor = Some((0, 3));
        assert_eq!(ed.selected_text().as_deref(), Some("lo\nwo"));
    }

    #[test]
    fn delete_selection_without_selection_does_nothing() {
        let mut ed = editor_with("abc", 0, 1);
        ed.delete_selection();
        assert_eq!(text(&ed), "abc");
        assert!(!ed.is_modified);
    }

    #[test]
    fn buffer_helper_is_available() {
        // Sanity: a raw Buffer can be swapped in and edited through the editor.
        let mut ed = editor_with("", 0, 0);
        ed.buffer = Buffer::from_str("xy");
        ed.cursor.set_position(0, 2);
        ed.delete_char_before();
        assert_eq!(text(&ed), "x");
    }
}
