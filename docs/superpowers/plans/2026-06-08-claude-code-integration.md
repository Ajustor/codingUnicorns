# Claude Code Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Project convention:** This repo does NOT auto-commit. The `Commit` steps below are written for completeness but must only be run when the user explicitly asks. Otherwise leave changes staged/unstaged and continue.

**Goal:** Add a native egui "Claude" chat panel (right dock) that drives the installed `claude` CLI as an agentic assistant with per-action permission confirmation.

**Architecture:** One `claude -p --output-format stream-json` process per turn, continuity via `--resume <session_id>`. Per-action confirmation via `--permission-prompt-tool`: Claude calls a host MCP tool; the editor re-invokes its own binary as the MCP server, which relays each request to the running editor over localhost TCP and returns the user's allow/deny.

**Tech Stack:** Rust, egui/eframe 0.31, `serde_json`, `crossbeam-channel`, `std::process`, `std::net` (all already in the dependency set). Reference spec: `docs/superpowers/specs/2026-06-08-claude-code-integration-design.md`.

---

## File Structure

- Create `src/claude/mod.rs` — module root; declares submodules; re-exports `ClaudeSession`, `ClaudeEvent`, `PermissionRequest`.
- Create `src/claude/protocol.rs` — stream-json event types + `parse_line`.
- Create `src/claude/session.rs` — transcript model, `build_prompt`, session_id capture, event application.
- Create `src/claude/process.rs` — spawn a turn, stdout reader thread, events channel, cancel.
- Create `src/claude/permission.rs` — editor-side IPC listener + MCP-server entrypoint + mcp-config writer.
- Create `src/ui/claude_panel.rs` — the right-dock egui panel.
- Modify `src/main.rs` — `mod claude;`, `--claude-permission-server` early dispatch.
- Modify `src/ui/mod.rs` — `pub mod claude_panel;`.
- Modify `src/config/mod.rs` — `claude_binary`, `claude_auto_allow_read` fields.
- Modify `src/app/mod.rs` — own session/panel state, poll events, drain permission requests, repaint.
- Modify `src/ui/layout.rs` — render right dock + toggle.
- Modify `src/ui/palette.rs` (+ keybindings) — `ToggleClaude` command.

---

## Task 1: stream-json protocol parser

**Files:**
- Create: `src/claude/protocol.rs`
- Test: inline `#[cfg(test)]` module in `src/claude/protocol.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// src/claude/protocol.rs
use serde_json::Value;

/// A parsed event from `claude -p --output-format stream-json --verbose`.
#[derive(Debug, Clone, PartialEq)]
pub enum ClaudeEvent {
    /// First event of a turn; carries the session id to reuse via `--resume`.
    Init { session_id: String },
    /// A complete assistant text block.
    AssistantText(String),
    /// Claude is invoking a tool.
    ToolUse { id: String, name: String, input: Value },
    /// Final event of the turn.
    Result { text: String, cost_usd: f64, session_id: Option<String> },
}

/// Parse one stdout line into zero or more events.
/// Unknown/irrelevant lines yield an empty vec (forward-compatible).
pub fn parse_line(line: &str) -> Vec<ClaudeEvent> {
    let line = line.trim();
    if line.is_empty() {
        return vec![];
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    match v.get("type").and_then(|t| t.as_str()) {
        Some("system") if v.get("subtype").and_then(|s| s.as_str()) == Some("init") => {
            match v.get("session_id").and_then(|s| s.as_str()) {
                Some(id) => vec![ClaudeEvent::Init { session_id: id.to_string() }],
                None => vec![],
            }
        }
        Some("assistant") => parse_assistant(&v),
        Some("result") => vec![ClaudeEvent::Result {
            text: v.get("result").and_then(|r| r.as_str()).unwrap_or("").to_string(),
            cost_usd: v.get("total_cost_usd").and_then(|c| c.as_f64()).unwrap_or(0.0),
            session_id: v.get("session_id").and_then(|s| s.as_str()).map(|s| s.to_string()),
        }],
        _ => vec![],
    }
}

fn parse_assistant(v: &Value) -> Vec<ClaudeEvent> {
    let Some(content) = v.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_array())
    else {
        return vec![];
    };
    let mut out = Vec::new();
    for block in content {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    if !t.is_empty() {
                        out.push(ClaudeEvent::AssistantText(t.to_string()));
                    }
                }
            }
            Some("tool_use") => out.push(ClaudeEvent::ToolUse {
                id: block.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string(),
                name: block.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                input: block.get("input").cloned().unwrap_or(Value::Null),
            }),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_init_session_id() {
        let line = r#"{"type":"system","subtype":"init","session_id":"abc123","model":"x"}"#;
        assert_eq!(parse_line(line), vec![ClaudeEvent::Init { session_id: "abc123".into() }]);
    }

    #[test]
    fn parses_assistant_text_and_tool_use() {
        let line = r#"{"type":"assistant","message":{"content":[
            {"type":"text","text":"Hello"},
            {"type":"tool_use","id":"t1","name":"Edit","input":{"file":"a.rs"}}
        ]}}"#;
        let evs = parse_line(line);
        assert_eq!(evs[0], ClaudeEvent::AssistantText("Hello".into()));
        match &evs[1] {
            ClaudeEvent::ToolUse { name, .. } => assert_eq!(name, "Edit"),
            other => panic!("expected tool_use, got {other:?}"),
        }
    }

    #[test]
    fn parses_result() {
        let line = r#"{"type":"result","result":"done","total_cost_usd":0.01,"session_id":"abc"}"#;
        assert_eq!(
            parse_line(line),
            vec![ClaudeEvent::Result { text: "done".into(), cost_usd: 0.01, session_id: Some("abc".into()) }]
        );
    }

    #[test]
    fn ignores_unknown_and_blank() {
        assert!(parse_line("").is_empty());
        assert!(parse_line("not json").is_empty());
        assert!(parse_line(r#"{"type":"stream_event"}"#).is_empty());
    }
}
```

- [ ] **Step 2: Register the module so it compiles.** Add to `src/claude/mod.rs` (created in this step):

```rust
// src/claude/mod.rs
pub mod protocol;
```

And add `mod claude;` to `src/main.rs` after the existing `mod` lines (near line 4-17).

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p coding-unicorns claude::protocol`
Expected: 4 tests pass.

- [ ] **Step 4: Commit** (only if the user asked)

```bash
git add src/claude/mod.rs src/claude/protocol.rs src/main.rs
git commit -m "feat(claude): stream-json protocol parser"
```

---

## Task 2: session model + prompt building

**Files:**
- Create: `src/claude/session.rs`
- Modify: `src/claude/mod.rs`
- Test: inline `#[cfg(test)]` in `src/claude/session.rs`

- [ ] **Step 1: Write the failing tests + implementation skeleton**

```rust
// src/claude/session.rs
use super::protocol::ClaudeEvent;

#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct EditorContext {
    /// Workspace-relative or absolute path of the current file, if any.
    pub current_file: Option<String>,
    /// Selected text, if any.
    pub selection: Option<String>,
    /// 1-based inclusive selection line range, if a selection exists.
    pub selection_lines: Option<(usize, usize)>,
}

#[derive(Default)]
pub struct ClaudeSession {
    pub transcript: Vec<Message>,
    pub session_id: Option<String>,
    pub running: bool,
    pub cost_usd: f64,
}

impl ClaudeSession {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the full prompt sent to `claude`: a compact context header (only the
    /// parts that exist) followed by the user's message.
    pub fn build_prompt(user_text: &str, ctx: &EditorContext) -> String {
        let mut header = String::new();
        if let Some(file) = &ctx.current_file {
            header.push_str(&format!("[current file: {file}]\n"));
        }
        if let (Some(sel), Some((a, b))) = (&ctx.selection, ctx.selection_lines) {
            header.push_str(&format!("[selection lines {a}-{b}]\n```\n{sel}\n```\n"));
        }
        if header.is_empty() {
            user_text.to_string()
        } else {
            format!("{header}\n{user_text}")
        }
    }

    /// Start a new conversation (drop session continuity + transcript).
    pub fn reset(&mut self) {
        self.transcript.clear();
        self.session_id = None;
        self.cost_usd = 0.0;
        self.running = false;
    }

    /// Apply one streamed event to the transcript / session state.
    pub fn apply(&mut self, ev: ClaudeEvent) {
        match ev {
            ClaudeEvent::Init { session_id } => {
                if self.session_id.is_none() {
                    self.session_id = Some(session_id);
                }
            }
            ClaudeEvent::AssistantText(text) => {
                self.transcript.push(Message { role: Role::Assistant, text });
            }
            ClaudeEvent::ToolUse { name, input, .. } => {
                self.transcript.push(Message {
                    role: Role::Tool,
                    text: format!("{name} {input}"),
                });
            }
            ClaudeEvent::Result { cost_usd, session_id, .. } => {
                self.cost_usd += cost_usd;
                if let Some(id) = session_id {
                    self.session_id = Some(id);
                }
                self.running = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_with_no_context_is_message_only() {
        let ctx = EditorContext::default();
        assert_eq!(ClaudeSession::build_prompt("hi", &ctx), "hi");
    }

    #[test]
    fn prompt_includes_file_and_selection() {
        let ctx = EditorContext {
            current_file: Some("src/a.rs".into()),
            selection: Some("let x = 1;".into()),
            selection_lines: Some((10, 10)),
        };
        let p = ClaudeSession::build_prompt("explain", &ctx);
        assert!(p.contains("[current file: src/a.rs]"));
        assert!(p.contains("[selection lines 10-10]"));
        assert!(p.contains("let x = 1;"));
        assert!(p.trim_end().ends_with("explain"));
    }

    #[test]
    fn init_sets_session_id_once() {
        let mut s = ClaudeSession::new();
        s.apply(ClaudeEvent::Init { session_id: "first".into() });
        s.apply(ClaudeEvent::Init { session_id: "second".into() });
        assert_eq!(s.session_id.as_deref(), Some("first"));
    }

    #[test]
    fn result_clears_running_and_adds_cost() {
        let mut s = ClaudeSession::new();
        s.running = true;
        s.apply(ClaudeEvent::Result { text: "ok".into(), cost_usd: 0.02, session_id: None });
        assert!(!s.running);
        assert!((s.cost_usd - 0.02).abs() < 1e-9);
    }
}
```

- [ ] **Step 2: Register the module.** Add to `src/claude/mod.rs`:

```rust
pub mod protocol;
pub mod session;

pub use session::{ClaudeSession, EditorContext, Message, Role};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p coding-unicorns claude::session`
Expected: 4 tests pass.

- [ ] **Step 4: Commit** (only if the user asked)

```bash
git add src/claude/session.rs src/claude/mod.rs
git commit -m "feat(claude): session model + prompt building"
```

---

## Task 3: turn process (spawn + stdout reader)

**Files:**
- Create: `src/claude/process.rs`
- Modify: `src/claude/mod.rs`

This task has no unit test (it spawns a real subprocess); it is verified by `cargo check` and later end-to-end. Mirror the thread+channel pattern in `src/lsp/transport.rs:18-76`.

- [ ] **Step 1: Implement the turn process**

```rust
// src/claude/process.rs
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use crossbeam_channel::{unbounded, Receiver};

use super::protocol::{parse_line, ClaudeEvent};

/// Parameters for one conversation turn.
pub struct TurnRequest {
    pub binary: String,
    pub prompt: String,
    pub workspace: std::path::PathBuf,
    pub session_id: Option<String>,
    /// localhost port the permission MCP server connects back to.
    pub perm_port: u16,
    /// shared secret the MCP server authenticates with.
    pub perm_token: String,
    /// path to the temp mcp-config file.
    pub mcp_config: std::path::PathBuf,
    /// path to the editor's own executable (re-invoked as the MCP server).
    pub self_exe: std::path::PathBuf,
}

/// A running turn: events stream over `rx`; `cancel()` kills the process.
pub struct Turn {
    pub rx: Receiver<ClaudeEvent>,
    child: Child,
}

impl Turn {
    pub fn cancel(&mut self) {
        let _ = self.child.kill();
    }
}

/// Spawn `claude` for one turn and stream parsed events.
pub fn spawn_turn(req: &TurnRequest) -> std::io::Result<Turn> {
    let mut cmd = Command::new(&req.binary);
    cmd.arg("-p")
        .arg(&req.prompt)
        .args(["--output-format", "stream-json", "--verbose"])
        .args(["--permission-prompt-tool", "mcp__editor__approve"])
        .arg("--mcp-config")
        .arg(&req.mcp_config)
        .arg("--add-dir")
        .arg(&req.workspace);
    if let Some(id) = &req.session_id {
        cmd.args(["--resume", id]);
    }
    cmd.current_dir(&req.workspace)
        .env("NO_COLOR", "1")
        .env("CU_PERM_PORT", req.perm_port.to_string())
        .env("CU_PERM_TOKEN", &req.perm_token)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (tx, rx) = unbounded::<ClaudeEvent>();

    // stdout → parsed events
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            for ev in parse_line(&line) {
                if tx.send(ev).is_err() {
                    return;
                }
            }
        }
    });

    // stderr → log (surfaced only on failure)
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            log::debug!("claude stderr: {line}");
        }
    });

    Ok(Turn { rx, child })
}
```

- [ ] **Step 2: Register the module.** Add `pub mod process;` to `src/claude/mod.rs`.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check`
Expected: `Finished` with no errors.

- [ ] **Step 4: Commit** (only if the user asked)

```bash
git add src/claude/process.rs src/claude/mod.rs
git commit -m "feat(claude): turn subprocess + stdout event reader"
```

---

## Task 4: permission IPC (editor side) + mcp-config

**Files:**
- Create: `src/claude/permission.rs`
- Modify: `src/claude/mod.rs`
- Test: inline `#[cfg(test)]` in `src/claude/permission.rs`

The editor listens on `127.0.0.1:0`. The MCP server (Task 5) connects, sends one
JSON line `{"token","tool","input"}`, and reads back one line `{"decision"}`.

- [ ] **Step 1: Implement the listener + wire types, with a round-trip test**

```rust
// src/claude/permission.rs
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender};

use serde_json::{json, Value};

/// A permission request surfaced to the UI.
pub struct PermissionRequest {
    pub tool: String,
    pub input: Value,
    /// Send the decision back to the waiting MCP server connection.
    pub reply: Sender<Decision>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    Allow,
    Deny,
}

/// Editor-side listener. Each inbound connection becomes a `PermissionRequest`
/// queued on `requests`; the UI resolves it and the reply is written back.
pub struct PermissionListener {
    pub port: u16,
    pub token: String,
    pub requests: Receiver<PermissionRequest>,
}

impl PermissionListener {
    /// Bind an ephemeral localhost port and start accepting permission requests.
    pub fn start(token: String) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let (tx, rx) = channel::<PermissionRequest>();
        let tok = token.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                let tok = tok.clone();
                std::thread::spawn(move || handle_conn(stream, &tok, &tx));
            }
        });
        Ok(Self { port, token, requests: rx })
    }
}

fn handle_conn(stream: TcpStream, token: &str, tx: &Sender<PermissionRequest>) {
    let mut writer = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
        return;
    };
    if v.get("token").and_then(|t| t.as_str()) != Some(token) {
        let _ = writeln!(writer, "{}", json!({"decision": "deny"}));
        return;
    }
    let (reply_tx, reply_rx) = channel::<Decision>();
    let req = PermissionRequest {
        tool: v.get("tool").and_then(|t| t.as_str()).unwrap_or("").to_string(),
        input: v.get("input").cloned().unwrap_or(Value::Null),
        reply: reply_tx,
    };
    if tx.send(req).is_err() {
        let _ = writeln!(writer, "{}", json!({"decision": "deny"}));
        return;
    }
    // Wait for the UI decision (default deny after 5 minutes).
    let decision = reply_rx
        .recv_timeout(std::time::Duration::from_secs(300))
        .unwrap_or(Decision::Deny);
    let s = match decision {
        Decision::Allow => "allow",
        Decision::Deny => "deny",
    };
    let _ = writeln!(writer, "{}", json!({"decision": s}));
}

/// Write the temp mcp-config that points `claude` at this editor binary running
/// as the permission MCP server. Returns the config path.
pub fn write_mcp_config(
    dir: &std::path::Path,
    self_exe: &std::path::Path,
    port: u16,
) -> std::io::Result<std::path::PathBuf> {
    let cfg = json!({
        "mcpServers": {
            "editor": {
                "command": self_exe.to_string_lossy(),
                "args": ["--claude-permission-server", "--port", port.to_string()]
            }
        }
    });
    let path = dir.join("cu-claude-mcp.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&cfg)?)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn allow_round_trip() {
        let listener = PermissionListener::start("secret".into()).unwrap();
        let port = listener.port;

        // Simulate the MCP server connecting.
        let client = std::thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            writeln!(s, "{}", json!({"token":"secret","tool":"Edit","input":{}})).unwrap();
            let mut reader = BufReader::new(s);
            let mut resp = String::new();
            reader.read_line(&mut resp).unwrap();
            resp
        });

        // Editor side: receive request, allow it.
        let req = listener.requests.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(req.tool, "Edit");
        req.reply.send(Decision::Allow).unwrap();

        let resp = client.join().unwrap();
        assert!(resp.contains("allow"));
    }

    #[test]
    fn wrong_token_denied() {
        let listener = PermissionListener::start("secret".into()).unwrap();
        let port = listener.port;
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        writeln!(s, "{}", json!({"token":"wrong","tool":"Edit","input":{}})).unwrap();
        let mut reader = BufReader::new(s);
        let mut resp = String::new();
        reader.read_line(&mut resp).unwrap();
        assert!(resp.contains("deny"));
    }
}
```

- [ ] **Step 2: Register the module.** Add `pub mod permission;` to `src/claude/mod.rs`.

- [ ] **Step 3: Run tests**

Run: `cargo test -p coding-unicorns claude::permission`
Expected: 2 tests pass.

- [ ] **Step 4: Commit** (only if the user asked)

```bash
git add src/claude/permission.rs src/claude/mod.rs
git commit -m "feat(claude): permission IPC listener + mcp-config"
```

---

## Task 5: permission MCP server (stdio JSON-RPC)

**Files:**
- Modify: `src/claude/permission.rs` (add the server entrypoint)

The editor binary, when launched with `--claude-permission-server --port P` (and
env `CU_PERM_TOKEN`), runs a minimal MCP server over stdio exposing one tool,
`approve`. On `tools/call` it connects to the editor's listener on port P, sends
the request, and returns the `--permission-prompt-tool` result.

> **Version note (highest-risk step):** verify the exact `tools/call` argument
> shape Claude sends to the permission tool and the result shape it expects, by
> running once with `claude --debug` against this server. The schema below
> matches Claude Code 2.1.x: arguments contain `tool_name` + `input`; the result
> text must be JSON `{"behavior":"allow","updatedInput":<input>}` or
> `{"behavior":"deny","message":"..."}`.

- [ ] **Step 1: Implement the server entrypoint**

```rust
// src/claude/permission.rs (append)

/// Entry point when the binary is re-invoked as the permission MCP server.
/// Speaks minimal MCP (JSON-RPC 2.0) over stdio; blocks until stdin closes.
pub fn run_permission_mcp_server(port: u16, token: String) {
    use std::io::BufRead;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let Ok(req) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let resp = match method {
            "initialize" => Some(json!({
                "jsonrpc":"2.0","id":id,
                "result":{
                    "protocolVersion":"2024-11-05",
                    "capabilities":{"tools":{}},
                    "serverInfo":{"name":"editor","version":"0.1.0"}
                }
            })),
            "tools/list" => Some(json!({
                "jsonrpc":"2.0","id":id,
                "result":{"tools":[{
                    "name":"approve",
                    "description":"Ask the editor user to approve a tool use.",
                    "inputSchema":{"type":"object","properties":{
                        "tool_name":{"type":"string"},
                        "input":{"type":"object"}
                    }}
                }]}
            })),
            "tools/call" => {
                let args = req.get("params").and_then(|p| p.get("arguments"));
                let tool = args.and_then(|a| a.get("tool_name")).and_then(|t| t.as_str()).unwrap_or("");
                let input = args.and_then(|a| a.get("input")).cloned().unwrap_or(Value::Null);
                let decision = ask_editor(port, &token, tool, &input);
                let payload = match decision {
                    Decision::Allow => json!({"behavior":"allow","updatedInput": input}),
                    Decision::Deny => json!({"behavior":"deny","message":"Denied by user"}),
                };
                Some(json!({
                    "jsonrpc":"2.0","id":id,
                    "result":{"content":[{"type":"text","text": payload.to_string()}]}
                }))
            }
            // notifications (no id) — no response
            _ => None,
        };
        if let Some(r) = resp {
            let _ = writeln!(stdout, "{r}");
            let _ = stdout.flush();
        }
    }
}

/// Connect to the editor listener and block for the user's decision.
fn ask_editor(port: u16, token: &str, tool: &str, input: &Value) -> Decision {
    let Ok(stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return Decision::Deny;
    };
    let mut writer = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return Decision::Deny,
    };
    let req = json!({"token": token, "tool": tool, "input": input});
    if writeln!(writer, "{req}").is_err() {
        return Decision::Deny;
    }
    let mut reader = BufReader::new(stream);
    let mut resp = String::new();
    if reader.read_line(&mut resp).is_err() {
        return Decision::Deny;
    }
    match serde_json::from_str::<Value>(resp.trim()).ok()
        .and_then(|v| v.get("decision").and_then(|d| d.as_str()).map(str::to_string))
        .as_deref()
    {
        Some("allow") => Decision::Allow,
        _ => Decision::Deny,
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check`
Expected: `Finished` with no errors.

- [ ] **Step 3: Commit** (only if the user asked)

```bash
git add src/claude/permission.rs
git commit -m "feat(claude): minimal MCP permission server over stdio"
```

---

## Task 6: binary entrypoint dispatch

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Dispatch the permission-server mode before the GUI**

Insert at the very top of `fn main()` in `src/main.rs`, before `install_panic_logger();`:

```rust
    // Re-invoked by `claude` as the permission MCP server — no GUI.
    let raw_args: Vec<String> = std::env::args().collect();
    if raw_args.iter().any(|a| a == "--claude-permission-server") {
        let port = raw_args
            .iter()
            .position(|a| a == "--port")
            .and_then(|i| raw_args.get(i + 1))
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(0);
        let token = std::env::var("CU_PERM_TOKEN").unwrap_or_default();
        if port != 0 {
            claude::permission::run_permission_mcp_server(port, token);
        }
        return Ok(());
    }
```

Note: `fn main() -> eframe::Result<()>`, so `return Ok(())` is valid.

- [ ] **Step 2: Verify it compiles**

Run: `cargo check`
Expected: `Finished` with no errors.

- [ ] **Step 3: Commit** (only if the user asked)

```bash
git add src/main.rs
git commit -m "feat(claude): permission-server binary dispatch"
```

---

## Task 7: config fields

**Files:**
- Modify: `src/config/mod.rs`

- [ ] **Step 1: Add fields with serde defaults**

Find the main `Config` struct in `src/config/mod.rs`. Add these fields (with the
existing `#[serde(default)]` convention used elsewhere in that struct):

```rust
    /// Command used to launch Claude Code (default "claude").
    #[serde(default = "default_claude_binary")]
    pub claude_binary: String,
    /// Auto-approve read-only tools (Read/Glob/Grep) without a prompt.
    #[serde(default = "default_true")]
    pub claude_auto_allow_read: bool,
```

Add the default helpers near the other `default_*` fns in the file:

```rust
fn default_claude_binary() -> String {
    "claude".to_string()
}
fn default_true() -> bool {
    true
}
```

If `default_true` already exists in the file, reuse it (do not duplicate). In the
`Default for Config` impl (or `Config::default`-style constructor), set
`claude_binary: "claude".to_string()` and `claude_auto_allow_read: true`.

- [ ] **Step 2: Verify it compiles**

Run: `cargo check`
Expected: `Finished`.

- [ ] **Step 3: Commit** (only if the user asked)

```bash
git add src/config/mod.rs
git commit -m "feat(claude): config (binary path, auto-allow read)"
```

---

## Task 8: the chat panel UI

**Files:**
- Create: `src/ui/claude_panel.rs`
- Modify: `src/ui/mod.rs` (add `pub mod claude_panel;`)

The panel renders transcript + input + an inline permission dialog. It holds only
UI state; it borrows `&mut ClaudeSession` and a current `Option<PermissionRequest>`
from the app. Sending a message is signalled back to the app via a return value
(the app owns process spawning).

- [ ] **Step 1: Implement the panel**

```rust
// src/ui/claude_panel.rs
use crate::claude::permission::{Decision, PermissionRequest};
use crate::claude::session::{ClaudeSession, Role};

#[derive(Default)]
pub struct ClaudePanel {
    pub input: String,
}

/// What the panel asks the app to do after a frame.
pub enum ClaudeAction {
    None,
    Send(String),
    NewConversation,
    Cancel,
    Permission(Decision),
}

impl ClaudePanel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &ClaudeSession,
        pending: Option<&PermissionRequest>,
    ) -> ClaudeAction {
        let mut action = ClaudeAction::None;

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Claude").strong().color(egui::Color32::WHITE));
            if session.running {
                ui.spinner();
                if ui.small_button("Cancel").clicked() {
                    action = ClaudeAction::Cancel;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("New").clicked() {
                    action = ClaudeAction::NewConversation;
                }
                if session.cost_usd > 0.0 {
                    ui.label(
                        egui::RichText::new(format!("${:.4}", session.cost_usd))
                            .small()
                            .color(egui::Color32::from_gray(150)),
                    );
                }
            });
        });
        ui.separator();

        // Transcript (scrolls, sticks to bottom).
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .max_height(ui.available_height() - 80.0)
            .show(ui, |ui| {
                for msg in &session.transcript {
                    let (prefix, color) = match msg.role {
                        Role::User => ("you", egui::Color32::from_rgb(120, 170, 255)),
                        Role::Assistant => ("claude", egui::Color32::from_rgb(180, 230, 180)),
                        Role::Tool => ("tool", egui::Color32::from_gray(150)),
                    };
                    ui.label(egui::RichText::new(prefix).small().color(color));
                    ui.label(egui::RichText::new(&msg.text).color(egui::Color32::from_gray(220)));
                    ui.add_space(4.0);
                }
            });

        // Inline permission dialog.
        if let Some(req) = pending {
            ui.separator();
            ui.label(
                egui::RichText::new(format!("Allow tool: {}?", req.tool))
                    .strong()
                    .color(egui::Color32::from_rgb(255, 200, 60)),
            );
            ui.label(
                egui::RichText::new(req.input.to_string())
                    .small()
                    .color(egui::Color32::from_gray(180)),
            );
            ui.horizontal(|ui| {
                if ui.button("Allow").clicked() {
                    action = ClaudeAction::Permission(Decision::Allow);
                }
                if ui.button("Deny").clicked() {
                    action = ClaudeAction::Permission(Decision::Deny);
                }
            });
        }

        // Input box.
        ui.separator();
        let resp = ui.add_enabled(
            !session.running,
            egui::TextEdit::multiline(&mut self.input)
                .desired_rows(2)
                .hint_text("Ask Claude…  (Enter to send, Shift+Enter for newline)")
                .desired_width(f32::INFINITY),
        );
        let send = resp.has_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
        if send && !self.input.trim().is_empty() {
            let text = std::mem::take(&mut self.input);
            action = ClaudeAction::Send(text.trim().to_string());
        }

        action
    }
}
```

- [ ] **Step 2: Register + compile**

Add `pub mod claude_panel;` to `src/ui/mod.rs`. Run: `cargo check`
Expected: `Finished` (the panel is not yet wired; unused-warnings are fine under the crate's `#![allow(dead_code)]`).

- [ ] **Step 3: Commit** (only if the user asked)

```bash
git add src/ui/claude_panel.rs src/ui/mod.rs
git commit -m "feat(claude): right-dock chat panel UI"
```

---

## Task 9: app wiring (state, events, permission draining)

**Files:**
- Modify: `src/app/mod.rs`

- [ ] **Step 1: Add fields to the app struct**

In the `CodingUnicorns` struct (near `show_terminal` etc.), add:

```rust
    pub show_claude: bool,
    pub claude_session: crate::claude::session::ClaudeSession,
    pub claude_panel: crate::ui::claude_panel::ClaudePanel,
    pub claude_turn: Option<crate::claude::process::Turn>,
    pub claude_perm: Option<crate::claude::permission::PermissionListener>,
    pub claude_pending: Option<crate::claude::permission::PermissionRequest>,
```

In the constructor (near `show_terminal: true`), initialise:

```rust
            show_claude: false,
            claude_session: crate::claude::session::ClaudeSession::new(),
            claude_panel: crate::ui::claude_panel::ClaudePanel::new(),
            claude_turn: None,
            claude_perm: None,
            claude_pending: None,
```

- [ ] **Step 2: Add the per-frame poll + a `start_claude_turn` helper**

Add this method to `impl CodingUnicorns` (a new `src/app/claude_ops.rs` is
optional; inline in `app/mod.rs` is fine to start):

```rust
    pub(crate) fn poll_claude(&mut self, ctx: &egui::Context) {
        // Drain streamed events from the active turn.
        if let Some(turn) = &self.claude_turn {
            let events: Vec<_> = turn.rx.try_iter().collect();
            for ev in events {
                self.claude_session.apply(ev);
            }
            ctx.request_repaint();
        }
        // Pick up a pending permission request (if none is currently shown).
        if self.claude_pending.is_none() {
            if let Some(perm) = &self.claude_perm {
                if let Ok(req) = perm.requests.try_recv() {
                    self.claude_pending = Some(req);
                    ctx.request_repaint();
                }
            }
        }
        // Drop the finished turn handle.
        if !self.claude_session.running {
            self.claude_turn = None;
        }
    }

    pub(crate) fn start_claude_turn(&mut self, user_text: String) {
        use crate::claude::{permission, process, session::EditorContext};

        let Some(workspace) = self.workspace_path.clone() else {
            return;
        };
        // Ensure a permission listener exists for this session.
        if self.claude_perm.is_none() {
            let token = uuid::Uuid::new_v4().to_string();
            match permission::PermissionListener::start(token) {
                Ok(l) => self.claude_perm = Some(l),
                Err(e) => {
                    log::error!("claude permission listener: {e}");
                    return;
                }
            }
        }
        let perm = self.claude_perm.as_ref().unwrap();
        let self_exe = std::env::current_exe().unwrap_or_default();
        let mcp_config = match permission::write_mcp_config(
            &std::env::temp_dir(),
            &self_exe,
            perm.port,
        ) {
            Ok(p) => p,
            Err(e) => {
                log::error!("claude mcp-config: {e}");
                return;
            }
        };

        // Build context header from the editor.
        let ctx = EditorContext {
            current_file: self
                .editor
                .current_path
                .as_ref()
                .map(|p| p.display().to_string()),
            selection: self.editor.selected_text_pub(),
            selection_lines: self.editor.selection_line_range_pub(),
        };
        let prompt = crate::claude::session::ClaudeSession::build_prompt(&user_text, &ctx);

        self.claude_session.transcript.push(crate::claude::session::Message {
            role: crate::claude::session::Role::User,
            text: user_text,
        });
        self.claude_session.running = true;

        let req = process::TurnRequest {
            binary: self.config.claude_binary.clone(),
            prompt,
            workspace,
            session_id: self.claude_session.session_id.clone(),
            perm_port: perm.port,
            perm_token: perm.token.clone(),
            mcp_config,
            self_exe,
        };
        match process::spawn_turn(&req) {
            Ok(turn) => self.claude_turn = Some(turn),
            Err(e) => {
                self.claude_session.running = false;
                self.claude_session.transcript.push(crate::claude::session::Message {
                    role: crate::claude::session::Role::Assistant,
                    text: format!("Failed to launch `{}`: {e}. Is Claude Code installed and on PATH?", self.config.claude_binary),
                });
            }
        }
    }
```

- [ ] **Step 3: Add the editor helpers used above**

In `src/editor/mod.rs` (or `word_analysis.rs`), add two public helpers that
expose the current selection (the editor already tracks selection via
`cursor.selection_range()` — see `src/editor/cursor.rs`):

```rust
    /// The currently selected text, if any.
    pub fn selected_text_pub(&self) -> Option<String> {
        let ((sr, sc), (er, ec)) = self.cursor.selection_range()?;
        let start = self.buffer.char_index(sr, sc);
        let end = self.buffer.char_index(er, ec);
        Some(self.buffer.rope_slice(start, end))
    }

    /// The 1-based inclusive line range of the selection, if any.
    pub fn selection_line_range_pub(&self) -> Option<(usize, usize)> {
        let ((sr, _), (er, _)) = self.cursor.selection_range()?;
        Some((sr + 1, er + 1))
    }
```

Verify `selection_range()` returns `Option<((usize,usize),(usize,usize))>` (it is
used in `src/editor/multi_cursor.rs:52`); adapt destructuring if the tuple shape
differs.

- [ ] **Step 4: Call `poll_claude` from the update loop**

In the eframe `update` method of `app/mod.rs`, just before the terminal repaint
block (near line 1018, `if self.show_terminal {`), add:

```rust
        self.poll_claude(ctx);
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check`
Expected: `Finished`. Fix any selection-API mismatch surfaced here.

- [ ] **Step 6: Commit** (only if the user asked)

```bash
git add src/app/mod.rs src/editor/mod.rs
git commit -m "feat(claude): app state, event polling, turn launch"
```

---

## Task 10: render the dock + toggle command

**Files:**
- Modify: `src/ui/layout.rs`
- Modify: `src/ui/palette.rs` (+ command enum + keybinding)

- [ ] **Step 1: Render the right dock**

In `src/ui/layout.rs`, before the `CentralPanel` that hosts the editor (the
big block around line 800+), add:

```rust
    if app.show_claude {
        egui::SidePanel::right("claude_panel")
            .resizable(true)
            .default_width(360.0)
            .min_width(260.0)
            .show(ctx, |ui| {
                let pending = app.claude_pending.as_ref();
                let action = app.claude_panel.show(ui, &app.claude_session, pending);
                match action {
                    crate::ui::claude_panel::ClaudeAction::Send(text) => {
                        app.start_claude_turn(text);
                    }
                    crate::ui::claude_panel::ClaudeAction::NewConversation => {
                        app.claude_session.reset();
                    }
                    crate::ui::claude_panel::ClaudeAction::Cancel => {
                        if let Some(turn) = &mut app.claude_turn {
                            turn.cancel();
                        }
                        app.claude_session.running = false;
                    }
                    crate::ui::claude_panel::ClaudeAction::Permission(decision) => {
                        if let Some(req) = app.claude_pending.take() {
                            let _ = req.reply.send(decision);
                        }
                    }
                    crate::ui::claude_panel::ClaudeAction::None => {}
                }
            });
    }
```

`SidePanel::right` must be declared before `CentralPanel` so the central area
shrinks to fit (egui panel ordering rule).

- [ ] **Step 2: Add the toggle command**

In `src/ui/palette.rs`, add `ToggleClaude` to the `PaletteCommand` enum and a
palette entry labelled "Toggle Claude panel". In `src/app/mod.rs` where palette
commands are handled (near `PaletteCommand::ToggleTerminal => ...`, ~line 975),
add:

```rust
                    PaletteCommand::ToggleClaude => self.show_claude = !self.show_claude,
```

Add a keybinding: in the key handling block that toggles the terminal/sidebar
(`app/mod.rs` ~line 411), add a branch for `Ctrl+Shift+I`:

```rust
        if ctrl && shift && key == egui::Key::I {
            self.show_claude = !self.show_claude;
        }
```

Match the exact modifier/key idiom already used in that block.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check`
Expected: `Finished`.

- [ ] **Step 4: Commit** (only if the user asked)

```bash
git add src/ui/layout.rs src/ui/palette.rs src/app/mod.rs
git commit -m "feat(claude): right dock render + toggle command/keybinding"
```

---

## Task 11: end-to-end manual verification

**Files:** none (manual checklist).

- [ ] **Step 1: Build release**

Run: `cargo build --release`
Expected: `Finished`.

- [ ] **Step 2: Manual test checklist**

Open the app in a workspace, press `Ctrl+Shift+I`:
1. Panel opens on the right.
2. Type "list the files in this folder" → assistant text streams into the
   transcript; on the `Read`/`LS`/`Bash` tool, a permission dialog appears
   (unless auto-allowed). Allow it → Claude continues; a `result` ends the turn.
3. Select a few lines in a file, ask "explain this selection" → the prompt
   carries the file path + selection (verify Claude references the right code).
4. Ask Claude to edit a file → an `Edit`/`Write` permission dialog appears;
   **Deny** → no change; ask again and **Allow** → file changes on disk.
5. "New" clears the conversation; the next turn starts a fresh `session_id`.
6. "Cancel" during a long turn kills the process and returns to idle.
7. Rename `claude` off PATH (or set a bad `claude_binary`) → sending a message
   shows the friendly "Is Claude Code installed?" message, no crash.

- [ ] **Step 3: Commit** (only if the user asked)

```bash
git add -A
git commit -m "docs(claude): manual verification checklist done"
```

---

## Self-Review (completed during planning)

- **Spec coverage:** native panel (T8), agentic + per-action confirm (T4/T5/T10),
  architecture A (T3-T6), per-turn `--resume` (T3/T9), stream-json parse (T1),
  right dock (T10), file+selection context (T9), auth reuse (no key — T3 spawns
  bare `claude`), no new heavy deps (uses serde_json/std). All covered.
- **Placeholder scan:** no TBD/TODO; all code steps contain real code; the one
  flagged risk (MCP permission-tool schema, T5) is called out with a concrete
  default + how to verify, not left vague.
- **Type consistency:** `ClaudeEvent`, `ClaudeSession`/`apply`/`build_prompt`,
  `TurnRequest`/`Turn`/`spawn_turn`, `PermissionListener`/`PermissionRequest`/
  `Decision`/`write_mcp_config`/`run_permission_mcp_server`, `ClaudePanel`/
  `ClaudeAction` names match across tasks. MCP tool name `mcp__editor__approve`
  (server "editor" + tool "approve") matches `--permission-prompt-tool` (T3) and
  the `tools/list`/`tools/call` handler (T5).
- **Risk note:** T5 stdin/permission-tool schema is the only version-dependent
  piece; verify against `claude --debug` on first run before relying on it.
```
