use std::path::PathBuf;

use super::WritingUnicorns;
use super::workspace_search::{find_definition_in_buffer, search_workspace_for_symbol};

impl WritingUnicorns {
    pub(crate) fn ensure_lsp_for_file(&mut self, path: &std::path::Path) {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if let Some(workspace) = self.workspace_path.clone() {
                // Prefer the plugin manager (covers installed FFI modules), then builtins.
                if let Some((cmd, args)) = self.plugin_manager.lsp_server_for_ext(ext) {
                    self.lsp
                        .ensure_started_with_cmd(ext, &cmd, &args, &workspace);
                } else {
                    self.lsp.ensure_started(ext, &workspace);
                }
            }
        }
    }

    /// Notify the LSP server that the file content changed.
    pub fn notify_lsp_change(&mut self, path: &std::path::Path, content: &str, version: i32) {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if let Some(client) = self.lsp.get_mut(ext) {
                let uri = format!("file://{}", path.display());
                client.did_change(&uri, version, content);
            }
        }
    }

    /// Request hover information from the LSP server.
    /// Returns the request id, or `None` if no LSP is connected for this file.
    pub fn lsp_hover(&mut self, path: &std::path::Path, line: u32, col: u32) -> Option<u64> {
        let ext = path.extension()?.to_str()?;
        let client = self.lsp.get_mut(ext)?;
        if !client.is_connected {
            return None;
        }
        let uri = format!("file://{}", path.display());
        Some(client.request_hover(&uri, line, col))
    }

    /// Navigate to the definition of `word` via LSP (async), then file-path lookup, then workspace search.
    pub fn handle_go_to_definition(&mut self, word: &str) {
        // Strategy 0: try LSP definition (async — will navigate when the response arrives).
        let mut lsp_sent = false;
        if let Some(path) = self.editor.current_path.clone() {
            let (row, col) = self.editor.cursor.position();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if let Some(client) = self.lsp.get_mut(ext) {
                    if client.is_connected {
                        let uri = format!("file://{}", path.display());
                        self.pending_definition_id =
                            Some(client.request_definition(&uri, row as u32, col as u32));
                        lsp_sent = true;
                    }
                }
            }
        }
        // Only fall back to regex when no LSP server is connected for this file type.
        if !lsp_sent {
            self.handle_go_to_definition_regex(word);
        }
    }

    /// Regex/workspace-search-based go-to-definition (synchronous fallback).
    pub fn handle_go_to_definition_regex(&mut self, word: &str) {
        // Strategy 0: search current file first (fastest, most likely match).
        let current_content = self.editor.buffer.to_string();
        if let Some((_, line)) = find_definition_in_buffer(&current_content, word) {
            if let Some(current_path) = self.editor.current_path.clone() {
                let (row, col) = self.editor.cursor.position();
                self.nav_history.push(current_path, row, col);
            }
            let max = self.editor.buffer.num_lines().saturating_sub(1);
            self.editor.cursor.set_position(line.min(max), 0);
            self.editor.scroll_to_cursor = true;
            return;
        }

        // Strategy 1: looks like a file path — try resolving it.
        let base_dirs: Vec<PathBuf> = [
            self.editor
                .current_path
                .as_ref()
                .and_then(|p| p.parent().map(|p| p.to_path_buf())),
            self.workspace_path.clone(),
        ]
        .into_iter()
        .flatten()
        .collect();

        let extensions = [
            "", ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".toml",
        ];

        for base in &base_dirs {
            for ext in &extensions {
                let candidate = base.join(format!("{}{}", word, ext));
                if candidate.is_file() {
                    self.open_file(candidate);
                    return;
                }
            }
        }

        // Strategy 2: search workspace for a definition.
        // Build patterns (most specific first to avoid false positives from the bare fallback).
        let patterns: Vec<String> = vec![
            format!("fn {}(", word),
            format!("fn {} (", word),
            format!("pub fn {}(", word),
            format!("pub fn {} (", word),
            format!("pub async fn {}(", word),
            format!("async fn {}(", word),
            format!("fn {}(&self", word),
            format!("fn {}(&mut self", word),
            format!("struct {}", word),
            format!("enum {}", word),
            format!("trait {}", word),
            format!("class {}", word),
            format!("interface {}", word),
            format!("type {} =", word),
            format!("const {}", word),
            format!("let {} =", word),
            format!("def {}(", word),
            format!("function {}(", word),
            format!("export function {}(", word),
            format!("export class {}", word),
            format!("fn {}", word), // fallback
        ];
        if let Some(ws) = self.workspace_path.clone() {
            // Prefer same directory as the current file for faster, more relevant results.
            let current_dir = self
                .editor
                .current_path
                .as_ref()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()));
            if let Some(dir) = current_dir {
                if let Some((path, line)) = search_workspace_for_symbol(&dir, &patterns, 200, 3) {
                    self.push_nav_and_goto(path, line);
                    return;
                }
            }
            // Fall back to full workspace search with higher limits.
            if let Some((path, line)) = search_workspace_for_symbol(&ws, &patterns, 2000, 10) {
                self.push_nav_and_goto(path, line);
            }
        }
    }

    /// Send a Find-All-References LSP request from the current cursor position.
    pub fn request_find_references(&mut self) {
        if let Some(path) = self.editor.current_path.clone() {
            let (row, col) = self.editor.cursor.position();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if let Some(client) = self.lsp.get_mut(ext) {
                    if client.is_connected {
                        let uri = format!("file://{}", path.display());
                        self.pending_references_id =
                            Some(client.request_references(&uri, row as u32, col as u32));
                    }
                }
            }
        }
    }

    /// Open the rename dialog with the current word under cursor.
    pub fn start_rename(&mut self) {
        if let Some(word) = self.editor.current_word_full_pub() {
            self.rename_new_name = word;
        } else {
            self.rename_new_name = String::new();
        }
        self.rename_dialog_open = true;
    }

    /// Apply rename edits from LSP to files on disk.
    #[allow(clippy::type_complexity)]
    pub fn apply_rename_edits(&mut self, edits: Vec<(PathBuf, Vec<(u32, u32, u32, String)>)>) {
        for (path, file_edits) in edits {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
                // Sort edits in reverse order so positions stay valid
                let mut sorted_edits = file_edits.clone();
                sorted_edits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
                for (line_num, start_col, end_col, new_text) in sorted_edits {
                    if let Some(line) = lines.get_mut(line_num as usize) {
                        let chars: Vec<char> = line.chars().collect();
                        let start = (start_col as usize).min(chars.len());
                        let end = (end_col as usize).min(chars.len());
                        let mut new_line: String = chars[..start].iter().collect();
                        new_line.push_str(&new_text);
                        new_line.push_str(&chars[end..].iter().collect::<String>());
                        *line = new_line;
                    }
                }
                let new_content = lines.join("\n");
                let _ = std::fs::write(&path, new_content);
                // Reload if it's the current file
                if self.editor.current_path.as_deref() == Some(&path) {
                    if let Ok(c) = std::fs::read_to_string(&path) {
                        let p = path.clone();
                        self.editor.set_content(c, Some(p));
                    }
                }
            }
        }
    }

    /// Request code actions at the cursor.
    pub fn request_code_actions_at_cursor(&mut self) {
        if let Some(path) = self.editor.current_path.clone() {
            let (row, col) = self.editor.cursor.position();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if let Some(client) = self.lsp.get_mut(ext) {
                    if client.is_connected {
                        let uri = format!("file://{}", path.display());
                        let diag_messages: Vec<String> = self
                            .editor
                            .diagnostics
                            .iter()
                            .filter(|d| d.line as usize == row)
                            .map(|d| d.message.clone())
                            .collect();
                        self.pending_code_actions_id = Some(client.request_code_actions(
                            &uri,
                            row as u32,
                            col as u32,
                            &diag_messages,
                        ));
                        self.code_actions_last_request = Some(std::time::Instant::now());
                    }
                }
            }
        }
    }
}
