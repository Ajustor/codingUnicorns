//! Session restore: per-workspace open tabs, active tab and cursor/scroll
//! position of each tab (see [`crate::config::session`] for persistence).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::CodingUnicorns;
use crate::config::session::{Sessions, TabSession, WorkspaceSession};
use crate::tabs::TabManager;

/// Cursor + scroll of a file the editor showed at some point.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ViewState {
    pub row: usize,
    pub col: usize,
    pub scroll: egui::Vec2,
}

/// The parts of the tab layout whose change triggers a session save.
type LayoutKey = (Option<PathBuf>, Vec<PathBuf>, Option<usize>);

pub struct SessionState {
    sessions: Sessions,
    /// Where `sessions` is written (`None` disables persistence).
    store_path: Option<PathBuf>,
    /// Last known cursor/scroll per file, so switching tabs (and restarting)
    /// returns to where the user was.
    pub view_states: HashMap<PathBuf, ViewState>,
    last_layout: Option<LayoutKey>,
}

impl SessionState {
    /// Session state backed by the real `sessions.toml` in the config dir.
    pub fn load() -> Self {
        let path = Sessions::sessions_path();
        Self {
            sessions: Sessions::load_from(&path),
            store_path: Some(path),
            view_states: HashMap::new(),
            last_layout: None,
        }
    }
}

/// Whether a tab refers to a real file that belongs in a session (not the
/// settings tab, not an `untitled-N` placeholder).
fn is_session_tab(path: &Path, is_settings: bool) -> bool {
    !is_settings && path.is_absolute()
}

fn layout_key(ws: Option<&Path>, tabs: &TabManager) -> LayoutKey {
    let mut active = None;
    let mut paths = Vec::new();
    for t in &tabs.tabs {
        if is_session_tab(&t.path, t.is_settings) {
            if tabs.active_tab == Some(t.id) {
                active = Some(paths.len());
            }
            paths.push(t.path.clone());
        }
    }
    (ws.map(Path::to_path_buf), paths, active)
}

/// Build the session to persist from the open tabs and known view states.
pub fn snapshot(tabs: &TabManager, views: &HashMap<PathBuf, ViewState>) -> WorkspaceSession {
    let mut session = WorkspaceSession::default();
    for t in &tabs.tabs {
        if !is_session_tab(&t.path, t.is_settings) {
            continue;
        }
        if tabs.active_tab == Some(t.id) {
            session.active = Some(session.tabs.len());
        }
        let v = views.get(&t.path).copied().unwrap_or_default();
        session.tabs.push(TabSession {
            path: t.path.to_string_lossy().to_string(),
            row: v.row,
            col: v.col,
            scroll_x: v.scroll.x,
            scroll_y: v.scroll.y,
        });
    }
    session
}

/// Tabs of `session` whose file still exists, plus the index of the tab to
/// activate (the saved active tab, or the closest surviving tab before it).
pub fn restorable(session: &WorkspaceSession) -> (Vec<(PathBuf, ViewState)>, Option<usize>) {
    let mut out = Vec::new();
    let mut active = None;
    for (i, t) in session.tabs.iter().enumerate() {
        let path = PathBuf::from(&t.path);
        if !path.is_file() {
            continue;
        }
        if session.active.is_some_and(|a| i <= a) {
            active = Some(out.len());
        }
        out.push((
            path,
            ViewState {
                row: t.row,
                col: t.col,
                scroll: egui::vec2(t.scroll_x, t.scroll_y),
            },
        ));
    }
    if active.is_none() && !out.is_empty() {
        active = Some(0);
    }
    (out, active)
}

/// Clamp a saved cursor to a buffer of `num_lines` lines, where
/// `line_len(row)` gives a line's length in chars.
pub fn clamp_cursor(
    view: &ViewState,
    num_lines: usize,
    line_len: impl Fn(usize) -> usize,
) -> (usize, usize) {
    let row = view.row.min(num_lines.saturating_sub(1));
    (row, view.col.min(line_len(row)))
}

impl CodingUnicorns {
    /// Remember the cursor/scroll of the file currently in the editor.
    pub fn remember_view_state(&mut self) {
        if let Some(path) = self.editor.current_path.clone() {
            let (row, col) = self.editor.cursor.position();
            self.session.view_states.insert(
                path,
                ViewState {
                    row,
                    col,
                    scroll: self.editor.scroll_offset,
                },
            );
        }
    }

    /// Re-apply the remembered cursor/scroll for `path` (just loaded into the
    /// editor), clamped to the new content.
    pub fn apply_view_state(&mut self, path: &Path) {
        let Some(view) = self.session.view_states.get(path).copied() else {
            return;
        };
        let buf = &self.editor.buffer;
        let (row, col) = clamp_cursor(&view, buf.num_lines(), |r| buf.line_char_len_fast(r));
        self.editor.cursor.set_position(row, col);
        self.editor.scroll_offset = egui::vec2(view.scroll.x.max(0.0), view.scroll.y.max(0.0));
    }

    /// Persist the current workspace's tabs to `sessions.toml`.
    pub fn save_session(&mut self) {
        let Some(ws) = self.workspace_path.clone() else {
            return;
        };
        self.remember_view_state();
        let session = snapshot(&self.tab_manager, &self.session.view_states);
        self.session.sessions.set(&ws, session);
        self.session.last_layout = Some(layout_key(Some(&ws), &self.tab_manager));
        if let Some(path) = &self.session.store_path {
            self.session.sessions.save_to(path);
        }
    }

    /// Called once per frame: saves the session when tabs were opened, closed
    /// or switched (cheap comparison, writes only on change).
    pub fn tick_session(&mut self) {
        if self.workspace_path.is_none() {
            return;
        }
        let key = layout_key(self.workspace_path.as_deref(), &self.tab_manager);
        if self.session.last_layout.as_ref() != Some(&key) {
            self.save_session();
        }
    }

    /// Reopen the saved tabs of `ws`. Returns true if at least one tab was
    /// restored. Unsaved tabs are kept; when the editor holds unsaved edits the
    /// saved tabs are only added, without switching away from the current one.
    pub fn restore_session(&mut self, ws: &Path) -> bool {
        let Some(saved) = self.session.sessions.get(ws).cloned() else {
            return false;
        };
        let (tabs, active) = restorable(&saved);
        if tabs.is_empty() {
            return false;
        }
        let keep_current = self.editor.is_modified;
        let previous_active = self.tab_manager.active_tab;
        if !keep_current {
            let stale: Vec<usize> = self
                .tab_manager
                .tabs
                .iter()
                .filter(|t| !t.is_modified && !t.is_settings)
                .map(|t| t.id)
                .collect();
            for id in stale {
                self.tab_manager.close(id);
            }
        }
        for (path, view) in &tabs {
            self.session.view_states.insert(path.clone(), *view);
            self.tab_manager.open(path.clone(), String::new());
        }
        if keep_current {
            self.tab_manager.active_tab = previous_active;
        } else if let Some((path, _)) = active.and_then(|i| tabs.get(i)) {
            let path = path.clone();
            // Make sure the outgoing file doesn't overwrite the saved view.
            self.editor.current_path = None;
            self.open_file(path);
        }
        self.session.last_layout = Some(layout_key(Some(ws), &self.tab_manager));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "line one\nline two\n").unwrap();
        p
    }

    #[test]
    fn snapshot_keeps_order_skips_settings_and_untitled_and_tracks_active() {
        let dir = tempfile::tempdir().unwrap();
        let a = abs(dir.path(), "a.rs");
        let b = abs(dir.path(), "b.rs");
        let mut tm = TabManager::new();
        tm.open(a.clone(), String::new());
        tm.open_settings();
        tm.open_untitled();
        tm.open(b.clone(), String::new());
        let mut views = HashMap::new();
        views.insert(
            b.clone(),
            ViewState {
                row: 3,
                col: 2,
                scroll: egui::vec2(1.0, 40.0),
            },
        );
        let s = snapshot(&tm, &views);
        assert_eq!(s.tabs.len(), 2);
        assert_eq!(PathBuf::from(&s.tabs[0].path), a);
        assert_eq!(PathBuf::from(&s.tabs[1].path), b);
        assert_eq!(s.active, Some(1));
        assert_eq!((s.tabs[1].row, s.tabs[1].col), (3, 2));
        assert_eq!(s.tabs[1].scroll_y, 40.0);
        assert_eq!((s.tabs[0].row, s.tabs[0].col), (0, 0));
    }

    #[test]
    fn snapshot_with_settings_active_has_no_active_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut tm = TabManager::new();
        tm.open(abs(dir.path(), "a.rs"), String::new());
        tm.open_settings();
        assert_eq!(snapshot(&tm, &HashMap::new()).active, None);
    }

    #[test]
    fn restorable_skips_missing_files_and_remaps_active() {
        let dir = tempfile::tempdir().unwrap();
        let a = abs(dir.path(), "a.rs");
        let c = abs(dir.path(), "c.rs");
        let gone = dir.path().join("gone.rs");
        let tab = |p: &Path, row| TabSession {
            path: p.to_string_lossy().to_string(),
            row,
            ..Default::default()
        };
        let session = WorkspaceSession {
            active: Some(2),
            tabs: vec![tab(&a, 1), tab(&gone, 0), tab(&c, 5)],
        };
        let (tabs, active) = restorable(&session);
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].0, a);
        assert_eq!(tabs[1].0, c);
        assert_eq!(tabs[1].1.row, 5);
        assert_eq!(active, Some(1));

        // Active tab vanished → the closest surviving tab before it.
        let session = WorkspaceSession {
            active: Some(1),
            ..session
        };
        assert_eq!(restorable(&session).1, Some(0));

        // No active recorded → first tab.
        let session = WorkspaceSession {
            active: None,
            ..session
        };
        assert_eq!(restorable(&session).1, Some(0));

        // Nothing survives → nothing to restore.
        let session = WorkspaceSession {
            active: Some(0),
            tabs: vec![tab(&gone, 0)],
        };
        assert_eq!(restorable(&session), (vec![], None));
    }

    #[test]
    fn clamp_cursor_limits_row_and_col_to_content() {
        let lens = [5usize, 2];
        let v = |row, col| ViewState {
            row,
            col,
            ..Default::default()
        };
        assert_eq!(clamp_cursor(&v(1, 1), 2, |r| lens[r]), (1, 1));
        assert_eq!(clamp_cursor(&v(9, 9), 2, |r| lens[r]), (1, 2));
        assert_eq!(clamp_cursor(&v(0, 9), 2, |r| lens[r]), (0, 5));
        assert_eq!(clamp_cursor(&v(3, 3), 0, |_| 0), (0, 0));
    }

    #[test]
    fn layout_key_changes_on_open_close_and_switch() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let mut tm = TabManager::new();
        let a = tm.open(abs(ws, "a.rs"), String::new());
        let k1 = layout_key(Some(ws), &tm);
        let b = tm.open(abs(ws, "b.rs"), String::new());
        let k2 = layout_key(Some(ws), &tm);
        assert_ne!(k1, k2);
        tm.active_tab = Some(a);
        let k3 = layout_key(Some(ws), &tm);
        assert_ne!(k2, k3);
        tm.close(b);
        assert_eq!(layout_key(Some(ws), &tm), k1);
    }
}
