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

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(content: &str) -> Editor {
        let mut ed = Editor::new();
        ed.set_content(content.to_string(), None);
        ed
    }

    fn find(ed: &mut Editor, query: &str) {
        ed.find_query = query.to_string();
        ed.update_find_matches();
    }

    #[test]
    fn empty_query_clears_matches() {
        let mut ed = editor("foo\nfoo");
        find(&mut ed, "foo");
        assert_eq!(ed.find_matches, vec![0, 1]);
        find(&mut ed, "");
        assert!(ed.find_matches.is_empty());
    }

    #[test]
    fn case_insensitive_search_jumps_to_next_match_after_cursor() {
        let mut ed = editor("Foo\nbar\nfoo\nbaz");
        ed.cursor.set_position(1, 2);
        find(&mut ed, "FOO");
        assert_eq!(ed.find_matches, vec![0, 2]);
        assert_eq!(ed.find_current, 1);
        assert_eq!(ed.cursor.position(), (2, 0));
        assert!(ed.scroll_to_cursor);
    }

    #[test]
    fn search_wraps_to_first_match_when_cursor_is_past_all_matches() {
        let mut ed = editor("foo\nbar\nbaz");
        ed.cursor.set_position(2, 0);
        find(&mut ed, "foo");
        assert_eq!(ed.find_current, 0);
        assert_eq!(ed.cursor.position(), (0, 0));
    }

    #[test]
    fn case_sensitive_search() {
        let mut ed = editor("Foo\nbar\nfoo");
        ed.find_case_sensitive = true;
        find(&mut ed, "foo");
        assert_eq!(ed.find_matches, vec![2]);
        find(&mut ed, "Foo");
        assert_eq!(ed.find_matches, vec![0]);
    }

    #[test]
    fn regex_search_honours_case_sensitivity() {
        let mut ed = editor("Foo1\nbar\nfoo22");
        ed.find_use_regex = true;
        find(&mut ed, r"^f\w+\d$");
        assert_eq!(ed.find_matches, vec![0, 2]);
        ed.find_case_sensitive = true;
        find(&mut ed, r"^f\w+\d$");
        assert_eq!(ed.find_matches, vec![2]);
    }

    #[test]
    fn invalid_regex_yields_no_matches() {
        let mut ed = editor("(foo)");
        ed.find_use_regex = true;
        ed.cursor.set_position(0, 3);
        find(&mut ed, "(");
        assert!(ed.find_matches.is_empty());
        assert_eq!(ed.find_current, 0);
        assert_eq!(ed.cursor.position(), (0, 3), "cursor untouched");
    }

    #[test]
    fn find_next_and_prev_cycle_through_matches() {
        let mut ed = editor("x\nx\ny\nx");
        find(&mut ed, "x");
        assert_eq!(ed.find_matches, vec![0, 1, 3]);
        assert_eq!(ed.cursor.row, 0);

        ed.find_next();
        assert_eq!(ed.cursor.row, 1);
        ed.find_next();
        assert_eq!(ed.cursor.row, 3);
        ed.find_next();
        assert_eq!(ed.cursor.row, 0, "wraps forward");

        ed.find_prev();
        assert_eq!(ed.cursor.row, 3, "wraps backward");
        ed.find_prev();
        assert_eq!(ed.cursor.row, 1);
        assert_eq!(ed.find_current, 1);
    }

    #[test]
    fn find_next_and_prev_without_matches_do_nothing() {
        let mut ed = editor("abc\ndef");
        ed.cursor.set_position(1, 2);
        find(&mut ed, "zzz");
        ed.find_next();
        ed.find_prev();
        assert_eq!(ed.cursor.position(), (1, 2));
        assert!(!ed.scroll_to_cursor);
    }

    #[test]
    fn replace_current_replaces_first_occurrence_on_match_line() {
        let mut ed = editor("FOO foo\nbar\nfoo");
        find(&mut ed, "foo");
        ed.replace_query = "baz".to_string();
        ed.replace_current();
        assert_eq!(ed.buffer.to_string(), "baz foo\nbar\nfoo");
        assert!(ed.is_modified);
        assert_eq!(ed.content_version, 1);
        // Matches are recomputed after the replacement.
        assert_eq!(ed.find_matches, vec![0, 2]);
    }

    #[test]
    fn replace_current_noops_without_query_or_matches() {
        let mut ed = editor("foo");
        ed.replace_query = "bar".to_string();
        ed.replace_current();
        find(&mut ed, "zzz");
        ed.replace_current();
        assert_eq!(ed.buffer.to_string(), "foo");
        assert!(!ed.is_modified);
    }

    #[test]
    fn replace_current_skips_stale_match_line() {
        let mut ed = editor("foo\nbar");
        find(&mut ed, "foo");
        // Buffer changes behind the search's back: the match row no longer contains the query.
        ed.buffer.replace_line(0, "qux");
        ed.replace_query = "x".to_string();
        ed.replace_current();
        assert_eq!(ed.buffer.to_string(), "qux\nbar");
        assert!(!ed.is_modified);
    }

    #[test]
    fn replace_all_replaces_every_occurrence_case_insensitively() {
        let mut ed = editor("Foo foo\nxfoox\nbar");
        ed.find_query = "foo".to_string();
        ed.replace_query = "baz".to_string();
        ed.replace_all_matches();
        assert_eq!(ed.buffer.to_string(), "baz baz\nxbazx\nbar");
        assert!(ed.is_modified);
        assert_eq!(ed.content_version, 1);
        assert!(ed.find_matches.is_empty());
    }

    #[test]
    fn replace_all_with_empty_replacement_deletes_matches() {
        let mut ed = editor("a-b-c");
        ed.find_query = "-".to_string();
        ed.replace_all_matches();
        assert_eq!(ed.buffer.to_string(), "abc");
    }

    #[test]
    fn replace_all_with_empty_query_is_noop() {
        let mut ed = editor("abc");
        ed.replace_query = "x".to_string();
        ed.replace_all_matches();
        assert_eq!(ed.buffer.to_string(), "abc");
        assert!(!ed.is_modified);
    }

    #[test]
    #[ignore = "BUG: replace uses the byte offset from str::find as a char column, corrupting lines with non-ASCII text before the match"]
    fn replace_current_handles_non_ascii_prefix() {
        let mut ed = editor("é foo");
        find(&mut ed, "foo");
        ed.replace_query = "bar".to_string();
        ed.replace_current();
        assert_eq!(ed.buffer.to_string(), "é bar");
    }

    #[test]
    #[ignore = "BUG: replace_current/replace_all ignore find_case_sensitive and always match case-insensitively"]
    fn replace_current_respects_case_sensitivity() {
        let mut ed = editor("foo Foo");
        ed.find_case_sensitive = true;
        find(&mut ed, "Foo");
        ed.replace_query = "X".to_string();
        ed.replace_current();
        assert_eq!(ed.buffer.to_string(), "foo X");
    }

    #[test]
    #[ignore = "BUG: replace_all_matches loops forever when the replacement contains the query"]
    fn replace_all_terminates_when_replacement_contains_query() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut ed = editor("foo");
            ed.find_query = "foo".to_string();
            ed.replace_query = "foobar".to_string();
            ed.replace_all_matches();
            let _ = tx.send(ed.buffer.to_string());
        });
        let result = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("replace_all_matches did not terminate");
        assert_eq!(result, "foobar");
    }

    fn select(ed: &mut Editor, row: usize, from: usize, to: usize) {
        ed.cursor.set_position(row, from);
        ed.cursor.start_selection();
        ed.cursor.set_position(row, to);
    }

    #[test]
    fn word_occurrences_find_whole_words_except_the_selected_one() {
        let mut ed = editor("foo bar foo\nfoobar foo\n_foo foo\nfo");
        select(&mut ed, 0, 0, 3);
        ed.update_word_occurrences(0, 100);
        assert_eq!(ed.word_occurrences_word, "foo");
        assert_eq!(ed.word_occurrences, vec![(0, 8, 11), (1, 7, 10), (2, 5, 8)]);
    }

    #[test]
    fn word_occurrences_only_scan_visible_lines() {
        let mut ed = editor("foo\nfoo\nfoo\nfoo");
        select(&mut ed, 0, 0, 3);
        ed.update_word_occurrences(2, 2);
        assert_eq!(ed.word_occurrences, vec![(2, 0, 3)]);
    }

    #[test]
    fn word_occurrences_are_cached_until_word_or_content_changes() {
        let mut ed = editor("abc abc");
        select(&mut ed, 0, 0, 3);
        ed.update_word_occurrences(0, 0);
        assert_eq!(ed.word_occurrences, vec![(0, 4, 7)]);

        ed.word_occurrences.clear();
        ed.update_word_occurrences(0, 0);
        assert!(ed.word_occurrences.is_empty(), "cache hit: no recompute");

        ed.content_version += 1;
        ed.update_word_occurrences(0, 0);
        assert_eq!(ed.word_occurrences, vec![(0, 4, 7)], "recomputed");
    }

    #[test]
    fn word_occurrences_require_a_single_word_selection_of_three_chars() {
        let mut ed = editor("ab ab ab\nfoo bar foo bar");
        // No selection.
        ed.cursor.set_position(1, 1);
        ed.update_word_occurrences(0, 1);
        assert!(ed.word_occurrences.is_empty());
        assert_eq!(ed.word_occurrences_version, 0);

        // Too short.
        select(&mut ed, 0, 0, 2);
        ed.update_word_occurrences(0, 1);
        assert!(ed.word_occurrences.is_empty());

        // Contains a non-word character.
        select(&mut ed, 1, 0, 7);
        ed.update_word_occurrences(0, 1);
        assert!(ed.word_occurrences.is_empty());
        assert_eq!(ed.word_occurrences_word, "");

        // Multi-line selection.
        ed.cursor.set_position(0, 0);
        ed.cursor.start_selection();
        ed.cursor.set_position(1, 3);
        ed.update_word_occurrences(0, 1);
        assert!(ed.word_occurrences.is_empty());
    }
}
