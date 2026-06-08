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
    /// optional model override, passed as `--model` (set via the `/model` command).
    pub model: Option<String>,
}

/// A running turn: events stream over `rx`; `cancel()` kills the process.
pub struct Turn {
    pub rx: Receiver<ClaudeEvent>,
    child: Child,
}

impl Turn {
    pub fn cancel(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        // Kill + reap so abandoning a turn never leaks a zombie `claude` process.
        let _ = self.child.kill();
        let _ = self.child.wait();
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
    if let Some(model) = &req.model {
        cmd.args(["--model", model]);
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
