use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::client::DapClient;
use super::types::{DapConfig, DebugSessionState, Scope, StackFrame, Variable, WatchResult};

/// Manages the active debug session and breakpoint storage.
#[derive(Default)]
pub struct DapManager {
    pub session: Option<DapClient>,
    /// Breakpoints per file: file path → set of 1-based line numbers.
    pub breakpoints: HashMap<PathBuf, HashSet<usize>>,
    /// Set when a pause just happened — caller may want to navigate to the top frame.
    pub just_paused: bool,
    /// Watch expressions, persisted per workspace in `.coding-unicorns/watches.toml`.
    watches: Vec<String>,
    /// Workspace the watches were loaded from (and are saved to).
    workspace: Option<PathBuf>,
}

/// On-disk format of `.coding-unicorns/watches.toml`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct WatchFile {
    #[serde(default)]
    expressions: Vec<String>,
}

fn watch_file_path(workspace: &Path) -> PathBuf {
    workspace.join(".coding-unicorns").join("watches.toml")
}

impl DapManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind watch persistence to a workspace, loading its saved expressions.
    /// Cheap no-op when the workspace is unchanged, so it can run every frame.
    pub fn set_workspace(&mut self, workspace: &Path) {
        if self.workspace.as_deref() == Some(workspace) {
            return;
        }
        self.workspace = Some(workspace.to_path_buf());
        self.watches = std::fs::read_to_string(watch_file_path(workspace))
            .ok()
            .and_then(|s| toml::from_str::<WatchFile>(&s).ok())
            .map(|f| f.expressions)
            .unwrap_or_default();
        self.sync_watches();
    }

    pub fn watches(&self) -> &[String] {
        &self.watches
    }

    /// Latest evaluation of a watch, `None` when no session is active.
    pub fn watch_result(&self, expr: &str) -> Option<&WatchResult> {
        self.session.as_ref()?.watch_results.get(expr)
    }

    /// Add a watch expression (trimmed; empty or duplicate ones are ignored).
    pub fn add_watch(&mut self, expr: &str) {
        let expr = expr.trim();
        if expr.is_empty() || self.watches.iter().any(|w| w == expr) {
            return;
        }
        self.watches.push(expr.to_string());
        self.save_watches();
        self.sync_watches();
    }

    pub fn remove_watch(&mut self, idx: usize) {
        if idx < self.watches.len() {
            self.watches.remove(idx);
            self.save_watches();
            self.sync_watches();
        }
    }

    fn sync_watches(&mut self) {
        if let Some(sess) = &mut self.session {
            sess.set_watches(&self.watches);
        }
    }

    fn save_watches(&self) {
        let Some(ws) = &self.workspace else {
            return;
        };
        let path = watch_file_path(ws);
        if self.watches.is_empty() && !path.exists() {
            return; // don't create the folder just to store nothing
        }
        let file = WatchFile {
            expressions: self.watches.clone(),
        };
        if let Ok(content) = toml::to_string_pretty(&file) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, content);
        }
    }

    /// Toggle a breakpoint at the given file/line. Returns the new set for this file.
    pub fn toggle_breakpoint(&mut self, file: &Path, line: usize) -> Vec<usize> {
        let set = self.breakpoints.entry(file.to_path_buf()).or_default();
        if set.contains(&line) {
            set.remove(&line);
        } else {
            set.insert(line);
        }
        let mut lines: Vec<usize> = set.iter().cloned().collect();
        lines.sort_unstable();
        // Sync with active session if any.
        if let Some(sess) = &mut self.session {
            sess.set_breakpoints(file, &lines);
        }
        lines
    }

    /// Returns the sorted breakpoint lines for a given file (0-based for display comparison).
    pub fn breakpoint_lines_for(&self, file: &Path) -> HashSet<usize> {
        self.breakpoints.get(file).cloned().unwrap_or_default()
    }

    /// Start a new debug session.
    pub fn start_session(
        &mut self,
        cfg: &DapConfig,
        workspace: &Path,
        current_file: Option<&Path>,
    ) -> anyhow::Result<()> {
        let mut client = DapClient::start(cfg, workspace)?;
        // If a current file is set, substitute ${file} in the launch config.
        if let Some(file) = current_file {
            client.set_file_variable(file);
        }
        // Queue all stored breakpoints.
        let bps: Vec<(PathBuf, Vec<usize>)> = self
            .breakpoints
            .iter()
            .map(|(f, ls)| {
                let mut lines: Vec<usize> = ls.iter().cloned().collect();
                lines.sort_unstable();
                (f.clone(), lines)
            })
            .collect();
        for (file, lines) in &bps {
            client.set_breakpoints(file, lines);
        }
        client.set_watches(&self.watches);
        self.session = Some(client);
        Ok(())
    }

    /// Stop the active debug session.
    pub fn stop_session(&mut self) {
        if let Some(sess) = &mut self.session {
            sess.disconnect();
        }
        self.session = None;
    }

    /// Poll the active session. Call every frame.
    pub fn poll(&mut self) {
        self.just_paused = false;
        if let Some(sess) = &mut self.session {
            let paused = sess.poll();
            if paused {
                self.just_paused = true;
            }
            // Clean up terminated sessions automatically.
            if sess.state == DebugSessionState::Terminated && !sess.is_alive() {
                // Keep the session alive a bit so the UI can show the final state;
                // layout.rs is responsible for calling stop_session() on user action.
            }
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(
            self.session.as_ref().map(|s| &s.state),
            Some(DebugSessionState::Running) | Some(DebugSessionState::Launching)
        )
    }

    pub fn is_paused(&self) -> bool {
        matches!(
            self.session.as_ref().map(|s| &s.state),
            Some(DebugSessionState::Paused { .. })
        )
    }

    pub fn is_active(&self) -> bool {
        self.session.is_some()
    }

    pub fn paused_thread_id(&self) -> Option<i64> {
        match self.session.as_ref()?.state {
            DebugSessionState::Paused { thread_id } => Some(thread_id),
            _ => None,
        }
    }

    pub fn call_stack(&self) -> &[StackFrame] {
        self.session
            .as_ref()
            .map(|s| s.call_stack.as_slice())
            .unwrap_or(&[])
    }

    /// Index of the call-stack frame whose scopes are shown.
    pub fn selected_frame(&self) -> usize {
        self.session.as_ref().map(|s| s.selected_frame).unwrap_or(0)
    }

    pub fn select_frame(&mut self, idx: usize) {
        if let Some(sess) = &mut self.session {
            sess.select_frame(idx);
        }
    }

    /// Scopes (Locals, Globals…) of the selected frame.
    pub fn scopes(&self) -> &[Scope] {
        self.session
            .as_ref()
            .map(|s| s.scopes.as_slice())
            .unwrap_or(&[])
    }

    /// Fetched children of a scope or structured variable, `None` if not loaded.
    pub fn children(&self, variables_reference: i64) -> Option<&[Variable]> {
        self.session
            .as_ref()?
            .children
            .get(&variables_reference)
            .map(|v| v.as_slice())
    }

    pub fn is_loading(&self, variables_reference: i64) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.is_loading(variables_reference))
    }

    /// Lazily fetch the children of a scope or structured variable.
    pub fn request_variables(&mut self, variables_reference: i64) {
        if let Some(sess) = &mut self.session {
            sess.request_variables(variables_reference);
        }
    }

    pub fn output_log(&self) -> &[String] {
        self.session
            .as_ref()
            .map(|s| s.output_log.as_slice())
            .unwrap_or(&[])
    }

    pub fn session_state(&self) -> DebugSessionState {
        self.session
            .as_ref()
            .map(|s| s.state.clone())
            .unwrap_or(DebugSessionState::Idle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dap::transport::DapTransport;
    use crossbeam_channel::{unbounded, Sender};
    use serde_json::{json, Value};
    use std::io::Write;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);
    impl Write for Shared {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn sent(out: &Shared) -> Vec<Value> {
        let bytes = std::mem::take(&mut *out.0.lock().unwrap());
        let (tx, rx) = unbounded();
        crate::dap::transport::read_messages(std::io::Cursor::new(bytes), &tx);
        drop(tx);
        rx.try_iter().collect()
    }

    fn cfg() -> DapConfig {
        DapConfig {
            adapter_cmd: "definitely-not-a-real-dap-adapter-xyz".into(),
            adapter_args: vec![],
            launch_config: json!({}),
        }
    }

    fn with_session(m: &mut DapManager) -> (Shared, Sender<Value>, Arc<AtomicBool>) {
        let out = Shared::default();
        let (tx, rx) = unbounded();
        let alive = Arc::new(AtomicBool::new(true));
        let t = DapTransport::from_parts(Box::new(out.clone()), rx, alive.clone());
        m.session = Some(DapClient::for_test(t, &cfg(), Path::new("/ws")));
        sent(&out); // discard `initialize`
        (out, tx, alive)
    }

    #[test]
    fn toggle_breakpoint_adds_and_removes_sorted() {
        let mut m = DapManager::new();
        let f = Path::new("/ws/a.rs");
        assert_eq!(m.toggle_breakpoint(f, 10), vec![10]);
        assert_eq!(m.toggle_breakpoint(f, 2), vec![2, 10]);
        assert_eq!(m.toggle_breakpoint(f, 10), vec![2]);
        assert_eq!(m.breakpoint_lines_for(f), HashSet::from([2]));
        assert!(m.breakpoint_lines_for(Path::new("/other")).is_empty());
    }

    #[test]
    fn idle_manager_reports_defaults() {
        let m = DapManager::new();
        assert!(!m.is_active());
        assert!(!m.is_running());
        assert!(!m.is_paused());
        assert_eq!(m.paused_thread_id(), None);
        assert!(m.call_stack().is_empty());
        assert!(m.scopes().is_empty());
        assert!(m.children(1).is_none());
        assert!(!m.is_loading(1));
        assert_eq!(m.selected_frame(), 0);
        assert!(m.output_log().is_empty());
        assert_eq!(m.session_state(), DebugSessionState::Idle);
    }

    #[test]
    fn watches_are_deduplicated_and_persisted_per_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = DapManager::new();
        m.set_workspace(dir.path());
        assert!(m.watches().is_empty());
        assert!(
            !dir.path().join(".coding-unicorns").exists(),
            "loading creates nothing"
        );
        m.add_watch("  x + 1 ");
        m.add_watch("x + 1");
        m.add_watch("   ");
        m.add_watch("y");
        assert_eq!(m.watches(), ["x + 1", "y"]);
        assert!(m.watch_result("y").is_none(), "no session → no result");

        let mut m2 = DapManager::new();
        m2.set_workspace(dir.path());
        assert_eq!(m2.watches(), ["x + 1", "y"]);

        m2.remove_watch(0);
        m2.remove_watch(9); // out of range: no-op
        let mut m3 = DapManager::new();
        m3.set_workspace(dir.path());
        assert_eq!(m3.watches(), ["y"]);

        // Switching to a workspace without a file clears the list.
        let other = tempfile::tempdir().unwrap();
        m3.set_workspace(other.path());
        assert!(m3.watches().is_empty());
    }

    #[test]
    fn corrupt_watch_file_yields_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let cu = dir.path().join(".coding-unicorns");
        std::fs::create_dir_all(&cu).unwrap();
        std::fs::write(cu.join("watches.toml"), "expressions = 3").unwrap();
        let mut m = DapManager::new();
        m.set_workspace(dir.path());
        assert!(m.watches().is_empty());
    }

    #[test]
    fn watches_without_workspace_stay_in_memory() {
        let mut m = DapManager::new();
        m.add_watch("a");
        assert_eq!(m.watches(), ["a"]);
        assert!(!Path::new(".coding-unicorns/watches.toml").exists());
    }

    #[test]
    fn watches_are_pushed_to_the_session() {
        let mut m = DapManager::new();
        let (out, tx, _alive) = with_session(&mut m);
        m.add_watch("v");
        assert_eq!(m.watch_result("v"), Some(&WatchResult::Pending));
        tx.send(json!({"type": "event", "event": "stopped", "body": {"threadId": 1}}))
            .unwrap();
        m.poll();
        let seq = sent(&out)[0]["seq"].as_u64().unwrap();
        tx.send(
            json!({"type": "response", "command": "stackTrace", "request_seq": seq,
            "body": {"stackFrames": [{"id": 3}]}}),
        )
        .unwrap();
        m.poll();
        let msgs = sent(&out);
        let ev = msgs.iter().find(|m| m["command"] == "evaluate").unwrap();
        assert_eq!(ev["arguments"]["expression"], "v");
        tx.send(json!({"type": "response", "command": "evaluate",
            "request_seq": ev["seq"], "body": {"result": "7"}}))
            .unwrap();
        m.poll();
        assert!(matches!(
            m.watch_result("v"),
            Some(WatchResult::Value { value, .. }) if value == "7"
        ));
        m.remove_watch(0);
        assert!(m.watch_result("v").is_none());
    }

    #[test]
    fn scope_queries_delegate_to_session() {
        let mut m = DapManager::new();
        // Without a session these are no-ops.
        m.request_variables(5);
        m.select_frame(1);
        let (out, tx, _alive) = with_session(&mut m);
        tx.send(json!({"type": "event", "event": "stopped", "body": {"threadId": 1}}))
            .unwrap();
        m.poll();
        let seq = sent(&out)[0]["seq"].as_u64().unwrap();
        tx.send(
            json!({"type": "response", "command": "stackTrace", "request_seq": seq,
            "body": {"stackFrames": [{"id": 1}, {"id": 2}]}}),
        )
        .unwrap();
        m.poll();
        let seq = sent(&out)[0]["seq"].as_u64().unwrap();
        tx.send(
            json!({"type": "response", "command": "scopes", "request_seq": seq,
            "body": {"scopes": [{"name": "Globals", "variablesReference": 8, "expensive": true}]}}),
        )
        .unwrap();
        m.poll();
        assert_eq!(m.scopes()[0].name, "Globals");
        assert!(sent(&out).is_empty(), "expensive scope not fetched eagerly");
        m.request_variables(8);
        assert!(m.is_loading(8));
        let msgs = sent(&out);
        tx.send(json!({"type": "response", "command": "variables",
            "request_seq": msgs[0]["seq"], "body": {"variables": [{"name": "g"}]}}))
            .unwrap();
        m.poll();
        assert_eq!(m.children(8).unwrap()[0].name, "g");
        m.select_frame(1);
        assert_eq!(m.selected_frame(), 1);
        assert!(m.children(8).is_none());
    }

    #[test]
    fn start_session_with_missing_adapter_errors_and_stays_idle() {
        let mut m = DapManager::new();
        let dir = tempfile::tempdir().unwrap();
        assert!(m.start_session(&cfg(), dir.path(), None).is_err());
        assert!(!m.is_active());
    }

    #[test]
    fn start_session_with_short_lived_adapter_queues_stored_breakpoints() {
        let mut m = DapManager::new();
        m.toggle_breakpoint(Path::new("/ws/a.py"), 3);
        let dir = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let (cmd, args) = ("cmd", vec!["/C".to_string(), "exit 0".to_string()]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c".to_string(), "exit 0".to_string()]);
        let c = DapConfig {
            adapter_cmd: cmd.into(),
            adapter_args: args,
            launch_config: json!({"program": "${file}"}),
        };
        m.start_session(&c, dir.path(), Some(Path::new("/ws/main.py")))
            .unwrap();
        let sess = m.session.as_ref().unwrap();
        assert_eq!(sess.state, DebugSessionState::Launching);
        assert!(m.is_active());
        m.stop_session();
        assert!(!m.is_active());
    }

    #[test]
    fn session_state_queries_follow_client_state() {
        let mut m = DapManager::new();
        let (_out, tx, _alive) = with_session(&mut m);
        assert!(m.is_active());
        assert!(m.is_running(), "Launching counts as running");
        tx.send(json!({"type": "event", "event": "stopped", "body": {"threadId": 9}}))
            .unwrap();
        m.poll();
        assert!(m.just_paused);
        assert!(m.is_paused());
        assert!(!m.is_running());
        assert_eq!(m.paused_thread_id(), Some(9));
        assert_eq!(
            m.session_state(),
            DebugSessionState::Paused { thread_id: 9 }
        );

        // just_paused is reset on the next poll.
        m.poll();
        assert!(!m.just_paused);

        tx.send(json!({"type": "event", "event": "continued"}))
            .unwrap();
        tx.send(json!({"type": "event", "event": "output", "body": {"output": "hi"}}))
            .unwrap();
        m.poll();
        assert!(m.is_running());
        assert_eq!(m.paused_thread_id(), None);
        assert_eq!(m.output_log(), ["hi".to_string()]);
        assert!(m.call_stack().is_empty());
        assert!(m.scopes().is_empty());
    }

    #[test]
    fn toggle_breakpoint_syncs_active_session() {
        let mut m = DapManager::new();
        let (out, tx, _alive) = with_session(&mut m);
        tx.send(json!({"type": "event", "event": "initialized"}))
            .unwrap();
        m.poll();
        sent(&out);
        m.toggle_breakpoint(Path::new("/ws/x.py"), 4);
        let msgs = sent(&out);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["command"], "setBreakpoints");
        assert_eq!(msgs[0]["arguments"]["breakpoints"], json!([{"line": 4}]));
    }

    #[test]
    fn stop_session_disconnects_and_clears() {
        let mut m = DapManager::new();
        let (out, _tx, _alive) = with_session(&mut m);
        m.stop_session();
        assert!(!m.is_active());
        let msgs = sent(&out);
        assert_eq!(msgs[0]["command"], "disconnect");
        // Stopping with no session is a no-op.
        m.stop_session();
    }

    #[test]
    fn poll_without_session_is_noop() {
        let mut m = DapManager::new();
        m.just_paused = true;
        m.poll();
        assert!(!m.just_paused);
    }

    #[test]
    fn dead_adapter_shows_terminated() {
        let mut m = DapManager::new();
        let (_out, _tx, alive) = with_session(&mut m);
        alive.store(false, std::sync::atomic::Ordering::Relaxed);
        m.poll();
        assert_eq!(m.session_state(), DebugSessionState::Terminated);
        assert!(m.is_active(), "session kept for UI until stop_session");
    }
}
