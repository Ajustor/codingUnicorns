use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::process_ext::CommandExt as _;
use crossbeam_channel::{unbounded, Receiver, Sender};
use serde_json::Value;

/// Stdio transport for the Debug Adapter Protocol.
/// Uses the same `Content-Length: N\r\n\r\n{body}` framing as LSP.
pub struct DapTransport {
    stdin: Box<dyn Write + Send>,
    pub receiver: Receiver<Value>,
    /// Cleared by the reader thread when the adapter process exits.
    pub is_alive: Arc<AtomicBool>,
    _child: Option<Child>,
}

impl DapTransport {
    pub fn spawn(command: &str, args: &[&str], workspace: &str) -> anyhow::Result<Self> {
        let mut child = Command::new(command)
            .no_window()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(workspace)
            .spawn()?;

        let stdin = child.stdin.take().unwrap();
        let stdout: ChildStdout = child.stdout.take().unwrap();
        let (tx, rx) = unbounded::<Value>();
        let alive = Arc::new(AtomicBool::new(true));
        let alive_clone = alive.clone();

        std::thread::spawn(move || {
            read_messages(BufReader::new(stdout), &tx);
            alive_clone.store(false, Ordering::Relaxed);
        });

        Ok(Self {
            stdin: Box::new(stdin),
            receiver: rx,
            is_alive: alive,
            _child: Some(child),
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
            _child: None,
        }
    }

    pub fn send(&mut self, msg: &Value) -> anyhow::Result<()> {
        write_message(&mut self.stdin, msg)
    }
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
        assert!(DapTransport::spawn(
            "definitely-not-a-real-dap-adapter-xyz",
            &[],
            &dir.path().to_string_lossy()
        )
        .is_err());
    }

    #[test]
    fn spawn_real_process_marks_dead_on_exit() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let (cmd, args) = ("cmd", vec!["/C", "exit 0"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "exit 0"]);
        let t = DapTransport::spawn(cmd, &args, &dir.path().to_string_lossy()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while t.is_alive.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!t.is_alive.load(Ordering::Relaxed));
    }
}
