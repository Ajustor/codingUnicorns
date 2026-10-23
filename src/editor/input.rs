use super::auto_close;
use super::cursor::Cursor;
use super::Editor;

/// One cursor's edit, in char indices of the text before the batch: replace
/// `start..end` with `text` and put the caret `caret` chars into it.
pub(super) struct CursorEdit {
    start: usize,
    end: usize,
    text: String,
    caret: usize,
}

impl CursorEdit {
    fn new(start: usize, end: usize, text: String, caret: usize) -> Self {
        Self {
            start,
            end,
            text,
            caret,
        }
    }
}

impl Editor {
    /// Insert a typed char at every cursor. Consecutive calls form one typing burst
    /// sharing a single undo checkpoint; the burst ends as soon as anything else
    /// moves a cursor, changes the selection or edits the buffer.
    pub fn insert_char(&mut self, ch: char, auto_close_enabled: bool) {
        self.insert_char_with(ch, auto_close_enabled, true);
    }

    fn insert_char_with(&mut self, ch: char, auto_close_enabled: bool, smart_indent: bool) {
        if self.typing_burst != Some(self.typing_burst_state()) {
            self.buffer.checkpoint();
        }
        self.insert_char_inner(ch, auto_close_enabled, smart_indent);
        self.typing_burst = Some(self.typing_burst_state());
    }

    fn insert_char_inner(&mut self, ch: char, auto_close_enabled: bool, smart_indent: bool) {
        // Dedent: a closing bracket typed on a whitespace-only line drops one indent
        // level (single cursor only — other cursors' columns would need shifting).
        if smart_indent
            && matches!(ch, '}' | ']' | ')')
            && !self.cursor.has_selection()
            && self.extra_cursors.is_empty()
        {
            self.dedent_before_cursor();
        }
        let edits = self
            .all_cursors()
            .map(|c| self.char_edit(c, ch, auto_close_enabled))
            .collect();
        self.apply_cursor_edits(edits);
    }

    /// The edit typing `ch` makes at one cursor: wrap its selection in a pair,
    /// replace it, step over the closing char already there, or insert (with
    /// its auto-closed pair).
    fn char_edit(&self, c: &Cursor, ch: char, auto_close: bool) -> CursorEdit {
        let (start, end) = self.span_of(c);
        if start != end {
            if let Some(close) = auto_close.then(|| auto_close::closing_pair(ch)).flatten() {
                let text = format!("{ch}{}{close}", self.buffer.rope_slice(start, end));
                let caret = text.chars().count();
                return CursorEdit::new(start, end, text, caret);
            }
        } else if auto_close && auto_close::is_closing(ch) && self.char_at(start) == Some(ch) {
            // Skip-close: type over the closer instead of doubling it.
            return CursorEdit::new(start, start + 1, ch.to_string(), 1);
        }
        let mut text = ch.to_string();
        if let Some(close) = auto_close.then(|| auto_close::closing_pair(ch)).flatten() {
            let on_line = |c: Option<char>| c.filter(|c| *c != '\n');
            let next = on_line(self.char_at(end));
            let prev = on_line(start.checked_sub(1).and_then(|i| self.char_at(i)));
            if !auto_close::should_skip_quote_auto_close(ch, prev)
                && auto_close::should_auto_close(ch, next)
            {
                text.push(close);
            }
        }
        CursorEdit::new(start, end, text, 1)
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
        let edits = self
            .all_cursors()
            .map(|c| {
                let (start, end) = self.span_of(c);
                if start != end || start == 0 {
                    return CursorEdit::new(start, end, String::new(), 0);
                }
                // Pair-delete: `(|)` loses both chars.
                let prev = self.char_at(start - 1);
                let pair = prev
                    .and_then(auto_close::closing_pair)
                    .is_some_and(|close| self.char_at(start) == Some(close));
                let end = if pair { start + 1 } else { start };
                // At a line start this removes the line break: the lines join.
                CursorEdit::new(start - 1, end, String::new(), 0)
            })
            .collect();
        self.apply_cursor_edits(edits);
    }

    pub fn delete_char_after(&mut self) {
        self.buffer.checkpoint();
        let len = self.buffer.rope_len();
        let edits = self
            .all_cursors()
            .map(|c| {
                let (start, end) = self.span_of(c);
                let end = if start == end {
                    (end + 1).min(len)
                } else {
                    end
                };
                CursorEdit::new(start, end, String::new(), 0)
            })
            .collect();
        self.apply_cursor_edits(edits);
    }

    /// Ctrl+Backspace / Ctrl+Delete: delete the word before / after every cursor
    /// (or its selection, when it has one), as one undo step.
    pub fn delete_word(&mut self, forward: bool) {
        let ranges: Vec<(usize, usize)> = std::iter::once(&self.cursor)
            .chain(self.extra_cursors.iter())
            .map(|c| {
                let (a, b) = c.selection_range().unwrap_or_else(|| {
                    let here = c.position();
                    let to = if forward {
                        super::cursor::word_right(&self.buffer, here.0, here.1)
                    } else {
                        super::cursor::word_left(&self.buffer, here.0, here.1)
                    };
                    (here.min(to), here.max(to))
                });
                (
                    self.buffer.char_index(a.0, a.1),
                    self.buffer.char_index(b.0, b.1),
                )
            })
            .collect();
        if ranges.iter().all(|(s, e)| s == e) {
            return;
        }
        self.buffer.checkpoint();

        // Delete back to front so earlier indices stay valid; overlapping ranges
        // (two cursors in one word) are clamped to what's still there.
        let mut order: Vec<usize> = (0..ranges.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(ranges[i].0));
        let mut floor = usize::MAX;
        for &i in &order {
            let (s, e) = ranges[i];
            self.buffer.delete_range(s, e.min(floor));
            floor = floor.min(s);
        }

        // Each cursor lands at its range start, minus everything deleted before it.
        let removed_before = |idx: usize| -> usize {
            let mut spans: Vec<(usize, usize)> =
                ranges.iter().copied().filter(|(s, _)| *s < idx).collect();
            spans.sort();
            let (mut total, mut reach) = (0, 0);
            for (s, e) in spans {
                let (s, e) = (s.max(reach), e.min(idx));
                if e > s {
                    total += e - s;
                }
                reach = reach.max(e);
            }
            total
        };
        let new_pos: Vec<(usize, usize)> = ranges
            .iter()
            .map(|(s, _)| self.buffer.position_of(s - removed_before(*s)))
            .collect();
        for (i, (row, col)) in new_pos.into_iter().enumerate() {
            let c = if i == 0 {
                &mut self.cursor
            } else {
                &mut self.extra_cursors[i - 1]
            };
            c.clear_selection();
            c.set_position(row, col);
        }
        self.dedup_cursors();
        self.is_modified = true;
        self.content_version = self.content_version.wrapping_add(1);
        self.cursor_blink_epoch = std::time::Instant::now();
    }

    /// Enter: split the line at every cursor and auto-indent the new line (see
    /// `newline_text`).
    pub fn insert_newline(&mut self) {
        self.insert_newline_impl(true);
    }

    /// One indent level in the file's detected style.
    pub(super) fn indent_unit(&self) -> String {
        if self.detected_indent_spaces {
            " ".repeat(self.detected_indent_size.max(1))
        } else {
            "\t".to_string()
        }
    }

    /// True when some cursor's selection spans more than one line: Tab then
    /// indents those lines instead of replacing the selection.
    pub(super) fn has_multiline_selection(&self) -> bool {
        self.all_cursors()
            .any(|c| c.selection_range().is_some_and(|(s, e)| s.0 != e.0))
    }

    /// Tab / Shift+Tab on whole lines: add one indent level to (or remove one
    /// from) every line a cursor touches, as one undo step. A selection ending
    /// at column 0 leaves that last line alone; blank lines are not indented.
    /// Selections and carets keep their place in the text.
    pub(super) fn indent_lines(&mut self, outdent: bool) {
        let mut rows: Vec<usize> = self
            .all_cursors()
            .flat_map(|c| {
                let (s, e) = c.selection_range().unwrap_or((c.position(), c.position()));
                let last = if e.0 > s.0 && e.1 == 0 { e.0 - 1 } else { e.0 };
                s.0..=last
            })
            .collect();
        rows.sort_unstable();
        rows.dedup();

        let unit = self.indent_unit();
        let size = self.detected_indent_size.max(1);
        // Chars added (positive) or removed (negative) at the start of each row.
        let mut delta: std::collections::HashMap<usize, isize> = Default::default();
        for &row in &rows {
            let line = self.buffer.line(row);
            let d = if outdent {
                if line.starts_with('\t') {
                    -1
                } else {
                    -(line.chars().take(size).take_while(|c| *c == ' ').count() as isize)
                }
            } else if line.trim().is_empty() {
                0
            } else {
                unit.chars().count() as isize
            };
            if d != 0 {
                delta.insert(row, d);
            }
        }
        if delta.is_empty() {
            return;
        }

        self.buffer.checkpoint();
        for &row in rows.iter().rev() {
            match delta.get(&row) {
                Some(&d) if d > 0 => self.buffer.insert_str(row, 0, &unit),
                Some(&d) => {
                    let start = self.buffer.char_index(row, 0);
                    self.buffer.delete_range(start, start + d.unsigned_abs());
                }
                None => {}
            }
        }

        // An indented line shifts every column but 0 (a selection starting at
        // the line start keeps covering the whole line); an outdented one pulls
        // columns back, never past the line start.
        let shift = |(row, col): (usize, usize)| -> (usize, usize) {
            match delta.get(&row) {
                Some(&d) if d > 0 && col > 0 => (row, col + d as usize),
                Some(&d) if d < 0 => (row, col.saturating_sub(d.unsigned_abs())),
                _ => (row, col),
            }
        };
        for c in std::iter::once(&mut self.cursor).chain(self.extra_cursors.iter_mut()) {
            let (row, col) = shift(c.position());
            c.set_position(row, col);
            c.sel_anchor = c.sel_anchor.map(shift);
        }
        self.dedup_cursors();
        self.is_modified = true;
        self.content_version = self.content_version.wrapping_add(1);
        self.cursor_blink_epoch = std::time::Instant::now();
    }

    /// Text to insert for a newline at (row, col), and the caret offset inside it
    /// in chars. `rest` is what follows the inserted newline on that line (after
    /// a replaced selection). With `auto_indent` the new line copies the leading
    /// whitespace before the cursor, gains one level after an opening bracket,
    /// and a cursor between a bracket pair (`{|}`) pushes the closer onto its
    /// own line.
    fn newline_text(
        &self,
        row: usize,
        col: usize,
        rest: &str,
        auto_indent: bool,
    ) -> (String, usize) {
        if !auto_indent {
            return ("\n".to_string(), 1);
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
            let text = format!("\n{indent}");
            let caret = text.chars().count();
            return (text, caret);
        };
        let inner = format!("\n{indent}{}", self.indent_unit());
        let caret = inner.chars().count();
        let closer = auto_close::closing_pair(opener);
        let next = rest.chars().find(|c| !c.is_whitespace());
        if next.is_some() && next == closer {
            (format!("{inner}\n{indent}"), caret)
        } else {
            (inner, caret)
        }
    }

    fn insert_newline_impl(&mut self, auto_indent: bool) {
        self.buffer.checkpoint();
        let edits = self
            .all_cursors()
            .map(|c| {
                let (start, end) = self.span_of(c);
                let (row, col) = self.buffer.position_of(start);
                let (end_row, end_col) = self.buffer.position_of(end);
                let rest: String = self.buffer.line(end_row).chars().skip(end_col).collect();
                let (text, caret) = self.newline_text(row, col, &rest, auto_indent);
                CursorEdit::new(start, end, text, caret)
            })
            .collect();
        self.apply_cursor_edits(edits);
    }

    /// PageUp / PageDown: move every cursor one page (the rows that fit in
    /// `view_height`, minus one kept for context), extending selections with
    /// `select`, and scroll the view by the same amount so the caret keeps its
    /// place on screen.
    pub(super) fn move_page(&mut self, down: bool, select: bool, view_height: f32) {
        let lh = self.line_height.max(1.0);
        let rows = ((view_height / lh) as usize).max(2) - 1;
        let cols = self.wrap.cols;
        let wrapped = self.wrap.active();
        let buffer = &self.buffer;
        let last = buffer.num_lines().saturating_sub(1);
        for c in std::iter::once(&mut self.cursor).chain(self.extra_cursors.iter_mut()) {
            let start = c.row;
            for _ in 0..rows {
                if wrapped {
                    super::wrap::move_visual(c, buffer, cols, down, select);
                } else {
                    match (down, select) {
                        (true, true) => c.move_down_select(buffer),
                        (true, false) => c.move_down(buffer),
                        (false, true) => c.move_up_select(buffer),
                        (false, false) => c.move_up(buffer),
                    }
                }
            }
            // Less than a page left: land on the file's first / last char.
            if !wrapped && down && start + rows > last {
                c.col = buffer.line_len(c.row);
            } else if !wrapped && !down && start < rows {
                c.col = 0;
            }
        }
        self.dedup_cursors();

        let total_rows = if wrapped {
            self.wrap.total_rows()
        } else {
            self.buffer.num_lines()
        };
        let max_y = ((total_rows + 1) as f32 * lh - view_height).max(0.0);
        let step = rows as f32 * lh;
        self.scroll_offset.y = if down {
            (self.scroll_offset.y + step).min(max_y)
        } else {
            (self.scroll_offset.y - step).max(0.0)
        };
        self.scroll_to_cursor = true;
        self.cursor_blink_epoch = std::time::Instant::now();
    }

    // ── Edits at every cursor ──────────────────────────────────────────────

    /// The primary cursor, then the extra ones.
    fn all_cursors(&self) -> impl Iterator<Item = &Cursor> {
        std::iter::once(&self.cursor).chain(self.extra_cursors.iter())
    }

    /// A cursor's selection (or caret) as char indices, start first.
    fn span_of(&self, c: &Cursor) -> (usize, usize) {
        let idx =
            |(r, col): (usize, usize)| self.buffer.char_index(r, col).min(self.buffer.rope_len());
        match c.selection_range() {
            Some((s, e)) => (idx(s), idx(e)),
            None => {
                let i = idx(c.position());
                (i, i)
            }
        }
    }

    fn char_at(&self, idx: usize) -> Option<char> {
        (idx < self.buffer.rope_len())
            .then(|| self.buffer.rope_slice(idx, idx + 1).chars().next())
            .flatten()
    }

    /// Apply one edit per cursor (`edits[0]` for the primary, then the extras
    /// in order), all expressed against the text before any of them, then put
    /// every cursor after its own inserted text. Edits are applied bottom to
    /// top so each one's indices stay valid; overlapping ones (two cursors in
    /// one selection) are clamped so text is never deleted twice.
    pub(super) fn apply_cursor_edits(&mut self, mut edits: Vec<CursorEdit>) {
        let mut order: Vec<usize> = (0..edits.len()).collect();
        order.sort_by_key(|&i| (edits[i].start, i));
        let mut reach = 0;
        for &i in &order {
            let e = &mut edits[i];
            e.start = e.start.max(reach);
            e.end = e.end.max(e.start);
            reach = e.end;
        }
        for &i in order.iter().rev() {
            let e = &edits[i];
            self.buffer.delete_range(e.start, e.end);
            let (row, col) = self.buffer.position_of(e.start);
            self.buffer.insert_str(row, col, &e.text);
        }
        // Each caret lands in its edit's text, shifted by the edits before it.
        let mut carets = vec![0; edits.len()];
        let mut shift: isize = 0;
        for &i in &order {
            let e = &edits[i];
            carets[i] = (e.start as isize + shift) as usize + e.caret;
            shift += e.text.chars().count() as isize - (e.end - e.start) as isize;
        }
        let positions: Vec<(usize, usize)> =
            carets.iter().map(|&c| self.buffer.position_of(c)).collect();
        let changed = edits.iter().any(|e| e.start != e.end || !e.text.is_empty());
        for (i, (row, col)) in positions.into_iter().enumerate() {
            let c = if i == 0 {
                &mut self.cursor
            } else {
                &mut self.extra_cursors[i - 1]
            };
            c.clear_selection();
            c.set_position(row, col);
        }
        self.dedup_cursors();
        if changed {
            self.is_modified = true;
            self.content_version = self.content_version.wrapping_add(1);
        }
        self.cursor_blink_epoch = std::time::Instant::now();
    }

    /// Ctrl+V: `text` replaces every cursor's selection, as one undo step. It
    /// goes in as is: no auto-indent, auto-close or dedent, since pasted text
    /// carries its own indentation. The buffer is LF-only (CRLF is re-applied
    /// on save).
    pub(super) fn paste_text_at_cursors(&mut self, text: &str) {
        self.buffer.checkpoint();
        let text = text.replace('\r', "");
        let caret = text.chars().count();
        let edits = self
            .all_cursors()
            .map(|c| {
                let (start, end) = self.span_of(c);
                CursorEdit::new(start, end, text.clone(), caret)
            })
            .collect();
        self.apply_cursor_edits(edits);
    }

    /// Ctrl+V with as many lines as cursors: line k goes to the k-th cursor
    /// from the top, replacing its selection (one undo step).
    pub(super) fn paste_lines_at_cursors(&mut self, lines: &[&str]) {
        self.buffer.checkpoint();
        let spans: Vec<(usize, usize)> = self.all_cursors().map(|c| self.span_of(c)).collect();
        let mut by_position: Vec<usize> = (0..spans.len()).collect();
        by_position.sort_by_key(|&i| spans[i]);
        let mut edits: Vec<Option<CursorEdit>> = (0..spans.len()).map(|_| None).collect();
        for (k, &i) in by_position.iter().enumerate() {
            let text = lines.get(k).copied().unwrap_or_default().to_string();
            let caret = text.chars().count();
            edits[i] = Some(CursorEdit::new(spans[i].0, spans[i].1, text, caret));
        }
        self.apply_cursor_edits(edits.into_iter().flatten().collect());
    }

    /// The text each cursor would copy, top to bottom: its selection, or its
    /// whole line when it has none.
    pub(super) fn cursor_texts(&self) -> Vec<String> {
        let mut spans: Vec<(usize, usize, usize)> = self
            .all_cursors()
            .map(|c| {
                let (s, e) = self.span_of(c);
                (s, e, c.row)
            })
            .collect();
        spans.sort();
        spans
            .into_iter()
            .map(|(s, e, row)| {
                if s == e {
                    self.buffer.line(row)
                } else {
                    self.buffer.rope_slice(s, e)
                }
            })
            .collect()
    }

    pub(super) fn all_cursors_have_selection(&self) -> bool {
        self.all_cursors().all(|c| c.has_selection())
    }

    /// Ctrl+X: delete every cursor's selection (one undo step). Cursors
    /// without a selection are left alone.
    pub(super) fn cut_selections(&mut self) {
        self.buffer.checkpoint();
        let edits = self
            .all_cursors()
            .map(|c| {
                let (s, e) = self.span_of(c);
                CursorEdit::new(s, e, String::new(), 0)
            })
            .collect();
        self.apply_cursor_edits(edits);
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
    fn insert_char_replaces_an_extra_cursors_multi_line_selection() {
        let mut ed = editor_with("aa\nbb\ncc", 2, 2);
        ed.extra_cursors.push(selecting((0, 1), 1, 1));
        ed.insert_char('X', false);
        // The extra's "a\nb" is replaced, like the primary's selection would be.
        assert_eq!(text(&ed), "aXb\nccX");
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
    fn backspace_deletes_an_extra_cursors_multi_line_selection() {
        let mut ed = editor_with("ab\ncd\nef", 2, 0);
        ed.extra_cursors.push(selecting((0, 1), 1, 1));
        ed.delete_char_before();
        // The extra's "b\nc" goes, and the primary at (2,0) joins its line up.
        assert_eq!(text(&ed), "adef");
        assert_eq!(ed.extra_cursors[0].position(), (0, 1));
        assert_eq!(ed.cursor.position(), (0, 2));
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
    fn delete_removes_an_extra_cursors_multi_line_selection() {
        let mut ed = editor_with("ab\ncd\nef", 2, 2);
        ed.extra_cursors.push(selecting((0, 1), 1, 1));
        ed.delete_char_after();
        assert_eq!(text(&ed), "ad\nef");
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

        // A multi-line extra selection is replaced by the newline too.
        let mut ed = editor_with("ab\ncd\nef", 2, 2);
        ed.extra_cursors.push(selecting((0, 1), 1, 1));
        ed.insert_newline();
        assert_eq!(text(&ed), "a\nd\nef\n");
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
    fn pasted_newline_does_not_indent() {
        let mut ed = editor_with("    {", 0, 5);
        ed.paste_text_at_cursors("\n");
        assert_eq!(text(&ed), "    {\n");
        assert_eq!(ed.cursor.position(), (1, 0));
    }

    #[test]
    fn a_multi_line_paste_is_one_undo_step() {
        let mut ed = editor_with("x", 0, 1);
        ed.paste_text_at_cursors("a\r\nb\nc");
        assert_eq!(text(&ed), "xa\nb\nc");
        assert_eq!(ed.cursor.position(), (2, 1));
        assert!(ed.buffer.undo());
        assert_eq!(text(&ed), "x", "a single undo restores the pre-paste text");
    }

    #[test]
    fn paste_replaces_every_selection() {
        let mut ed = editor_with("one two", 0, 0);
        ed.cursor = selecting((0, 0), 0, 3);
        ed.extra_cursors = vec![selecting((0, 4), 0, 7)];
        ed.paste_text_at_cursors("1\n2\n3");
        assert_eq!(text(&ed), "1\n2\n3 1\n2\n3");
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
        ed.paste_text_at_cursors("}");
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

    // ── Every cursor edits, every cursor moves (CU-20) ───────────────────────

    #[test]
    fn backspace_deletes_every_cursors_selection() {
        // Ctrl+D x3 on `foo foo foo` then Backspace.
        let mut ed = editor_with("foo foo foo", 0, 0);
        ed.cursor = selecting((0, 0), 0, 3);
        ed.extra_cursors = vec![selecting((0, 4), 0, 7), selecting((0, 8), 0, 11)];
        ed.delete_char_before();
        assert_eq!(text(&ed), "  ");
        assert_eq!(ed.cursor.position(), (0, 0));
        assert_eq!(ed.extra_cursors[0].position(), (0, 1));
        assert_eq!(ed.extra_cursors[1].position(), (0, 2));
    }

    #[test]
    fn typing_at_an_earlier_extra_cursor_shifts_the_primary() {
        // The Ctrl+D search wrapped: the extra cursor is before the primary.
        let mut ed = editor_with("foo foo bar", 0, 0);
        ed.cursor = selecting((0, 4), 0, 7);
        ed.extra_cursors = vec![selecting((0, 0), 0, 3)];
        ed.insert_char('x', false);
        ed.insert_char('y', false);
        assert_eq!(text(&ed), "xy xy bar");
        assert_eq!(ed.cursor.position(), (0, 5));
        assert_eq!(ed.extra_cursors[0].position(), (0, 2));
    }

    #[test]
    fn auto_closed_pairs_shift_later_cursors_by_both_chars() {
        let mut ed = editor_with("a b", 0, 1);
        ed.extra_cursors = vec![cursor_at(0, 3)];
        ed.insert_char('(', true);
        assert_eq!(text(&ed), "a() b()");
        assert_eq!(ed.cursor.position(), (0, 2));
        assert_eq!(ed.extra_cursors[0].position(), (0, 6));
    }

    #[test]
    fn joining_lines_at_several_cursors_joins_the_right_lines() {
        let mut ed = editor_with("a\nb\nc\nd", 1, 0);
        ed.extra_cursors = vec![cursor_at(2, 0)];
        ed.delete_char_before();
        assert_eq!(text(&ed), "abc\nd");
        assert_eq!(ed.cursor.position(), (0, 1));
        assert_eq!(ed.extra_cursors[0].position(), (0, 2));

        let mut ed = editor_with("a\nb\nc\nd", 0, 1);
        ed.extra_cursors = vec![cursor_at(1, 1)];
        ed.delete_char_after();
        assert_eq!(text(&ed), "abc\nd");
    }

    #[test]
    fn distributed_paste_replaces_each_selection_with_its_line() {
        let mut ed = editor_with("foo foo", 0, 0);
        ed.cursor = selecting((0, 4), 0, 7);
        ed.extra_cursors = vec![selecting((0, 0), 0, 3)];
        // Line k goes to the k-th cursor from the top.
        ed.paste_lines_at_cursors(&["first", "second"]);
        assert_eq!(text(&ed), "first second");
        assert_eq!(ed.extra_cursors[0].position(), (0, 5));
        assert_eq!(ed.cursor.position(), (0, 12));
        assert!(!ed.cursor.has_selection());
    }

    #[test]
    fn cut_removes_every_selection_and_copy_lists_them_top_to_bottom() {
        let mut ed = editor_with("ab\ncd", 1, 0);
        ed.cursor = selecting((1, 0), 1, 1);
        ed.extra_cursors = vec![selecting((0, 1), 1, 0)];
        assert_eq!(ed.cursor_texts(), ["b\n", "c"]);
        ed.cut_selections();
        assert_eq!(text(&ed), "ad");
        assert!(ed.buffer.undo(), "one undo step");
        assert_eq!(text(&ed), "ab\ncd");
    }
}
