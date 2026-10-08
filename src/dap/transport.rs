use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::types::DapConfig;
use crate::extension::manifest::DebuggerTransport;

use crate::process_ext::CommandExt as _;
use crossbeam_channel::{unbounded, Receiver, Sender};
use serde_json::Value;

/// Transport for the Debug Adapter Protocol: the adapter's stdio, or a TCP
/// connection to it. Uses the same `Content-Length: N\r\n\r\n{body}`
/// framing as LSP.
pub struct DapTransport {
    stdin: Box<dyn Write + Send>,
    pub receiver: Receiver<Value>,
    /// Cleared by the reader thread when the adapter closes the stream.
    pub is_alive: Arc<AtomicBool>,
    /// Port of a TCP adapter, to open child sessions on.
    pub port: Option<u16>,
    child: Option<Child>,
}

/// How long a TCP adapter may take to start listening.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

impl DapTransport {
    /// Start the adapter of `cfg` and connect to it. For a TCP adapter this
    /// blocks until it listens (call it off the UI thread).
    pub fn open(cfg: &DapConfig, workspace: &Path) -> anyhow::Result<Self> {
        let args: Vec<&str> = cfg.adapter_args.iter().map(String::as_str).collect();
        match cfg.transport {
            DebuggerTransport::Stdio => Self::spawn(&cfg.adapter_cmd, &args, workspace),
            DebuggerTransport::Tcp => Self::spawn_tcp(&cfg.adapter_cmd, &args, workspace),
        }
    }

    pub fn spawn(command: &str, args: &[&str], workspace: &Path) -> anyhow::Result<Self> {
        let mut child = Command::new(command)
            .no_window()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(workspace)
            .spawn()?;

        let stdin = child.stdin.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let stdout: ChildStdout = child.stdout.take().unwrap();
        let (tx, rx) = unbounded::<Value>();
        let alive = Arc::new(AtomicBool::new(true));
        let alive_clone = alive.clone();

        // Adapter errors (e.g. "No module named debugpy") show in the
        // debug output as `stderr` output events.
        forward_as_output(stderr, tx.clone());
        std::thread::spawn(move || {
            read_messages(BufReader::new(stdout), &tx);
            alive_clone.store(false, Ordering::Relaxed);
        });

        Ok(Self {
            stdin: Box::new(stdin),
            receiver: rx,
            is_alive: alive,
            port: None,
            child: Some(child),
        })
    }

    /// Start an adapter that listens on TCP: `${port}` in `args` becomes a
    /// free local port; connect once it accepts connections.
    pub fn spawn_tcp(command: &str, args: &[&str], workspace: &Path) -> anyhow::Result<Self> {
        let port = TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port();
        let args: Vec<String> = args
            .iter()
            .map(|a| a.replace("${port}", &port.to_string()))
            .collect();
        let mut child = Command::new(command)
            .no_window()
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(workspace)
            .spawn()?;
        let (tx, rx) = unbounded::<Value>();
        // The adapter's own logs show in the debug output.
        if let Some(out) = child.stdout.take() {
            forward_as_output(out, tx.clone());
        }
        if let Some(err) = child.stderr.take() {
            forward_as_output(err, tx.clone());
        }
        let start = Instant::now();
        let stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(s) => break s,
                Err(e) => {
                    if let Ok(Some(status)) = child.try_wait() {
                        // Give the log forwarders a moment to catch up.
                        std::thread::sleep(Duration::from_millis(100));
                        let logs: Vec<String> = rx
                            .try_iter()
                            .filter_map(|m| m["body"]["output"].as_str().map(String::from))
                            .collect();
                        anyhow::bail!(
                            "adapter exited ({status}) before listening: {}",
                            logs.join(" | ")
                        );
                    }
                    if start.elapsed() > CONNECT_TIMEOUT {
                        let _ = child.kill();
                        let _ = child.wait();
                        anyhow::bail!("adapter not listening on port {port}: {e}");
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        };
        let mut transport = Self::over_tcp(stream, tx, rx)?;
        transport.port = Some(port);
        transport.child = Some(child);
        Ok(transport)
    }

    /// One more connection to a TCP adapter, for a child session.
    pub fn connect(port: u16) -> anyhow::Result<Self> {
        let stream = TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            Duration::from_secs(5),
        )?;
        let (tx, rx) = unbounded::<Value>();
        let mut transport = Self::over_tcp(stream, tx, rx)?;
        transport.port = Some(port);
        Ok(transport)
    }

    fn over_tcp(stream: TcpStream, tx: Sender<Value>, rx: Receiver<Value>) -> anyhow::Result<Self> {
        let reader = stream.try_clone()?;
        let alive = Arc::new(AtomicBool::new(true));
        let alive_clone = alive.clone();
        std::thread::spawn(move || {
            read_messages(BufReader::new(reader), &tx);
            alive_clone.store(false, Ordering::Relaxed);
        });
        Ok(Self {
            stdin: Box::new(stream),
            receiver: rx,
            is_alive: alive,
            port: None,
            child: None,
        })
    }

    /// Build a transport over arbitrary I/O (no child process) — test seam.
    #[cfg(test)]
    pub(crate) fn from_parts(
        writer: Box<dyn Write + Send>,
        receiver: Receiver<Value>,
        is_alive: Arc<AtomicBool>,
    ) -> Self {
        Self {
            stdin: writer,
            receiver,
            is_alive,
            port: None,
            child: None,
        }
    }

    pub fn send(&mut self, msg: &Value) -> anyhow::Result<()> {
        write_message(&mut self.stdin, msg)
    }
}

impl DapTransport {
    /// Wait for the adapter process to exit until `deadline`, then kill it
    /// (blocking). Used when the IDE quits: a background wait would be cut
    /// short and leave the adapter running.
    pub fn close(&mut self, deadline: Instant) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        while Instant::now() < deadline {
            if !matches!(child.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for DapTransport {
    /// Stop the adapter process. It gets a moment to handle the last
    /// `disconnect` (and terminate the debuggee) before being killed.
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        std::thread::spawn(move || {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(3) {
                if !matches!(child.try_wait(), Ok(None)) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        });
    }
}

/// Forward the lines of an adapter's stdout/stderr as DAP `output` events.
fn forward_as_output(stream: impl Read + Send + 'static, tx: Sender<Value>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            let _ = tx.send(serde_json::json!({
                "type": "event",
                "event": "output",
                "body": { "category": "stderr", "output": line },
            }));
        }
    });
}

/// Read `Content-Length`-framed JSON messages from `reader` until EOF or a
/// truncated body, forwarding every successfully parsed message to `tx`.
pub(crate) fn read_messages<R: BufRead>(mut reader: R, tx: &Sender<Value>) {
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let header = header.trim();
        if header.is_empty() {
            continue;
        }
        let content_length: usize = if let Some(s) = header.strip_prefix("Content-Length: ") {
            s.trim().parse().unwrap_or(0)
        } else {
            // Text the adapter (or the program it runs) printed outside the
            // protocol, e.g. `[Console]::WriteLine` under PowerShell: show it.
            if !looks_like_header(header) {
                let _ = tx.send(serde_json::json!({
                    "type": "event",
                    "event": "output",
                    "body": { "category": "stdout", "output": header },
                }));
            }
            continue;
        };
        let mut blank = String::new();
        let _ = reader.read_line(&mut blank);
        let mut buf = vec![0u8; content_length];
        if reader.read_exact(&mut buf).is_err() {
            break;
        }
        if let Ok(msg) = serde_json::from_slice::<Value>(&buf) {
            let _ = tx.send(msg);
        }
    }
}

/// `Name: value` with an HTTP-style header name.
fn looks_like_header(line: &str) -> bool {
    line.split_once(": ").is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// Write one `Content-Length`-framed JSON message and flush.
pub(crate) fn write_message<W: Write>(w: &mut W, msg: &Value) -> anyhow::Result<()> {
    let body = serde_json::to_string(msg)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    w.write_all(header.as_bytes())?;
    w.write_all(body.as_bytes())?;
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    fn read_all(input: &[u8]) -> Vec<Value> {
        let (tx, rx) = unbounded();
        read_messages(Cursor::new(input.to_vec()), &tx);
        drop(tx);
        rx.try_iter().collect()
    }

    fn frame(body: &str) -> String {
        format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
    }

    #[test]
    fn write_message_frames_body() {
        let mut out = Vec::new();
        write_message(&mut out, &json!({"seq": 1, "type": "request"})).unwrap();
        let body = r#"{"seq":1,"type":"request"}"#;
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
        );
    }

    #[test]
    fn roundtrip_multiple_messages() {
        let msgs = vec![
            json!({"seq": 1, "type": "event", "event": "initialized"}),
            json!({"seq": 2, "type": "response", "command": "launch", "success": true}),
            json!({"seq": 3, "type": "event", "event": "output", "body": {"output": "ünï\n"}}),
        ];
        let mut out = Vec::new();
        for m in &msgs {
            write_message(&mut out, m).unwrap();
        }
        assert_eq!(read_all(&out), msgs);
    }

    #[test]
    fn skips_noise_headers_and_blank_lines() {
        let input = format!("\r\nX-Junk: 1\r\n{}", frame(r#"{"seq":9}"#));
        assert_eq!(read_all(input.as_bytes()), vec![json!({"seq": 9})]);
    }

    #[test]
    fn stray_text_becomes_output() {
        let input = format!(
            "console x=42
{}",
            frame(r#"{"seq":9}"#)
        );
        let msgs = read_all(input.as_bytes());
        assert_eq!(msgs[0]["event"], "output");
        assert_eq!(msgs[0]["body"]["output"], "console x=42");
        assert_eq!(msgs[1], json!({"seq": 9}));
    }

    #[test]
    fn partial_frame_at_eof_is_discarded() {
        let input = format!("{}Content-Length: 50\r\n\r\n{{\"seq\"", frame("true"));
        assert_eq!(read_all(input.as_bytes()), vec![json!(true)]);
    }

    #[test]
    fn bad_json_is_skipped() {
        let input = format!("{}{}", frame("nope"), frame("[1]"));
        assert_eq!(read_all(input.as_bytes()), vec![json!([1])]);
    }

    #[test]
    fn bad_length_is_treated_as_zero() {
        let input = format!("Content-Length: -5\r\n\r\n{}", frame("2"));
        assert_eq!(read_all(input.as_bytes()), vec![json!(2)]);
    }

    #[derive(Clone, Default)]
    struct Shared(Arc<std::sync::Mutex<Vec<u8>>>);
    impl Write for Shared {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn send_uses_framing() {
        let buf = Shared::default();
        let (_tx, rx) = unbounded();
        let mut t =
            DapTransport::from_parts(Box::new(buf.clone()), rx, Arc::new(AtomicBool::new(true)));
        t.send(&json!({"seq": 5})).unwrap();
        t.send(&json!({"seq": 6})).unwrap();
        let written = buf.0.lock().unwrap().clone();
        assert_eq!(
            read_all(&written),
            vec![json!({"seq": 5}), json!({"seq": 6})]
        );
    }

    #[test]
    fn spawn_missing_binary_fails() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            DapTransport::spawn("definitely-not-a-real-dap-adapter-xyz", &[], dir.path()).is_err()
        );
    }

    #[test]
    fn spawn_real_process_marks_dead_on_exit() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let (cmd, args) = ("cmd", vec!["/C", "exit 0"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "exit 0"]);
        let t = DapTransport::spawn(cmd, &args, dir.path()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while t.is_alive.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!t.is_alive.load(Ordering::Relaxed));
    }

    /// A tiny TCP "adapter": a script that listens on the given port and
    /// echoes one framed message back, using whatever runtime is present.
    fn echo_server() -> Option<(String, Vec<String>)> {
        let script = r#"
const net = require('net');
const srv = net.createServer(s => s.pipe(s));
srv.listen(Number(process.argv[1]), '127.0.0.1');
setTimeout(() => process.exit(0), 20000);
"#;
        super::super::adapters::find_on_path("node")?;
        Some((
            "node".into(),
            vec!["-e".into(), script.into(), "${port}".into()],
        ))
    }

    #[test]
    fn tcp_adapter_gets_a_port_and_child_connections() {
        let Some((cmd, args)) = echo_server() else {
            return; // no node on this machine
        };
        let dir = tempfile::tempdir().unwrap();
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut t = DapTransport::spawn_tcp(&cmd, &args, dir.path()).unwrap();
        let port = t.port.unwrap();
        t.send(&json!({"seq": 1})).unwrap();
        let echoed = t.receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(echoed["seq"], 1);
        let mut child = DapTransport::connect(port).unwrap();
        assert_eq!(child.port, Some(port));
        child.send(&json!({"seq": 2})).unwrap();
        assert_eq!(
            child
                .receiver
                .recv_timeout(Duration::from_secs(10))
                .unwrap()["seq"],
            2
        );
    }

    #[test]
    fn tcp_adapter_that_exits_early_reports_why() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let (cmd, args) = ("cmd", vec!["/C", "echo boom ${port}"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "echo boom ${port}"]);
        let err = DapTransport::spawn_tcp(cmd, &args, dir.path())
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("exited") && err.contains("boom"), "{err}");
        assert!(!err.contains("${port}"), "port substituted: {err}");
    }
}
