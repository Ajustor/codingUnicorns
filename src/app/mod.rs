use std::path::PathBuf;

use crate::config::Config;
use crate::dap::manager::DapManager;
use crate::editor::Editor;
use crate::filetree::FileTree;
use crate::git::GitStatus;
use crate::lsp::{LspClient, LspManager};
use crate::nav_history::NavigationHistory;
use crate::plugin::builtin::word_count::WordCountPlugin;
use crate::plugin::manager::PluginManager;
use crate::plugin::PluginContext;
use crate::runner::RunManager;
use crate::tabs::TabManager;
use crate::terminal::Terminal;
use crate::ui::debugger::DebuggerPanel;
use crate::ui::layout::SidebarTab;
use crate::ui::palette::CommandPalette;
use crate::ui::run_panel::RunPanel;
use crate::ui::search::WorkspaceSearch;
use crate::ui::settings::SettingsPanel;
use crate::ui::shortcuts::ShortcutsHelp;
use crate::ui::statusbar::StatusBar;

mod debug_ops;
pub mod file_ops;
mod lsp_ops;
mod navigation;
mod workspace_search;

pub struct WritingUnicorns {
    pub config: Config,
    pub tab_manager: TabManager,
    pub editor: Editor,
    pub file_tree: FileTree,
    pub git_status: GitStatus,
    pub git_panel: crate::ui::git_panel::GitPanel,
    pub lsp: LspManager,
    pub pending_hover_id: Option<u64>,
    pub lsp_hover_result: Option<String>,
    /// Pending LSP go-to-definition request id.
    pub pending_definition_id: Option<u64>,
    /// Pending LSP completion request id.
    pub pending_completion_id: Option<u64>,
    /// Last editor content_version sent to the LSP server (to detect changes).
    pub last_lsp_content_version: i32,
    pub terminals: Vec<Terminal>,
    pub active_terminal: usize,
    pub status_bar: StatusBar,
    pub command_palette: CommandPalette,
    pub shortcuts_help: ShortcutsHelp,
    pub settings_panel: SettingsPanel,
    pub sidebar_tab: SidebarTab,
    pub show_terminal: bool,
    pub show_sidebar: bool,
    pub sidebar_width: f32,
    pub terminal_height: f32,
    pub workspace_search: WorkspaceSearch,
    pub workspace_path: Option<PathBuf>,
    pub runner: RunManager,
    pub run_panel: RunPanel,
    pub folder_pending: Option<std::sync::mpsc::Receiver<std::path::PathBuf>>,
    pub file_pending: Option<std::sync::mpsc::Receiver<std::path::PathBuf>>,
    pub plugin_manager: PluginManager,
    pub plugin_status: Option<String>,
    pub extension_registry: crate::extension::registry::ExtensionRegistry,
    pub extensions_panel: crate::extension::ui::ExtensionsPanel,
    /// Whether the "unsaved files" quit dialog is showing.
    pub show_close_warning: bool,
    /// Set to true after user confirms quitting — lets the next close go through.
    confirmed_close: bool,
    /// Tab id pending Ctrl+W close with unsaved warning.
    pub close_tab_id_pending: Option<usize>,
    /// Document symbols for the outline panel.
    pub outline_symbols: Vec<crate::lsp::client::DocumentSymbol>,
    /// Pending LSP documentSymbol request id.
    pub pending_symbols_id: Option<u64>,
    /// Content version at time of last symbol request.
    pub outline_last_version: i32,
    /// Time of last symbol request.
    pub outline_last_request: Option<std::time::Instant>,
    /// Find-all-references results: (path, line, preview).
    pub references_result: Vec<(std::path::PathBuf, u32, String)>,
    /// Pending LSP references request id.
    pub pending_references_id: Option<u64>,
    /// Whether to show the references panel.
    pub show_references: bool,
    /// Rename dialog state.
    pub rename_dialog_open: bool,
    pub rename_new_name: String,
    pub rename_pending_id: Option<u64>,
    /// Code actions.
    pub code_actions: Vec<crate::lsp::client::CodeAction>,
    pub pending_code_actions_id: Option<u64>,
    pub show_code_actions_menu: bool,
    pub code_actions_pos: egui::Pos2,
    pub code_actions_last_request: Option<std::time::Instant>,
    /// Pending signature help request id.
    pub pending_signature_id: Option<u64>,
    /// Pending LSP formatting request id.
    pub pending_format_id: Option<u64>,
    // ── Auto-save ────────────────────────────────────────────────────────────
    /// Last content_version seen — used to detect edits for auto-save.
    pub last_edit_version_seen: i32,
    /// When the last edit was made — drives the 2-second auto-save timer.
    pub last_edit_instant: Option<std::time::Instant>,
    // ── DAP Debugger ─────────────────────────────────────────────────────────
    pub dap: DapManager,
    pub debugger_panel: DebuggerPanel,
    // ── Navigation History ────────────────────────────────────────────────────
    pub nav_history: NavigationHistory,
    // ── Merge tool ───────────────────────────────────────────────────────────
    pub merge_view: Option<crate::ui::merge_panel::MergeView>,
    // ── Split Editor ─────────────────────────────────────────────────────────
    /// Second editor pane (None = no split).
    pub editor2: Option<Editor>,
    pub tab_manager2: Option<TabManager>,
    /// Which pane is active: 0 = left/main, 1 = right/split.
    pub active_pane: usize,
    /// Split ratio (0.0 - 1.0), default 0.5.
    pub split_ratio: f32,
    // ── Image viewer ─────────────────────────────────────────────────────────
    /// Raw image data waiting to be turned into a texture during the next frame.
    pub pending_image: Option<ImageData>,
    /// Cached egui texture + original pixel size for the currently-displayed image.
    pub image_texture: Option<(egui::TextureHandle, egui::Vec2)>,
}

/// Raw RGBA pixel data for an image file opened in the editor.
pub struct ImageData {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl WritingUnicorns {
    pub fn new(_cc: &eframe::CreationContext<'_>, initial_path: Option<PathBuf>) -> Self {
        let config = Config::load();
        let mut plugin_manager = PluginManager::new();
        plugin_manager.register(Box::new(WordCountPlugin::new()));
        let mut extension_registry = crate::extension::registry::ExtensionRegistry::new();
        extension_registry.load_installed();
        // Load installed FFI language modules into the plugin manager.
        for ext in &extension_registry.installed {
            if let Some(lib_path) = &ext.lib_path {
                let lsp_server = ext.manifest.capabilities.lsp_server.clone();
                let lsp_args = ext.manifest.capabilities.lsp_args.clone();
                match crate::extension::ffi_plugin::FfiLangPlugin::load(
                    lib_path, lsp_server, lsp_args,
                ) {
                    Ok(plugin) => plugin_manager.register(Box::new(plugin)),
                    Err(e) => eprintln!(
                        "Failed to load extension {}: {e}",
                        ext.manifest.extension.id
                    ),
                }
            }
        }
        let initial_terminal_height = config.terminal_height;
        let mut app = Self {
            config,
            tab_manager: TabManager::new(),
            editor: Editor::new(),
            file_tree: FileTree::new(),
            git_status: GitStatus::new(),
            git_panel: crate::ui::git_panel::GitPanel::new(),
            lsp: LspManager::new(),
            pending_hover_id: None,
            lsp_hover_result: None,
            pending_definition_id: None,
            pending_completion_id: None,
            last_lsp_content_version: 0,
            terminals: vec![Terminal::new()],
            active_terminal: 0,
            status_bar: StatusBar::new(),
            command_palette: CommandPalette::new(),
            shortcuts_help: ShortcutsHelp::new(),
            settings_panel: SettingsPanel::new(),
            sidebar_tab: SidebarTab::default(),
            show_terminal: true,
            show_sidebar: true,
            sidebar_width: 220.0,
            terminal_height: initial_terminal_height,
            workspace_search: WorkspaceSearch::new(),
            workspace_path: None,
            runner: RunManager::new(),
            run_panel: RunPanel::new(),
            folder_pending: None,
            file_pending: None,
            plugin_manager,
            plugin_status: None,
            extension_registry,
            extensions_panel: crate::extension::ui::ExtensionsPanel::new(),
            show_close_warning: false,
            confirmed_close: false,
            close_tab_id_pending: None,
            outline_symbols: vec![],
            pending_symbols_id: None,
            outline_last_version: -1,
            outline_last_request: None,
            references_result: vec![],
            pending_references_id: None,
            show_references: false,
            rename_dialog_open: false,
            rename_new_name: String::new(),
            rename_pending_id: None,
            code_actions: vec![],
            pending_code_actions_id: None,
            show_code_actions_menu: false,
            code_actions_pos: egui::Pos2::ZERO,
            code_actions_last_request: None,
            pending_signature_id: None,
            pending_format_id: None,
            last_edit_version_seen: 0,
            last_edit_instant: None,
            dap: DapManager::new(),
            debugger_panel: DebuggerPanel::new(),
            nav_history: NavigationHistory::new(),
            merge_view: None,
            editor2: None,
            tab_manager2: None,
            active_pane: 0,
            split_ratio: 0.5,
            pending_image: None,
            image_texture: None,
        };

        if let Some(path) = initial_path {
            // CLI argument takes priority — open directly, skip saved state.
            if path.is_dir() {
                app.open_folder(path);
            } else if path.is_file() {
                app.open_file(path);
            }
        } else {
            // No CLI arg: restore last workspace and last file from config.
            if let Some(ws_str) = app.config.last_workspace.clone() {
                let ws_path = PathBuf::from(&ws_str);
                if ws_path.is_dir() {
                    app.workspace_path = Some(ws_path.clone());
                    app.file_tree.load(ws_path.clone());
                    app.git_status.load(ws_path.clone());
                    app.runner.load_for_workspace(&ws_path);
                }
            }
            if let Some(file_str) = app.config.last_file.clone() {
                let file_path = PathBuf::from(&file_str);
                if file_path.is_file() {
                    // Read directly to avoid a redundant config save on startup.
                    if let Ok(content) = std::fs::read_to_string(&file_path) {
                        app.tab_manager.open(file_path.clone(), content.clone());
                        app.editor.set_content(content, Some(file_path));
                    }
                }
            }
        }

        app
    }
}

impl eframe::App for WritingUnicorns {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle window close request — warn about unsaved files
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        let any_unsaved = self.tab_manager.tabs.iter().any(|t| t.is_modified);
        if close_requested && any_unsaved && !self.confirmed_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.show_close_warning = true;
        }

        // App-quit unsaved-changes dialog
        if self.show_close_warning {
            egui::Window::new("Unsaved Changes")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("You have unsaved files. What would you like to do?");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Save All & Quit").clicked() {
                            let _ = self.editor.save();
                            self.confirmed_close = true;
                            self.show_close_warning = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("Quit Without Saving").clicked() {
                            self.confirmed_close = true;
                            self.show_close_warning = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_close_warning = false;
                        }
                    });
                });
        }

        // Ctrl+W close-tab unsaved-changes dialog
        if let Some(pending_id) = self.close_tab_id_pending {
            egui::Window::new("Close Tab")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    let tab_title = self
                        .tab_manager
                        .tabs
                        .iter()
                        .find(|t| t.id == pending_id)
                        .map(|t| t.title.clone())
                        .unwrap_or_default();
                    ui.label(format!("\"{}\" has unsaved changes.", tab_title));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Save & Close").clicked() {
                            let _ = self.editor.save();
                            self.tab_manager.close(pending_id);
                            self.close_tab_id_pending = None;
                            self.load_active_tab();
                        }
                        if ui.button("Discard & Close").clicked() {
                            self.tab_manager.close(pending_id);
                            self.close_tab_id_pending = None;
                            self.load_active_tab();
                        }
                        if ui.button("Cancel").clicked() {
                            self.close_tab_id_pending = None;
                        }
                    });
                });
        }

        // All global shortcuts in one ctx.input() call to avoid double-read
        let (
            want_open_folder,
            want_open_file,
            want_new,
            want_palette,
            want_palette_commands,
            want_terminal,
            want_sidebar,
            want_help,
            want_settings,
            want_search,
            want_close_tab,
            want_run,
            want_debug_f5,
            want_breakpoint_f9,
            want_step_over_f10,
            want_step_in_f11,
            want_step_out_shift_f11,
            want_goto_def,
            want_nav_back,
            want_nav_forward,
            want_split,
        ) = ctx.input(|i| {
            (
                self.config.keybindings.open_folder.matches(i),
                self.config.keybindings.open_file.matches(i),
                self.config.keybindings.new_file.matches(i),
                self.config.keybindings.command_palette.matches(i),
                i.key_pressed(egui::Key::P) && i.modifiers.ctrl && i.modifiers.shift,
                self.config.keybindings.toggle_terminal.matches(i),
                self.config.keybindings.toggle_sidebar.matches(i),
                self.config.keybindings.shortcuts_help.matches(i),
                self.config.keybindings.settings.matches(i),
                self.config.keybindings.format_document.matches(i),
                self.config.keybindings.close_tab.matches(i),
                // F5 without shift = run (existing) or debug continue
                self.config.keybindings.debug_start.matches(i) && !i.modifiers.shift,
                // F5 = debug start/continue
                self.config.keybindings.debug_start.matches(i),
                // F9 = toggle breakpoint
                self.config.keybindings.debug_toggle_breakpoint.matches(i),
                // F10 = step over
                self.config.keybindings.debug_step_over.matches(i),
                // F11 = step in
                self.config.keybindings.debug_step_into.matches(i),
                // Shift+F11 = step out
                self.config.keybindings.debug_step_out.matches(i),
                // F12 = go to definition
                self.config.keybindings.goto_definition.matches(i),
                // Alt+Left = navigate back
                self.config.keybindings.navigate_back.matches(i),
                // Alt+Right = navigate forward
                self.config.keybindings.navigate_forward.matches(i),
                // Ctrl+\ = toggle split editor
                self.config.keybindings.toggle_split.matches(i),
            )
        });

        if want_open_folder {
            self.folder_pending = Some(self.trigger_open_folder());
        }
        if want_open_file {
            self.file_pending = Some(self.trigger_open_file());
        }
        if want_new {
            self.open_new_file();
        }
        if want_palette_commands {
            self.command_palette.toggle_commands();
        } else if want_palette {
            self.command_palette.toggle();
        }
        if want_terminal {
            self.show_terminal = !self.show_terminal;
        }
        if want_sidebar {
            self.show_sidebar = !self.show_sidebar;
        }
        if want_help {
            self.shortcuts_help.toggle();
        }
        if want_settings {
            self.settings_panel.toggle();
        }
        if want_search {
            self.show_sidebar = true;
            self.sidebar_tab = SidebarTab::Search;
        }
        if want_run {
            self.run_active_config();
        }
        // ── DAP keybindings ──────────────────────────────────────────────────
        if want_breakpoint_f9 {
            self.toggle_breakpoint_at_cursor();
        }
        if want_debug_f5 {
            if self.dap.is_paused() {
                if let Some(tid) = self.dap.paused_thread_id() {
                    if let Some(sess) = &mut self.dap.session {
                        sess.continue_execution(tid);
                    }
                }
            } else if !self.dap.is_active() {
                self.start_debug_session();
            }
        }
        if want_step_over_f10 {
            if let Some(tid) = self.dap.paused_thread_id() {
                if let Some(sess) = &mut self.dap.session {
                    sess.next_step(tid);
                }
            }
        }
        if want_step_in_f11 {
            if let Some(tid) = self.dap.paused_thread_id() {
                if let Some(sess) = &mut self.dap.session {
                    sess.step_in(tid);
                }
            }
        }
        if want_step_out_shift_f11 {
            if let Some(tid) = self.dap.paused_thread_id() {
                if let Some(sess) = &mut self.dap.session {
                    sess.step_out(tid);
                }
            }
        }
        // F12 = go to definition
        if want_goto_def {
            if let Some(word) = self.editor.current_word_full_pub() {
                self.handle_go_to_definition(&word);
            }
        }
        // Alt+Left = navigate back
        if want_nav_back {
            if let Some(entry) = self.nav_history.go_back() {
                self.open_file_at_line(entry.path, entry.row);
            }
        }
        // Alt+Right = navigate forward
        if want_nav_forward {
            if let Some(entry) = self.nav_history.go_forward() {
                self.open_file_at_line(entry.path, entry.row);
            }
        }
        // Ctrl+\ = toggle split editor
        if want_split {
            self.toggle_split();
        }
        // Poll the DAP session every frame.
        self.dap.poll();
        if want_close_tab && self.close_tab_id_pending.is_none() {
            if let Some(id) = self.tab_manager.active_tab {
                let is_modified = self
                    .tab_manager
                    .tabs
                    .iter()
                    .find(|t| t.id == id)
                    .map(|t| t.is_modified)
                    .unwrap_or(false);
                if is_modified {
                    self.close_tab_id_pending = Some(id);
                } else {
                    self.tab_manager.close(id);
                    self.load_active_tab();
                }
            }
        }

        // Poll all LSP clients for incoming messages (also drives auto-restart).
        let (lsp_responses, reconnected_exts) = self.lsp.poll_all();
        // Re-open the current file on any reconnected LSP server so it receives diagnostics.
        if !reconnected_exts.is_empty() {
            if let Some(ref path) = self.editor.current_path.clone() {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_string();
                if reconnected_exts.contains(&ext) {
                    let uri = format!("file://{}", path.display());
                    let content = self.editor.buffer.to_string();
                    let lang_id = ext.as_str();
                    if let Some(client) = self.lsp.get_mut(&ext) {
                        client.did_open(&uri, lang_id, &content);
                    }
                }
            }
        }
        for (_ext, msgs) in lsp_responses {
            for (id, response) in msgs {
                if Some(id) == self.pending_hover_id {
                    self.lsp_hover_result = LspClient::parse_hover(&response);
                    self.pending_hover_id = None;
                } else if Some(id) == self.pending_definition_id {
                    if let Some((path, line)) = LspClient::parse_definition(&response) {
                        if let Some(current_path) = self.editor.current_path.clone() {
                            let (row, col) = self.editor.cursor.position();
                            self.nav_history.push(current_path, row, col);
                        }
                        self.open_file_at_line(path, line as usize);
                        self.pending_definition_id = None;
                    }
                } else if Some(id) == self.pending_completion_id {
                    let items = LspClient::parse_completions(&response);
                    if !items.is_empty() {
                        self.editor
                            .autocomplete
                            .set_lsp_suggestions(items.iter().map(|i| i.label.clone()).collect());
                    }
                    self.pending_completion_id = None;
                } else if Some(id) == self.pending_symbols_id {
                    self.outline_symbols = LspClient::parse_document_symbols(&response);
                    self.pending_symbols_id = None;
                } else if Some(id) == self.pending_references_id {
                    let refs = LspClient::parse_references(&response);
                    self.references_result = refs
                        .into_iter()
                        .map(|(path, line)| {
                            let preview = std::fs::read_to_string(&path)
                                .ok()
                                .and_then(|s| {
                                    s.lines().nth(line as usize).map(|l| l.trim().to_string())
                                })
                                .unwrap_or_default();
                            (path, line, preview)
                        })
                        .collect();
                    self.show_references = !self.references_result.is_empty();
                    self.pending_references_id = None;
                } else if Some(id) == self.rename_pending_id {
                    let edits = LspClient::apply_rename(&response);
                    if !edits.is_empty() {
                        self.apply_rename_edits(edits);
                    }
                    self.rename_pending_id = None;
                } else if Some(id) == self.pending_code_actions_id {
                    self.code_actions = LspClient::parse_code_actions(&response);
                    self.pending_code_actions_id = None;
                    if !self.code_actions.is_empty() {
                        self.show_code_actions_menu = true;
                    }
                } else if Some(id) == self.pending_signature_id {
                    self.editor.signature_help_text = LspClient::parse_signature_help(&response);
                    self.pending_signature_id = None;
                    self.editor.signature_help_request_pending = false;
                } else if Some(id) == self.pending_format_id {
                    self.pending_format_id = None;
                    let edits = LspClient::parse_text_edits(&response);
                    if !edits.is_empty() {
                        // Apply edits in reverse order (bottom-to-top) to preserve positions.
                        let mut sorted = edits;
                        sorted.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
                        self.editor.buffer.checkpoint();
                        for (sl, sc, el, ec, new_text) in sorted {
                            let sl = sl as usize;
                            let sc = sc as usize;
                            let el = el as usize;
                            let ec = ec as usize;
                            // Delete range then insert
                            if sl == el {
                                let len = ec.saturating_sub(sc);
                                for _ in 0..len {
                                    self.editor.buffer.delete_char(sl, sc);
                                }
                                for (i, ch) in new_text.chars().enumerate() {
                                    self.editor.buffer.insert_char(sl, sc + i, ch);
                                }
                            } else {
                                // Multi-line edit: replace with new_text
                                let mut row = el;
                                while row > sl {
                                    self.editor.buffer.delete_line(row);
                                    row -= 1;
                                }
                                self.editor.buffer.replace_line(sl, &new_text);
                            }
                        }
                        self.editor.is_modified = true;
                        self.editor.content_version = self.editor.content_version.wrapping_add(1);
                    }
                }
            }
        }

        // Trigger an LSP hover request if the editor signals one is needed.
        if self.editor.hover_lsp_request_pending && self.pending_hover_id.is_none() {
            if let Some(path) = self.editor.current_path.clone() {
                let row = self.editor.hover_row;
                let col = self.editor.hover_col;
                self.pending_hover_id = self.lsp_hover(&path, row, col);
                if self.pending_hover_id.is_none() {
                    // No LSP client for this file — don't retry every frame.
                    self.editor.hover_lsp_request_pending = false;
                }
            }
        }

        // Send didChange to LSP when the buffer was modified.
        if self.editor.content_version != self.last_lsp_content_version {
            if let Some(path) = self.editor.current_path.clone() {
                let content = self.editor.buffer.to_string();
                let version = self.editor.content_version;
                self.notify_lsp_change(&path, &content, version);
                self.last_lsp_content_version = version;
            }
        }

        // Update diagnostics for the current file.
        if let Some(path) = self.editor.current_path.clone() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let uri = format!("file://{}", path.display());
                if let Some(client) = self.lsp.get(ext) {
                    self.editor.diagnostics = client.get_diagnostics(&uri);
                }
            }
        }

        // Handle Ctrl+Space LSP completion request from the editor.
        if self.editor.completion_request_pending {
            self.editor.completion_request_pending = false;
            if let Some(path) = self.editor.current_path.clone() {
                let row = self.editor.completion_trigger_row;
                let col = self.editor.completion_trigger_col;
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if let Some(client) = self.lsp.get_mut(ext) {
                        if client.is_connected {
                            let uri = format!("file://{}", path.display());
                            self.pending_completion_id =
                                Some(client.request_completions(&uri, row as u32, col as u32));
                        }
                    }
                }
            }
        }

        // Trigger document symbol request for outline panel.
        if let Some(path) = self.editor.current_path.clone() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let supported = matches!(ext, "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go");
                if supported && self.pending_symbols_id.is_none() {
                    let version_changed = self.editor.content_version != self.outline_last_version;
                    let time_elapsed = self
                        .outline_last_request
                        .map(|t| t.elapsed() > std::time::Duration::from_secs(2))
                        .unwrap_or(true);
                    if version_changed || time_elapsed {
                        if let Some(client) = self.lsp.get_mut(ext) {
                            if client.is_connected {
                                let uri = format!("file://{}", path.display());
                                self.pending_symbols_id =
                                    Some(client.request_document_symbols(&uri));
                                self.outline_last_version = self.editor.content_version;
                                self.outline_last_request = Some(std::time::Instant::now());
                            }
                        }
                    }
                }
            }
        }

        // Trigger LSP formatting request.
        if self.editor.format_request_pending && self.pending_format_id.is_none() {
            self.editor.format_request_pending = false;
            if let Some(path) = self.editor.current_path.clone() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if let Some(client) = self.lsp.get_mut(ext) {
                        if client.is_connected {
                            let uri = format!("file://{}", path.display());
                            let tab_size = self.editor.detected_indent_size as u32;
                            let insert_spaces = self.editor.detected_indent_spaces;
                            self.pending_format_id =
                                Some(client.request_formatting(&uri, tab_size, insert_spaces));
                        }
                    }
                }
            }
        }

        // Trigger signature help request from editor.
        if self.editor.signature_help_request_pending && self.pending_signature_id.is_none() {
            if let Some(path) = self.editor.current_path.clone() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if let Some(client) = self.lsp.get_mut(ext) {
                        if client.is_connected {
                            let uri = format!("file://{}", path.display());
                            let row = self.editor.signature_help_row;
                            let col = self.editor.signature_help_col;
                            self.pending_signature_id =
                                Some(client.request_signature_help(&uri, row, col));
                            self.editor.signature_help_request_pending = false;
                        }
                    }
                }
            }
        }

        // Auto-trigger code actions when cursor line has diagnostics.
        {
            let (cur_row, _) = self.editor.cursor.position();
            let has_diag = self
                .editor
                .diagnostics
                .iter()
                .any(|d| d.line as usize == cur_row);
            let should_request = has_diag
                && self.pending_code_actions_id.is_none()
                && self
                    .code_actions_last_request
                    .map(|t| t.elapsed() > std::time::Duration::from_secs(1))
                    .unwrap_or(true);
            if should_request {
                self.request_code_actions_at_cursor();
            }
        }

        // Reload blame data when path changes.
        if self.editor.show_blame {
            let current_path = self.editor.current_path.clone();
            if current_path != self.editor.blame_path {
                if let Some(ref path) = current_path {
                    self.editor.blame_data = crate::git::blame_file(path);
                    self.editor.blame_path = current_path.clone();
                }
            }
        }

        // Global shortcuts for new features.
        let (want_find_refs, want_rename, want_code_actions, want_blame) = ctx.input(|i| {
            (
                self.config.keybindings.find_references.matches(i),
                self.config.keybindings.rename_symbol.matches(i),
                self.config.keybindings.code_actions.matches(i),
                self.config.keybindings.toggle_blame.matches(i),
            )
        });
        if want_find_refs {
            self.request_find_references();
        }
        if want_rename {
            self.start_rename();
        }
        if want_code_actions {
            self.show_code_actions_menu = true;
        }
        if want_blame {
            self.editor.show_blame = !self.editor.show_blame;
            if self.editor.show_blame {
                if let Some(ref path) = self.editor.current_path.clone() {
                    self.editor.blame_data = crate::git::blame_file(path);
                    self.editor.blame_path = self.editor.current_path.clone();
                }
            }
        }

        // Rename dialog
        if self.rename_dialog_open {
            let mut confirmed = false;
            let mut cancelled = false;
            egui::Window::new("Rename Symbol")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("New name:");
                    let resp = ui.text_edit_singleline(&mut self.rename_new_name);
                    resp.request_focus();
                    ui.horizontal(|ui| {
                        if ui.button("Rename").clicked()
                            || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                        {
                            confirmed = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancelled = true;
                        }
                    });
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cancelled = true;
                    }
                });
            if confirmed && !self.rename_new_name.is_empty() {
                self.rename_dialog_open = false;
                if let Some(path) = self.editor.current_path.clone() {
                    let (row, col) = self.editor.cursor.position();
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if let Some(client) = self.lsp.get_mut(ext) {
                            if client.is_connected {
                                let uri = format!("file://{}", path.display());
                                let name = self.rename_new_name.clone();
                                self.rename_pending_id = Some(
                                    client.request_rename(&uri, row as u32, col as u32, &name),
                                );
                            }
                        }
                    }
                }
            }
            if cancelled {
                self.rename_dialog_open = false;
            }
        }

        // Code actions menu
        if self.show_code_actions_menu && !self.code_actions.is_empty() {
            let actions = self.code_actions.clone();
            let mut close = false;
            egui::Window::new("Code Actions")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    for action in &actions {
                        if ui.button(&action.title).clicked() {
                            close = true;
                        }
                    }
                    if ui.button("Cancel").clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        close = true;
                    }
                });
            if close {
                self.show_code_actions_menu = false;
            }
        }

        crate::ui::layout::render(self, ctx);

        // Reload LSP + plugins when a module installation just completed.
        if self.extensions_panel.plugins_changed {
            self.extensions_panel.plugins_changed = false;
            self.extension_registry.load_installed();
            // Re-register all installed FFI language plugins.
            for ext in &self.extension_registry.installed {
                if let Some(lib_path) = &ext.lib_path {
                    let lsp_server = ext.manifest.capabilities.lsp_server.clone();
                    let lsp_args = ext.manifest.capabilities.lsp_args.clone();
                    if let Ok(plugin) = crate::extension::ffi_plugin::FfiLangPlugin::load(
                        lib_path, lsp_server, lsp_args,
                    ) {
                        self.plugin_manager.register(Box::new(plugin));
                    }
                }
            }
            // Ensure the LSP for the currently open file is started with the new plugins.
            if let Some(path) = self.editor.current_path.clone() {
                self.ensure_lsp_for_file(&path);
            }
        }

        // Handle Ctrl+click go-to-definition request emitted by the editor.
        if let Some(word) = self.editor.go_to_definition_request.take() {
            self.handle_go_to_definition(&word);
        }

        // Run plugins each frame and collect their status text.
        let buffer_text = self.editor.buffer.to_string();
        let filename = self.editor.current_path.as_ref().and_then(|p| p.to_str());
        let plugin_ctx = PluginContext {
            buffer_text: &buffer_text,
            filename,
            cursor_row: self.editor.cursor.row,
            cursor_col: self.editor.cursor.col,
            is_modified: self.editor.is_modified,
            hovered_word: self.editor.hovered_word(),
        };
        let responses = self.plugin_manager.update_all(&plugin_ctx);
        self.plugin_status = responses
            .into_iter()
            .filter_map(|r| r.status_text)
            .next_back();

        if self.command_palette.is_open() {
            let (opened_file, cmd) =
                self.command_palette
                    .show(ctx, &mut self.file_tree, &mut self.workspace_path);
            if let Some(path) = opened_file {
                self.open_file(path);
            }
            if let Some(cmd) = cmd {
                use crate::ui::palette::PaletteCommand;
                match cmd {
                    PaletteCommand::ToggleTerminal => self.show_terminal = !self.show_terminal,
                    PaletteCommand::ToggleSidebar => self.show_sidebar = !self.show_sidebar,
                    PaletteCommand::GoToLine => self.editor.show_goto_line = true,
                    PaletteCommand::SaveFile => {
                        let _ = self.editor.save();
                    }
                    PaletteCommand::NewFile => self.open_new_file(),
                    PaletteCommand::OpenFolder => {
                        self.folder_pending = Some(self.trigger_open_folder());
                    }
                    PaletteCommand::OpenSettings => {
                        self.tab_manager.open_settings();
                        self.settings_panel.open = true;
                    }
                    PaletteCommand::Find => self.editor.show_find = true,
                    PaletteCommand::FindReplace => {
                        self.editor.show_find = true;
                        self.editor.show_replace = true;
                    }
                    PaletteCommand::RestartLsp => {
                        self.lsp.restart_all();
                        // Re-start LSP for the current file if an extension provides one
                        if let Some(path) = self.editor.current_path.clone() {
                            self.ensure_lsp_for_file(&path);
                        }
                    }
                }
            }
        }

        self.shortcuts_help.show(ctx, &self.config.keybindings);

        if self.settings_panel.show(ctx, &mut self.config) {
            self.config.save();
        }

        // Auto-save on focus loss when enabled
        if self.config.editor.auto_save && self.editor.is_modified {
            let window_focused = ctx.input(|i| i.focused);
            if !window_focused {
                let _ = self.editor.save();
            }
        }

        // Request continuous repaint while terminal is visible (for live output)
        if self.show_terminal {
            ctx.request_repaint();
        }
    }
}
