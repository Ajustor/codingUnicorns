use std::path::PathBuf;

use serde_json::Value;

use super::workspace_search::{find_definition_in_buffer, search_workspace_for_symbol};
use super::CodingUnicorns;

/// How long a request may wait for its answer before it is given up (a
/// server busy indexing can be slow; one that never answers must not block
/// the feature until the app restarts).
const LSP_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// A request waiting for its answer from one language server. Ids are
/// counted per server and restart with it, so a response is matched on the
/// server (its language key) as well as on the id.
#[derive(Debug, Clone)]
pub struct PendingLsp {
    ext: String,
    id: u64,
    sent: std::time::Instant,
}

impl PendingLsp {
    pub fn new(ext: &str, id: u64) -> Self {
        Self {
            ext: ext.to_string(),
            id,
            sent: std::time::Instant::now(),
        }
    }

    /// True when the response `id` from the server of `ext` answers `pending`.
    pub fn answers(pending: &Option<Self>, ext: &str, id: u64) -> bool {
        pending.as_ref().is_some_and(|p| p.ext == ext && p.id == id)
    }

    /// The answer will never come: the server restarted or stopped, or it is
    /// overdue.
    fn is_lost(&self, reconnected: &[String], connected: bool, now: std::time::Instant) -> bool {
        !connected
            || reconnected.contains(&self.ext)
            || now.duration_since(self.sent) > LSP_REQUEST_TIMEOUT
    }
}

impl CodingUnicorns {
    /// Give up the requests whose answer can no longer come, so hover,
    /// completion, outline, formatting… can be asked again.
    pub(super) fn expire_lsp_requests(&mut self, reconnected: &[String]) {
        let now = std::time::Instant::now();
        let lsp = &self.lsp;
        let pending = [
            &mut self.pending_hover_id,
            &mut self.pending_definition_id,
            &mut self.pending_completion_id,
            &mut self.pending_symbols_id,
            &mut self.pending_references_id,
            &mut self.pending_code_actions_id,
            &mut self.pending_signature_id,
            &mut self.pending_format_id,
        ];
        for slot in pending {
            let lost = slot.as_ref().is_some_and(|p| {
                let connected = lsp.get(&p.ext).is_some_and(|c| c.is_connected);
                p.is_lost(reconnected, connected, now)
            });
            if lost {
                *slot = None;
            }
        }
    }

    /// Refresh the Problems panel cache when any server published diagnostics.
    pub(crate) fn sync_problems(&mut self) {
        let lsp = &self.lsp;
        self.problems_panel
            .sync(lsp.diagnostics_revision(), || lsp.workspace_diagnostics());
    }

    /// Open `path` and put the cursor at 0-based `line`/`col` (clamped),
    /// recording the jump in the navigation history.
    pub fn goto_location(&mut self, path: PathBuf, line: usize, col: usize) {
        self.push_nav_and_goto(path, line);
        let (row, _) = self.editor.cursor.position();
        let len = self.editor.buffer.line(row).chars().count();
        self.editor.cursor.set_position(row, col.min(len));
        self.editor.scroll_to_cursor = true;
    }

    /// Called after each palette frame: sends the debounced `#` query to every
    /// server, fetches document symbols for `@`, and navigates to a picked symbol.
    pub(crate) fn drive_palette_symbols(&mut self, ctx: &egui::Context) {
        if !self.command_palette.is_open() {
            self.palette_ws_symbol_ids.clear();
            self.palette_doc_symbols_id = None;
        }
        let now = std::time::Instant::now();
        if let Some(query) = self.command_palette.take_workspace_symbol_query(now) {
            self.palette_ws_symbol_ids = self.lsp.request_workspace_symbols(&query);
            if self.palette_ws_symbol_ids.is_empty() {
                // No server to ask: settle on an empty result instead of "Searching…".
                self.command_palette
                    .receive_workspace_symbols(vec![], false);
            }
        }
        if self.command_palette.workspace_symbol_debounce_pending()
            || self.command_palette.workspace_symbols_pending
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if self.command_palette.take_document_symbols_request() {
            self.request_palette_document_symbols();
        }
        if let Some((path, line, col)) = self.command_palette.picked_location.take() {
            self.goto_location(path, line, col);
        }
    }

    /// Seed `@` mode with the outline's symbols and ask the server for fresh ones.
    fn request_palette_document_symbols(&mut self) {
        let path = self.editor.current_path.clone();
        self.command_palette.document_symbols_path = path.clone();
        self.command_palette.document_symbols = match path {
            Some(_) => self.outline_symbols.clone(),
            None => vec![],
        };
        self.palette_doc_symbols_id = None;
        let Some(path) = path else { return };
        let Some(ext) = crate::language::language_key(&path) else {
            return;
        };
        let ext = ext.as_str();
        if let Some(client) = self.lsp.get_mut(ext) {
            if client.is_connected {
                let uri = crate::lsp::client::path_to_uri(&path);
                let id = client.request_document_symbols(&uri);
                self.palette_doc_symbols_id = Some((ext.to_string(), id));
            }
        }
    }

    /// Route responses to the palette's symbol requests (matched on server +
    /// id, since ids are only unique per server). Returns true when consumed.
    pub(crate) fn handle_palette_lsp_response(
        &mut self,
        ext: &str,
        id: u64,
        response: &Value,
    ) -> bool {
        let matches = |k: &(String, u64)| k.0 == ext && k.1 == id;
        if self.palette_doc_symbols_id.as_ref().is_some_and(matches) {
            self.palette_doc_symbols_id = None;
            self.command_palette.document_symbols =
                crate::lsp::LspClient::parse_document_symbols(response);
            return true;
        }
        if let Some(pos) = self.palette_ws_symbol_ids.iter().position(matches) {
            self.palette_ws_symbol_ids.remove(pos);
            let symbols = crate::lsp::LspClient::parse_workspace_symbols(response);
            let still_pending = !self.palette_ws_symbol_ids.is_empty();
            self.command_palette
                .receive_workspace_symbols(symbols, still_pending);
            return true;
        }
        false
    }

    /// Report finished language server downloads and start the server of
    /// the current file once its files are in.
    pub(crate) fn poll_lsp_downloads(&mut self, ctx: &egui::Context) {
        if !self.lsp_downloads.is_running() {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        let done = self.lsp_downloads.poll();
        for (module, result) in &done {
            match result {
                Ok(()) => self.toast(format!("{module} language server installed")),
                Err(e) => {
                    log::warn!("{module} language server download failed: {e}");
                    self.toast(format!(
                        "Could not download the {module} language server: {e}"
                    ));
                }
            }
        }
        if done.iter().any(|(_, r)| r.is_ok()) {
            if let Some(path) = self.editor.current_path.clone() {
                // Starts the server and sends it the open file.
                self.notify_lsp_opened(&path);
            }
        }
    }

    pub(crate) fn ensure_lsp_for_file(&mut self, path: &std::path::Path) {
        if let Some(ext) = crate::language::language_key(path).as_deref() {
            if let Some(workspace) = self.workspace_path.clone() {
                if self.lsp.get(ext).is_some() {
                    return;
                }
                // A server declared in a module's manifest: its archive is
                // downloaded first when it has one (the server starts once
                // it is in, see `poll_lsp_downloads`).
                let server = crate::lsp::server::ModuleServer::for_language(
                    &self.extension_registry.installed,
                    ext,
                );
                if let Some(server) = server {
                    if server.needs_download() {
                        if self.lsp_downloads.start(&server, None) {
                            self.toast(format!(
                                "Downloading the {} language server (first use)…",
                                server.module
                            ));
                        }
                        return;
                    }
                    let (cmd, args) = server.command_line(&workspace);
                    let init = self.extension_registry.lsp_init_options(ext);
                    self.lsp
                        .ensure_started_with_cmd(ext, &cmd, &args, &workspace, init);
                    return;
                }
                // Servers named by a module's library (`lsp_server_command_ffi`).
                if let Some((cmd, args)) = self.plugin_manager.lsp_server_for_ext(ext) {
                    let init = self.extension_registry.lsp_init_options(ext);
                    self.lsp
                        .ensure_started_with_cmd(ext, &cmd, &args, &workspace, init);
                } else {
                    self.lsp.ensure_started(ext, &workspace);
                }
            }
        }
    }

    /// Notify the LSP server that the file content changed.
    pub fn notify_lsp_change(&mut self, path: &std::path::Path, content: &str, version: i32) {
        if let Some(ext) = crate::language::language_key(path).as_deref() {
            if let Some(client) = self.lsp.get_mut(ext) {
                let uri = crate::lsp::client::path_to_uri(path);
                client.did_change(&uri, version, content);
            }
        }
    }

    /// Request hover information from the LSP server.
    /// Returns the request id, or `None` if no LSP is connected for this file.
    pub fn lsp_hover(&mut self, path: &std::path::Path, line: u32, col: u32) -> Option<PendingLsp> {
        let ext = crate::language::language_key(path)?;
        let ext = ext.as_str();
        let client = self.lsp.get_mut(ext)?;
        if !client.is_connected {
            return None;
        }
        let uri = crate::lsp::client::path_to_uri(path);
        Some(PendingLsp::new(ext, client.request_hover(&uri, line, col)))
    }

    /// Navigate to the definition of `word` via LSP (async), then file-path lookup, then workspace search.
    pub fn handle_go_to_definition(&mut self, word: &str) {
        // Strategy 0: try LSP definition (async — will navigate when the response arrives).
        let mut lsp_sent = false;
        if let Some(path) = self.editor.current_path.clone() {
            let (row, col) = self.editor.cursor.position();
            if let Some(ext) = crate::language::language_key(&path).as_deref() {
                if let Some(client) = self.lsp.get_mut(ext) {
                    if client.is_connected {
                        let uri = crate::lsp::client::path_to_uri(&path);
                        self.pending_definition_id = Some(PendingLsp::new(
                            ext,
                            client.request_definition(&uri, row as u32, col as u32),
                        ));
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
            if let Some(ext) = crate::language::language_key(&path).as_deref() {
                if let Some(client) = self.lsp.get_mut(ext) {
                    if client.is_connected {
                        let uri = crate::lsp::client::path_to_uri(&path);
                        self.pending_references_id = Some(PendingLsp::new(
                            ext,
                            client.request_references(&uri, row as u32, col as u32),
                        ));
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

    /// Apply rename edits from LSP: to the editor's buffer for the file it shows
    /// (the server computed them on that text, unsaved changes included), to
    /// the files on disk for the others.
    #[allow(clippy::type_complexity)]
    pub fn apply_rename_edits(&mut self, edits: Vec<(PathBuf, Vec<(u32, u32, u32, String)>)>) {
        use crate::editor::text_format;
        for (path, file_edits) in edits {
            if self.editor.current_path.as_deref() == Some(&path) {
                // In memory, as one undo step; the user saves it like any edit.
                let ranges = file_edits
                    .into_iter()
                    .map(|(line, sc, ec, text)| (line, sc, line, ec, text))
                    .collect();
                self.editor.apply_text_edits(ranges);
                continue;
            }
            let Ok((raw, lossy)) = text_format::read_text_file(&path) else {
                continue;
            };
            // Rewriting a lossily decoded file would corrupt its non-UTF-8 bytes.
            if lossy {
                log::warn!("rename: skipping non-UTF-8 file {}", path.display());
                continue;
            }
            {
                // Edit LF-only text, then write back the file's own line ending and BOM.
                // `split` (not `lines`) keeps a trailing newline.
                let (content, format) = text_format::normalize(raw);
                let mut lines: Vec<String> = content.split('\n').map(str::to_string).collect();
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
                if let Err(e) = std::fs::write(&path, text_format::encode(&new_content, format)) {
                    self.toast(format!("Rename: could not write {}: {e}", path.display()));
                }
            }
        }
    }

    /// Request code actions at the cursor.
    pub fn request_code_actions_at_cursor(&mut self) {
        if let Some(path) = self.editor.current_path.clone() {
            let (row, col) = self.editor.cursor.position();
            if let Some(ext) = crate::language::language_key(&path).as_deref() {
                if let Some(client) = self.lsp.get_mut(ext) {
                    if client.is_connected {
                        let uri = crate::lsp::client::path_to_uri(&path);
                        let diag_messages: Vec<String> = self
                            .editor
                            .diagnostics
                            .iter()
                            .filter(|d| d.line as usize == row)
                            .map(|d| d.message.clone())
                            .collect();
                        self.pending_code_actions_id = Some(PendingLsp::new(
                            ext,
                            client.request_code_actions(
                                &uri,
                                row as u32,
                                col as u32,
                                &diag_messages,
                            ),
                        ));
                        self.code_actions_last_request = Some(std::time::Instant::now());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PendingLsp;
    use std::time::{Duration, Instant};

    #[test]
    fn a_response_answers_only_its_own_server_and_id() {
        let p = Some(PendingLsp::new("rs", 7));
        assert!(PendingLsp::answers(&p, "rs", 7));
        assert!(
            !PendingLsp::answers(&p, "ts", 7),
            "same id from another server"
        );
        assert!(!PendingLsp::answers(&p, "rs", 8));
        assert!(!PendingLsp::answers(&None, "rs", 7));
    }

    #[test]
    fn a_request_is_lost_on_restart_disconnect_or_timeout() {
        let p = PendingLsp::new("rs", 1);
        let now = Instant::now();
        assert!(!p.is_lost(&[], true, now));
        assert!(
            p.is_lost(&["rs".to_string()], true, now),
            "server restarted"
        );
        assert!(!p.is_lost(&["ts".to_string()], true, now), "another server");
        assert!(p.is_lost(&[], false, now), "server gone");
        assert!(
            p.is_lost(&[], true, now + Duration::from_secs(16)),
            "overdue"
        );
    }
}
