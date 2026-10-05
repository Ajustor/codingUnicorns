use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use serde_json::{json, Value};

use super::transport::DapTransport;
use super::types::{Breakpoint, DapConfig, DebugSessionState, StackFrame, Variable};

pub struct DapClient {
    transport: DapTransport,
    next_seq: u64,
    pub state: DebugSessionState,
    pub call_stack: Vec<StackFrame>,
    pub variables: Vec<Variable>,
    pub output_log: Vec<String>,
    workspace: PathBuf,
    launch_config: Value,
    /// seq of pending stackTrace request (to match the response).
    pending_stack_seq: Option<u64>,
    /// seq of pending scopes request.
    pending_scopes_seq: Option<u64>,
    /// seq of pending variables request.
    pending_vars_seq: Option<u64>,
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
            variables: vec![],
            output_log: vec![],
            workspace: workspace.to_path_buf(),
            launch_config,
            pending_stack_seq: None,
            pending_scopes_seq: None,
            pending_vars_seq: None,
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
        let _ = self.transport.send(&json!({
            "seq": seq,
            "type": "request",
            "command": "launch",
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
                            // Request scopes for the top frame.
                            if let Some(frame) = self.call_stack.first() {
                                let frame_id = frame.id;
                                let seq = self.next_seq();
                                self.pending_scopes_seq = Some(seq);
                                let _ = self.transport.send(&json!({
                                    "seq": seq,
                                    "type": "request",
                                    "command": "scopes",
                                    "arguments": { "frameId": frame_id }
                                }));
                            }
                        }
                        "scopes" if Some(seq) == self.pending_scopes_seq => {
                            self.pending_scopes_seq = None;
                            // Request variables for the first scope (locals).
                            if let Some(scope) =
                                msg["body"]["scopes"].as_array().and_then(|a| a.first())
                            {
                                let vars_ref = scope["variablesReference"].as_i64().unwrap_or(0);
                                if vars_ref > 0 {
                                    let seq = self.next_seq();
                                    self.pending_vars_seq = Some(seq);
                                    let _ = self.transport.send(&json!({
                                        "seq": seq,
                                        "type": "request",
                                        "command": "variables",
                                        "arguments": { "variablesReference": vars_ref }
                                    }));
                                }
                            }
                        }
                        "variables" if Some(seq) == self.pending_vars_seq => {
                            self.pending_vars_seq = None;
                            self.variables.clear();
                            if let Some(vars) = msg["body"]["variables"].as_array() {
                                for v in vars.iter().take(100) {
                                    self.variables.push(Variable {
                                        name: v["name"].as_str().unwrap_or("").to_string(),
                                        value: v["value"].as_str().unwrap_or("").to_string(),
                                        var_type: v["type"].as_str().map(|s| s.to_string()),
                                        variables_reference: v["variablesReference"]
                                            .as_i64()
                                            .unwrap_or(0),
                                    });
                                }
                            }
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
        assert_eq!(h.client.variables.len(), 2);
        assert_eq!(h.client.variables[0].name, "x");
        assert_eq!(h.client.variables[0].var_type.as_deref(), Some("int"));
        assert_eq!(h.client.variables[1].variables_reference, 5);
        assert_eq!(h.client.variables[1].var_type, None);
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
        assert!(h.client.variables.is_empty());
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
        h.client.pending_vars_seq = Some(8);
        let vars: Vec<Value> = (0..150)
            .map(|i| json!({"name": format!("v{i}"), "value": "0"}))
            .collect();
        h.push(
            json!({"type": "response", "command": "variables", "request_seq": 8,
            "body": {"variables": vars}}),
        );
        h.client.poll();
        assert_eq!(h.client.variables.len(), 100);
        assert_eq!(h.client.variables[99].name, "v99");
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
