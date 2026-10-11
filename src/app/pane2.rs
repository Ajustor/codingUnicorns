//! The split view's right pane: its own editor and tab bar, guarded like the
//! main pane so its unsaved changes are never dropped silently.
//!
//! The right pane keeps one document: switching its tab away from a modified
//! file is refused until it is saved (the main pane parks documents instead).
//! Closing a modified tab or the whole split asks first, quitting saves it
//! with "Save All & Quit", and a save never overwrites a change made to the
//! file since the pane loaded it (from the main pane or outside the app).

use std::path::PathBuf;

use super::file_watch::{disk_matches, FileStamp};
use super::CodingUnicorns;

/// What closing the right pane's unsaved file was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane2Close {
    /// Its tab (by id in `tab_manager2`).
    Tab(usize),
    /// The whole split view.
    Split,
}

impl CodingUnicorns {
    /// The right pane has unsaved changes.
    pub fn pane2_modified(&self) -> bool {
        self.editor2.as_ref().is_some_and(|e| e.is_modified)
    }

    /// Remember the on-disk state of the file the right pane just loaded or saved.
    pub(super) fn record_pane2_stamp(&mut self) {
        self.pane2_stamp = self
            .editor2
            .as_ref()
            .and_then(|e| e.current_path.as_deref())
            .and_then(FileStamp::of);
    }

    /// True when the right pane's file changed on disk since the pane loaded or
    /// saved it, to something else than what it shows.
    fn pane2_changed_on_disk(&self) -> bool {
        let Some(e2) = &self.editor2 else {
            return false;
        };
        let Some(path) = &e2.current_path else {
            return false;
        };
        let now = FileStamp::of(path);
        now.is_some() && now != self.pane2_stamp && !disk_matches(path, &e2.buffer.to_string())
    }

    /// Ctrl+S in the right pane: false (the save is swallowed) when it would
    /// overwrite a newer version of the file.
    pub(super) fn pane2_save_allowed(&mut self) -> bool {
        if !self.pane2_changed_on_disk() {
            return true;
        }
        self.toast(
            "The file changed since the right pane opened it (saved from the left pane?): \
             reopen it there before saving",
        );
        false
    }

    /// Save the right pane's file (quit's "Save All", the close dialog). False
    /// when it changed on disk meanwhile or can't be written.
    pub fn save_pane2(&mut self) -> bool {
        if !self.pane2_modified() {
            return true;
        }
        if !self.pane2_save_allowed() {
            return false;
        }
        let Some(e2) = self.editor2.as_mut() else {
            return true;
        };
        if let Err(e) = e2.save() {
            self.toast(format!("Could not save the right pane: {e}"));
            return false;
        }
        self.record_pane2_stamp();
        true
    }

    /// Open `path` in the right pane, unless that would drop its unsaved changes.
    pub fn open_file_in_pane2(&mut self, path: PathBuf) {
        let shown = self.editor2.as_ref().and_then(|e| e.current_path.clone());
        if shown.as_ref() == Some(&path) {
            if let Some(tm2) = self.tab_manager2.as_mut() {
                tm2.open(path, String::new());
            }
            return;
        }
        if self.pane2_modified() {
            self.toast("Save the right pane's file before opening another one there");
            return;
        }
        if let Ok((content, lossy)) = crate::editor::text_format::read_text_file(&path) {
            if let Some(ref mut tm2) = self.tab_manager2 {
                tm2.open(path.clone(), content.clone());
            }
            if let Some(ref mut e2) = self.editor2 {
                e2.set_content(content, Some(path));
                e2.decoded_lossy = lossy;
            }
            self.record_pane2_stamp();
        }
    }

    /// Per frame: keep the right pane's tab flag, close requests and file in
    /// step, and show the close dialog.
    pub(super) fn tick_pane2(&mut self, ctx: &egui::Context) {
        let Some(e2) = self.editor2.as_ref() else {
            self.pane2_close_pending = None;
            return;
        };
        let modified = e2.is_modified;
        if let Some(tm2) = self.tab_manager2.as_mut() {
            if let Some(id) = tm2.active_tab {
                if let Some(tab) = tm2.tabs.iter_mut().find(|t| t.id == id) {
                    tab.is_modified = modified;
                }
            }
            if self.pane2_close_pending.is_none() {
                self.pane2_close_pending = tm2.close_request.take().map(Pane2Close::Tab);
            }
        }
        // Saved since (by this pane, or by the main one with the same file):
        // adopt the new stamp, reloading only when the text differs.
        if !modified {
            let path = e2.current_path.clone();
            let changed = path
                .as_deref()
                .is_some_and(|p| FileStamp::of(p).is_some_and(|s| Some(s) != self.pane2_stamp));
            if changed {
                if self.pane2_changed_on_disk() {
                    self.reload_pane2();
                }
                self.record_pane2_stamp();
            }
        }
        self.show_pane2_close_dialog(ctx);
    }

    /// Re-read the right pane's (unmodified) file, keeping its cursor.
    fn reload_pane2(&mut self) {
        let Some(e2) = self.editor2.as_mut() else {
            return;
        };
        let Some(path) = e2.current_path.clone() else {
            return;
        };
        if let Ok((content, lossy)) = crate::editor::text_format::read_text_file(&path) {
            let (row, col) = e2.cursor.position();
            e2.set_content(content, Some(path));
            e2.decoded_lossy = lossy;
            let row = row.min(e2.buffer.num_lines().saturating_sub(1));
            let col = col.min(e2.buffer.line_len(row));
            e2.cursor.set_position(row, col);
        }
    }

    /// Close tab `id` of the right pane and show the next one (or close the
    /// split when it was the last).
    fn close_pane2_tab(&mut self, id: usize) {
        if let Some(e2) = self.editor2.as_mut() {
            e2.is_modified = false; // discarded or saved by now
        }
        let next = self.tab_manager2.as_mut().and_then(|tm2| {
            tm2.close(id);
            tm2.active_tab
                .and_then(|a| tm2.tabs.iter().find(|t| t.id == a))
                .map(|t| t.path.clone())
        });
        if let Some(path) = next {
            self.open_file_in_pane2(path);
        }
        // No tab left: the layout closes the split on the next frame.
    }

    fn show_pane2_close_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pane2_close_pending else {
            return;
        };
        let name = self
            .editor2
            .as_ref()
            .and_then(|e| e.current_path.as_ref())
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut choice = None;
        egui::Window::new("Unsaved Changes")
            .id(egui::Id::new("pane2-close"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("\"{name}\" has unsaved changes in the right pane."));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save & Close").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Discard & Close").clicked() {
                        choice = Some(false);
                    }
                    if ui.button("Cancel").clicked() {
                        self.pane2_close_pending = None;
                    }
                });
            });
        let Some(save) = choice else {
            return;
        };
        self.pane2_close_pending = None;
        if save && !self.save_pane2() {
            return;
        }
        match pending {
            Pane2Close::Tab(id) => self.close_pane2_tab(id),
            Pane2Close::Split => {
                if let Some(e2) = self.editor2.as_mut() {
                    e2.is_modified = false;
                }
                self.toggle_split();
            }
        }
    }
}
