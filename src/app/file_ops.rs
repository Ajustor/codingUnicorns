use std::path::PathBuf;

use super::{CodingUnicorns, ImageData};

/// Returns true if the path has an image file extension we can display.
pub(crate) fn is_image_file(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "bmp" | "webp" | "ico" | "gif")
    )
}

/// Paths opened as a workspace or file are kept absolute: `cu .` passes a
/// relative one, which would end up as `${workspaceFolder}` / `${file}` in run
/// and debug configurations and as the saved last workspace.
pub(crate) fn absolute_path(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}

/// Why `name` can't be a new file/folder name, if it can't.
fn invalid_file_name(name: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        Some("the name is empty")
    } else if name == "." || name == ".." {
        Some("the name is reserved")
    } else if name.contains(['/', '\\']) {
        Some("the name can't contain a path separator")
    } else {
        None
    }
}

/// Where `path` is after `old` was renamed to `new`: itself, or a file inside
/// a renamed folder. `None` when the rename doesn't concern it.
fn renamed_path(
    path: &std::path::Path,
    old: &std::path::Path,
    new: &std::path::Path,
) -> Option<PathBuf> {
    path.strip_prefix(old).ok().map(|rest| new.join(rest))
}

/// True when `to` is another entry than `from` (not merely `from` spelled with
/// another case on a case-insensitive file system): renaming would replace it.
fn would_replace(from: &std::path::Path, to: &std::path::Path) -> bool {
    to.exists()
        && match (std::fs::canonicalize(from), std::fs::canonicalize(to)) {
            (Ok(a), Ok(b)) => a != b,
            _ => true,
        }
}

impl CodingUnicorns {
    /// Rename `old` (file or folder) to `new_name` in the same folder, never
    /// replacing another entry, then point open tabs and editors at the new path.
    pub fn rename_path(&mut self, old: &std::path::Path, new_name: &str) {
        if let Some(why) = invalid_file_name(new_name) {
            self.toast(format!("Can't rename: {why}"));
            return;
        }
        let Some(parent) = old.parent() else {
            return;
        };
        let new = parent.join(new_name);
        if new == old {
            return;
        }
        if would_replace(old, &new) {
            self.toast(format!("Can't rename: {new_name} already exists"));
            return;
        }
        if let Err(e) = std::fs::rename(old, &new) {
            self.toast(format!("Can't rename: {e}"));
            return;
        }
        let managers = std::iter::once(&mut self.tab_manager).chain(self.tab_manager2.as_mut());
        for tm in managers {
            let moved: Vec<(usize, PathBuf)> = tm
                .tabs
                .iter()
                .filter_map(|t| renamed_path(&t.path, old, &new).map(|p| (t.id, p)))
                .collect();
            for (id, path) in moved {
                tm.retarget(id, path);
            }
        }
        for editor in std::iter::once(&mut self.editor).chain(self.editor2.as_mut()) {
            if let Some(path) = editor
                .current_path
                .as_deref()
                .and_then(|p| renamed_path(p, old, &new))
            {
                // Keep the text (and unsaved changes); only the file it saves to moves.
                editor.set_path(path);
            }
        }
        if let Some(path) = self.editor.current_path.clone() {
            self.record_file_stamp(&path);
        }
    }
    /// Open paths handed over by another launch (see `single_instance`) and bring the
    /// window to the front.
    pub(super) fn open_forwarded_paths(&mut self, ctx: &egui::Context) {
        let Some(server) = &self.single_instance else {
            return;
        };
        let paths: Vec<PathBuf> = server.rx.try_iter().collect();
        if paths.is_empty() {
            return;
        }
        for path in paths {
            if path.is_dir() {
                self.open_folder(path);
            } else if path.is_file() {
                self.open_file(path);
            } else {
                log::warn!("forwarded path does not exist: {}", path.display());
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    pub fn open_file(&mut self, path: PathBuf) {
        self.open_file_with(path, false);
    }

    /// Open `path` in the editor. A modified file being left is parked (kept in
    /// memory) and a parked file comes back as it was; only `reload` re-reads
    /// the file already shown when it has unsaved changes (the user discarding
    /// them for the disk version).
    pub(super) fn open_file_with(&mut self, path: PathBuf, reload: bool) {
        let path = absolute_path(path);
        if !reload && self.editor.is_modified && self.editor.current_path.as_ref() == Some(&path) {
            // Already shown with unsaved changes (its tab clicked again, a
            // go-to-definition in the same file): keep them.
            self.tab_manager.open(path, String::new());
            return;
        }
        // Keep the outgoing file's cursor/scroll so returning to it restores them.
        self.remember_view_state();
        // Always clear any previous image state when opening a new file.
        self.pending_image = None;
        self.image_texture = None;
        self.image_view.reset();

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
                        file_size: std::fs::metadata(&path).ok().map(|m| m.len()),
                        format: path
                            .extension()
                            .map(|e| e.to_string_lossy().to_uppercase())
                            .unwrap_or_default(),
                    });
                }
            }
            // Open a tab for the image (empty content — we won't edit it).
            self.park_current_doc(Some(&path));
            self.tab_manager.open(path.clone(), String::new());
            self.editor.set_content(String::new(), Some(path.clone()));
            self.config.last_file = Some(path.to_string_lossy().to_string());
            self.config.save();
            self.editor.focus_requested = true;
            return;
        }

        if let Some(doc) = self.take_parked_doc(&path) {
            self.park_current_doc(Some(&path));
            self.tab_manager.open(path.clone(), String::new());
            self.editor.unpark(doc, path.clone());
            // Not record_file_stamp: the stamp from the last load/save must stay,
            // so a change made on disk meanwhile still raises the conflict prompt.
            self.config.last_file = Some(path.to_string_lossy().to_string());
            self.config.save();
            self.notify_lsp_opened(&path);
            self.editor.refresh_line_diff();
            self.editor.focus_requested = true;
            return;
        }

        // Park only once the new content is in hand: a file that fails to load
        // leaves the current one shown, untouched.
        if let Ok((content, lossy)) = crate::editor::text_format::read_text_file(&path) {
            self.park_current_doc(Some(&path));
            self.tab_manager.open(path.clone(), content.clone());
            self.editor.set_content(content, Some(path.clone()));
            if lossy {
                self.editor.decoded_lossy = true;
                log::warn!("{} is not valid UTF-8; decoded lossily", path.display());
                self.toast("File is not valid UTF-8: invalid bytes shown as \u{FFFD}");
            }
            if self.editor.large_file {
                self.toast("Large file: language server, git markers, folding and minimap are off");
            }
            self.config.last_file = Some(path.to_string_lossy().to_string());
            self.config.save();
            self.notify_lsp_opened(&path);
            self.editor.refresh_line_diff();
            self.editor.focus_requested = true;
            self.apply_view_state(&path);
            self.record_file_stamp(&path);
        }
    }

    /// Start the language server for `path` and send it the editor's text.
    pub(super) fn notify_lsp_opened(&mut self, path: &std::path::Path) {
        self.last_lsp_content_version = 0;
        // A large file would be sent whole to the server on every keystroke.
        if self.editor.large_file {
            return;
        }
        self.ensure_lsp_for_file(path);
        // Notify LSP server that a file was opened — with the normalized
        // (LF-only) buffer text, so later edits line up with it.
        if let Some(ext) = crate::language::language_key(path).as_deref() {
            let lang_id = self.extension_registry.lsp_language_id(ext);
            let uri = crate::lsp::client::path_to_uri(path);
            if let Some(client) = self.lsp.get_mut(ext) {
                client.did_open(&uri, &lang_id, &self.editor.buffer.to_string());
            }
        }
    }

    /// Park the editor's document if it has unsaved changes and another file
    /// (`next`, or an untitled buffer for `None`) is about to replace it.
    fn park_current_doc(&mut self, next: Option<&std::path::Path>) {
        let Some(current) = self.editor.current_path.clone() else {
            return;
        };
        let has_tab = self.tab_manager.tabs.iter().any(|t| t.path == current);
        if self.editor.is_modified && has_tab && next != Some(current.as_path()) {
            let doc = self.editor.park();
            self.parked_docs.insert(current, doc);
        }
    }

    /// The parked document of `path`, if its tab is still open (a closed tab's
    /// edits were discarded with it).
    fn take_parked_doc(&mut self, path: &std::path::Path) -> Option<crate::editor::ParkedDoc> {
        let doc = self.parked_docs.remove(path)?;
        self.tab_manager
            .tabs
            .iter()
            .any(|t| t.path == path)
            .then_some(doc)
    }

    /// Forget parked documents whose tab was closed.
    pub fn prune_parked_docs(&mut self) {
        let tabs = &self.tab_manager.tabs;
        self.parked_docs
            .retain(|path, _| tabs.iter().any(|t| &t.path == path));
    }

    /// Write every parked document to disk (quit's "Save All"). Stops at the
    /// first file that changed on disk meanwhile or can't be written, and
    /// returns false then: that file stays parked and modified.
    pub fn save_parked_docs(&mut self) -> bool {
        let paths: Vec<PathBuf> = self.parked_docs.keys().cloned().collect();
        paths.iter().all(|path| self.save_parked_doc(path))
    }

    /// Write the parked document of `path`, if any. False (with a toast) when
    /// the file changed on disk meanwhile or can't be written: it stays parked.
    fn save_parked_doc(&mut self, path: &std::path::Path) -> bool {
        let Some(doc) = self.parked_docs.get(path) else {
            return true;
        };
        if self.parked_changed_on_disk(path) {
            self.toast(format!(
                "{} changed on disk: open its tab to resolve it",
                path.display()
            ));
            return false;
        }
        if let Err(e) = std::fs::write(path, doc.encoded()) {
            self.toast(format!("Could not save {}: {e}", path.display()));
            return false;
        }
        self.parked_docs.remove(path);
        self.record_file_stamp(path);
        if let Some(tab) = self.tab_manager.tabs.iter_mut().find(|t| t.path == path) {
            tab.is_modified = false;
        }
        true
    }

    /// Save the file of tab `id`: the editor's when it is the one shown, else
    /// its parked document. False when nothing could be saved.
    pub fn save_tab(&mut self, id: usize) -> bool {
        let Some(path) = self
            .tab_manager
            .tabs
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.path.clone())
        else {
            return false;
        };
        if self.editor.current_path.as_ref() == Some(&path) {
            self.save_editor_guarded(true)
        } else {
            self.save_parked_doc(&path)
        }
    }

    pub fn open_file_at_line(&mut self, path: PathBuf, line: usize) {
        self.open_file(path);
        let max = self.editor.buffer.num_lines().saturating_sub(1);
        self.editor.cursor.set_position(line.min(max), 0);
        self.editor.scroll_to_cursor = true;
    }

    pub fn open_folder(&mut self, path: PathBuf) {
        let path = absolute_path(path);
        // Persist the outgoing workspace's tabs before switching.
        let switching = self.workspace_path.as_ref() != Some(&path);
        if switching {
            self.save_session();
        }
        self.workspace_path = Some(path.clone());
        self.file_tree.show_gitignored = self.config.editor.show_gitignored;
        self.file_tree.load(path.clone());
        self.git_status.load(path.clone());
        self.runner.load_for_workspace(&path);
        let path_str = path.to_string_lossy().to_string();
        self.config.push_recent_workspace(&path_str);
        self.config.last_workspace = Some(path_str);
        self.config.save();
        if switching {
            self.restore_session(&path);
            self.start_file_watcher(&path);
        }
    }

    /// Open a folder picked from the "Open Recent" list. A folder that no
    /// longer exists is dropped from the list instead.
    pub fn open_recent_workspace(&mut self, path: PathBuf) {
        if path.is_dir() {
            self.open_folder(path);
        } else {
            self.config.remove_recent_workspace(&path.to_string_lossy());
            self.config.save();
            self.toast(format!("Folder not found: {}", path.display()));
        }
    }

    /// Clear the "Open Recent" list.
    pub fn clear_recent_workspaces(&mut self) {
        self.config.recent_workspaces.clear();
        self.config.save();
    }

    pub fn open_new_file(&mut self) {
        self.park_current_doc(None);
        self.editor.set_content(String::new(), None);
        self.tab_manager.open_untitled();
    }

    /// Load the currently active tab into the editor, or clear the editor if none.
    pub fn load_active_tab(&mut self) {
        if let Some(id) = self.tab_manager.active_tab {
            if let Some(tab) = self.tab_manager.tabs.iter().find(|t| t.id == id) {
                if tab.page.is_some() {
                    return; // drawn by its module, no file to load
                }
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

    /// Ask where to save the untitled buffer (once; the answer arrives in
    /// `save_as_pending`, see [`Self::finish_save_as`]).
    pub fn request_save_as(&mut self) {
        if self.save_as_pending.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let dir = self.workspace_path.clone();
        std::thread::spawn(move || {
            let mut dialog = rfd::FileDialog::new();
            if let Some(dir) = dir {
                dialog = dialog.set_directory(dir);
            }
            if let Some(path) = dialog.save_file() {
                let _ = tx.send(path);
            }
        });
        self.save_as_pending = Some(rx);
    }

    /// Save the untitled buffer as `path`: its tab takes the file's name and
    /// the file is then opened like any other (language server, stamps).
    pub fn finish_save_as(&mut self, path: PathBuf) {
        if self.editor.current_path.is_some() {
            return; // another file is shown by now
        }
        let path = absolute_path(path);
        self.editor.set_path(path.clone());
        if !self.save_editor_guarded(true) {
            self.editor.current_path = None;
            self.toast(format!("Could not save {}", path.display()));
            return;
        }
        if let Some(id) = self.tab_manager.active_tab {
            self.tab_manager.retarget(id, path.clone());
        }
        self.toast("Saved");
        self.open_file(path);
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
        // The command is typed into the active terminal: quote for its shell.
        let shell = self
            .terminals
            .get(self.active_terminal)
            .map(|t| crate::runner::ShellKind::from_shell_name(&t.shell_name))
            .unwrap_or(crate::runner::ShellKind::Posix);

        if let Some(cmd) =
            self.runner
                .build_command(workspace.as_deref(), current_file.as_deref(), shell)
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

#[cfg(test)]
mod tests {
    use super::absolute_path;
    use super::{invalid_file_name, renamed_path, would_replace};
    use std::path::{Path, PathBuf};

    #[test]
    fn new_names_must_be_a_single_plain_name() {
        assert_eq!(invalid_file_name("main.rs"), None);
        assert!(invalid_file_name("  ").is_some());
        assert!(invalid_file_name("..").is_some());
        assert!(invalid_file_name("a/b").is_some());
        assert!(invalid_file_name("a\\b").is_some());
    }

    #[test]
    fn renamed_path_follows_the_file_and_the_files_of_a_folder() {
        let (old, new) = (Path::new("/w/src"), Path::new("/w/lib"));
        assert_eq!(renamed_path(old, old, new), Some(PathBuf::from("/w/lib")));
        assert_eq!(
            renamed_path(Path::new("/w/src/a/b.rs"), old, new),
            Some(PathBuf::from("/w/lib/a/b.rs"))
        );
        assert_eq!(renamed_path(Path::new("/w/srcx/b.rs"), old, new), None);
        assert_eq!(renamed_path(Path::new("/w/other.rs"), old, new), None);
    }

    #[test]
    fn renaming_onto_another_file_would_replace_it() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.rs"), dir.path().join("b.rs"));
        std::fs::write(&a, "a").unwrap();
        assert!(!would_replace(&a, &b), "free name");
        std::fs::write(&b, "b").unwrap();
        assert!(would_replace(&a, &b), "b.rs would be lost");
    }

    #[test]
    fn relative_paths_are_made_absolute_from_the_current_dir() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(absolute_path(PathBuf::from(".")), cwd);
        assert_eq!(absolute_path(PathBuf::from("sub")), cwd.join("sub"));
    }

    #[test]
    fn absolute_paths_are_kept() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(absolute_path(cwd.clone()), cwd);
    }

    /// `cu .` then a run configuration: `${workspaceFolder}` is the full path.
    #[test]
    fn workspace_folder_variable_is_absolute_for_a_relative_launch() {
        let ws = absolute_path(PathBuf::from("."));
        let cfg = crate::runner::RunConfig {
            name: "x".into(),
            command: "echo ${workspaceFolder}".into(),
            cwd: "${workspaceRoot}".into(),
            env: vec![],
            args: vec![],
            debug: None,
        };
        let r = cfg.resolve(Some(&ws), None, crate::runner::ShellKind::Posix);
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(r.command, format!("echo {}", cwd.display()));
        assert!(Path::new(&r.cwd).is_absolute(), "cwd {:?}", r.cwd);
    }
}
