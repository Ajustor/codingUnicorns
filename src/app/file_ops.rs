use std::path::PathBuf;

use super::{ImageData, WritingUnicorns};

/// Returns true if the path has an image file extension we can display.
pub(crate) fn is_image_file(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "bmp" | "webp" | "ico")
    )
}

impl WritingUnicorns {
    pub fn open_file(&mut self, path: PathBuf) {
        // Always clear any previous image state when opening a new file.
        self.pending_image = None;
        self.image_texture = None;

        // Handle image files separately — load pixel data now, create texture during render.
        if is_image_file(&path) {
            // Guard against huge images.
            const MAX_DIM: u32 = 8192;
            if let Ok(img) = image::open(&path) {
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                if w <= MAX_DIM && h <= MAX_DIM {
                    self.pending_image = Some(ImageData {
                        pixels: rgba.into_raw(),
                        width: w,
                        height: h,
                    });
                }
            }
            // Open a tab for the image (empty content — we won't edit it).
            self.tab_manager.open(path.clone(), String::new());
            self.editor.set_content(String::new(), Some(path.clone()));
            self.config.last_file = Some(path.to_string_lossy().to_string());
            self.config.save();
            return;
        }

        if let Ok(content) = std::fs::read_to_string(&path) {
            self.tab_manager.open(path.clone(), content.clone());
            self.editor.set_content(content.clone(), Some(path.clone()));
            self.config.last_file = Some(path.to_string_lossy().to_string());
            self.config.save();
            self.ensure_lsp_for_file(&path);
            // Notify LSP server that a file was opened.
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let lang_id = match ext {
                    "rs" => "rust",
                    "ts" | "tsx" => "typescript",
                    "js" | "jsx" => "javascript",
                    "py" => "python",
                    _ => ext,
                };
                let uri = format!("file://{}", path.display());
                if let Some(client) = self.lsp.get_mut(ext) {
                    client.did_open(&uri, lang_id, &content);
                }
            }
            self.last_lsp_content_version = 0;
            self.editor.refresh_line_diff();
        }
    }

    pub fn open_file_at_line(&mut self, path: PathBuf, line: usize) {
        self.open_file(path);
        let max = self.editor.buffer.num_lines().saturating_sub(1);
        self.editor.cursor.set_position(line.min(max), 0);
        self.editor.scroll_to_cursor = true;
    }

    pub fn open_file_in_pane2(&mut self, path: PathBuf) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(ref mut tm2) = self.tab_manager2 {
                tm2.open(path.clone(), content.clone());
            }
            if let Some(ref mut e2) = self.editor2 {
                e2.set_content(content, Some(path));
            }
        }
    }

    pub fn open_folder(&mut self, path: PathBuf) {
        self.workspace_path = Some(path.clone());
        self.file_tree.load(path.clone());
        self.git_status.load(path.clone());
        self.runner.load_for_workspace(&path);
        self.config.last_workspace = Some(path.to_string_lossy().to_string());
        self.config.save();
    }

    pub fn open_new_file(&mut self) {
        self.editor.set_content(String::new(), None);
        self.tab_manager.open_untitled();
    }

    /// Load the currently active tab into the editor, or clear the editor if none.
    pub fn load_active_tab(&mut self) {
        if let Some(id) = self.tab_manager.active_tab {
            if let Some(tab) = self.tab_manager.tabs.iter().find(|t| t.id == id) {
                let path = tab.path.clone();
                // Delegate to open_file so image state is handled correctly.
                self.open_file(path);
                return;
            }
        }
        self.pending_image = None;
        self.image_texture = None;
        self.editor.set_content(String::new(), None);
    }

    pub fn trigger_open_folder(&mut self) -> std::sync::mpsc::Receiver<PathBuf> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                let _ = tx.send(path);
            }
        });
        rx
    }

    pub fn trigger_open_file(&mut self) -> std::sync::mpsc::Receiver<PathBuf> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new().pick_file() {
                let _ = tx.send(path);
            }
        });
        rx
    }

    pub fn run_active_config(&mut self) {
        let workspace = self.workspace_path.clone();
        let current_file = self.editor.current_path.clone();

        if let Some(cmd) = self
            .runner
            .build_command(workspace.as_deref(), current_file.as_deref())
        {
            self.show_terminal = true;
            if self.terminal_height < 150.0 {
                self.terminal_height = 250.0;
            }
            if let Some(terminal) = self.terminals.get_mut(self.active_terminal) {
                terminal.send_input(&cmd);
                terminal.scroll_to_bottom();
            }
            self.runner.is_running = true;
        }
    }
}
