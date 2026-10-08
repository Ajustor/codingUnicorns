use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};

use crossbeam_channel::{unbounded, Receiver, Sender};

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

/// Build the `claude` command line for one turn (not yet spawned).
fn build_command(req: &TurnRequest) -> Command {
    let mut cmd = Command::new(&req.binary);
    crate::no_window(&mut cmd)
        .arg("-p")
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
    cmd
}

/// Parse stream-json lines from `stdout` and forward the events until EOF or
/// until the receiver is dropped.
fn forward_events(stdout: impl Read, tx: Sender<ClaudeEvent>) {
    let reader = BufReader::new(stdout);
    for line in reader.lines().map_while(Result::ok) {
        for ev in parse_line(&line) {
            if tx.send(ev).is_err() {
                return;
            }
        }
    }
}

/// Drain `stderr` into the debug log (surfaced only on failure).
fn log_stderr(stderr: impl Read) {
    let reader = BufReader::new(stderr);
    for line in reader.lines().map_while(Result::ok) {
        log::debug!("claude stderr: {line}");
    }
}

/// Spawn `claude` for one turn and stream parsed events.
pub fn spawn_turn(req: &TurnRequest) -> std::io::Result<Turn> {
    let mut child = build_command(req).spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (tx, rx) = unbounded::<ClaudeEvent>();

    // stdout → parsed events
    std::thread::spawn(move || forward_events(stdout, tx));

    // stderr → log (surfaced only on failure)
    std::thread::spawn(move || log_stderr(stderr));

    Ok(Turn { rx, child })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::path::PathBuf;

    fn req() -> TurnRequest {
        TurnRequest {
            binary: "claude".into(),
            prompt: "hello world".into(),
            workspace: PathBuf::from("ws"),
            session_id: None,
            perm_port: 4242,
            perm_token: "tok".into(),
            mcp_config: PathBuf::from("cfg.json"),
            model: None,
        }
    }

    fn args(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn env(cmd: &Command, key: &str) -> Option<String> {
        cmd.get_envs()
            .find(|(k, _)| *k == OsStr::new(key))
            .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    }

    #[test]
    fn base_command_line() {
        let cmd = build_command(&req());
        assert_eq!(cmd.get_program(), OsStr::new("claude"));
        assert_eq!(
            args(&cmd),
            [
                "-p",
                "hello world",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-prompt-tool",
                "mcp__editor__approve",
                "--mcp-config",
                "cfg.json",
                "--add-dir",
                "ws",
            ]
        );
        assert_eq!(cmd.get_current_dir(), Some(std::path::Path::new("ws")));
    }

    #[test]
    fn env_carries_port_and_token_not_argv() {
        let cmd = build_command(&req());
        assert_eq!(env(&cmd, "NO_COLOR").as_deref(), Some("1"));
        assert_eq!(env(&cmd, "CU_PERM_PORT").as_deref(), Some("4242"));
        assert_eq!(env(&cmd, "CU_PERM_TOKEN").as_deref(), Some("tok"));
        assert!(!args(&cmd)
            .iter()
            .any(|a| a.contains("tok") && a != "hello world"));
    }

    #[test]
    fn resume_and_model_are_appended() {
        let mut r = req();
        r.session_id = Some("sess-1".into());
        r.model = Some("opus".into());
        let a = args(&build_command(&r));
        assert_eq!(&a[a.len() - 4..], ["--resume", "sess-1", "--model", "opus"]);
    }

    #[test]
    fn model_without_session() {
        let mut r = req();
        r.model = Some("sonnet".into());
        let a = args(&build_command(&r));
        assert!(!a.contains(&"--resume".to_string()));
        assert_eq!(&a[a.len() - 2..], ["--model", "sonnet"]);
    }

    #[test]
    fn forward_events_parses_each_line() {
        let input = concat!(
            r#"{"type":"system","subtype":"init","session_id":"s1"}"#,
            "\n",
            "garbage\n",
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#,
            "\n",
            r#"{"type":"result","result":"ok","total_cost_usd":0.5}"#,
            "\n",
        );
        let (tx, rx) = unbounded();
        forward_events(input.as_bytes(), tx);
        let evs: Vec<_> = rx.try_iter().collect();
        assert_eq!(
            evs,
            vec![
                ClaudeEvent::Init {
                    session_id: "s1".into()
                },
                ClaudeEvent::AssistantText("hi".into()),
                ClaudeEvent::Result {
                    text: "ok".into(),
                    cost_usd: 0.5,
                    session_id: None
                },
            ]
        );
    }

    #[test]
    fn forward_events_stops_when_receiver_dropped() {
        let input = concat!(
            r#"{"type":"system","subtype":"init","session_id":"s1"}"#,
            "\n",
            r#"{"type":"system","subtype":"init","session_id":"s2"}"#,
            "\n",
        );
        let (tx, rx) = unbounded();
        drop(rx);
        // Must return rather than loop / panic.
        forward_events(input.as_bytes(), tx);
    }

    #[test]
    fn log_stderr_drains_input() {
        let mut cursor = std::io::Cursor::new(b"line one\nline two\n".to_vec());
        log_stderr(&mut cursor);
        assert_eq!(cursor.position(), 18);
    }

    #[test]
    fn spawn_missing_binary_errors() {
        let mut r = req();
        r.binary = "cu-definitely-not-an-installed-binary-xyz".into();
        r.workspace = std::env::temp_dir();
        assert!(spawn_turn(&r).is_err());
    }
}
