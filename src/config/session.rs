//! Per-workspace editor sessions (open tabs, active tab, cursor + scroll),
//! persisted in `sessions.toml` next to `config.toml`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Saved state of one open tab.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TabSession {
    pub path: String,
    #[serde(default)]
    pub row: usize,
    #[serde(default)]
    pub col: usize,
    #[serde(default)]
    pub scroll_x: f32,
    #[serde(default)]
    pub scroll_y: f32,
}

/// Saved tabs of one workspace, in tab-bar order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSession {
    /// Index into `tabs` of the active tab.
    #[serde(default)]
    pub active: Option<usize>,
    #[serde(default)]
    pub tabs: Vec<TabSession>,
}

/// All saved sessions, keyed by workspace path.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Sessions {
    #[serde(default)]
    pub workspaces: BTreeMap<String, WorkspaceSession>,
}

/// Map key for a workspace folder (trailing separators stripped).
pub fn workspace_key(ws: &Path) -> String {
    ws.to_string_lossy()
        .trim_end_matches(['/', '\\'])
        .to_string()
}

impl Sessions {
    pub fn sessions_path() -> PathBuf {
        super::Config::config_path().with_file_name("sessions.toml")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::sessions_path())
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match toml::to_string_pretty(self) {
            Ok(content) => {
                if let Err(e) = std::fs::write(path, content) {
                    log::warn!("failed to write {}: {e}", path.display());
                }
            }
            Err(e) => log::warn!("failed to serialize sessions: {e}"),
        }
    }

    pub fn get(&self, ws: &Path) -> Option<&WorkspaceSession> {
        self.workspaces.get(&workspace_key(ws))
    }

    /// Store `session` for `ws`; an empty session removes the entry.
    pub fn set(&mut self, ws: &Path, session: WorkspaceSession) {
        let key = workspace_key(ws);
        if session.tabs.is_empty() {
            self.workspaces.remove(&key);
        } else {
            self.workspaces.insert(key, session);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> WorkspaceSession {
        WorkspaceSession {
            active: Some(1),
            tabs: vec![
                TabSession {
                    path: "C:\\proj\\src\\main.rs".into(),
                    row: 10,
                    col: 4,
                    scroll_x: 0.0,
                    scroll_y: 120.5,
                },
                TabSession {
                    path: "/proj/README.md".into(),
                    ..Default::default()
                },
            ],
        }
    }

    #[test]
    fn round_trip_through_file_with_windows_path_keys() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested").join("sessions.toml");
        let mut s = Sessions::default();
        s.set(Path::new("C:\\proj\\"), sample());
        s.save_to(&file);
        let back = Sessions::load_from(&file);
        assert_eq!(back, s);
        assert_eq!(back.get(Path::new("C:\\proj")), Some(&sample()));
    }

    #[test]
    fn set_empty_session_removes_entry() {
        let mut s = Sessions::default();
        s.set(Path::new("/ws"), sample());
        assert!(s.get(Path::new("/ws")).is_some());
        s.set(Path::new("/ws"), WorkspaceSession::default());
        assert!(s.get(Path::new("/ws")).is_none());
        assert!(s.workspaces.is_empty());
    }

    #[test]
    fn load_missing_or_corrupt_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Sessions::load_from(&dir.path().join("none.toml")),
            Sessions::default()
        );
        let bad = dir.path().join("bad.toml");
        std::fs::write(&bad, "[[[ nope").unwrap();
        assert_eq!(Sessions::load_from(&bad), Sessions::default());
    }

    #[test]
    fn missing_tab_fields_default() {
        let s: Sessions = toml::from_str(
            r#"
            [workspaces."/ws"]
            [[workspaces."/ws".tabs]]
            path = "/ws/a.rs"
            "#,
        )
        .unwrap();
        let ws = s.get(Path::new("/ws")).unwrap();
        assert_eq!(ws.active, None);
        assert_eq!(ws.tabs[0].row, 0);
        assert_eq!(ws.tabs[0].scroll_y, 0.0);
    }

    #[test]
    fn sessions_path_is_next_to_config() {
        let p = Sessions::sessions_path();
        assert!(p.ends_with(Path::new("coding-unicorns").join("sessions.toml")));
    }
}
