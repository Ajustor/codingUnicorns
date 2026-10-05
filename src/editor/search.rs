use super::Editor;

impl Editor {
    /// Build the matcher for the current find settings (plain or regex,
    /// case-sensitive or not). Shared by find and replace so they always agree.
    /// Returns `None` for an empty query or an invalid regex.
    pub(super) fn find_regex(&self) -> Option<regex::Regex> {
        if self.find_query.is_empty() {
            return None;
        }
        let pattern = if self.find_use_regex {
            self.find_query.clone()
        } else {
            regex::escape(&self.find_query)
        };
        regex::RegexBuilder::new(&pattern)
            .case_insensitive(!self.find_case_sensitive)
            .build()
            .ok()
    }

    /// Replace the first (`all == false`) or every match of `re` in `line`.
    /// Regex mode expands `$1`-style captures; plain mode inserts literally.
    fn replace_in_line(&self, re: &regex::Regex, line: &str, all: bool) -> String {
        let limit = if all { 0 } else { 1 };
        if self.find_use_regex {
            re.replacen(line, limit, self.replace_query.as_str())
                .into_owned()
        } else {
            re.replacen(line, limit, regex::NoExpand(&self.replace_query))
                .into_owned()
        }
    }

    pub(super) fn update_find_matches(&mut self) {
        self.find_matches.clear();
        // Pick up from nearest match to current cursor row
        let cursor_row = self.cursor.row;
        if let Some(re) = self.find_regex() {
            for i in 0..self.buffer.num_lines() {
                if re.is_match(&self.buffer.line(i)) {
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
        let Some(re) = self.find_regex() else {
            return;
        };
        if self.find_matches.is_empty() {
            return;
        }
        let row = self.find_matches[self.find_current];
        let line = self.buffer.line(row);
        if !re.is_match(&line) {
            return;
        }
        let new_line = self.replace_in_line(&re, &line, false);
        self.buffer.replace_line(row, &new_line);
        self.is_modified = true;
        self.content_version = self.content_version.wrapping_add(1);
        self.update_find_matches();
    }

    /// Replace all occurrences of `find_query` with `replace_query`.
    /// Each original occurrence is replaced exactly once (the replacement text
    /// is never re-scanned, so a replacement containing the query terminates).
    pub fn replace_all_matches(&mut self) {
        let Some(re) = self.find_regex() else {
            return;
        };
        for row in 0..self.buffer.num_lines() {
            let line = self.buffer.line(row);
            if re.is_match(&line) {
                let new_line = self.replace_in_line(&re, &line, true);
                self.buffer.replace_line(row, &new_line);
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
    fn replace_current_handles_non_ascii_prefix() {
        let mut ed = editor("é foo");
        find(&mut ed, "foo");
        ed.replace_query = "bar".to_string();
        ed.replace_current();
        assert_eq!(ed.buffer.to_string(), "é bar");
    }

    #[test]
    fn replace_current_respects_case_sensitivity() {
        let mut ed = editor("foo Foo");
        ed.find_case_sensitive = true;
        find(&mut ed, "Foo");
        ed.replace_query = "X".to_string();
        ed.replace_current();
        assert_eq!(ed.buffer.to_string(), "foo X");
    }

    #[test]
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

    #[test]
    fn replace_all_handles_non_ascii_prefix() {
        let mut ed = editor(
            "é foo ü foo
ßfooß",
        );
        ed.find_query = "foo".to_string();
        ed.replace_query = "bar".to_string();
        ed.replace_all_matches();
        assert_eq!(
            ed.buffer.to_string(),
            "é bar ü bar
ßbarß"
        );
    }

    #[test]
    fn replace_all_respects_case_sensitivity() {
        let mut ed = editor(
            "foo Foo
FOO",
        );
        ed.find_case_sensitive = true;
        ed.find_query = "Foo".to_string();
        ed.replace_query = "X".to_string();
        ed.replace_all_matches();
        assert_eq!(
            ed.buffer.to_string(),
            "foo X
FOO"
        );
    }

    #[test]
    fn replace_in_plain_mode_treats_query_and_replacement_literally() {
        let mut ed = editor("a.b axb $1");
        ed.find_query = ".".to_string();
        ed.replace_query = "$0".to_string();
        ed.replace_all_matches();
        assert_eq!(ed.buffer.to_string(), "a$0b axb $1");
    }

    #[test]
    fn replace_honours_regex_mode_with_captures() {
        let mut ed = editor(
            "foo1 Foo22
bar",
        );
        ed.find_use_regex = true;
        find(&mut ed, r"f(o+)(\d+)");
        assert_eq!(ed.find_matches, vec![0]);
        ed.replace_query = "${2}x".to_string();
        ed.replace_current();
        assert_eq!(
            ed.buffer.to_string(),
            "1x Foo22
bar"
        );
        ed.replace_all_matches();
        assert_eq!(
            ed.buffer.to_string(),
            "1x 22x
bar"
        );
    }

    #[test]
    fn replace_with_invalid_regex_is_noop() {
        let mut ed = editor("(foo)");
        ed.find_use_regex = true;
        ed.find_query = "(".to_string();
        ed.replace_query = "x".to_string();
        ed.replace_all_matches();
        ed.replace_current();
        assert_eq!(ed.buffer.to_string(), "(foo)");
        assert!(!ed.is_modified);
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
