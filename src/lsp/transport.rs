use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::{unbounded, Receiver};
use serde_json::Value;

pub struct LspTransport {
    stdin: ChildStdin,
    pub receiver: Receiver<Value>,
    /// Set to false by the reader thread when the server process exits.
    pub is_alive: Arc<AtomicBool>,
    _child: Child,
}

impl LspTransport {
    pub fn spawn(command: &str, args: &[&str], workspace: &str) -> anyhow::Result<Self> {
        let mut child = Command::new(command)
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
            let mut reader = BufReader::new(stdout);
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

                let content_length: usize = if let Some(s) = header.strip_prefix("Content-Length: ")
                {
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
            // Reader thread is exiting — mark the transport as dead.
            alive_clone.store(false, Ordering::Relaxed);
        });

        Ok(Self {
            stdin,
            receiver: rx,
            is_alive: alive,
            _child: child,
        })
    }

    pub fn send(&mut self, msg: &Value) -> anyhow::Result<()> {
        let body = serde_json::to_string(msg)?;
        debug_log('>', &body);
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin.write_all(header.as_bytes())?;
        self.stdin.write_all(body.as_bytes())?;
        self.stdin.flush()?;
        Ok(())
    }
}

/// Append one line of LSP traffic to `%TEMP%/cu-lsp.log` for debugging.
/// `dir` is '>' for outgoing (to server) and '<' for incoming (from server).
/// Bodies are truncated; this is a temporary diagnostic aid.
pub(super) fn debug_log(dir: char, body: &str) {
    let path = std::env::temp_dir().join("cu-lsp.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let truncated: String = body.chars().take(600).collect();
        let _ = writeln!(f, "{dir} {truncated}");
    }
}
