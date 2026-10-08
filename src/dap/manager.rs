use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::client::DapClient;
use super::launcher::{LaunchEvent, LaunchPlan, Preparing};
use super::transport::DapTransport;
use super::types::{DapConfig, DebugSessionState, Scope, StackFrame, Variable, WatchResult};

/// Manages the active debug session and breakpoint storage.
#[derive(Default)]
pub struct DapManager {
    /// The session started by the user (the adapter's main connection).
    pub session: Option<DapClient>,
    /// Sessions the adapter asked for with `startDebugging` (js-debug runs
    /// the program in one). The latest live one is the one shown.
    child_sessions: Vec<DapClient>,
    /// Workspace of the running session (child sessions use it too).
    session_workspace: PathBuf,
    /// Adapter download / `preLaunchTask` running before the session starts.
    preparing: Option<Preparing>,
    /// Workspace and file (`${file}`) of the session being prepared.
    preparing_for: (PathBuf, Option<PathBuf>),
    /// Messages shown before a session exists (task output, launch errors).
    /// Handed over to the session log once it starts.
    log: Vec<String>,
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
        self.active()?.watch_results.get(expr)
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
        let watches = self.watches.clone();
        for sess in self.all_sessions_mut() {
            sess.set_watches(&watches);
        }
    }

    /// The session shown and driven by the UI: the latest live child
    /// session, else the main one.
    fn active(&self) -> Option<&DapClient> {
        self.child_sessions
            .iter()
            .rev()
            .find(|c| c.state != DebugSessionState::Terminated)
            .or(self.session.as_ref())
    }

    /// Mutable [`Self::active`], for stepping and continuing.
    pub fn active_mut(&mut self) -> Option<&mut DapClient> {
        match self
            .child_sessions
            .iter()
            .rposition(|c| c.state != DebugSessionState::Terminated)
        {
            Some(i) => self.child_sessions.get_mut(i),
            None => self.session.as_mut(),
        }
    }

    fn all_sessions_mut(&mut self) -> impl Iterator<Item = &mut DapClient> {
        self.session
            .iter_mut()
            .chain(self.child_sessions.iter_mut())
    }

    /// Stored breakpoints, sorted per file.
    fn sorted_breakpoints(&self) -> Vec<(PathBuf, Vec<usize>)> {
        self.breakpoints
            .iter()
            .map(|(f, ls)| {
                let mut lines: Vec<usize> = ls.iter().cloned().collect();
                lines.sort_unstable();
                (f.clone(), lines)
            })
            .collect()
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
        // Sync with the running sessions, if any.
        for sess in self.all_sessions_mut() {
            sess.set_breakpoints(file, &lines);
        }
        lines
    }

    /// Returns the sorted breakpoint lines for a given file (0-based for display comparison).
    pub fn breakpoint_lines_for(&self, file: &Path) -> HashSet<usize> {
        self.breakpoints.get(file).cloned().unwrap_or_default()
    }

    /// Prepare (adapter download, `preLaunchTask`) then start a session.
    pub fn launch(&mut self, plan: LaunchPlan, current_file: Option<&Path>) {
        self.stop_session();
        self.log.clear();
        self.preparing_for = (plan.workspace.clone(), current_file.map(Path::to_path_buf));
        self.preparing = Some(super::launcher::prepare(plan));
    }

    /// Show why a session could not be launched (replaces the old session).
    pub fn launch_failed(&mut self, message: impl Into<String>) {
        self.stop_session();
        self.log.clear();
        push_log(&mut self.log, message.into());
    }

    /// Whether a new session can be started: none is running or preparing
    /// (a terminated one is replaced).
    pub fn can_start(&self) -> bool {
        !self.is_preparing()
            && self
                .session
                .as_ref()
                .is_none_or(|s| s.state == DebugSessionState::Terminated)
    }

    /// Start a new debug session (blocks while a TCP adapter starts).
    pub fn start_session(
        &mut self,
        cfg: &DapConfig,
        workspace: &Path,
        current_file: Option<&Path>,
    ) -> anyhow::Result<()> {
        let transport = DapTransport::open(cfg, workspace)?;
        self.attach_session(cfg, transport, workspace, current_file);
        Ok(())
    }

    /// Start a session over a connected adapter.
    fn attach_session(
        &mut self,
        cfg: &DapConfig,
        transport: DapTransport,
        workspace: &Path,
        current_file: Option<&Path>,
    ) {
        // Expand every VS Code variable of the launch arguments up front.
        let mut cfg = cfg.clone();
        super::client::map_strings(&mut cfg.launch_config, &|s| {
            crate::runner::expand_variables(s, Some(workspace), current_file)
        });
        let mut client = DapClient::from_transport(transport, &cfg, workspace);
        for (file, lines) in &self.sorted_breakpoints() {
            client.set_breakpoints(file, lines);
        }
        client.set_watches(&self.watches);
        // Keep the preparation output (build log) above the session's.
        client.output_log = std::mem::take(&mut self.log);
        self.session = Some(client);
        self.child_sessions.clear();
        self.session_workspace = workspace.to_path_buf();
    }

    /// Stop the active debug session.
    pub fn stop_session(&mut self) {
        if let Some(p) = self.preparing.take() {
            p.cancel();
            push_log(&mut self.log, "Launch cancelled".into());
        }
        for child in self.child_sessions.iter_mut().rev() {
            child.disconnect();
        }
        self.child_sessions.clear();
        if let Some(sess) = &mut self.session {
            sess.disconnect();
        }
        self.session = None;
    }

    /// End every session and wait (briefly) for the adapters to exit, so
    /// none outlives the IDE. Call when quitting.
    pub fn shutdown(&mut self) {
        if let Some(p) = self.preparing.take() {
            p.cancel();
        }
        for sess in self
            .child_sessions
            .iter_mut()
            .rev()
            .chain(self.session.iter_mut())
        {
            sess.disconnect();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
        for sess in self
            .child_sessions
            .iter_mut()
            .chain(self.session.iter_mut())
        {
            sess.close(deadline);
        }
        self.child_sessions.clear();
        self.session = None;
    }

    /// Poll the sessions. Call every frame.
    pub fn poll(&mut self) {
        self.just_paused = false;
        self.poll_preparing();
        let mut paused = false;
        for sess in self.all_sessions_mut() {
            paused |= sess.poll();
        }
        self.just_paused = paused;
        self.open_child_sessions();
        // Program output arrives on child sessions: show it in one log.
        if let Some(root) = &mut self.session {
            for child in &mut self.child_sessions {
                root.output_log.append(&mut child.output_log);
            }
        }
    }

    /// Open the child sessions the adapter asked for (`startDebugging`).
    fn open_child_sessions(&mut self) {
        let mut requests = Vec::new();
        for sess in self.all_sessions_mut() {
            if let Some(port) = sess.port() {
                requests.extend(sess.take_child_requests().into_iter().map(|r| (port, r)));
            }
        }
        for (port, args) in requests {
            match DapClient::start_child(port, args, &self.session_workspace) {
                Ok(mut child) => {
                    for (file, lines) in &self.sorted_breakpoints() {
                        child.set_breakpoints(file, lines);
                    }
                    child.set_watches(&self.watches);
                    self.child_sessions.push(child);
                }
                Err(e) => {
                    if let Some(root) = &mut self.session {
                        root.output_log
                            .push(format!("[dap] could not open a child session: {e}"));
                    }
                }
            }
        }
    }

    /// Apply the progress of a preparation; start the session when ready.
    fn poll_preparing(&mut self) {
        let Some(p) = &self.preparing else {
            return;
        };
        let events: Vec<LaunchEvent> = p.events.try_iter().collect();
        let mut ready = None;
        let mut ended = false;
        for ev in events {
            match ev {
                LaunchEvent::Log(line) => push_log(&mut self.log, line),
                LaunchEvent::Ready(r) => ready = Some(r),
                LaunchEvent::Failed(e) => {
                    push_log(&mut self.log, format!("Debug launch failed: {e}"));
                    ended = true;
                }
            }
        }
        if ready.is_none() && !ended {
            return;
        }
        self.preparing = None;
        let (workspace, file) = std::mem::take(&mut self.preparing_for);
        if let Some(ready) = ready {
            let (cfg, transport) = *ready;
            self.attach_session(&cfg, transport, &workspace, file.as_deref());
        }
    }

    /// Whether a preparation (download, build task) is running.
    pub fn is_preparing(&self) -> bool {
        self.preparing.is_some()
    }

    pub fn is_running(&self) -> bool {
        self.is_preparing()
            || matches!(
                self.active().map(|s| &s.state),
                Some(DebugSessionState::Running) | Some(DebugSessionState::Launching)
            )
    }

    pub fn is_paused(&self) -> bool {
        matches!(
            self.active().map(|s| &s.state),
            Some(DebugSessionState::Paused { .. })
        )
    }

    pub fn is_active(&self) -> bool {
        self.session.is_some() || self.is_preparing()
    }

    pub fn paused_thread_id(&self) -> Option<i64> {
        match self.active()?.state {
            DebugSessionState::Paused { thread_id } => Some(thread_id),
            _ => None,
        }
    }

    pub fn call_stack(&self) -> &[StackFrame] {
        self.active()
            .map(|s| s.call_stack.as_slice())
            .unwrap_or(&[])
    }

    /// Index of the call-stack frame whose scopes are shown.
    pub fn selected_frame(&self) -> usize {
        self.active().map(|s| s.selected_frame).unwrap_or(0)
    }

    pub fn select_frame(&mut self, idx: usize) {
        if let Some(sess) = self.active_mut() {
            sess.select_frame(idx);
        }
    }

    /// Scopes (Locals, Globals…) of the selected frame.
    pub fn scopes(&self) -> &[Scope] {
        self.active().map(|s| s.scopes.as_slice()).unwrap_or(&[])
    }

    /// Fetched children of a scope or structured variable, `None` if not loaded.
    pub fn children(&self, variables_reference: i64) -> Option<&[Variable]> {
        self.active()?
            .children
            .get(&variables_reference)
            .map(|v| v.as_slice())
    }

    pub fn is_loading(&self, variables_reference: i64) -> bool {
        self.active()
            .is_some_and(|s| s.is_loading(variables_reference))
    }

    /// Lazily fetch the children of a scope or structured variable.
    pub fn request_variables(&mut self, variables_reference: i64) {
        if let Some(sess) = self.active_mut() {
            sess.request_variables(variables_reference);
        }
    }

    pub fn output_log(&self) -> &[String] {
        self.session
            .as_ref()
            .map(|s| s.output_log.as_slice())
            .unwrap_or(&self.log)
    }

    pub fn session_state(&self) -> DebugSessionState {
        if self.is_preparing() {
            return DebugSessionState::Preparing;
        }
        self.active()
            .map(|s| s.state.clone())
            .unwrap_or(DebugSessionState::Idle)
    }
}

/// Lines kept in the pre-session log.
const MAX_LOG_LINES: usize = 1000;

fn push_log(log: &mut Vec<String>, line: String) {
    if log.len() >= MAX_LOG_LINES {
        log.drain(..MAX_LOG_LINES / 10);
    }
    log.push(line);
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
            transport: Default::default(),
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
            transport: Default::default(),
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

    fn exiting_adapter() -> DapConfig {
        #[cfg(windows)]
        let (cmd, args) = ("cmd", vec!["/C".to_string(), "exit 0".to_string()]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c".to_string(), "exit 0".to_string()]);
        DapConfig {
            transport: Default::default(),
            adapter_cmd: cmd.into(),
            adapter_args: args,
            launch_config: json!({"program": "${workspaceFolder}/${fileBasenameNoExtension}"}),
        }
    }

    fn plan(dir: &Path, tasks: Vec<crate::runner::vscode_tasks::TaskStep>) -> LaunchPlan {
        LaunchPlan {
            config: exiting_adapter(),
            adapter: None,
            tasks,
            workspace: dir.to_path_buf(),
        }
    }

    fn task(line: &str) -> crate::runner::vscode_tasks::TaskStep {
        crate::runner::vscode_tasks::TaskStep {
            label: "build".into(),
            command: crate::runner::vscode_tasks::TaskCommand::Shell(line.into()),
            cwd: None,
            env: vec![],
        }
    }

    /// Poll until the preparation is over.
    fn settle(m: &mut DapManager) {
        let start = std::time::Instant::now();
        while m.is_preparing() {
            assert!(start.elapsed() < std::time::Duration::from_secs(30));
            std::thread::sleep(std::time::Duration::from_millis(20));
            m.poll();
        }
    }

    #[test]
    fn launch_runs_tasks_then_starts_the_session_with_their_output() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = DapManager::new();
        assert!(m.can_start());
        m.launch(
            plan(dir.path(), vec![task("echo built")]),
            Some(Path::new("/ws/app.cs")),
        );
        assert!(m.is_active() && m.is_running() && !m.can_start());
        assert_eq!(m.session_state(), DebugSessionState::Preparing);
        settle(&mut m);
        let sess = m.session.as_ref().expect("session started");
        let program = sess_launch_program(sess);
        assert!(program.ends_with("app"), "variables expanded: {program}");
        assert!(!program.contains("${"), "{program}");
        assert!(
            m.output_log().iter().any(|l| l == "built"),
            "{:?}",
            m.output_log()
        );
    }

    fn sess_launch_program(s: &DapClient) -> String {
        s.launch_config_for_test()["program"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn failed_task_reports_and_allows_a_new_launch() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = DapManager::new();
        m.launch(plan(dir.path(), vec![task("exit 2")]), None);
        settle(&mut m);
        assert!(!m.is_active());
        assert!(m.can_start());
        assert!(
            m.output_log()
                .iter()
                .any(|l| l.contains("task `build` failed")),
            "{:?}",
            m.output_log()
        );
    }

    #[test]
    fn stopping_a_preparation_cancels_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = DapManager::new();
        let slow = if cfg!(windows) {
            "ping -n 30 127.0.0.1"
        } else {
            "sleep 30"
        };
        m.launch(plan(dir.path(), vec![task(slow)]), None);
        m.stop_session();
        assert!(!m.is_active());
        assert_eq!(m.output_log().last().unwrap(), "Launch cancelled");
    }

    #[test]
    fn launch_failed_replaces_a_terminated_session() {
        let mut m = DapManager::new();
        let (_out, _tx, alive) = with_session(&mut m);
        assert!(!m.can_start(), "running session");
        alive.store(false, std::sync::atomic::Ordering::Relaxed);
        m.poll();
        assert!(m.can_start(), "terminated session can be replaced");
        m.launch_failed("No debugger");
        assert!(!m.is_active());
        assert_eq!(m.output_log(), ["No debugger"]);
    }

    /// End to end with a module's real debugger (downloaded when the module
    /// says so). `cargo test e2e_module_debugger -- --ignored` with:
    /// - `CU_E2E_MANIFEST`: the module's manifest.toml
    /// - `CU_E2E_FILE`: a source file whose line 2 runs and which prints
    ///   `x=42` (the program's output must reach the debug log)
    /// - `CU_E2E_LAUNCH` (optional): launch arguments as JSON, else the
    ///   module's `default_launch`
    /// - `CU_E2E_BUILD` (optional): shell command run first (build task)
    #[test]
    #[ignore]
    fn e2e_module_debugger_stops_at_breakpoint() {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let file = PathBuf::from(var("CU_E2E_FILE").unwrap());
        let project = file.parent().unwrap().to_path_buf();
        let modules = tempfile::tempdir().unwrap();
        let manifest = std::fs::read_to_string(var("CU_E2E_MANIFEST").unwrap()).unwrap();
        let module = crate::extension::registry::InstalledExtension {
            manifest: crate::extension::manifest::ExtensionManifest::parse(&manifest).unwrap(),
            path: modules.path().join("module"),
            lib_path: None,
            enabled: true,
            source: None,
            update_available: None,
        };
        let ext = file.extension().unwrap().to_string_lossy().to_string();
        let launch: Option<Value> = var("CU_E2E_LAUNCH").map(|j| serde_json::from_str(&j).unwrap());
        let adapter_type = launch
            .as_ref()
            .and_then(|l| l["type"].as_str().map(String::from));
        let adapter =
            crate::dap::adapters::DebugAdapter::find([&module], adapter_type.as_deref(), &ext)
                .expect("module debugger");
        let launch = launch
            .or_else(|| adapter.default_launch())
            .expect("launch arguments");
        let tasks = var("CU_E2E_BUILD")
            .map(|b| vec![task(&b)])
            .unwrap_or_default();
        let mut m = DapManager::new();
        m.toggle_breakpoint(&file, 2);
        m.launch(
            LaunchPlan {
                config: adapter.config(PathBuf::new(), launch),
                adapter: Some(adapter),
                tasks,
                workspace: project.clone(),
            },
            Some(&file),
        );
        let start = std::time::Instant::now();
        let wait = |m: &mut DapManager, done: &dyn Fn(&DapManager) -> bool, what: &str| {
            while !done(m) {
                assert!(
                    start.elapsed() < std::time::Duration::from_secs(240),
                    "{what}: {:?} {:?}",
                    m.session_state(),
                    m.output_log()
                );
                std::thread::sleep(std::time::Duration::from_millis(50));
                m.poll();
            }
        };
        wait(&mut m, &|m| m.is_paused(), "not paused");
        wait(
            &mut m,
            &|m| !m.call_stack().is_empty() && !m.scopes().is_empty(),
            "no stack",
        );
        assert_eq!(m.call_stack()[0].line, 2);
        eprintln!("stack: {:?}", &m.call_stack()[..1]);
        eprintln!("scopes: {:?}", m.scopes());
        let tid = m.paused_thread_id().unwrap();
        m.active_mut().unwrap().continue_execution(tid);
        wait(
            &mut m,
            &|m| m.output_log().iter().any(|l| l.contains("x=42")),
            "no program output",
        );
        m.shutdown();
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
