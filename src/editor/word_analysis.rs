use super::Editor;

impl Editor {
    /// Public wrapper for `current_word_full` (used by app.rs).
    pub fn current_word_full_pub(&self) -> Option<String> {
        self.current_word_full()
    }

    /// Returns the full word under the primary cursor (extending left and right from cursor),
    /// or the existing selection text if a selection is active.
    pub(super) fn current_word_full(&self) -> Option<String> {
        if let Some(text) = self.selected_text() {
            if !text.is_empty() && !text.contains('\n') {
                return Some(text);
            }
        }
        let (row, col) = self.cursor.position();
        let line = self.buffer.line(row);
        let chars: Vec<char> = line.chars().collect();
        let col = col.min(chars.len());
        let mut start = col;
        while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
            start -= 1;
        }
        let mut end = col;
        while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
            end += 1;
        }
        if start == end {
            return None;
        }
        Some(chars[start..end].iter().collect())
    }

    /// Returns (word_start_col, word) for the partial word ending at the cursor.
    pub(super) fn current_word_at_cursor(&self) -> (usize, String) {
        let (row, col) = self.cursor.position();
        let line = self.buffer.line(row);
        let chars: Vec<char> = line.chars().collect();
        let col = col.min(chars.len());
        let mut start = col;
        while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
            start -= 1;
        }
        let word: String = chars[start..col].iter().collect();
        (start, word)
    }

    /// Collect all words of length ≥ 2 present in the buffer (for autocomplete suggestions).
    pub(super) fn buffer_words(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        for i in 0..self.buffer.num_lines() {
            let line = self.buffer.line(i);
            let mut word = String::new();
            for ch in line.chars() {
                if ch.is_alphanumeric() || ch == '_' {
                    word.push(ch);
                } else {
                    if word.chars().count() >= 2 {
                        seen.insert(word.clone());
                    }
                    word.clear();
                }
            }
            if word.chars().count() >= 2 {
                seen.insert(word);
            }
        }
        let mut result: Vec<String> = seen.into_iter().collect();
        result.sort();
        result
    }

    /// The word currently under the mouse pointer (used to populate `PluginContext`).
    pub fn hovered_word(&self) -> Option<&str> {
        self.hover_word.as_deref()
    }

    /// Search the current buffer for a definition of `word` and return a short signature string.
    pub(super) fn lookup_signature_in_buffer(&self, word: &str) -> Option<String> {
        let content = self.buffer.to_string();
        for raw_line in content.lines() {
            let trimmed = raw_line.trim();

            // Function definitions
            let fn_needle_paren = format!("fn {}(", word);
            let fn_needle_space = format!("fn {} (", word);
            if (trimmed.contains(&fn_needle_paren) || trimmed.contains(&fn_needle_space))
                && (trimmed.starts_with("fn ")
                    || trimmed.starts_with("pub fn ")
                    || trimmed.starts_with("async fn ")
                    || trimmed.starts_with("pub async fn ")
                    || trimmed.starts_with("pub(crate) fn ")
                    || trimmed.starts_with("unsafe fn ")
                    || trimmed.starts_with("pub unsafe fn "))
            {
                // Strip trailing `{` to keep the signature clean
                let sig = trimmed.trim_end_matches('{').trim_end();
                return Some(sig.to_string());
            }

            // Struct definitions
            if trimmed.starts_with(&format!("struct {} ", word))
                || trimmed.starts_with(&format!("struct {}{}", word, '{'))
                || trimmed.starts_with(&format!("pub struct {} ", word))
                || trimmed.starts_with(&format!("pub struct {}{}", word, '{'))
                || trimmed.starts_with(&format!("pub(crate) struct {} ", word))
            {
                let sig = trimmed.trim_end_matches('{').trim_end();
                return Some(sig.to_string());
            }

            // Enum definitions
            if trimmed.starts_with(&format!("enum {} ", word))
                || trimmed.starts_with(&format!("enum {}{}", word, '{'))
                || trimmed.starts_with(&format!("pub enum {} ", word))
                || trimmed.starts_with(&format!("pub enum {}{}", word, '{'))
            {
                let sig = trimmed.trim_end_matches('{').trim_end();
                return Some(sig.to_string());
            }

            // Type aliases
            if trimmed.starts_with(&format!("type {} ", word))
                || trimmed.starts_with(&format!("pub type {} ", word))
            {
                let sig = trimmed.trim_end_matches(';').trim_end();
                return Some(sig.to_string());
            }

            // Let bindings (typed or inferred)
            if trimmed.starts_with(&format!("let {}: ", word))
                || trimmed.starts_with(&format!("let mut {}: ", word))
                || trimmed.starts_with(&format!("let {} =", word))
                || trimmed.starts_with(&format!("let mut {} =", word))
            {
                // Return just the declaration part (up to `=` or `;`)
                let end = trimmed
                    .find('=')
                    .or_else(|| trimmed.find(';'))
                    .unwrap_or(trimmed.len());
                return Some(trimmed[..end].trim_end().to_string());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn editor(content: &str) -> Editor {
        let mut ed = Editor::new();
        ed.set_content(content.to_string(), None);
        ed
    }

    fn write(root: &Path, rel: &str, content: &[u8]) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    // ── current_word_full ───────────────────────────────────────────────────

    #[test]
    fn current_word_full_extends_both_directions() {
        let mut ed = editor("let foo_bar2 = 1;");
        ed.cursor.set_position(0, 6);
        assert_eq!(ed.current_word_full().as_deref(), Some("foo_bar2"));
        ed.cursor.set_position(0, 4);
        assert_eq!(ed.current_word_full_pub().as_deref(), Some("foo_bar2"));
        // Cursor right after the word still picks it up.
        ed.cursor.set_position(0, 12);
        assert_eq!(ed.current_word_full().as_deref(), Some("foo_bar2"));
    }

    #[test]
    fn current_word_full_returns_none_between_words() {
        let mut ed = editor("a  = b");
        ed.cursor.set_position(0, 2);
        assert_eq!(ed.current_word_full(), None);
        ed.cursor.set_position(0, 99);
        assert_eq!(ed.current_word_full().as_deref(), Some("b"), "col clamped");
    }

    #[test]
    fn current_word_full_prefers_single_line_selection() {
        let mut ed = editor("hello world\nnext");
        ed.cursor.set_position(0, 2);
        ed.cursor.start_selection();
        ed.cursor.set_position(0, 8);
        assert_eq!(ed.current_word_full().as_deref(), Some("llo wo"));

        // Multi-line selection is ignored: falls back to the word at the cursor.
        ed.cursor.set_position(1, 2);
        assert_eq!(ed.current_word_full().as_deref(), Some("next"));

        // Empty selection is ignored too.
        ed.cursor.clear_selection();
        ed.cursor.set_position(0, 1);
        ed.cursor.start_selection();
        assert_eq!(ed.current_word_full().as_deref(), Some("hello"));
    }

    // ── current_word_at_cursor ──────────────────────────────────────────────

    #[test]
    fn current_word_at_cursor_returns_prefix_up_to_cursor() {
        let mut ed = editor("call my_func(x)");
        ed.cursor.set_position(0, 9);
        assert_eq!(ed.current_word_at_cursor(), (5, "my_f".to_string()));
        ed.cursor.set_position(0, 13);
        assert_eq!(ed.current_word_at_cursor(), (13, String::new()));
        ed.cursor.set_position(0, 0);
        assert_eq!(ed.current_word_at_cursor(), (0, String::new()));
        ed.cursor.set_position(0, 100);
        assert_eq!(ed.current_word_at_cursor(), (15, String::new()));
    }

    // ── buffer_words ────────────────────────────────────────────────────────

    #[test]
    fn buffer_words_are_unique_sorted_and_at_least_two_chars() {
        let ed = editor("let x = foo(bar, foo);\nbar_baz é9 a\nzz");
        assert_eq!(
            ed.buffer_words(),
            vec!["bar", "bar_baz", "foo", "let", "zz", "é9"]
        );
        assert!(editor("").buffer_words().is_empty());
    }

    #[test]
    fn hovered_word_reflects_state() {
        let mut ed = editor("x");
        assert_eq!(ed.hovered_word(), None);
        ed.hover_word = Some("thing".to_string());
        assert_eq!(ed.hovered_word(), Some("thing"));
    }

    // ── lookup_signature_in_buffer ──────────────────────────────────────────

    #[test]
    fn finds_function_signatures() {
        let ed = editor(
            "foo(1);\n    pub fn foo(a: u8) -> u8 {\n}\nfn bar (x: i32) {}\nasync fn baz() {\npub(crate) fn qux() {\nunsafe fn raw() {",
        );
        assert_eq!(
            ed.lookup_signature_in_buffer("foo").as_deref(),
            Some("pub fn foo(a: u8) -> u8")
        );
        assert_eq!(
            ed.lookup_signature_in_buffer("bar").as_deref(),
            Some("fn bar (x: i32) {}")
        );
        assert_eq!(
            ed.lookup_signature_in_buffer("baz").as_deref(),
            Some("async fn baz()")
        );
        assert_eq!(
            ed.lookup_signature_in_buffer("qux").as_deref(),
            Some("pub(crate) fn qux()")
        );
        assert_eq!(
            ed.lookup_signature_in_buffer("raw").as_deref(),
            Some("unsafe fn raw()")
        );
    }

    #[test]
    fn function_lookup_requires_exact_name_and_definition() {
        let ed = editor("fn foobar() {}\nlet y = call_fn foo(1);");
        assert_eq!(ed.lookup_signature_in_buffer("foo"), None);
        assert_eq!(ed.lookup_signature_in_buffer("missing"), None);
    }

    #[test]
    fn finds_struct_enum_and_type_definitions() {
        let ed = editor(
            "pub struct Foo {\nstruct Bar{\npub(crate) struct Baz {\nenum Color {\npub enum Shape{\npub type Res = Result<u8>;\ntype Id = u32;",
        );
        assert_eq!(
            ed.lookup_signature_in_buffer("Foo").as_deref(),
            Some("pub struct Foo")
        );
        assert_eq!(
            ed.lookup_signature_in_buffer("Bar").as_deref(),
            Some("struct Bar")
        );
        assert_eq!(
            ed.lookup_signature_in_buffer("Baz").as_deref(),
            Some("pub(crate) struct Baz")
        );
        assert_eq!(
            ed.lookup_signature_in_buffer("Color").as_deref(),
            Some("enum Color")
        );
        assert_eq!(
            ed.lookup_signature_in_buffer("Shape").as_deref(),
            Some("pub enum Shape")
        );
        assert_eq!(
            ed.lookup_signature_in_buffer("Res").as_deref(),
            Some("pub type Res = Result<u8>")
        );
        assert_eq!(
            ed.lookup_signature_in_buffer("Id").as_deref(),
            Some("type Id = u32")
        );
    }

    #[test]
    fn finds_let_bindings_up_to_the_initializer() {
        let ed = editor("let mut count: usize = 0;\n  let x = 5;\nlet total: u8;\nlet mut y = 1;");
        assert_eq!(
            ed.lookup_signature_in_buffer("count").as_deref(),
            Some("let mut count: usize")
        );
        assert_eq!(ed.lookup_signature_in_buffer("x").as_deref(), Some("let x"));
        assert_eq!(
            ed.lookup_signature_in_buffer("total").as_deref(),
            Some("let total: u8")
        );
        assert_eq!(
            ed.lookup_signature_in_buffer("y").as_deref(),
            Some("let mut y")
        );
    }

    #[test]
    fn first_definition_wins() {
        let ed = editor("pub enum Foo {\nstruct Foo {\nfn Foo() {");
        assert_eq!(
            ed.lookup_signature_in_buffer("Foo").as_deref(),
            Some("pub enum Foo")
        );
    }
}
