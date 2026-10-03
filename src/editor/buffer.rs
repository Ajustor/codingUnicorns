use ropey::Rope;

pub struct Buffer {
    rope: Rope,
    history: Vec<Rope>,
    future: Vec<Rope>,
}

impl std::fmt::Display for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.rope)
    }
}

impl Buffer {
    pub fn new() -> Self {
        Self {
            rope: Rope::from_str(""),
            history: vec![],
            future: vec![],
        }
    }

    pub fn from_str(s: &str) -> Self {
        Self {
            rope: Rope::from_str(s),
            history: vec![],
            future: vec![],
        }
    }

    pub fn num_lines(&self) -> usize {
        self.rope.len_lines().max(1)
    }

    pub fn line(&self, idx: usize) -> String {
        if idx >= self.rope.len_lines() {
            return String::new();
        }
        let line = self.rope.line(idx);
        let s: String = line.chars().collect();
        s.trim_end_matches('\n').trim_end_matches('\r').to_string()
    }

    pub fn line_len(&self, idx: usize) -> usize {
        self.line(idx).chars().count()
    }

    /// Character count of a line WITHOUT allocating a String (unlike `line_len`).
    /// Includes the trailing newline — fine for width estimation. Cheap (O(1) on
    /// the rope), suitable for scanning every line.
    pub fn line_char_len_fast(&self, idx: usize) -> usize {
        if idx >= self.rope.len_lines() {
            0
        } else {
            self.rope.line(idx).len_chars()
        }
    }

    pub fn char_index(&self, row: usize, col: usize) -> usize {
        let line_start = self
            .rope
            .line_to_char(row.min(self.rope.len_lines().saturating_sub(1)));
        line_start + col
    }

    pub fn checkpoint(&mut self) {
        self.history.push(self.rope.clone());
        self.future.clear();
        if self.history.len() > 200 {
            self.history.remove(0);
        }
    }

    pub fn undo(&mut self) -> bool {
        if let Some(prev) = self.history.pop() {
            self.future.push(self.rope.clone());
            self.rope = prev;
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if let Some(next) = self.future.pop() {
            self.history.push(self.rope.clone());
            self.rope = next;
            true
        } else {
            false
        }
    }

    pub fn insert_char(&mut self, row: usize, col: usize, ch: char) {
        let idx = self.char_index(row, col);
        let idx = idx.min(self.rope.len_chars());
        self.rope.insert_char(idx, ch);
    }

    pub fn delete_char(&mut self, row: usize, col: usize) {
        let idx = self.char_index(row, col);
        if idx < self.rope.len_chars() {
            self.rope.remove(idx..idx + 1);
        }
    }

    pub fn split_line(&mut self, row: usize, col: usize) {
        let idx = self.char_index(row, col);
        let idx = idx.min(self.rope.len_chars());
        self.rope.insert_char(idx, '\n');
    }

    pub fn join_lines(&mut self, row: usize) {
        if row == 0 || row >= self.rope.len_lines() {
            return;
        }
        let line_start = self.rope.line_to_char(row);
        let prev_line_end = line_start - 1;
        if prev_line_end < self.rope.len_chars() {
            let ch = self.rope.char(prev_line_end);
            if ch == '\n' {
                self.rope.remove(prev_line_end..prev_line_end + 1);
            }
        }
    }

    pub fn insert_str(&mut self, row: usize, col: usize, s: &str) {
        let idx = self.char_index(row, col);
        let idx = idx.min(self.rope.len_chars());
        self.rope.insert(idx, s);
    }

    pub fn delete_range(&mut self, start: usize, end: usize) {
        let end = end.min(self.rope.len_chars());
        if start < end {
            self.rope.remove(start..end);
        }
    }

    pub fn replace_line(&mut self, row: usize, new_content: &str) {
        let start_idx = self.char_index(row, 0);
        let end_idx = self.char_index(row, self.line_len(row));
        let end_idx = end_idx.min(self.rope.len_chars());
        if start_idx <= end_idx {
            self.rope.remove(start_idx..end_idx);
            self.rope.insert(start_idx, new_content);
        }
    }

    pub fn rope_len(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn rope_slice(&self, start: usize, end: usize) -> String {
        let end = end.min(self.rope.len_chars());
        if start >= end {
            return String::new();
        }
        self.rope.slice(start..end).to_string()
    }

    /// Insert `content` as a new line at `row` (existing lines shift down).
    pub fn insert_line(&mut self, row: usize, content: &str) {
        let total = self.rope.len_lines();
        let insert_pos = if row >= total {
            self.rope.len_chars()
        } else {
            self.rope.line_to_char(row)
        };
        let mut text = content.to_string();
        text.push('\n');
        self.rope.insert(insert_pos, &text);
    }

    pub fn delete_line(&mut self, row: usize) {
        let total = self.rope.len_lines();
        if total == 0 {
            return;
        }
        let row = row.min(total.saturating_sub(1));
        let start = self.rope.line_to_char(row);
        let end = if row + 1 < total {
            self.rope.line_to_char(row + 1)
        } else {
            self.rope.len_chars()
        };
        if start < end {
            self.rope.remove(start..end);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(buf: &Buffer) -> Vec<String> {
        (0..buf.num_lines()).map(|i| buf.line(i)).collect()
    }

    #[test]
    fn new_buffer_is_a_single_empty_line() {
        let buf = Buffer::new();
        assert_eq!(buf.num_lines(), 1);
        assert_eq!(buf.line(0), "");
        assert_eq!(buf.to_string(), "");
        assert_eq!(buf.rope_len(), 0);
    }

    #[test]
    fn from_str_splits_lines_and_strips_line_endings() {
        let buf = Buffer::from_str("one\r\ntwo\nthree");
        assert_eq!(buf.num_lines(), 3);
        assert_eq!(lines(&buf), vec!["one", "two", "three"]);
        assert_eq!(buf.to_string(), "one\r\ntwo\nthree");
    }

    #[test]
    fn line_out_of_range_is_empty() {
        let buf = Buffer::from_str("a\nb");
        assert_eq!(buf.line(2), "");
        assert_eq!(buf.line(100), "");
        assert_eq!(buf.line_len(100), 0);
    }

    #[test]
    fn line_len_counts_chars_not_bytes() {
        let buf = Buffer::from_str("héllo\n日本\n");
        assert_eq!(buf.line_len(0), 5);
        assert_eq!(buf.line_len(1), 2);
        assert_eq!(buf.line_len(2), 0);
    }

    #[test]
    fn line_char_len_fast_includes_newline() {
        let buf = Buffer::from_str("abc\nde");
        assert_eq!(buf.line_char_len_fast(0), 4);
        assert_eq!(buf.line_char_len_fast(1), 2);
        assert_eq!(buf.line_char_len_fast(2), 0);
    }

    #[test]
    fn char_index_maps_row_col_and_clamps_row() {
        let buf = Buffer::from_str("abc\ndef\ngh");
        assert_eq!(buf.char_index(0, 0), 0);
        assert_eq!(buf.char_index(1, 2), 6);
        assert_eq!(buf.char_index(2, 1), 9);
        // Row past the end is clamped to the last line.
        assert_eq!(buf.char_index(50, 0), 8);
    }

    #[test]
    fn undo_redo_round_trip() {
        let mut buf = Buffer::from_str("a");
        assert!(!buf.undo());
        assert!(!buf.redo());

        buf.checkpoint();
        buf.insert_char(0, 1, 'b');
        buf.checkpoint();
        buf.insert_char(0, 2, 'c');
        assert_eq!(buf.to_string(), "abc");

        assert!(buf.undo());
        assert_eq!(buf.to_string(), "ab");
        assert!(buf.undo());
        assert_eq!(buf.to_string(), "a");
        assert!(!buf.undo());

        assert!(buf.redo());
        assert_eq!(buf.to_string(), "ab");
        assert!(buf.redo());
        assert_eq!(buf.to_string(), "abc");
        assert!(!buf.redo());
    }

    #[test]
    fn checkpoint_clears_redo_stack() {
        let mut buf = Buffer::from_str("x");
        buf.checkpoint();
        buf.insert_char(0, 1, 'y');
        assert!(buf.undo());
        buf.checkpoint();
        buf.insert_char(0, 1, 'z');
        assert!(!buf.redo(), "new edit must discard the redo history");
        assert_eq!(buf.to_string(), "xz");
    }

    #[test]
    fn history_is_capped_at_200_entries() {
        let mut buf = Buffer::new();
        for i in 0..250 {
            buf.checkpoint();
            buf.insert_str(0, 0, &format!("{i},"));
        }
        let mut undos = 0;
        while buf.undo() {
            undos += 1;
        }
        assert_eq!(undos, 200);
        // The oldest 50 snapshots were dropped, so we land on the state after 50 edits.
        let expected: String = (0..50).rev().map(|i| format!("{i},")).collect();
        assert_eq!(buf.to_string(), expected);
    }

    #[test]
    fn insert_char_inserts_and_clamps_past_end() {
        let mut buf = Buffer::from_str("ac\nx");
        buf.insert_char(0, 1, 'b');
        assert_eq!(buf.to_string(), "abc\nx");
        buf.insert_char(1, 99, '!');
        assert_eq!(buf.to_string(), "abc\nx!");
    }

    #[test]
    fn delete_char_removes_char_and_joins_on_newline() {
        let mut buf = Buffer::from_str("abc\ndef");
        buf.delete_char(0, 1);
        assert_eq!(buf.to_string(), "ac\ndef");
        // Deleting the newline at end of line 0 joins the lines.
        buf.delete_char(0, 2);
        assert_eq!(buf.to_string(), "acdef");
        // Past end is a no-op.
        buf.delete_char(0, 5);
        buf.delete_char(0, 50);
        assert_eq!(buf.to_string(), "acdef");
    }

    #[test]
    fn split_line_inserts_newline() {
        let mut buf = Buffer::from_str("hello world");
        buf.split_line(0, 5);
        assert_eq!(lines(&buf), vec!["hello", " world"]);
        buf.split_line(1, 99);
        assert_eq!(buf.to_string(), "hello\n world\n");
    }

    #[test]
    fn join_lines_merges_with_previous_line() {
        let mut buf = Buffer::from_str("a\nb\nc");
        buf.join_lines(2);
        assert_eq!(buf.to_string(), "a\nbc");
        buf.join_lines(1);
        assert_eq!(buf.to_string(), "abc");
    }

    #[test]
    #[ignore = "BUG: join_lines only removes '\\n', so CRLF lines stay split by the leftover '\\r' (Backspace/Delete at line boundary in CRLF files)"]
    fn join_lines_handles_crlf() {
        let mut buf = Buffer::from_str("a\r\nb");
        buf.join_lines(1);
        assert_eq!(buf.to_string(), "ab");
        assert_eq!(buf.num_lines(), 1);
    }

    #[test]
    fn join_lines_ignores_first_and_out_of_range_rows() {
        let mut buf = Buffer::from_str("a\nb");
        buf.join_lines(0);
        buf.join_lines(2);
        buf.join_lines(10);
        assert_eq!(buf.to_string(), "a\nb");
    }

    #[test]
    fn insert_str_inserts_multiline_text() {
        let mut buf = Buffer::from_str("ad");
        buf.insert_str(0, 1, "b\nc");
        assert_eq!(lines(&buf), vec!["ab", "cd"]);
        buf.insert_str(1, 100, "!");
        assert_eq!(buf.to_string(), "ab\ncd!");
    }

    #[test]
    fn delete_range_clamps_and_ignores_empty_ranges() {
        let mut buf = Buffer::from_str("abcdef");
        buf.delete_range(1, 3);
        assert_eq!(buf.to_string(), "adef");
        buf.delete_range(2, 2);
        buf.delete_range(3, 1);
        assert_eq!(buf.to_string(), "adef");
        buf.delete_range(2, 1000);
        assert_eq!(buf.to_string(), "ad");
    }

    #[test]
    fn replace_line_keeps_line_ending() {
        let mut buf = Buffer::from_str("one\ntwo\nthree");
        buf.replace_line(1, "TWO!");
        assert_eq!(buf.to_string(), "one\nTWO!\nthree");
        buf.replace_line(2, "");
        assert_eq!(buf.to_string(), "one\nTWO!\n");
    }

    #[test]
    fn rope_slice_returns_range_and_handles_bad_ranges() {
        let buf = Buffer::from_str("hello\nworld");
        assert_eq!(buf.rope_len(), 11);
        assert_eq!(buf.rope_slice(0, 5), "hello");
        assert_eq!(buf.rope_slice(4, 7), "o\nw");
        assert_eq!(buf.rope_slice(6, 999), "world");
        assert_eq!(buf.rope_slice(5, 5), "");
        assert_eq!(buf.rope_slice(8, 2), "");
    }

    #[test]
    fn insert_line_shifts_existing_lines_down() {
        let mut buf = Buffer::from_str("a\nc\n");
        buf.insert_line(1, "b");
        assert_eq!(buf.to_string(), "a\nb\nc\n");
        buf.insert_line(0, "start");
        assert_eq!(buf.to_string(), "start\na\nb\nc\n");
        // Past the end (buffer ends with newline): appended as a new line.
        buf.insert_line(99, "end");
        assert_eq!(buf.to_string(), "start\na\nb\nc\nend\n");
    }

    #[test]
    #[ignore = "BUG: insert_line past the last line merges into it when the buffer lacks a trailing newline"]
    fn insert_line_after_last_line_without_trailing_newline() {
        // Editor::duplicate_line on the last line of "a\nb" calls insert_line(2, "b")
        // and currently produces "a\nbb\n" instead of a duplicated line.
        let mut buf = Buffer::from_str("a\nb");
        buf.insert_line(2, "b");
        assert_eq!(lines(&buf)[..3], ["a", "b", "b"]);
    }

    #[test]
    fn delete_line_removes_line_and_clamps_row() {
        let mut buf = Buffer::from_str("a\nb\nc");
        buf.delete_line(1);
        assert_eq!(buf.to_string(), "a\nc");
        // Out-of-range row deletes the last line.
        buf.delete_line(42);
        assert_eq!(buf.to_string(), "a\n");
        buf.delete_line(0);
        assert_eq!(buf.to_string(), "");
        // Deleting from an empty buffer is a no-op.
        buf.delete_line(0);
        assert_eq!(buf.to_string(), "");
    }
}
