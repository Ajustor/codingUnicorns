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

    /// Search all source files in the workspace for a definition of `word`.
    pub(super) fn lookup_signature_in_workspace(&self, word: &str) -> Option<String> {
        let workspace = self.workspace_path.as_ref()?;

        let patterns = [
            format!("fn {}(", word),
            format!("fn {} (", word),
            format!("pub fn {}(", word),
            format!("struct {} ", word),
            format!("struct {}{}", word, '{'),
            format!("pub struct {}", word),
            format!("enum {} ", word),
            format!("pub enum {}", word),
            format!("type {} ", word),
        ];

        let mut stack = vec![(workspace.to_path_buf(), 0usize)];
        let mut files_checked = 0;

        while let Some((dir, depth)) = stack.pop() {
            if depth > 8 || files_checked > 1000 {
                break;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if !matches!(name, "target" | ".git" | "node_modules" | ".cargo") {
                        stack.push((path, depth + 1));
                    }
                } else if path.is_file() {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if matches!(ext, "rs" | "ts" | "js" | "py" | "go") {
                        files_checked += 1;
                        let Ok(content) = std::fs::read_to_string(&path) else {
                            continue;
                        };
                        for line in content.lines() {
                            let trimmed = line.trim();
                            for pattern in &patterns {
                                if trimmed.contains(pattern.as_str()) {
                                    let sig = trimmed.trim_end_matches('{').trim_end();
                                    if !sig.is_empty() {
                                        return Some(sig.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }
}
