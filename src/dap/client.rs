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
}

impl DapClient {
    /// Spawn the debug adapter and send `initialize`.
    pub fn start(cfg: &DapConfig, workspace: &Path) -> anyhow::Result<Self> {
        let args_ref: Vec<&str> = cfg.adapter_args.iter().map(|s| s.as_str()).collect();
        let workspace_str = workspace.to_string_lossy();
        let mut transport = DapTransport::spawn(&cfg.adapter_cmd, &args_ref, &workspace_str)?;

        // Substitute ${workspaceFolder} in launch_config.
        let launch_config_str = cfg.launch_config.to_string();
        let launch_config_str =
            launch_config_str.replace("${workspaceFolder}", &workspace.to_string_lossy());
        let launch_config: Value =
            serde_json::from_str(&launch_config_str).unwrap_or(cfg.launch_config.clone());

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

        Ok(Self {
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
        })
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
        let uri = format!("file://{}", file.display());
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
        let _ = uri;
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
                            // Send all pending breakpoints then launch.
                            let bps = self.pending_breakpoints.clone();
                            for (file, lines) in &bps {
                                self.flush_breakpoints_for(file, lines);
                            }
                            self.send_configuration_done();
                            self.send_launch();
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
        let s = self.launch_config.to_string();
        let s = s.replace("${file}", path.to_string_lossy().as_ref());
        if let Ok(v) = serde_json::from_str::<Value>(&s) {
            self.launch_config = v;
        }
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
