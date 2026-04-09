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
