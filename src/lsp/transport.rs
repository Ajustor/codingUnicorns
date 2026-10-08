use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::process_ext::CommandExt as _;
use crossbeam_channel::{unbounded, Receiver, Sender};
use serde_json::Value;

pub struct LspTransport {
    stdin: Box<dyn Write + Send>,
    pub receiver: Receiver<Value>,
    /// Set to false by the reader thread when the server process exits.
    pub is_alive: Arc<AtomicBool>,
    _child: Option<Child>,
}

impl LspTransport {
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
            // Reader thread is exiting — mark the transport as dead.
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
            Ok(0) | Err(_) => break, // EOF or error → server closed
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

        // Skip blank line between header and body
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
    fn write_message_produces_content_length_frame() {
        let mut out = Vec::new();
        write_message(&mut out, &json!({"jsonrpc": "2.0", "id": 1})).unwrap();
        let s = String::from_utf8(out).unwrap();
        let body = r#"{"id":1,"jsonrpc":"2.0"}"#;
        assert_eq!(s, format!("Content-Length: {}\r\n\r\n{}", body.len(), body));
    }

    #[test]
    fn write_message_counts_bytes_not_chars() {
        let mut out = Vec::new();
        write_message(&mut out, &json!("é€")).unwrap();
        let s = String::from_utf8(out).unwrap();
        // "\"é€\"" = 1 + 2 + 3 + 1 bytes
        assert!(s.starts_with("Content-Length: 7\r\n\r\n"), "{s}");
    }

    #[test]
    fn write_then_read_roundtrip() {
        let mut out = Vec::new();
        let a = json!({"id": 1, "result": {"x": [1, 2, 3]}});
        let b = json!({"method": "textDocument/publishDiagnostics", "params": {}});
        write_message(&mut out, &a).unwrap();
        write_message(&mut out, &b).unwrap();
        assert_eq!(read_all(&out), vec![a, b]);
    }

    #[test]
    fn reads_multiple_frames_back_to_back() {
        let input = format!("{}{}{}", frame("1"), frame("[true]"), frame(r#"{"a":"b"}"#));
        assert_eq!(
            read_all(input.as_bytes()),
            vec![json!(1), json!([true]), json!({"a": "b"})]
        );
    }

    #[test]
    fn skips_blank_lines_and_unknown_headers() {
        let body = r#"{"id":7}"#;
        let input = format!(
            "\r\n\r\nContent-Type: application/vscode-jsonrpc\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        assert_eq!(read_all(input.as_bytes()), vec![json!({"id": 7})]);
    }

    #[test]
    fn tolerates_whitespace_around_length() {
        let body = r#"{"id":2}"#;
        let input = format!("Content-Length:  {} \r\n\r\n{}", body.len(), body);
        assert_eq!(read_all(input.as_bytes()), vec![json!({"id": 2})]);
    }

    #[test]
    fn utf8_body_uses_byte_length() {
        let body = r#"{"msg":"héllo €"}"#;
        let input = format!("{}{}", frame(body), frame("1"));
        assert_eq!(
            read_all(input.as_bytes()),
            vec![json!({"msg": "héllo €"}), json!(1)]
        );
    }

    #[test]
    fn truncated_body_stops_reading() {
        let good = frame(r#"{"id":1}"#);
        let input = format!("{good}Content-Length: 100\r\n\r\n{{\"id\":2");
        assert_eq!(read_all(input.as_bytes()), vec![json!({"id": 1})]);
    }

    #[test]
    fn header_without_body_stops_reading() {
        assert!(read_all(b"Content-Length: 10\r\n").is_empty());
    }

    #[test]
    fn invalid_json_body_is_dropped_but_stream_continues() {
        let input = format!("{}{}", frame("{not json}"), frame(r#"{"ok":true}"#));
        assert_eq!(read_all(input.as_bytes()), vec![json!({"ok": true})]);
    }

    #[test]
    fn unparseable_length_reads_empty_body_and_continues() {
        // Length "abc" → 0: the empty body fails to parse; the next frame still decodes.
        let input = format!("Content-Length: abc\r\n\r\n{}", frame("42"));
        assert_eq!(read_all(input.as_bytes()), vec![json!(42)]);
    }

    #[test]
    fn empty_input_yields_nothing() {
        assert!(read_all(b"").is_empty());
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
    fn from_parts_send_writes_frame_to_writer() {
        let buf = Shared::default();
        let (_tx, rx) = unbounded();
        let mut t =
            LspTransport::from_parts(Box::new(buf.clone()), rx, Arc::new(AtomicBool::new(true)));
        t.send(&json!({"id": 3})).unwrap();
        let written = buf.0.lock().unwrap().clone();
        assert_eq!(read_all(&written), vec![json!({"id": 3})]);
    }

    #[test]
    fn send_propagates_writer_errors() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("closed"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let (_tx, rx) = unbounded();
        let mut t = LspTransport::from_parts(Box::new(Broken), rx, Arc::new(AtomicBool::new(true)));
        assert!(t.send(&json!({})).is_err());
    }

    #[test]
    fn spawn_missing_binary_fails() {
        let dir = tempfile::tempdir().unwrap();
        let r = LspTransport::spawn(
            "definitely-not-a-real-lsp-binary-xyz",
            &[],
            &dir.path().to_string_lossy(),
        );
        assert!(r.is_err());
    }

    #[test]
    fn spawn_real_process_marks_dead_on_exit() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let (cmd, args) = ("cmd", vec!["/C", "exit 0"]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c", "exit 0"]);
        let t = LspTransport::spawn(cmd, &args, &dir.path().to_string_lossy()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while t.is_alive.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!t.is_alive.load(Ordering::Relaxed));
        assert!(t.receiver.try_recv().is_err());
    }
}
