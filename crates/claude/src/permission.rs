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
