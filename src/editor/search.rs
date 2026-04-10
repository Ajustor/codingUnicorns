use super::Editor;

impl Editor {
    pub(super) fn update_find_matches(&mut self) {
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

    pub(super) fn find_next(&mut self) {
        if self.find_matches.is_empty() {
            return;
        }
        self.find_current = (self.find_current + 1) % self.find_matches.len();
        let row = self.find_matches[self.find_current];
        self.cursor.set_position(row, 0);
        self.scroll_to_cursor = true;
    }

    pub(super) fn find_prev(&mut self) {
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

    /// Recompute visible word occurrences if the word under cursor changed.
    /// Only highlights when there is an active selection (not just cursor on a word).
    pub(super) fn update_word_occurrences(
        &mut self,
        first_visible_line: usize,
        last_visible_line: usize,
    ) {
        let (row, col) = self.cursor.position();

        // Only highlight when there is an active selection of a single word
        let word = if let Some(((sr, sc), (er, ec))) = self.cursor.selection_range() {
            if sr == er && ec > sc {
                let line = self.buffer.line(sr);
                let selected: String = line.chars().skip(sc).take(ec - sc).collect();
                if selected.len() >= 3 && selected.chars().all(|c| c.is_alphanumeric() || c == '_')
                {
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

        if word == self.word_occurrences_word
            && self.content_version == self.word_occurrences_version
        {
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

        for line_idx in
            first_visible_line..=last_visible_line.min(self.buffer.num_lines().saturating_sub(1))
        {
            let line = self.buffer.line(line_idx);
            let chars: Vec<char> = line.chars().collect();
            if chars.len() < word_len {
                continue;
            }
            let mut col_pos = 0;
            while col_pos + word_len <= chars.len() {
                if chars[col_pos..col_pos + word_len] == word_chars[..] {
                    let before_ok = col_pos == 0 || !is_word_char(chars[col_pos - 1]);
                    let after_ok = col_pos + word_len >= chars.len()
                        || !is_word_char(chars[col_pos + word_len]);
                    if before_ok && after_ok {
                        let is_cursor_pos =
                            line_idx == row && col_pos <= col && col <= col_pos + word_len;
                        if !is_cursor_pos {
                            self.word_occurrences
                                .push((line_idx, col_pos, col_pos + word_len));
                        }
                    }
                    col_pos += word_len;
                } else {
                    col_pos += 1;
                }
            }
        }
    }
}
