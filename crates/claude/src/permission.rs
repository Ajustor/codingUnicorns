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
        Ok(Self {
            port,
            token,
            requests: rx,
        })
    }
}

fn handle_conn(stream: TcpStream, token: &str, tx: &Sender<PermissionRequest>) {
    // A silent peer must not hold this thread forever while reading the request.
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
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
        tool: v
            .get("tool")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
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
///
/// Note: only `--port` is written here; the auth token travels via the
/// `CU_PERM_TOKEN` env var, not argv.
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

/// Entry point when the binary is re-invoked as the permission MCP server.
/// Speaks minimal MCP (JSON-RPC 2.0) over stdio; blocks until stdin closes.
///
/// The `token` is supplied by the caller via the `CU_PERM_TOKEN` environment
/// variable (set on the `claude` process by `process::spawn_turn` and inherited
/// by this re-invoked server) — it is NOT passed as a CLI argument. `write_mcp_config`
/// only writes `--port`; the dispatcher reads the token from the environment.
pub fn run_permission_mcp_server(port: u16, token: String) {
    let stdin = std::io::stdin();
    serve_mcp(stdin.lock(), std::io::stdout(), port, &token);
}

/// The MCP request loop behind [`run_permission_mcp_server`], generic over the
/// transport so it can be driven without real stdio.
fn serve_mcp(input: impl BufRead, mut stdout: impl Write, port: u16, token: &str) {
    for line in input.lines().map_while(Result::ok) {
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
                let tool = args
                    .and_then(|a| a.get("tool_name"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let input = args
                    .and_then(|a| a.get("input"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let decision = ask_editor(port, token, tool, &input);
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
    match serde_json::from_str::<Value>(resp.trim())
        .ok()
        .and_then(|v| {
            v.get("decision")
                .and_then(|d| d.as_str())
                .map(str::to_string)
        })
        .as_deref()
    {
        Some("allow") => Decision::Allow,
        _ => Decision::Deny,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
        let req = listener
            .requests
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
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
    /// Connected (client, server) socket pair over loopback.
    fn socket_pair() -> (TcpStream, TcpStream) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (server, _) = l.accept().unwrap();
        (client, server)
    }

    fn read_reply(s: TcpStream) -> String {
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut resp = String::new();
        BufReader::new(s).read_line(&mut resp).unwrap();
        resp
    }

    fn decision_of(resp: &str) -> String {
        let v: Value = serde_json::from_str(resp.trim()).unwrap();
        v["decision"].as_str().unwrap().to_string()
    }

    #[test]
    fn listener_reports_port_and_token() {
        let l = PermissionListener::start("abc".into()).unwrap();
        assert_ne!(l.port, 0);
        assert_eq!(l.token, "abc");
    }

    #[test]
    fn explicit_deny_round_trip_preserves_input() {
        let listener = PermissionListener::start("t".into()).unwrap();
        let port = listener.port;
        let client = std::thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            writeln!(
                s,
                "{}",
                json!({"token":"t","tool":"Bash","input":{"command":"ls"}})
            )
            .unwrap();
            read_reply(s)
        });
        let req = listener
            .requests
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert_eq!(req.tool, "Bash");
        assert_eq!(req.input, json!({"command":"ls"}));
        req.reply.send(Decision::Deny).unwrap();
        assert_eq!(decision_of(&client.join().unwrap()), "deny");
    }

    #[test]
    fn dropped_request_defaults_to_deny() {
        let (mut client, server) = socket_pair();
        let (tx, rx) = channel();
        let h = std::thread::spawn(move || handle_conn(server, "t", &tx));
        writeln!(client, "{}", json!({"token":"t","tool":"Edit"})).unwrap();
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(req); // UI discards the request without answering
        assert_eq!(decision_of(&read_reply(client)), "deny");
        h.join().unwrap();
    }

    #[test]
    fn missing_tool_and_input_default() {
        let (mut client, server) = socket_pair();
        let (tx, rx) = channel();
        let h = std::thread::spawn(move || handle_conn(server, "t", &tx));
        writeln!(client, "{}", json!({"token":"t"})).unwrap();
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(req.tool, "");
        assert_eq!(req.input, Value::Null);
        req.reply.send(Decision::Allow).unwrap();
        assert_eq!(decision_of(&read_reply(client)), "allow");
        h.join().unwrap();
    }

    #[test]
    fn closed_ui_channel_denies() {
        let (mut client, server) = socket_pair();
        let (tx, rx) = channel::<PermissionRequest>();
        drop(rx);
        writeln!(client, "{}", json!({"token":"t","tool":"Edit"})).unwrap();
        handle_conn(server, "t", &tx);
        assert_eq!(decision_of(&read_reply(client)), "deny");
    }

    #[test]
    fn missing_token_denied_without_queueing() {
        let (mut client, server) = socket_pair();
        let (tx, rx) = channel();
        writeln!(client, "{}", json!({"tool":"Edit"})).unwrap();
        handle_conn(server, "t", &tx);
        assert_eq!(decision_of(&read_reply(client)), "deny");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn malformed_request_closes_without_reply() {
        let (mut client, server) = socket_pair();
        let (tx, rx) = channel();
        writeln!(client, "not json").unwrap();
        handle_conn(server, "t", &tx);
        // Connection closed with no line written.
        assert_eq!(read_reply(client), "");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn write_mcp_config_points_at_self_exe() {
        let dir = std::env::temp_dir().join(format!("cu-claude-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = std::path::Path::new("/opt/editor/cu");
        let path = write_mcp_config(&dir, exe, 5151).unwrap();
        assert_eq!(path, dir.join("cu-claude-mcp.json"));
        let v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let server = &v["mcpServers"]["editor"];
        assert_eq!(server["command"], exe.to_string_lossy().as_ref());
        assert_eq!(
            server["args"],
            json!(["--claude-permission-server", "--port", "5151"])
        );
        // The token must never be written to disk.
        assert!(!std::fs::read_to_string(&path).unwrap().contains("token"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_mcp_config_fails_for_missing_dir() {
        let dir = std::env::temp_dir()
            .join(format!("cu-claude-missing-{}", std::process::id()))
            .join("nope");
        assert!(write_mcp_config(&dir, std::path::Path::new("x"), 1).is_err());
    }

    /// Run the MCP loop over `input` and return each JSON response line.
    fn run_mcp(input: &str, port: u16, token: &str) -> Vec<Value> {
        let mut out = Vec::new();
        serve_mcp(input.as_bytes(), &mut out, port, token);
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn mcp_initialize_and_tools_list() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            "garbage line\n",
            r#"{"jsonrpc":"2.0","id":"two","method":"tools/list"}"#,
            "\n",
        );
        let resps = run_mcp(input, 0, "t");
        assert_eq!(resps.len(), 2, "notification and garbage get no reply");
        assert_eq!(resps[0]["id"], 1);
        assert_eq!(resps[0]["jsonrpc"], "2.0");
        assert_eq!(resps[0]["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(resps[0]["result"]["serverInfo"]["name"], "editor");
        assert_eq!(resps[1]["id"], "two");
        let tools = resps[1]["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "approve");
        assert!(tools[0]["inputSchema"]["properties"]["tool_name"].is_object());
    }

    fn call_payload(resp: &Value) -> Value {
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn mcp_tools_call_allow_via_listener() {
        let listener = PermissionListener::start("sek".into()).unwrap();
        let port = listener.port;
        let ui = std::thread::spawn(move || {
            let req = listener
                .requests
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            let seen = (req.tool.clone(), req.input.clone());
            req.reply.send(Decision::Allow).unwrap();
            seen
        });
        let input = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"approve","arguments":{"tool_name":"Write","input":{"path":"a.txt"}}}}"#;
        let resps = run_mcp(input, port, "sek");
        let (tool, inp) = ui.join().unwrap();
        assert_eq!(tool, "Write");
        assert_eq!(inp, json!({"path":"a.txt"}));
        assert_eq!(resps.len(), 1);
        assert_eq!(resps[0]["id"], 7);
        assert_eq!(
            call_payload(&resps[0]),
            json!({"behavior":"allow","updatedInput":{"path":"a.txt"}})
        );
    }

    #[test]
    fn mcp_tools_call_wrong_token_is_denied() {
        let listener = PermissionListener::start("right".into()).unwrap();
        let input = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"arguments":{"tool_name":"Edit","input":{}}}}"#;
        let resps = run_mcp(input, listener.port, "wrong");
        assert_eq!(
            call_payload(&resps[0]),
            json!({"behavior":"deny","message":"Denied by user"})
        );
        assert!(listener.requests.try_recv().is_err());
    }

    #[test]
    fn mcp_tools_call_without_editor_denies() {
        // Port 0 can never be connected to.
        let input = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call"}"#;
        let resps = run_mcp(input, 0, "t");
        assert_eq!(call_payload(&resps[0])["behavior"], "deny");
    }

    /// Fake editor that answers the first connection with `reply` verbatim.
    fn fake_editor(reply: &'static str) -> (u16, std::thread::JoinHandle<Value>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut w = s.try_clone().unwrap();
            let mut line = String::new();
            BufReader::new(s).read_line(&mut line).unwrap();
            w.write_all(reply.as_bytes()).unwrap();
            serde_json::from_str(line.trim()).unwrap()
        });
        (port, h)
    }

    #[test]
    fn ask_editor_sends_token_tool_input() {
        let (port, h) = fake_editor("{\"decision\":\"allow\"}\n");
        let d = ask_editor(port, "tk", "Edit", &json!({"a":1}));
        assert_eq!(d, Decision::Allow);
        assert_eq!(
            h.join().unwrap(),
            json!({"token":"tk","tool":"Edit","input":{"a":1}})
        );
    }

    #[test]
    fn ask_editor_unexpected_replies_deny() {
        for reply in [
            "{\"decision\":\"deny\"}\n",
            "{\"decision\":\"ALLOW\"}\n",
            "{\"decision\":true}\n",
            "nonsense\n",
            "",
        ] {
            let (port, h) = fake_editor(reply);
            assert_eq!(
                ask_editor(port, "t", "x", &Value::Null),
                Decision::Deny,
                "reply {reply:?}"
            );
            h.join().unwrap();
        }
    }
}
