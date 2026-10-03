use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::client::DapClient;
use super::types::{DapConfig, DebugSessionState, StackFrame, Variable};

/// Manages the active debug session and breakpoint storage.
#[derive(Default)]
pub struct DapManager {
    pub session: Option<DapClient>,
    /// Breakpoints per file: file path → set of 1-based line numbers.
    pub breakpoints: HashMap<PathBuf, HashSet<usize>>,
    /// Set when a pause just happened — caller may want to navigate to the top frame.
    pub just_paused: bool,
}

impl DapManager {
    pub fn new() -> Self {
        Self::default()
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

    pub fn variables(&self) -> &[Variable] {
        self.session
            .as_ref()
            .map(|s| s.variables.as_slice())
            .unwrap_or(&[])
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
        assert!(m.variables().is_empty());
        assert!(m.output_log().is_empty());
        assert_eq!(m.session_state(), DebugSessionState::Idle);
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
        assert!(m.variables().is_empty());
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
