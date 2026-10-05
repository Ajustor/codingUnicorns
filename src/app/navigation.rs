use std::path::PathBuf;

use super::CodingUnicorns;
use crate::editor::Editor;
use crate::tabs::TabManager;

impl CodingUnicorns {
    pub fn push_nav_and_goto(&mut self, target_path: PathBuf, target_line: usize) {
        if let Some(current_path) = self.editor.current_path.clone() {
            let (row, col) = self.editor.cursor.position();
            self.nav_history.push(current_path, row, col);
        }
        self.open_file_at_line(target_path, target_line);
    }

    /// Alt+Left: go back, recording the current position so Alt+Right can
    /// return to it.
    pub fn navigate_back(&mut self) {
        let entry = match self.editor.current_path.clone() {
            Some(current_path) => {
                let (row, col) = self.editor.cursor.position();
                self.nav_history.go_back_from(current_path, row, col)
            }
            None => self.nav_history.go_back(),
        };
        if let Some(entry) = entry {
            self.open_file_at_line(entry.path, entry.row);
        }
    }

    /// Cycle to the next open tab (Ctrl+Tab).
    pub fn cycle_tab_next(&mut self) {
        let n = self.tab_manager.tabs.len();
        if n < 2 {
            return;
        }
        if let Some(active_id) = self.tab_manager.active_tab {
            let pos = self
                .tab_manager
                .tabs
                .iter()
                .position(|t| t.id == active_id)
                .unwrap_or(0);
            let next_pos = (pos + 1) % n;
            let next_id = self.tab_manager.tabs[next_pos].id;
            self.tab_manager.active_tab = Some(next_id);
            self.load_active_tab();
        }
    }

    /// Cycle to the previous open tab (Ctrl+Shift+Tab).
    pub fn cycle_tab_prev(&mut self) {
        let n = self.tab_manager.tabs.len();
        if n < 2 {
            return;
        }
        if let Some(active_id) = self.tab_manager.active_tab {
            let pos = self
                .tab_manager
                .tabs
                .iter()
                .position(|t| t.id == active_id)
                .unwrap_or(0);
            let prev_pos = if pos == 0 { n - 1 } else { pos - 1 };
            self.tab_manager.active_tab = Some(self.tab_manager.tabs[prev_pos].id);
            self.load_active_tab();
        }
    }

    /// Toggle the split editor (Ctrl+\).
    pub fn toggle_split(&mut self) {
        if self.editor2.is_some() {
            // Close split
            self.editor2 = None;
            self.tab_manager2 = None;
            self.active_pane = 0;
        } else {
            // Open split with current file
            let mut editor2 = Editor::new();
            let mut tab_manager2 = TabManager::new();
            if let Some(ref path) = self.editor.current_path.clone() {
                if let Ok((content, lossy)) = crate::editor::text_format::read_text_file(path) {
                    editor2.set_content(content.clone(), Some(path.clone()));
                    editor2.decoded_lossy = lossy;
                    tab_manager2.open(path.clone(), content);
                }
            }
            self.editor2 = Some(editor2);
            self.tab_manager2 = Some(tab_manager2);
            self.active_pane = 1;
        }
    }
}
