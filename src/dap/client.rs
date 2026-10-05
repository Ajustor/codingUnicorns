use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use serde_json::{json, Value};

use super::transport::DapTransport;
use super::types::{
    Breakpoint, DapConfig, DebugSessionState, Scope, StackFrame, Variable, WatchResult,
};

pub struct DapClient {
    transport: DapTransport,
    next_seq: u64,
    pub state: DebugSessionState,
    pub call_stack: Vec<StackFrame>,
    /// Index in `call_stack` of the frame whose scopes are shown.
    pub selected_frame: usize,
    /// Scopes of the selected frame, in adapter order.
    pub scopes: Vec<Scope>,
    /// Fetched children per `variablesReference` (scopes and structured
    /// variables share this namespace in DAP). Invalidated on every stop.
    pub children: HashMap<i64, Vec<Variable>>,
    /// Watch expressions, evaluated on every stop and frame change.
    watch_exprs: Vec<String>,
    /// Latest result per watch expression.
    pub watch_results: HashMap<String, WatchResult>,
    /// Pending `evaluate` requests: request seq → expression.
    pending_evals: HashMap<u64, String>,
    pub output_log: Vec<String>,
    workspace: PathBuf,
    launch_config: Value,
    /// seq of pending stackTrace request (to match the response).
    pending_stack_seq: Option<u64>,
    /// seq of pending scopes request.
    pending_scopes_seq: Option<u64>,
    /// Pending `variables` requests: request seq → variablesReference.
    pending_vars: HashMap<u64, i64>,
    /// Whether the adapter sent the `initialized` event.
    initialized: bool,
    /// Breakpoints that need to be sent after the `initialized` event.
    pending_breakpoints: Vec<(PathBuf, Vec<usize>)>,
    /// Whether `launch` has been sent. Per the DAP spec it goes out right after the
    /// `initialize` response; some adapters (e.g. debugpy) only emit `initialized`
    /// once they have received it.
    launch_sent: bool,
}

impl DapClient {
    /// Spawn the debug adapter and send `initialize`.
    pub fn start(cfg: &DapConfig, workspace: &Path) -> anyhow::Result<Self> {
        let args_ref: Vec<&str> = cfg.adapter_args.iter().map(|s| s.as_str()).collect();
        let workspace_str = workspace.to_string_lossy();
        let transport = DapTransport::spawn(&cfg.adapter_cmd, &args_ref, &workspace_str)?;
        Ok(Self::from_transport(transport, cfg, workspace))
    }

    /// Build a client on top of an already-connected transport and send `initialize`.
    fn from_transport(mut transport: DapTransport, cfg: &DapConfig, workspace: &Path) -> Self {
        // Substitute ${workspaceFolder} in launch_config (on parsed string
        // values, so backslashes in Windows paths need no JSON escaping).
        let mut launch_config = cfg.launch_config.clone();
        substitute_variable(
            &mut launch_config,
            "${workspaceFolder}",
            &workspace.to_string_lossy(),
        );

        // Send initialize request.
        let seq = 1u64;
        let _ = transport.send(&json!({
            "seq": seq,
            "type": "request",
            "command": "initialize",
            "arguments": {
                "clientID": "coding-unicorns",
                "clientName": "Coding Unicorns",
                "adapterID": "generic",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "supportsVariableType": true,
                "supportsRunInTerminalRequest": false
            }
        }));

        Self {
            transport,
            next_seq: seq + 1,
            state: DebugSessionState::Launching,
            call_stack: vec![],
            selected_frame: 0,
            scopes: vec![],
            children: HashMap::new(),
            watch_exprs: vec![],
            watch_results: HashMap::new(),
            pending_evals: HashMap::new(),
            output_log: vec![],
            workspace: workspace.to_path_buf(),
            launch_config,
            pending_stack_seq: None,
            pending_scopes_seq: None,
            pending_vars: HashMap::new(),
            initialized: false,
            pending_breakpoints: vec![],
            launch_sent: false,
        }
    }

    /// Test seam: build a client over an in-memory transport.
    #[cfg(test)]
    pub(crate) fn for_test(transport: DapTransport, cfg: &DapConfig, workspace: &Path) -> Self {
        Self::from_transport(transport, cfg, workspace)
    }

    fn next_seq(&mut self) -> u64 {
        let s = self.next_seq;
        self.next_seq += 1;
        s
    }

    /// Queue breakpoints for a file (sent once the adapter is initialized).
    pub fn set_breakpoints(&mut self, file: &Path, lines: &[usize]) {
        // Remove existing entry for this file then push new one.
        self.pending_breakpoints.retain(|(f, _)| f != file);
        if !lines.is_empty() {
            self.pending_breakpoints
                .push((file.to_path_buf(), lines.to_vec()));
        }
        if self.initialized {
            self.flush_breakpoints_for(file, lines);
        }
    }

    fn flush_breakpoints_for(&mut self, file: &Path, lines: &[usize]) {
        let bps: Vec<Value> = lines.iter().map(|l| json!({ "line": l })).collect();
        let seq = self.next_seq();
        let _ = self.transport.send(&json!({
            "seq": seq,
            "type": "request",
            "command": "setBreakpoints",
            "arguments": {
                "source": { "path": file.to_string_lossy(), "name": file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default() },
                "breakpoints": bps,
                "sourceModified": false
            }
        }));
    }

    fn send_configuration_done(&mut self) {
        let seq = self.next_seq();
        let _ = self.transport.send(&json!({
            "seq": seq,
            "type": "request",
            "command": "configurationDone"
        }));
    }

    fn send_launch(&mut self) {
        if self.launch_sent {
            return;
        }
        self.launch_sent = true;
        let seq = self.next_seq();
        let mut args = self.launch_config.clone();
        // Inject workspaceFolder if not already present.
        if args.get("cwd").is_none() {
            args["cwd"] = json!(self.workspace.to_string_lossy());
        }
        // launch.json entries may ask to attach to a running process instead.
        let command = if args["request"] == "attach" {
            "attach"
        } else {
            "launch"
        };
        let _ = self.transport.send(&json!({
            "seq": seq,
            "type": "request",
            "command": command,
            "arguments": args
        }));
    }

    pub fn continue_execution(&mut self, thread_id: i64) {
        let seq = self.next_seq();
        let _ = self.transport.send(&json!({
            "seq": seq,
            "type": "request",
            "command": "continue",
            "arguments": { "threadId": thread_id }
        }));
    }

    pub fn next_step(&mut self, thread_id: i64) {
        let seq = self.next_seq();
        let _ = self.transport.send(&json!({
            "seq": seq,
            "type": "request",
            "command": "next",
            "arguments": { "threadId": thread_id }
        }));
    }

    pub fn step_in(&mut self, thread_id: i64) {
        let seq = self.next_seq();
        let _ = self.transport.send(&json!({
            "seq": seq,
            "type": "request",
            "command": "stepIn",
            "arguments": { "threadId": thread_id }
        }));
    }

    pub fn step_out(&mut self, thread_id: i64) {
        let seq = self.next_seq();
        let _ = self.transport.send(&json!({
            "seq": seq,
            "type": "request",
            "command": "stepOut",
            "arguments": { "threadId": thread_id }
        }));
    }

    pub fn pause(&mut self, thread_id: i64) {
        let seq = self.next_seq();
        let _ = self.transport.send(&json!({
            "seq": seq,
            "type": "request",
            "command": "pause",
            "arguments": { "threadId": thread_id }
        }));
    }

    pub fn disconnect(&mut self) {
        let seq = self.next_seq();
        let _ = self.transport.send(&json!({
            "seq": seq,
            "type": "request",
            "command": "disconnect",
            "arguments": { "restart": false, "terminateDebuggee": true }
        }));
        self.state = DebugSessionState::Terminated;
    }

    /// Is the adapter process still alive?
    pub fn is_alive(&self) -> bool {
        self.transport.is_alive.load(Ordering::Relaxed)
    }

    /// Request the children of `variables_reference` unless they are already
    /// fetched or in flight. Used for lazy expansion of scopes and variables.
    pub fn request_variables(&mut self, variables_reference: i64) {
        if variables_reference <= 0
            || self.children.contains_key(&variables_reference)
            || self.is_loading(variables_reference)
        {
            return;
        }
        let seq = self.next_seq();
        self.pending_vars.insert(seq, variables_reference);
        let _ = self.transport.send(&json!({
            "seq": seq,
            "type": "request",
            "command": "variables",
            "arguments": { "variablesReference": variables_reference }
        }));
    }

    /// Whether a `variables` request for this reference is in flight.
    pub fn is_loading(&self, variables_reference: i64) -> bool {
        self.pending_vars
            .values()
            .any(|r| *r == variables_reference)
    }

    /// Show the scopes of another frame of the current call stack.
    pub fn select_frame(&mut self, idx: usize) {
        if idx >= self.call_stack.len() || !matches!(self.state, DebugSessionState::Paused { .. }) {
            return;
        }
        self.selected_frame = idx;
        self.request_scopes();
        self.evaluate_watches();
    }

    /// Replace the watch list. New expressions are evaluated right away when
    /// paused; results of removed ones are dropped.
    pub fn set_watches(&mut self, exprs: &[String]) {
        self.watch_results.retain(|e, _| exprs.contains(e));
        self.pending_evals.retain(|_, e| exprs.contains(e));
        let added: Vec<String> = exprs
            .iter()
            .filter(|e| !self.watch_exprs.contains(e))
            .cloned()
            .collect();
        self.watch_exprs = exprs.to_vec();
        for expr in added {
            self.evaluate_watch(expr);
        }
    }

    /// Re-evaluate every watch expression in the selected frame.
    fn evaluate_watches(&mut self) {
        self.pending_evals.clear();
        for expr in self.watch_exprs.clone() {
            self.evaluate_watch(expr);
        }
    }

    /// Send `evaluate` (context "watch") for one expression. Outside of a
    /// pause — or while the stack trace is still loading — it stays `Pending`
    /// and is evaluated once the frame is known.
    fn evaluate_watch(&mut self, expr: String) {
        let paused = matches!(self.state, DebugSessionState::Paused { .. });
        if !paused || self.pending_stack_seq.is_some() {
            self.watch_results
                .entry(expr)
                .or_insert(WatchResult::Pending);
            return;
        }
        let mut args = json!({ "expression": expr, "context": "watch" });
        if let Some(frame) = self.call_stack.get(self.selected_frame) {
            args["frameId"] = json!(frame.id);
        }
        let seq = self.next_seq();
        self.watch_results
            .insert(expr.clone(), WatchResult::Pending);
        self.pending_evals.insert(seq, expr);
        let _ = self.transport.send(&json!({
            "seq": seq,
            "type": "request",
            "command": "evaluate",
            "arguments": args
        }));
    }

    /// Forget scopes/variables of the previous frame or stop; late responses
    /// to the dropped requests are then ignored.
    fn clear_frame_data(&mut self) {
        self.scopes.clear();
        self.children.clear();
        self.pending_vars.clear();
        self.pending_scopes_seq = None;
    }

    /// Ask for the scopes of the selected frame (replacing the current ones).
    fn request_scopes(&mut self) {
        self.clear_frame_data();
        let Some(frame_id) = self.call_stack.get(self.selected_frame).map(|f| f.id) else {
            return;
        };
        let seq = self.next_seq();
        self.pending_scopes_seq = Some(seq);
        let _ = self.transport.send(&json!({
            "seq": seq,
            "type": "request",
            "command": "scopes",
            "arguments": { "frameId": frame_id }
        }));
    }

    /// Drain incoming messages and update internal state.
    /// Returns `true` if the session was just paused (caller may want to refresh the editor).
    pub fn poll(&mut self) -> bool {
        let mut just_paused = false;
        let msgs: Vec<Value> = self.transport.receiver.try_iter().collect();
        for msg in msgs {
            let msg_type = msg["type"].as_str().unwrap_or("");
            match msg_type {
                "event" => {
                    let event = msg["event"].as_str().unwrap_or("");
                    match event {
                        "initialized" => {
                            self.initialized = true;
                            // Adapters that emit `initialized` before answering
                            // `initialize` still need `launch` first (no-op if sent).
                            self.send_launch();
                            let bps = self.pending_breakpoints.clone();
                            for (file, lines) in &bps {
                                self.flush_breakpoints_for(file, lines);
                            }
                            self.send_configuration_done();
                        }
                        "stopped" => {
                            let thread_id = msg["body"]["threadId"].as_i64().unwrap_or(1);
                            self.state = DebugSessionState::Paused { thread_id };
                            just_paused = true;
                            // Variable references are only valid for one stop.
                            self.selected_frame = 0;
                            self.clear_frame_data();
                            self.pending_evals.clear();
                            // Request the call stack.
                            let seq = self.next_seq();
                            self.pending_stack_seq = Some(seq);
                            let _ = self.transport.send(&json!({
                                "seq": seq,
                                "type": "request",
                                "command": "stackTrace",
                                "arguments": { "threadId": thread_id, "startFrame": 0, "levels": 20 }
                            }));
                        }
                        "continued" => {
                            self.state = DebugSessionState::Running;
                        }
                        "terminated" | "exited" => {
                            self.state = DebugSessionState::Terminated;
                        }
                        "output" => {
                            if let Some(text) = msg["body"]["output"].as_str() {
                                // Limit log to last 500 lines.
                                if self.output_log.len() >= 500 {
                                    self.output_log.drain(..50);
                                }
                                for line in text.lines() {
                                    self.output_log.push(line.to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                "response" => {
                    let command = msg["command"].as_str().unwrap_or("");
                    let seq = msg["request_seq"].as_u64().unwrap_or(0);
                    match command {
                        "initialize" => {
                            if msg["success"].as_bool().unwrap_or(true) {
                                self.send_launch();
                            } else {
                                let err = msg["message"].as_str().unwrap_or("initialize failed");
                                self.output_log.push(format!("[dap] {err}"));
                                self.state = DebugSessionState::Terminated;
                            }
                        }
                        "stackTrace" if Some(seq) == self.pending_stack_seq => {
                            self.call_stack.clear();
                            if let Some(frames) = msg["body"]["stackFrames"].as_array() {
                                for f in frames {
                                    let file = f["source"]["path"].as_str().map(PathBuf::from);
                                    self.call_stack.push(StackFrame {
                                        id: f["id"].as_i64().unwrap_or(0),
                                        name: f["name"].as_str().unwrap_or("<unknown>").to_string(),
                                        file,
                                        line: f["line"].as_u64().unwrap_or(1) as usize,
                                    });
                                }
                            }
                            self.pending_stack_seq = None;
                            // Show the scopes of the top frame.
                            self.selected_frame = 0;
                            self.request_scopes();
                            self.evaluate_watches();
                        }
                        "evaluate" if self.pending_evals.contains_key(&seq) => {
                            let expr = self.pending_evals.remove(&seq).unwrap_or_default();
                            self.watch_results.insert(expr, parse_evaluate(&msg));
                        }
                        "scopes" if Some(seq) == self.pending_scopes_seq => {
                            self.pending_scopes_seq = None;
                            self.scopes = msg["body"]["scopes"]
                                .as_array()
                                .map(|a| a.iter().map(parse_scope).collect())
                                .unwrap_or_default();
                            // Eagerly fetch every cheap scope; expensive ones
                            // (e.g. Globals, Registers) wait for an expand.
                            let eager: Vec<i64> = self
                                .scopes
                                .iter()
                                .filter(|s| !s.expensive)
                                .map(|s| s.variables_reference)
                                .collect();
                            for r in eager {
                                self.request_variables(r);
                            }
                        }
                        "variables" if self.pending_vars.contains_key(&seq) => {
                            let vars_ref = self.pending_vars.remove(&seq).unwrap_or(0);
                            let vars: Vec<Variable> = msg["body"]["variables"]
                                .as_array()
                                .map(|a| a.iter().take(MAX_VARIABLES).map(parse_variable).collect())
                                .unwrap_or_default();
                            // A failed request stores an empty list so the UI stops
                            // showing "loading…" and does not retry every frame.
                            self.children.insert(vars_ref, vars);
                        }
                        "setBreakpoints" => {
                            // Update verified status (informational only for now).
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // If adapter process died unexpectedly, mark as terminated.
        if !self.is_alive() && self.state != DebugSessionState::Terminated {
            self.state = DebugSessionState::Terminated;
        }

        just_paused
    }

    /// Substitute `${file}` in the launch config with the given path.
    pub fn set_file_variable(&mut self, path: &Path) {
        substitute_variable(
            &mut self.launch_config,
            "${file}",
            path.to_string_lossy().as_ref(),
        );
    }
}

/// Children kept per `variables` response (protects the UI from huge arrays).
const MAX_VARIABLES: usize = 100;

fn parse_scope(s: &Value) -> Scope {
    Scope {
        name: s["name"].as_str().unwrap_or("Scope").to_string(),
        variables_reference: s["variablesReference"].as_i64().unwrap_or(0),
        expensive: s["expensive"].as_bool().unwrap_or(false),
    }
}

fn parse_variable(v: &Value) -> Variable {
    Variable {
        name: v["name"].as_str().unwrap_or("").to_string(),
        value: v["value"].as_str().unwrap_or("").to_string(),
        var_type: v["type"].as_str().map(|s| s.to_string()),
        variables_reference: v["variablesReference"].as_i64().unwrap_or(0),
    }
}

/// Turn an `evaluate` response into a watch result (errors shown inline).
fn parse_evaluate(msg: &Value) -> WatchResult {
    if msg["success"].as_bool().unwrap_or(true) {
        let body = &msg["body"];
        WatchResult::Value {
            value: body["result"].as_str().unwrap_or("").to_string(),
            var_type: body["type"].as_str().map(|s| s.to_string()),
            variables_reference: body["variablesReference"].as_i64().unwrap_or(0),
        }
    } else {
        let err = msg["body"]["error"]["format"]
            .as_str()
            .or_else(|| msg["message"].as_str())
            .unwrap_or("evaluation failed");
        WatchResult::Error(err.to_string())
    }
}

/// Replace `var` with `replacement` in every string value of a JSON tree.
/// Works on parsed values, so the replacement is never re-parsed as JSON.
fn substitute_variable(value: &mut Value, var: &str, replacement: &str) {
    match value {
        Value::String(s) if s.contains(var) => *s = s.replace(var, replacement),
        Value::Array(items) => {
            for item in items {
                substitute_variable(item, var, replacement);
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                substitute_variable(v, var, replacement);
            }
        }
        _ => {}
    }
}

/// Convert a `DapConfig` breakpoint list into `Breakpoint` structs.
pub fn make_breakpoints(file: &Path, lines: &[usize]) -> Vec<Breakpoint> {
    lines
        .iter()
        .map(|&line| Breakpoint {
            file: file.to_path_buf(),
            line,
            verified: false,
            id: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::{unbounded, Sender};
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

    struct Harness {
        client: DapClient,
        out: Shared,
        tx: Sender<Value>,
        alive: Arc<AtomicBool>,
    }

    impl Harness {
        /// Drain and decode everything the client wrote since the last call.
        fn sent(&self) -> Vec<Value> {
            let bytes = std::mem::take(&mut *self.out.0.lock().unwrap());
            let (tx, rx) = unbounded();
            crate::dap::transport::read_messages(std::io::Cursor::new(bytes), &tx);
            drop(tx);
            rx.try_iter().collect()
        }
        fn push(&self, v: Value) {
            self.tx.send(v).unwrap();
        }
    }

    fn cfg(launch: Value) -> DapConfig {
        DapConfig {
            adapter_cmd: "adapter".into(),
            adapter_args: vec![],
            launch_config: launch,
        }
    }

    fn harness_with(launch: Value) -> Harness {
        let out = Shared::default();
        let (tx, rx) = unbounded();
        let alive = Arc::new(AtomicBool::new(true));
        let transport = DapTransport::from_parts(Box::new(out.clone()), rx, alive.clone());
        let client = DapClient::for_test(transport, &cfg(launch), Path::new("/ws"));
        Harness {
            client,
            out,
            tx,
            alive,
        }
    }

    fn harness() -> Harness {
        harness_with(json!({"type": "test", "program": "${file}"}))
    }

    fn commands(msgs: &[Value]) -> Vec<String> {
        msgs.iter()
            .map(|m| m["command"].as_str().unwrap_or("").to_string())
            .collect()
    }

    #[test]
    fn start_sends_initialize_and_enters_launching() {
        let h = harness();
        let sent = h.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["seq"], 1);
        assert_eq!(sent[0]["type"], "request");
        assert_eq!(sent[0]["command"], "initialize");
        assert_eq!(sent[0]["arguments"]["linesStartAt1"], true);
        assert_eq!(sent[0]["arguments"]["clientID"], "coding-unicorns");
        assert_eq!(h.client.state, DebugSessionState::Launching);
        assert!(h.client.is_alive());
    }

    #[test]
    fn start_substitutes_workspace_folder() {
        let h = harness_with(json!({"cwd": "${workspaceFolder}/sub"}));
        assert_eq!(h.client.launch_config["cwd"], "/ws/sub");
    }

    #[test]
    fn substitution_walks_nested_values_and_keeps_special_chars() {
        let mut v = json!({
            "args": ["${file}", "--x=${file}", 3, null],
            "env": {"P": "${file}", "n": {"deep": "a ${file} b"}},
            "${file}": true
        });
        let p = r#"C:\dir "q"\a.py"#;
        substitute_variable(&mut v, "${file}", p);
        assert_eq!(v["args"][0], p);
        assert_eq!(v["args"][1], format!("--x={p}"));
        assert_eq!(v["args"][2], 3);
        assert_eq!(v["env"]["P"], p);
        assert_eq!(v["env"]["n"]["deep"], format!("a {p} b"));
        // Keys are left untouched.
        assert_eq!(v["${file}"], true);
    }

    #[test]
    fn substitution_handles_backslash_paths() {
        let (_tx, rx) = unbounded();
        let transport = DapTransport::from_parts(
            Box::new(Shared::default()),
            rx,
            Arc::new(AtomicBool::new(true)),
        );
        let mut client = DapClient::for_test(
            transport,
            &cfg(json!({"cwd": "${workspaceFolder}", "program": "${file}"})),
            Path::new(r"C:\Users\dev\proj"),
        );
        assert_eq!(client.launch_config["cwd"], r"C:\Users\dev\proj");
        client.set_file_variable(Path::new(r"C:\Users\dev\proj\main.py"));
        assert_eq!(
            client.launch_config["program"],
            r"C:\Users\dev\proj\main.py"
        );
    }

    #[test]
    fn start_with_missing_adapter_fails() {
        let dir = tempfile::tempdir().unwrap();
        let c = DapConfig {
            adapter_cmd: "definitely-not-a-real-dap-adapter-xyz".into(),
            adapter_args: vec!["--x".into()],
            launch_config: json!({}),
        };
        assert!(DapClient::start(&c, dir.path()).is_err());
    }

    #[test]
    fn seq_numbers_increase_per_request() {
        let mut h = harness();
        h.sent();
        h.client.continue_execution(3);
        h.client.next_step(3);
        h.client.step_in(3);
        h.client.step_out(3);
        h.client.pause(3);
        let sent = h.sent();
        assert_eq!(
            commands(&sent),
            vec!["continue", "next", "stepIn", "stepOut", "pause"]
        );
        let seqs: Vec<u64> = sent.iter().map(|m| m["seq"].as_u64().unwrap()).collect();
        assert_eq!(seqs, vec![2, 3, 4, 5, 6]);
        assert!(sent.iter().all(|m| m["arguments"]["threadId"] == 3));
    }

    #[test]
    fn disconnect_sends_request_and_terminates() {
        let mut h = harness();
        h.sent();
        h.client.disconnect();
        let sent = h.sent();
        assert_eq!(commands(&sent), vec!["disconnect"]);
        assert_eq!(sent[0]["arguments"]["terminateDebuggee"], true);
        assert_eq!(h.client.state, DebugSessionState::Terminated);
    }

    #[test]
    fn breakpoints_are_queued_until_initialized_then_flushed() {
        let mut h = harness();
        h.sent();
        h.client.set_breakpoints(Path::new("/ws/a.py"), &[3, 7]);
        h.client.set_breakpoints(Path::new("/ws/b.py"), &[1]);
        // Replacing a file's breakpoints keeps only the latest set.
        h.client.set_breakpoints(Path::new("/ws/a.py"), &[9]);
        // Clearing removes the file entirely.
        h.client.set_breakpoints(Path::new("/ws/b.py"), &[]);
        assert!(h.sent().is_empty(), "nothing sent before initialized");

        h.push(json!({"type": "event", "event": "initialized"}));
        assert!(!h.client.poll());
        let sent = h.sent();
        // No initialize response yet: launch still goes out before configuration.
        assert_eq!(
            commands(&sent),
            vec!["launch", "setBreakpoints", "configurationDone"]
        );
        let bp = &sent[1]["arguments"];
        assert_eq!(bp["source"]["name"], "a.py");
        assert_eq!(bp["breakpoints"], json!([{"line": 9}]));
        assert_eq!(bp["sourceModified"], false);
        // launch gets cwd injected from the workspace.
        assert_eq!(sent[0]["arguments"]["cwd"], "/ws");
        assert_eq!(sent[0]["arguments"]["type"], "test");
    }

    #[test]
    fn launch_is_sent_on_initialize_response_before_initialized_event() {
        // debugpy-style adapters only emit `initialized` after receiving `launch`.
        let mut h = harness();
        h.sent();
        h.client.set_breakpoints(Path::new("/ws/a.py"), &[3]);
        h.push(
            json!({"type": "response", "command": "initialize", "request_seq": 1, "success": true}),
        );
        h.client.poll();
        assert_eq!(commands(&h.sent()), vec!["launch"]);

        h.push(json!({"type": "event", "event": "initialized"}));
        h.client.poll();
        assert_eq!(
            commands(&h.sent()),
            vec!["setBreakpoints", "configurationDone"],
            "launch must not be sent twice"
        );
    }

    #[test]
    fn failed_initialize_terminates_without_launch() {
        let mut h = harness();
        h.sent();
        h.push(json!({"type": "response", "command": "initialize", "request_seq": 1, "success": false, "message": "boom"}));
        h.client.poll();
        assert!(h.sent().is_empty());
        assert_eq!(h.client.state, DebugSessionState::Terminated);
        assert!(h.client.output_log.iter().any(|l| l.contains("boom")));
    }

    #[test]
    fn breakpoints_after_initialized_are_sent_immediately() {
        let mut h = harness();
        h.push(json!({"type": "event", "event": "initialized"}));
        h.client.poll();
        h.sent();
        h.client.set_breakpoints(Path::new("/ws/c.rs"), &[4]);
        h.client.set_breakpoints(Path::new("/ws/c.rs"), &[]);
        let sent = h.sent();
        assert_eq!(commands(&sent), vec!["setBreakpoints", "setBreakpoints"]);
        assert_eq!(sent[0]["arguments"]["breakpoints"], json!([{"line": 4}]));
        assert_eq!(sent[1]["arguments"]["breakpoints"], json!([]));
    }

    #[test]
    fn attach_request_sends_attach_instead_of_launch() {
        let mut h = harness_with(json!({"request": "attach", "connect": {"port": 5678}}));
        h.sent();
        h.push(
            json!({"type": "response", "command": "initialize", "request_seq": 1, "success": true}),
        );
        h.client.poll();
        let sent = h.sent();
        assert_eq!(commands(&sent), vec!["attach"]);
        assert_eq!(sent[0]["arguments"]["connect"]["port"], 5678);
    }

    #[test]
    fn launch_keeps_explicit_cwd() {
        let mut h = harness_with(json!({"cwd": "/elsewhere"}));
        h.push(json!({"type": "event", "event": "initialized"}));
        h.client.poll();
        let sent = h.sent();
        let launch = sent.iter().find(|m| m["command"] == "launch").unwrap();
        assert_eq!(launch["arguments"]["cwd"], "/elsewhere");
    }

    #[test]
    fn set_file_variable_substitutes_placeholder() {
        let mut h = harness();
        h.client.set_file_variable(Path::new("/ws/main.py"));
        assert_eq!(h.client.launch_config["program"], "/ws/main.py");
    }

    #[test]
    fn set_file_variable_handles_quotes_in_path() {
        let mut h = harness();
        // Substitution works on parsed JSON strings, so a quote in the path is
        // substituted verbatim instead of breaking the config.
        h.client.set_file_variable(Path::new("/ws/we\"ird.py"));
        assert_eq!(h.client.launch_config["program"], "/ws/we\"ird.py");
    }

    #[test]
    fn full_stop_flow_fetches_stack_scopes_and_variables() {
        let mut h = harness();
        h.sent();
        h.push(json!({"type": "event", "event": "stopped", "body": {"threadId": 4}}));
        assert!(h.client.poll(), "stopped event reports just_paused");
        assert_eq!(h.client.state, DebugSessionState::Paused { thread_id: 4 });
        let sent = h.sent();
        assert_eq!(commands(&sent), vec!["stackTrace"]);
        assert_eq!(sent[0]["arguments"]["threadId"], 4);
        let stack_seq = sent[0]["seq"].as_u64().unwrap();

        h.push(json!({
            "type": "response", "command": "stackTrace", "request_seq": stack_seq,
            "body": {"stackFrames": [
                {"id": 11, "name": "main", "source": {"path": "/ws/main.py"}, "line": 12},
                {"id": 12}
            ]}
        }));
        assert!(!h.client.poll());
        assert_eq!(h.client.call_stack.len(), 2);
        assert_eq!(h.client.call_stack[0].name, "main");
        assert_eq!(
            h.client.call_stack[0].file.as_deref(),
            Some(Path::new("/ws/main.py"))
        );
        assert_eq!(h.client.call_stack[0].line, 12);
        assert_eq!(h.client.call_stack[1].name, "<unknown>");
        assert_eq!(h.client.call_stack[1].file, None);
        assert_eq!(h.client.call_stack[1].line, 1);
        let sent = h.sent();
        assert_eq!(commands(&sent), vec!["scopes"]);
        assert_eq!(sent[0]["arguments"]["frameId"], 11);
        let scopes_seq = sent[0]["seq"].as_u64().unwrap();

        h.push(json!({
            "type": "response", "command": "scopes", "request_seq": scopes_seq,
            "body": {"scopes": [{"name": "Locals", "variablesReference": 77}]}
        }));
        h.client.poll();
        let sent = h.sent();
        assert_eq!(commands(&sent), vec!["variables"]);
        assert_eq!(sent[0]["arguments"]["variablesReference"], 77);
        let vars_seq = sent[0]["seq"].as_u64().unwrap();

        h.push(json!({
            "type": "response", "command": "variables", "request_seq": vars_seq,
            "body": {"variables": [
                {"name": "x", "value": "1", "type": "int", "variablesReference": 0},
                {"name": "obj", "value": "{...}", "variablesReference": 5}
            ]}
        }));
        h.client.poll();
        let vars = &h.client.children[&77];
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[0].name, "x");
        assert_eq!(vars[0].var_type.as_deref(), Some("int"));
        assert_eq!(vars[1].variables_reference, 5);
        assert_eq!(vars[1].var_type, None);
        assert!(!h.client.is_loading(77));
        assert!(h.sent().is_empty());
    }

    /// Drive a stop up to the `scopes` response; returns the scopes seq.
    fn stop_with_frames(h: &mut Harness, frames: Value) -> u64 {
        h.sent();
        h.push(json!({"type": "event", "event": "stopped", "body": {"threadId": 1}}));
        h.client.poll();
        let stack_seq = h.sent()[0]["seq"].as_u64().unwrap();
        h.push(json!({
            "type": "response", "command": "stackTrace", "request_seq": stack_seq,
            "body": {"stackFrames": frames}
        }));
        h.client.poll();
        let sent = h.sent();
        assert_eq!(commands(&sent), vec!["scopes"]);
        sent[0]["seq"].as_u64().unwrap()
    }

    fn var_requests(sent: &[Value]) -> Vec<(u64, i64)> {
        sent.iter()
            .filter(|m| m["command"] == "variables")
            .map(|m| {
                (
                    m["seq"].as_u64().unwrap(),
                    m["arguments"]["variablesReference"].as_i64().unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn all_cheap_scopes_are_fetched_and_expensive_ones_lazily() {
        let mut h = harness();
        let scopes_seq = stop_with_frames(&mut h, json!([{"id": 1, "name": "f"}]));
        h.push(json!({
            "type": "response", "command": "scopes", "request_seq": scopes_seq,
            "body": {"scopes": [
                {"name": "Locals", "variablesReference": 10},
                {"name": "Globals", "variablesReference": 20, "expensive": true},
                {"name": "Registers", "variablesReference": 30, "expensive": false},
                {"name": "Empty", "variablesReference": 0}
            ]}
        }));
        h.client.poll();
        assert_eq!(h.client.scopes.len(), 4);
        assert_eq!(h.client.scopes[0].name, "Locals");
        assert!(h.client.scopes[1].expensive);
        assert!(!h.client.scopes[2].expensive);
        let reqs = var_requests(&h.sent());
        assert_eq!(
            reqs.iter().map(|r| r.1).collect::<Vec<_>>(),
            vec![10, 30],
            "expensive and empty scopes are not fetched eagerly"
        );
        assert!(h.client.is_loading(10) && h.client.is_loading(30));

        // Expanding the expensive scope fetches it once, even if asked twice.
        h.client.request_variables(20);
        h.client.request_variables(20);
        let lazy = var_requests(&h.sent());
        assert_eq!(lazy.len(), 1);
        assert_eq!(lazy[0].1, 20);

        // Responses land under their own reference, whatever the order.
        h.push(
            json!({"type": "response", "command": "variables", "request_seq": lazy[0].0,
            "body": {"variables": [{"name": "G", "value": "1"}]}}),
        );
        h.push(
            json!({"type": "response", "command": "variables", "request_seq": reqs[0].0,
            "body": {"variables": [{"name": "l", "value": "2", "variablesReference": 40}]}}),
        );
        h.client.poll();
        assert_eq!(h.client.children[&20][0].name, "G");
        assert_eq!(h.client.children[&10][0].name, "l");
        assert!(!h.client.children.contains_key(&30));

        // Already fetched → no new request.
        h.client.request_variables(10);
        h.client.request_variables(0);
        assert!(h.sent().is_empty());
    }

    #[test]
    fn structured_variables_expand_lazily_and_failures_stop_loading() {
        let mut h = harness();
        let scopes_seq = stop_with_frames(&mut h, json!([{"id": 1, "name": "f"}]));
        h.push(
            json!({"type": "response", "command": "scopes", "request_seq": scopes_seq,
            "body": {"scopes": [{"name": "Locals", "variablesReference": 10}]}}),
        );
        h.client.poll();
        h.sent();
        h.client.request_variables(40);
        let reqs = var_requests(&h.sent());
        assert_eq!(reqs[0].1, 40);
        h.push(
            json!({"type": "response", "command": "variables", "request_seq": reqs[0].0,
            "success": false, "message": "gone"}),
        );
        h.client.poll();
        assert_eq!(h.client.children.get(&40).map(|v| v.len()), Some(0));
        assert!(!h.client.is_loading(40));
    }

    #[test]
    fn new_stop_invalidates_variables_and_ignores_late_responses() {
        let mut h = harness();
        let scopes_seq = stop_with_frames(&mut h, json!([{"id": 1, "name": "f"}]));
        h.push(
            json!({"type": "response", "command": "scopes", "request_seq": scopes_seq,
            "body": {"scopes": [{"name": "Locals", "variablesReference": 10}]}}),
        );
        h.client.poll();
        let old = var_requests(&h.sent());
        h.client.children.insert(99, vec![]);

        h.push(json!({"type": "event", "event": "stopped", "body": {"threadId": 1}}));
        h.client.poll();
        assert!(h.client.scopes.is_empty());
        assert!(h.client.children.is_empty());
        // Late answer to the previous stop's request is dropped.
        h.push(
            json!({"type": "response", "command": "variables", "request_seq": old[0].0,
            "body": {"variables": [{"name": "stale"}]}}),
        );
        h.client.poll();
        assert!(h.client.children.is_empty());
    }

    fn evals(sent: &[Value]) -> Vec<&Value> {
        sent.iter().filter(|m| m["command"] == "evaluate").collect()
    }

    #[test]
    fn watches_are_pending_until_stopped_then_evaluated_in_top_frame() {
        let mut h = harness();
        h.sent();
        h.client
            .set_watches(&["a + 1".to_string(), "bad(".to_string()]);
        assert!(h.sent().is_empty(), "not paused → nothing evaluated");
        assert_eq!(h.client.watch_results["a + 1"], WatchResult::Pending);

        h.push(json!({"type": "event", "event": "stopped", "body": {"threadId": 1}}));
        h.client.poll();
        assert_eq!(commands(&h.sent()), vec!["stackTrace"]);
        // Added while the stack is loading → still deferred.
        h.client
            .set_watches(&["a + 1".to_string(), "bad(".to_string(), "c".to_string()]);
        assert!(h.sent().is_empty());
        let stack_seq = h.client.next_seq - 1;
        h.push(
            json!({"type": "response", "command": "stackTrace", "request_seq": stack_seq,
            "body": {"stackFrames": [{"id": 31, "name": "f"}, {"id": 32, "name": "g"}]}}),
        );
        h.client.poll();
        let sent = h.sent();
        let ev = evals(&sent);
        assert_eq!(ev.len(), 3);
        assert_eq!(ev[0]["arguments"]["expression"], "a + 1");
        assert_eq!(ev[0]["arguments"]["context"], "watch");
        assert_eq!(ev[0]["arguments"]["frameId"], 31);

        h.push(json!({"type": "response", "command": "evaluate",
            "request_seq": ev[0]["seq"], "success": true,
            "body": {"result": "42", "type": "int", "variablesReference": 0}}));
        h.push(json!({"type": "response", "command": "evaluate",
            "request_seq": ev[1]["seq"], "success": false, "message": "SyntaxError",
            "body": {"error": {"id": 1, "format": "invalid syntax"}}}));
        h.push(json!({"type": "response", "command": "evaluate",
            "request_seq": ev[2]["seq"], "success": false, "message": "not defined"}));
        h.client.poll();
        assert_eq!(
            h.client.watch_results["a + 1"],
            WatchResult::Value {
                value: "42".into(),
                var_type: Some("int".into()),
                variables_reference: 0
            }
        );
        assert_eq!(
            h.client.watch_results["bad("],
            WatchResult::Error("invalid syntax".into())
        );
        assert_eq!(
            h.client.watch_results["c"],
            WatchResult::Error("not defined".into())
        );

        // Selecting another frame re-evaluates there.
        h.client.select_frame(1);
        let sent = h.sent();
        let ev = evals(&sent);
        assert_eq!(ev.len(), 3);
        assert!(ev.iter().all(|m| m["arguments"]["frameId"] == 32));
    }

    #[test]
    fn watch_added_while_paused_is_evaluated_immediately_and_removal_drops_it() {
        let mut h = harness();
        let scopes_seq = stop_with_frames(&mut h, json!([{"id": 5, "name": "f"}]));
        let _ = scopes_seq;
        h.client.set_watches(&["x".to_string()]);
        let sent = h.sent();
        let ev = evals(&sent);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0]["arguments"]["frameId"], 5);
        let seq = ev[0]["seq"].clone();

        // Re-setting the same list does not re-evaluate.
        h.client.set_watches(&["x".to_string()]);
        assert!(h.sent().is_empty());

        // Removed before the answer → late response ignored.
        h.client.set_watches(&[]);
        h.push(
            json!({"type": "response", "command": "evaluate", "request_seq": seq,
            "body": {"result": "1"}}),
        );
        h.client.poll();
        assert!(h.client.watch_results.is_empty());
    }

    #[test]
    fn structured_watch_result_is_expandable() {
        let mut h = harness();
        stop_with_frames(&mut h, json!([{"id": 5, "name": "f"}]));
        h.client.set_watches(&["obj".to_string()]);
        let sent = h.sent();
        let seq = evals(&sent)[0]["seq"].clone();
        h.push(
            json!({"type": "response", "command": "evaluate", "request_seq": seq,
            "body": {"result": "{...}", "variablesReference": 70}}),
        );
        h.client.poll();
        assert!(matches!(
            h.client.watch_results["obj"],
            WatchResult::Value {
                variables_reference: 70,
                var_type: None,
                ..
            }
        ));
        h.client.request_variables(70);
        assert_eq!(var_requests(&h.sent())[0].1, 70);
    }

    #[test]
    fn selecting_a_frame_requests_its_scopes() {
        let mut h = harness();
        let scopes_seq = stop_with_frames(
            &mut h,
            json!([{"id": 1, "name": "top"}, {"id": 2, "name": "caller"}]),
        );
        h.push(
            json!({"type": "response", "command": "scopes", "request_seq": scopes_seq,
            "body": {"scopes": [{"name": "Locals", "variablesReference": 10}]}}),
        );
        h.client.poll();
        h.sent();

        h.client.select_frame(1);
        assert_eq!(h.client.selected_frame, 1);
        assert!(h.client.scopes.is_empty() && h.client.children.is_empty());
        let sent = h.sent();
        assert_eq!(commands(&sent), vec!["scopes"]);
        assert_eq!(sent[0]["arguments"]["frameId"], 2);
        // The previous frame's scopes response is now stale.
        h.push(
            json!({"type": "response", "command": "scopes", "request_seq": scopes_seq,
            "body": {"scopes": [{"name": "Old", "variablesReference": 5}]}}),
        );
        h.client.poll();
        assert!(h.client.scopes.is_empty());

        // Out of range or not paused → ignored.
        h.client.select_frame(7);
        assert_eq!(h.client.selected_frame, 1);
        h.client.state = DebugSessionState::Running;
        h.client.select_frame(0);
        assert_eq!(h.client.selected_frame, 1);
        assert!(h.sent().is_empty());
    }

    #[test]
    fn stopped_without_thread_defaults_to_one() {
        let mut h = harness();
        h.push(json!({"type": "event", "event": "stopped", "body": {}}));
        h.client.poll();
        assert_eq!(h.client.state, DebugSessionState::Paused { thread_id: 1 });
    }

    #[test]
    fn stale_responses_are_ignored() {
        let mut h = harness();
        h.push(json!({"type": "event", "event": "stopped", "body": {"threadId": 1}}));
        h.client.poll();
        h.sent();
        // Wrong request_seq → ignored, no scopes request.
        h.push(json!({
            "type": "response", "command": "stackTrace", "request_seq": 999,
            "body": {"stackFrames": [{"id": 1, "name": "f"}]}
        }));
        h.push(json!({"type": "response", "command": "scopes", "request_seq": 999}));
        h.push(
            json!({"type": "response", "command": "variables", "request_seq": 999,
            "body": {"variables": [{"name": "z"}]}}),
        );
        h.push(json!({"type": "response", "command": "setBreakpoints", "request_seq": 1}));
        h.push(json!({"type": "response", "command": "evaluate", "request_seq": 1}));
        h.push(json!({"type": "request", "command": "runInTerminal"}));
        h.push(json!({"type": "event", "event": "module"}));
        h.client.poll();
        assert!(h.client.call_stack.is_empty());
        assert!(h.client.scopes.is_empty());
        assert!(h.client.children.is_empty());
        assert!(h.sent().is_empty());
    }

    #[test]
    fn empty_stack_and_scope_without_vars_send_nothing_more() {
        let mut h = harness();
        h.push(json!({"type": "event", "event": "stopped", "body": {"threadId": 1}}));
        h.client.poll();
        let seq = h.sent()[0]["seq"].as_u64().unwrap();
        h.push(
            json!({"type": "response", "command": "stackTrace", "request_seq": seq,
            "body": {"stackFrames": []}}),
        );
        h.client.poll();
        assert!(h.sent().is_empty());
        assert!(h.client.call_stack.is_empty());

        // Scope with variablesReference 0 → no variables request.
        h.client.pending_scopes_seq = Some(50);
        h.push(
            json!({"type": "response", "command": "scopes", "request_seq": 50,
            "body": {"scopes": [{"variablesReference": 0}]}}),
        );
        h.client.poll();
        assert!(h.sent().is_empty());
        assert_eq!(h.client.pending_scopes_seq, None);
    }

    #[test]
    fn variables_are_capped_at_100() {
        let mut h = harness();
        h.client.pending_vars.insert(8, 3);
        let vars: Vec<Value> = (0..150)
            .map(|i| json!({"name": format!("v{i}"), "value": "0"}))
            .collect();
        h.push(
            json!({"type": "response", "command": "variables", "request_seq": 8,
            "body": {"variables": vars}}),
        );
        h.client.poll();
        assert_eq!(h.client.children[&3].len(), 100);
        assert_eq!(h.client.children[&3][99].name, "v99");
    }

    #[test]
    fn continued_terminated_exited_events_update_state() {
        let mut h = harness();
        h.push(json!({"type": "event", "event": "continued"}));
        h.client.poll();
        assert_eq!(h.client.state, DebugSessionState::Running);
        h.push(json!({"type": "event", "event": "terminated"}));
        h.client.poll();
        assert_eq!(h.client.state, DebugSessionState::Terminated);
        h.client.state = DebugSessionState::Running;
        h.push(json!({"type": "event", "event": "exited"}));
        h.client.poll();
        assert_eq!(h.client.state, DebugSessionState::Terminated);
    }

    #[test]
    fn output_events_are_split_into_lines_and_capped() {
        let mut h = harness();
        h.push(json!({"type": "event", "event": "output", "body": {"output": "a\nb\n"}}));
        h.push(json!({"type": "event", "event": "output", "body": {}}));
        h.client.poll();
        assert_eq!(h.client.output_log, vec!["a", "b"]);

        h.client.output_log = (0..500).map(|i| i.to_string()).collect();
        h.push(json!({"type": "event", "event": "output", "body": {"output": "new"}}));
        h.client.poll();
        assert_eq!(h.client.output_log.len(), 451);
        assert_eq!(h.client.output_log[0], "50");
        assert_eq!(h.client.output_log.last().unwrap(), "new");
    }

    #[test]
    fn adapter_death_terminates_session() {
        let mut h = harness();
        h.alive.store(false, Ordering::Relaxed);
        assert!(!h.client.is_alive());
        h.client.poll();
        assert_eq!(h.client.state, DebugSessionState::Terminated);
    }

    #[test]
    fn make_breakpoints_builds_unverified_entries() {
        let bps = make_breakpoints(Path::new("/f.rs"), &[1, 5]);
        assert_eq!(bps.len(), 2);
        assert_eq!(bps[1].line, 5);
        assert_eq!(bps[0].file, PathBuf::from("/f.rs"));
        assert!(bps.iter().all(|b| !b.verified && b.id.is_none()));
        assert!(make_breakpoints(Path::new("/f.rs"), &[]).is_empty());
    }
}
