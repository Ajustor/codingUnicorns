use super::highlight;
use super::Editor;

impl Editor {
    /// Remove extra cursors that share a position with the primary cursor or with each other.
    pub(super) fn dedup_cursors(&mut self) {
        let primary_pos = self.cursor.position();
        self.extra_cursors.retain(|c| c.position() != primary_pos);
        let mut seen = std::collections::HashSet::new();
        self.extra_cursors.retain(|c| seen.insert(c.position()));
    }

    /// Replace the partial word at the cursor with the confirmed autocomplete suggestion.
    pub(super) fn confirm_autocomplete(&mut self) {
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

    pub(super) fn trigger_autocomplete_update(&mut self) {
        let (_, word) = self.current_word_at_cursor();
        let buffer_words = self.buffer_words();
        let lang = self.highlighter.language.clone();
        let keywords = highlight::keywords_for_language(&lang);
        self.autocomplete.update(&word, &buffer_words, keywords);
    }

    /// Public entry point for the local (buffer words + language keywords) autocomplete.
    /// Used as a fallback when no LSP server is connected for the current file type.
    pub fn trigger_local_completion(&mut self) {
        self.trigger_autocomplete_update();
    }

    pub(super) fn all_cursor_rows(&self) -> Vec<usize> {
        let mut rows = vec![self.cursor.row];
        for ec in &self.extra_cursors {
            rows.push(ec.row);
        }
        rows.sort_unstable();
        rows.dedup();
        rows
    }

    /// Returns the set of lines covered by the current selection, or just the cursor lines.
    pub(super) fn selected_line_rows(&self) -> Vec<usize> {
        if let Some(((sr, _), (er, _))) = self.cursor.selection_range() {
            (sr..=er).collect()
        } else {
            self.all_cursor_rows()
        }
    }

    /// Returns the comment prefix for the current file based on its extension.
    pub(super) fn comment_prefix(&self) -> &'static str {
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
}

#[cfg(test)]
mod tests {
    use super::super::cursor::Cursor;
    use super::*;
    use std::path::PathBuf;

    fn editor(content: &str) -> Editor {
        let mut ed = Editor::new();
        ed.set_content(content.to_string(), None);
        ed
    }

    fn cursor_at(row: usize, col: usize) -> Cursor {
        let mut c = Cursor::new();
        c.set_position(row, col);
        c
    }

    #[test]
    fn dedup_cursors_removes_primary_duplicates_and_repeats() {
        let mut ed = editor("aaa\nbbb\nccc");
        ed.cursor.set_position(0, 1);
        ed.extra_cursors = vec![
            cursor_at(0, 1),
            cursor_at(1, 1),
            cursor_at(2, 0),
            cursor_at(1, 1),
            cursor_at(0, 1),
        ];
        ed.dedup_cursors();
        let positions: Vec<_> = ed.extra_cursors.iter().map(|c| c.position()).collect();
        assert_eq!(positions, vec![(1, 1), (2, 0)]);
    }

    #[test]
    fn confirm_autocomplete_replaces_partial_word() {
        let mut ed = editor("let foo_bar = 1;\nx = fo");
        ed.cursor.set_position(1, 6);
        ed.autocomplete
            .update("fo", &["foo_bar".to_string()], &[] as &[&str]);
        assert!(ed.autocomplete.visible);

        ed.confirm_autocomplete();

        assert_eq!(ed.buffer.line(1), "x = foo_bar");
        assert_eq!(ed.cursor.position(), (1, 11));
        assert!(ed.is_modified);
        assert_eq!(ed.content_version, 1);
        assert!(!ed.autocomplete.visible);
        // The edit is a single undo step.
        assert!(ed.buffer.undo());
        assert_eq!(ed.buffer.line(1), "x = fo");
    }

    #[test]
    fn confirm_autocomplete_without_suggestion_only_hides_popup() {
        let mut ed = editor("abc");
        ed.cursor.set_position(0, 3);
        ed.autocomplete.visible = true;
        ed.confirm_autocomplete();
        assert_eq!(ed.buffer.to_string(), "abc");
        assert!(!ed.is_modified);
        assert_eq!(ed.content_version, 0);
        assert!(!ed.autocomplete.visible);
    }

    #[test]
    fn trigger_autocomplete_update_uses_buffer_words() {
        let mut ed = editor("hello help world\nhe");
        ed.cursor.set_position(1, 2);
        ed.trigger_autocomplete_update();
        assert!(ed.autocomplete.visible);
        assert_eq!(ed.autocomplete.query, "he");
        let labels: Vec<_> = ed
            .autocomplete
            .suggestions
            .iter()
            .map(|s| s.label.as_str())
            .collect();
        assert!(labels.contains(&"hello"));
        assert!(labels.contains(&"help"));
        assert!(!labels.contains(&"world"));
        assert!(!labels.contains(&"he"), "the typed word itself is excluded");
    }

    #[test]
    fn trigger_local_completion_hides_popup_for_short_words() {
        let mut ed = editor("hello\nh");
        ed.cursor.set_position(1, 1);
        ed.autocomplete.visible = true;
        ed.trigger_local_completion();
        assert!(!ed.autocomplete.visible);

        ed.buffer.insert_char(1, 1, 'e');
        ed.cursor.set_position(1, 2);
        ed.trigger_local_completion();
        assert!(ed.autocomplete.visible);
        assert_eq!(ed.autocomplete.suggestions[0].label, "hello");
    }

    #[test]
    fn all_cursor_rows_is_sorted_and_unique() {
        let mut ed = editor("a\nb\nc\nd");
        ed.cursor.set_position(2, 0);
        ed.extra_cursors = vec![cursor_at(0, 0), cursor_at(2, 1), cursor_at(3, 0)];
        assert_eq!(ed.all_cursor_rows(), vec![0, 2, 3]);
    }

    #[test]
    fn selected_line_rows_uses_selection_or_cursor_rows() {
        let mut ed = editor("a\nb\nc\nd");
        ed.cursor.set_position(3, 0);
        ed.cursor.start_selection();
        ed.cursor.set_position(1, 0);
        assert_eq!(ed.selected_line_rows(), vec![1, 2, 3]);

        ed.cursor.clear_selection();
        ed.extra_cursors = vec![cursor_at(0, 0)];
        assert_eq!(ed.selected_line_rows(), vec![0, 1]);
    }

    #[test]
    fn comment_prefix_depends_on_extension() {
        let mut ed = Editor::new();
        assert_eq!(ed.comment_prefix(), "// ", "no path defaults to //");
        let cases = [
            ("main.rs", "// "),
            ("app.ts", "// "),
            ("script.py", "# "),
            ("run.sh", "# "),
            ("Cargo.toml", "# "),
            ("config.yml", "# "),
            ("query.sql", "-- "),
            ("init.lua", "-- "),
            ("Makefile", "// "),
        ];
        for (name, expected) in cases {
            ed.current_path = Some(PathBuf::from(name));
            assert_eq!(ed.comment_prefix(), expected, "for {name}");
        }
    }
}
