use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::process_ext::CommandExt as _;
use crossbeam_channel::{unbounded, Receiver, Sender};
use serde_json::Value;

pub struct LspTransport {
    writer: Box<dyn Write + Send>,
    pub receiver: Receiver<Value>,
    /// Set to false by the reader thread when the server closes the stream
    /// (process exit or closed connection).
    pub is_alive: Arc<AtomicBool>,
    child: Option<Child>,
    /// The connection, for a server the IDE connects to (shut down on drop).
    socket: Option<TcpStream>,
    /// Shared with the reader of a TCP connection (`abort`, `settle`).
    control: Arc<ReadControl>,
}

/// How often the reader of a TCP connection checks whether it must stop,
/// until the handshake is over.
const READ_POLL: Duration = Duration::from_millis(200);

/// What the handshake tells the reader of a TCP connection.
#[derive(Default)]
struct ReadControl {
    /// The handshake gave up: end the reader (`abort`).
    stop: AtomicBool,
    /// The handshake succeeded: stop polling and block on reads.
    settled: AtomicBool,
    /// Receive timeouts the reader went through.
    timeouts: std::sync::atomic::AtomicUsize,
}

/// Reads a TCP connection. During the handshake it wakes up every
/// [`READ_POLL`] to see whether `stop` is set, because shutting a socket down
/// does not reliably wake a blocked read on Windows. Once `settled` it blocks
/// normally: on Windows a socket that went through many receive timeouts may
/// lose data or fail with `WSA_IO_PENDING`, which would drop a healthy
/// connection.
struct StoppableReader {
    stream: TcpStream,
    control: Arc<ReadControl>,
    polling: bool,
}

impl Read for StoppableReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.control.stop.load(Ordering::Relaxed) {
                return Ok(0);
            }
            if self.polling
                && self.control.settled.load(Ordering::Relaxed)
                && self.stream.set_read_timeout(None).is_ok()
            {
                self.polling = false;
            }
            match self.stream.read(buf) {
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    self.control.timeouts.fetch_add(1, Ordering::Relaxed);
                }
                other => return other,
            }
        }
    }
}

/// How long connecting to a server that is already running may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);

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
        Ok(Self::over(
            Box::new(stdin),
            stdout,
            Some(child),
            None,
            Arc::default(),
        ))
    }

    /// Connect to a server already listening on `addr` (`host:port`), such
    /// as the Godot editor's. Nothing is started. Blocks (name lookup, up to
    /// 1 s per address): run it off the UI thread.
    pub fn connect(addr: &str) -> anyhow::Result<Self> {
        let stream = connect_tcp(addr, CONNECT_TIMEOUT)?;
        let reader = stream.try_clone()?;
        let writer = stream.try_clone()?;
        reader.set_read_timeout(Some(READ_POLL))?;
        let control = Arc::new(ReadControl::default());
        let reader = StoppableReader {
            stream: reader,
            control: control.clone(),
            polling: true,
        };
        // Keep the original as `socket` and shut that one down on drop: on
        // Windows a duplicated socket can report "not connected" for a
        // shutdown shortly after connecting.
        Ok(Self::over(
            Box::new(writer),
            reader,
            None,
            Some(stream),
            control,
        ))
    }

    /// Start the reader thread on `reader` and wrap everything up.
    fn over(
        writer: Box<dyn Write + Send>,
        reader: impl Read + Send + 'static,
        child: Option<Child>,
        socket: Option<TcpStream>,
        control: Arc<ReadControl>,
    ) -> Self {
        let (tx, rx) = unbounded::<Value>();
        let alive = Arc::new(AtomicBool::new(true));
        let alive_clone = alive.clone();
        std::thread::spawn(move || {
            read_messages(BufReader::new(reader), &tx);
            // Reader thread is exiting — mark the transport as dead.
            alive_clone.store(false, Ordering::Relaxed);
        });
        Self {
            writer,
            receiver: rx,
            is_alive: alive,
            child,
            socket,
            control,
        }
    }

    /// Build a transport over arbitrary I/O (no child process) — test seam.
    #[cfg(test)]
    pub(crate) fn from_parts(
        writer: Box<dyn Write + Send>,
        receiver: Receiver<Value>,
        is_alive: Arc<AtomicBool>,
    ) -> Self {
        Self {
            writer,
            receiver,
            is_alive,
            child: None,
            socket: None,
            control: Arc::default(),
        }
    }

    /// Process id of the spawned server — test seam.
    #[cfg(test)]
    pub(crate) fn child_id(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    pub fn send(&mut self, msg: &Value) -> anyhow::Result<()> {
        write_message(&mut self.writer, msg)
    }

    /// The handshake succeeded: the reader of a TCP connection stops polling
    /// and blocks on reads from now on.
    pub fn settle(&self) {
        self.control.settled.store(true, Ordering::Relaxed);
    }

    /// Receive timeouts the reader went through — test seam.
    #[cfg(test)]
    pub(crate) fn read_timeouts(&self) -> usize {
        self.control.timeouts.load(Ordering::Relaxed)
    }

    /// Give up on the connection: shut it down in both directions and stop
    /// the reader thread, which would otherwise wait for a server that never
    /// closes its side (a drop only sends FIN). For when nothing more
    /// needs delivering, e.g. a server that never answered `initialize`.
    pub fn abort(self) {
        if let Some(socket) = &self.socket {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
        self.control.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for LspTransport {
    /// Close a TCP connection: the reader thread holds its own handle, so
    /// dropping the writer alone would leave the server's client open. A
    /// spawned server gets a moment to exit (its stdin is closed first) before
    /// being killed, off the calling thread.
    fn drop(&mut self) {
        if let Some(socket) = &self.socket {
            // Write only (a FIN after the queued data): `Both` makes Windows
            // reset the connection if the server still answers, and it then
            // loses messages it received but has not read yet.
            let _ = socket.shutdown(std::net::Shutdown::Write);
        }
        let Some(mut child) = self.child.take() else {
            return;
        };
        // Close stdin now so a well-behaved server exits on EOF.
        self.writer = Box::new(std::io::sink());
        std::thread::spawn(move || {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(2) {
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

/// Connect to `addr` (`host:port`), trying each address it resolves to
/// (`localhost` is often `::1` first, while servers may listen on IPv4 only).
/// Blocks for the name lookup and up to `timeout` per address.
pub(crate) fn connect_tcp(addr: &str, timeout: Duration) -> anyhow::Result<TcpStream> {
    let sockets = addr
        .to_socket_addrs()
        .map_err(|e| anyhow::anyhow!("cannot resolve `{addr}`: {e}"))?;
    let mut last_error = None;
    for socket in sockets {
        match TcpStream::connect_timeout(&socket, timeout) {
            Ok(stream) => return Ok(stream),
            Err(e) => last_error = Some(e),
        }
    }
    match last_error {
        Some(e) => Err(anyhow::Error::new(e).context(format!("cannot connect to {addr}"))),
        None => Err(anyhow::anyhow!("no address for `{addr}`")),
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

        // Other headers (Content-Type…) may follow: the body starts after
        // the blank line that ends them.
        if !skip_to_body(&mut reader) {
            break;
        }

        let mut buf = vec![0u8; content_length];
        if reader.read_exact(&mut buf).is_err() {
            break;
        }

        if let Ok(msg) = serde_json::from_slice::<Value>(&buf) {
            let _ = tx.send(msg);
        }
    }
}

/// Read the remaining header lines up to and including the blank line before
/// the body. False at EOF.
pub(crate) fn skip_to_body<R: BufRead>(reader: &mut R) -> bool {
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return false,
            Ok(_) if line.trim().is_empty() => return true,
            Ok(_) => {}
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

    /// Whether process `pid` still exists and runs `image` (a freed pid may
    /// be reused by an unrelated process).
    fn process_exists(pid: u32, image: &str) -> bool {
        #[cfg(windows)]
        {
            let out = Command::new("tasklist")
                .no_window()
                .args(["/NH", "/FI", &format!("PID eq {pid}")])
                .args(["/FI", &format!("IMAGENAME eq {image}.exe")])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
        }
        #[cfg(not(windows))]
        {
            let out = Command::new("ps")
                .args(["-p", &pid.to_string(), "-o", "comm="])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).contains(image)
        }
    }

    #[test]
    fn dropping_the_transport_kills_a_server_that_does_not_exit() {
        #[cfg(windows)]
        let (cmd, args) = ("cmd", ["/C", "ping -n 30 127.0.0.1 >NUL"]);
        // Spawned directly: a shell may exec its last command (bash does),
        // which would change the process image name.
        #[cfg(not(windows))]
        let (cmd, args) = ("sleep", ["30"]);
        let transport = LspTransport::spawn(cmd, &args, ".").unwrap();
        let pid = transport.child_id().unwrap();
        assert!(process_exists(pid, cmd));
        drop(transport);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while process_exists(pid, cmd) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            !process_exists(pid, cmd),
            "the server is killed after the drop"
        );
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
    fn skips_headers_after_content_length() {
        let body = r#"{"id":8}"#;
        let input = format!(
            "Content-Length: {}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{}{}",
            body.len(),
            body,
            frame("2")
        );
        assert_eq!(read_all(input.as_bytes()), vec![json!({"id": 8}), json!(2)]);
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

    #[test]
    fn connect_talks_to_a_listening_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let (tx, rx) = unbounded();
            std::thread::spawn(move || read_messages(BufReader::new(stream), &tx));
            let msg = rx.recv_timeout(Duration::from_secs(10)).unwrap();
            write_message(&mut writer, &msg).unwrap();
            writer.shutdown(std::net::Shutdown::Both).unwrap();
        });
        let mut t = LspTransport::connect(&addr).unwrap();
        t.send(&json!({"id": 1})).unwrap();
        assert_eq!(
            t.receiver.recv_timeout(Duration::from_secs(10)).unwrap(),
            json!({"id": 1})
        );
        server.join().unwrap();
        // The server closed the connection: the transport is dead.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while t.is_alive.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!t.is_alive.load(Ordering::Relaxed));
    }

    #[test]
    fn dropping_a_connected_transport_closes_the_connection() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let t = LspTransport::connect(&listener.local_addr().unwrap().to_string()).unwrap();
        let (mut accepted, _) = listener.accept().unwrap();
        accepted
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        drop(t);
        let mut buf = [0u8; 1];
        assert_eq!(accepted.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn connect_tries_every_resolved_address() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(LspTransport::connect(&format!("localhost:{port}")).is_ok());
    }

    #[test]
    fn connect_fails_without_a_server() {
        // Port 1 is outside the ephemeral range (never handed out by
        // `bind(0)`), and nothing listens on it.
        assert!(LspTransport::connect("127.0.0.1:1").is_err());
        assert!(LspTransport::connect("not an address").is_err());
    }

    #[test]
    fn abort_ends_the_reader_of_a_server_that_never_answers() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let t = LspTransport::connect(&listener.local_addr().unwrap().to_string()).unwrap();
        // Accepted and silent: kept open for the whole test.
        let (_silent, _) = listener.accept().unwrap();
        let alive = t.is_alive.clone();
        t.abort();
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive.load(Ordering::Relaxed) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!alive.load(Ordering::Relaxed), "reader still running");
    }

    #[test]
    fn a_settled_connection_stops_polling_and_stays_usable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let t = LspTransport::connect(&listener.local_addr().unwrap().to_string()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        t.settle();
        // The reader notices on its next wake-up (within one poll period).
        std::thread::sleep(READ_POLL * 3);
        let timeouts = t.read_timeouts();
        std::thread::sleep(READ_POLL * 4);
        assert_eq!(t.read_timeouts(), timeouts, "no more receive timeouts");
        write_message(&mut server, &json!({"seq": 7})).unwrap();
        let msg = t.receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(msg["seq"], 7);
        assert!(t.is_alive.load(Ordering::Relaxed));
    }

    #[test]
    fn an_unsettled_connection_keeps_polling_for_abort() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let t = LspTransport::connect(&listener.local_addr().unwrap().to_string()).unwrap();
        let (_server, _) = listener.accept().unwrap();
        std::thread::sleep(READ_POLL * 3);
        assert!(t.read_timeouts() >= 1);
    }
}
