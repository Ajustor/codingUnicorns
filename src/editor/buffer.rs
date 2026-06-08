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
