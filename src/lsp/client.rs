use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use crossbeam_channel::Sender;
use serde_json::{json, Value};

use super::transport::LspTransport;

// ── URI <-> path conversion ──────────────────────────────────────────────────

/// Comparison key for a document URI: the decoded path, case-folded on
/// Windows (case-insensitive filesystem, servers often lowercase `c:`).
pub(crate) fn normalized_path_key(uri: &str) -> String {
    let p = uri_to_path_string(uri, cfg!(windows));
    if cfg!(windows) {
        p.to_lowercase()
    } else {
        p
    }
}

/// Convert a `file://` URI into a filesystem path.
///
/// Percent-escapes are decoded (as UTF-8), `file:///C:/x` (and `file:///c%3A/x`)
/// become `C:\x` on Windows, `file://host/share` becomes a UNC path, and Unix
/// URIs like `file:///home/me/a.rs` map to `/home/me/a.rs`. Anything that is
/// not a `file:` URI is returned verbatim.
pub fn uri_to_path(uri: &str) -> PathBuf {
    PathBuf::from(uri_to_path_string(uri, cfg!(windows)))
}

/// Convert a filesystem path into a `file://` URI (RFC 8089), the inverse of
/// [`uri_to_path`]: separators become `/`, a drive letter gets a leading `/`
/// (`file:///C:/x`), and every byte outside the unreserved set is
/// percent-encoded (spaces, `#`, `%`, non-ASCII, ...).
pub fn path_to_uri(path: &Path) -> String {
    path_to_uri_string(&path.to_string_lossy(), cfg!(windows))
}

fn is_drive_prefix(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2
        && b[0].is_ascii_alphabetic()
        && (b[1] == b':' || b[1] == b'|')
        && (b.len() == 2 || b[2] == b'/' || b[2] == b'\\')
}

fn percent_decode(s: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn uri_to_path_string(uri: &str, windows: bool) -> String {
    let rest = match uri.get(..7) {
        Some(scheme) if scheme.eq_ignore_ascii_case("file://") => &uri[7..],
        _ => return uri.to_string(),
    };
    // Drop any query / fragment.
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let (authority, path) = if is_drive_prefix(rest) {
        // Lenient: legacy `file://C:/x` / `file://C:\x` forms.
        ("", rest)
    } else {
        match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        }
    };
    let mut path = percent_decode(path);
    if path.starts_with('/') && is_drive_prefix(&path[1..]) {
        if windows {
            path.remove(0);
        }
    } else if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
        // UNC: file://server/share/x -> //server/share/x
        path = format!("//{}{}", percent_decode(authority), path);
    }
    if windows {
        // `C|` is an old spelling of `C:`.
        if is_drive_prefix(&path) && path.as_bytes()[1] == b'|' {
            path.replace_range(1..2, ":");
        }
        path = path.replace('/', "\\");
    }
    path
}

fn path_to_uri_string(path: &str, windows: bool) -> String {
    let mut p = if windows {
        let p = path.replace('\\', "/");
        // Strip verbatim prefixes produced by canonicalize().
        if let Some(unc) = p.strip_prefix("//?/UNC/") {
            format!("//{unc}")
        } else if let Some(local) = p.strip_prefix("//?/") {
            local.to_string()
        } else {
            p
        }
    } else {
        path.to_string()
    };
    let mut authority = String::new();
    if windows && p.starts_with("//") {
        let rest = &p[2..];
        let (host, tail) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        authority = host.to_string();
        p = tail.to_string();
    }
    if is_drive_prefix(&p) {
        p.insert(0, '/');
    }
    let mut out = String::from("file://");
    out.push_str(&authority);
    for (i, b) in p.bytes().enumerate() {
        let keep = b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/')
            // Keep the drive colon readable: `/C:/x`.
            || (b == b':' && i == 2 && is_drive_prefix(&p[1..]));
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub message: String,
    pub line: u32,
    pub col: u32,
    pub end_col: u32,
    pub severity: DiagSeverity,
}

#[derive(Debug, Clone)]
pub struct DocumentSymbol {
    pub name: String,
    pub kind: String,
    pub line: u32,
}

/// One result of a `workspace/symbol` query.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceSymbol {
    pub name: String,
    pub kind: String,
    /// Enclosing symbol (module, class, …) as reported by the server.
    pub container: Option<String>,
    pub path: PathBuf,
    pub line: u32,
    pub col: u32,
}

/// Human-readable name of an LSP `SymbolKind` number.
pub fn symbol_kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "File",
        2 => "Module",
        3 => "Namespace",
        4 => "Package",
        5 => "Class",
        6 => "Method",
        7 => "Property",
        8 => "Field",
        9 => "Constructor",
        10 => "Enum",
        11 => "Interface",
        12 => "Function",
        13 => "Variable",
        14 => "Constant",
        15 => "String",
        16 => "Number",
        17 => "Boolean",
        18 => "Array",
        19 => "Object",
        20 => "Key",
        21 => "Null",
        22 => "EnumMember",
        23 => "Struct",
        24 => "Event",
        25 => "Operator",
        26 => "TypeParameter",
        _ => "Symbol",
    }
}

#[derive(Debug, Clone)]
pub struct CodeAction {
    pub title: String,
    pub kind: Option<String>,
    pub command: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiagSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Debug, Clone)]
pub struct CompletionItem {
    pub label: String,
    pub detail: Option<String>,
    pub kind: String,
    pub insert_text: Option<String>,
}

struct LspClientInner {
    transport: LspTransport,
    next_id: u64,
    /// Channels for callers waiting on a specific request id.
    pending: HashMap<u64, Sender<Value>>,
}

pub struct LspClient {
    pub diagnostics: HashMap<String, Vec<Diagnostic>>,
    /// Bumped on every `publishDiagnostics`, so callers can cache derived views.
    pub diagnostics_generation: u64,
    pub completions: Vec<CompletionItem>,
    pub is_connected: bool,
    inner: Option<LspClientInner>,
    /// Command + args + workspace stored for auto-restart.
    restart_cmd: Option<(String, Vec<String>, PathBuf)>,
    /// When the crash was first detected (for exponential back-off).
    last_crash_time: Option<std::time::Instant>,
    /// Number of consecutive restart attempts (drives back-off exponent).
    restart_attempts: u32,
    /// Channel through which a background reconnect thread sends the new inner.
    reconnect_rx: Option<mpsc::Receiver<LspClientInner>>,
    /// Number of in-flight server work-done progresses (solution load, indexing…).
    /// > 0 means the server is busy and not yet ready to answer fully.
    work_done_active: u32,
    /// `initializationOptions` of the `initialize` request (from the
    /// module's manifest), kept for restarts.
    init_options: Option<Value>,
}

impl LspClient {
    pub fn new() -> Self {
        Self {
            diagnostics: HashMap::new(),
            diagnostics_generation: 0,
            completions: vec![],
            is_connected: false,
            inner: None,
            restart_cmd: None,
            last_crash_time: None,
            restart_attempts: 0,
            reconnect_rx: None,
            work_done_active: 0,
            init_options: None,
        }
    }

    /// Set the `initializationOptions` sent by the next (re)start.
    pub fn set_init_options(&mut self, options: Option<Value>) {
        self.init_options = options;
    }

    /// True while the server has at least one active work-done progress
    /// (e.g. csharp-ls loading the MSBuild solution).
    pub fn is_busy(&self) -> bool {
        self.work_done_active > 0
    }

    /// Test seam: a client already connected over `transport`.
    #[cfg(test)]
    pub(crate) fn connected_for_test(transport: LspTransport) -> Self {
        let mut c = Self::new();
        c.inner = Some(LspClientInner {
            transport,
            next_id: 2,
            pending: HashMap::new(),
        });
        c.is_connected = true;
        c
    }

    /// Test seam: a disconnected client whose background handshake has just
    /// completed over `transport` (picked up by the next `poll()`).
    #[cfg(test)]
    pub(crate) fn reconnecting_for_test(transport: LspTransport) -> Self {
        let mut c = Self::new();
        let (tx, rx) = mpsc::channel();
        tx.send(LspClientInner {
            transport,
            next_id: 2,
            pending: HashMap::new(),
        })
        .unwrap();
        c.reconnect_rx = Some(rx);
        c
    }

    fn next_id(inner: &mut LspClientInner) -> u64 {
        let id = inner.next_id;
        inner.next_id += 1;
        id
    }

    /// Spawn the LSP server process and perform the initialize handshake.
    ///
    /// Non-blocking: the (potentially slow) handshake runs on a background thread so
    /// the UI never freezes while a server like rust-analyzer or csharp-ls boots.
    /// `poll()` installs the client and flips `is_connected` once it's ready; the app
    /// treats that as a (re)connect and re-opens the current file so diagnostics flow.
    pub fn start(&mut self, command: &str, args: &[&str], workspace: &Path) -> anyhow::Result<()> {
        let cmd = command.to_string();
        let args_vec: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let workspace = workspace.to_path_buf();
        // Save restart info for auto-reconnect.
        self.restart_cmd = Some((cmd.clone(), args_vec.clone(), workspace.clone()));
        self.reconnect_rx = Some(Self::spawn_handshake(
            cmd,
            args_vec,
            workspace,
            self.init_options.clone(),
        ));
        Ok(())
    }

    /// Spawn the server and run the `initialize`/`initialized` handshake on a
    /// background thread, delivering the ready `LspClientInner` through the returned
    /// channel. Shared by the initial start and the crash auto-restart path.
    fn spawn_handshake(
        cmd: String,
        args: Vec<String>,
        workspace: PathBuf,
        init_options: Option<Value>,
    ) -> mpsc::Receiver<LspClientInner> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            let workspace_str = workspace.to_string_lossy().to_string();
            let Ok(mut transport) = LspTransport::spawn(&cmd, &args_ref, &workspace_str) else {
                return;
            };
            let id = 1u64;
            let mut params = json!({
                "processId": std::process::id(),
                "rootUri": path_to_uri(&workspace),
                "capabilities": {
                    "textDocument": {
                        "hover": { "contentFormat": ["plaintext", "markdown"] },
                        "completion": { "completionItem": { "snippetSupport": false } },
                        "publishDiagnostics": {}
                    },
                    "workspace": {
                        "symbol": {}
                    }
                }
            });
            if let Some(options) = init_options {
                params["initializationOptions"] = options;
            }
            if transport
                .send(&json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": "initialize",
                    "params": params
                }))
                .is_err()
            {
                return;
            }
            // Block (not busy-poll) on the reader channel; slow servers still connect.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while std::time::Instant::now() < deadline {
                if let Ok(msg) = transport
                    .receiver
                    .recv_timeout(std::time::Duration::from_millis(200))
                {
                    if msg.get("id").and_then(|v| v.as_u64()) == Some(id) {
                        let _ = transport.send(&json!({
                            "jsonrpc": "2.0",
                            "method": "initialized",
                            "params": {}
                        }));
                        let inner = LspClientInner {
                            transport,
                            next_id: id + 1,
                            pending: HashMap::new(),
                        };
                        let _ = tx.send(inner);
                        return;
                    }
                }
            }
        });
        rx
    }

    /// Notify the server that a file was opened.
    pub fn did_open(&mut self, uri: &str, language_id: &str, content: &str) {
        let Some(inner) = &mut self.inner else { return };
        let _ = inner.transport.send(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": content
                }
            }
        }));
    }

    /// Notify the server that a file changed.
    pub fn did_change(&mut self, uri: &str, version: i32, content: &str) {
        let Some(inner) = &mut self.inner else { return };
        let _ = inner.transport.send(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{ "text": content }]
            }
        }));
    }

    /// Request hover info. Returns the request id; match it in `poll()` results.
    pub fn request_hover(&mut self, uri: &str, line: u32, character: u32) -> u64 {
        let Some(inner) = &mut self.inner else {
            return 0;
        };
        let id = Self::next_id(inner);
        let _ = inner.transport.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }
        }));
        id
    }

    /// Request completion items. Returns the request id.
    pub fn request_completions(&mut self, uri: &str, line: u32, character: u32) -> u64 {
        let Some(inner) = &mut self.inner else {
            return 0;
        };
        let id = Self::next_id(inner);
        let _ = inner.transport.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }
        }));
        id
    }

    /// Request go-to-definition. Returns the request id.
    pub fn request_definition(&mut self, uri: &str, line: u32, character: u32) -> u64 {
        let Some(inner) = &mut self.inner else {
            return 0;
        };
        let id = Self::next_id(inner);
        let _ = inner.transport.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }
        }));
        id
    }

    /// Drain incoming messages from the server.
    ///
    /// Returns `(request_id, message)` pairs for responses. Notifications such
    /// as `textDocument/publishDiagnostics` are handled internally.
    pub fn poll(&mut self) -> Vec<(u64, Value)> {
        // Check if a background reconnect thread has finished.
        if let Some(rx) = &self.reconnect_rx {
            if let Ok(new_inner) = rx.try_recv() {
                self.inner = Some(new_inner);
                self.is_connected = true;
                self.restart_attempts = 0;
                self.reconnect_rx = None;
                self.last_crash_time = None;
                self.work_done_active = 0;
            }
        }

        let Some(inner) = &mut self.inner else {
            return vec![];
        };
        let mut results = Vec::new();

        while let Ok(msg) = inner.transport.receiver.try_recv() {
            let id = msg.get("id").and_then(|v| v.as_u64());
            let method = msg.get("method").and_then(|v| v.as_str());
            match (id, method) {
                // Server→client REQUEST (has both id and method): must respond, or
                // servers like csharp-ls stall and never answer our own requests.
                (Some(id), Some(method)) => {
                    let resp = Self::server_request_response(method, id, &msg);
                    let _ = inner.transport.send(&resp);
                }
                // Response to one of our requests (id, no method).
                (Some(id), None) => {
                    results.push((id, msg));
                }
                // Notification (method, no id).
                (None, Some("textDocument/publishDiagnostics")) => {
                    Self::process_diagnostics_msg(&mut self.diagnostics, &msg);
                    self.diagnostics_generation = self.diagnostics_generation.wrapping_add(1);
                }
                // Work-done progress — track busy state (solution load / indexing).
                (None, Some("$/progress")) => {
                    let kind = msg
                        .get("params")
                        .and_then(|p| p.get("value"))
                        .and_then(|v| v.get("kind"))
                        .and_then(|k| k.as_str());
                    match kind {
                        Some("begin") => self.work_done_active += 1,
                        Some("end") => {
                            self.work_done_active = self.work_done_active.saturating_sub(1)
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // Detect server crash: alive flag cleared by the reader thread on EOF.
        use std::sync::atomic::Ordering;
        if self.is_connected && !inner.transport.is_alive.load(Ordering::Relaxed) {
            self.is_connected = false;
            self.last_crash_time = Some(std::time::Instant::now());
            self.work_done_active = 0;
        }

        results
    }

    /// Called each frame. Schedules a non-blocking reconnect with exponential back-off.
    /// Returns `true` when a reconnect just succeeded (caller should re-open the current file).
    pub fn try_restart(&mut self) -> bool {
        // Already reconnecting or connected — nothing to do.
        if self.is_connected || self.reconnect_rx.is_some() {
            return false;
        }
        let Some(ref crash_time) = self.last_crash_time else {
            return false;
        };
        let delay = std::time::Duration::from_secs((2u64 << self.restart_attempts.min(4)).min(30));
        if crash_time.elapsed() < delay {
            return false;
        }
        let Some((cmd, args, workspace)) = self.restart_cmd.clone() else {
            return false;
        };

        self.restart_attempts += 1;
        self.reconnect_rx = Some(Self::spawn_handshake(
            cmd,
            args,
            workspace,
            self.init_options.clone(),
        ));

        false
    }

    /// Build a reply to a server→client request so the server doesn't stall.
    /// `workspace/configuration` must return one entry per requested item (null =
    /// use defaults); everything else (registerCapability, workDoneProgress/create,
    /// *​/refresh, …) is acked with a null result.
    fn server_request_response(method: &str, id: u64, msg: &Value) -> Value {
        match method {
            "workspace/configuration" => {
                let n = msg
                    .get("params")
                    .and_then(|p| p.get("items"))
                    .and_then(|i| i.as_array())
                    .map(|a| a.len())
                    .unwrap_or(1)
                    .max(1);
                let items: Vec<Value> = std::iter::repeat_n(Value::Null, n).collect();
                json!({ "jsonrpc": "2.0", "id": id, "result": items })
            }
            _ => json!({ "jsonrpc": "2.0", "id": id, "result": Value::Null }),
        }
    }

    fn process_diagnostics_msg(store: &mut HashMap<String, Vec<Diagnostic>>, msg: &Value) {
        let uri = msg["params"]["uri"].as_str().unwrap_or("").to_string();
        let mut diags = Vec::new();
        if let Some(arr) = msg["params"]["diagnostics"].as_array() {
            for d in arr {
                let severity = match d["severity"].as_u64().unwrap_or(1) {
                    1 => DiagSeverity::Error,
                    2 => DiagSeverity::Warning,
                    3 => DiagSeverity::Info,
                    _ => DiagSeverity::Hint,
                };
                diags.push(Diagnostic {
                    message: d["message"].as_str().unwrap_or("").to_string(),
                    line: d["range"]["start"]["line"].as_u64().unwrap_or(0) as u32,
                    col: d["range"]["start"]["character"].as_u64().unwrap_or(0) as u32,
                    end_col: d["range"]["end"]["character"].as_u64().unwrap_or(0) as u32,
                    severity,
                });
            }
        }
        store.insert(uri, diags);
    }

    /// Parse a hover response into a display string.
    pub fn parse_hover(response: &Value) -> Option<String> {
        let contents = response.get("result")?.get("contents")?;
        if let Some(s) = contents.as_str() {
            return Some(s.to_string());
        }
        if let Some(obj) = contents.as_object() {
            if let Some(value) = obj.get("value").and_then(|v| v.as_str()) {
                return Some(value.to_string());
            }
        }
        if let Some(arr) = contents.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
                if let Some(v) = item.get("value").and_then(|v| v.as_str()) {
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
            }
        }
        None
    }

    /// Parse a definition response into `(file_path, line)`.
    pub fn parse_definition(response: &Value) -> Option<(PathBuf, u32)> {
        let result = response.get("result")?;
        let loc = if result.is_array() {
            result.as_array()?.first()?
        } else {
            result
        };
        let uri = loc.get("uri").and_then(|v| v.as_str())?;
        let line = loc["range"]["start"]["line"].as_u64()? as u32;
        Some((uri_to_path(uri), line))
    }

    /// Parse a completion response into a list of items (capped at 50).
    pub fn parse_completions(response: &Value) -> Vec<CompletionItem> {
        let result = response.get("result");
        let items = result
            .and_then(|r| r.get("items"))
            .or(result)
            .and_then(|v| v.as_array());

        let mut completions = Vec::new();
        if let Some(arr) = items {
            for item in arr.iter().take(50) {
                let kind_num = item["kind"].as_u64().unwrap_or(0);
                let kind = match kind_num {
                    1 => "Text",
                    2 => "Method",
                    3 => "Function",
                    4 => "Constructor",
                    5 => "Field",
                    6 => "Variable",
                    7 => "Class",
                    8 => "Interface",
                    9 => "Module",
                    10 => "Property",
                    14 => "Keyword",
                    15 => "Snippet",
                    _ => "Value",
                };
                completions.push(CompletionItem {
                    label: item["label"].as_str().unwrap_or("").to_string(),
                    detail: item["detail"].as_str().map(|s| s.to_string()),
                    kind: kind.to_string(),
                    insert_text: item["insertText"].as_str().map(|s| s.to_string()),
                });
            }
        }
        completions
    }

    /// Diagnostics for a document URI. Falls back to comparing decoded paths,
    /// because servers often normalise URIs (percent-encoding, drive-letter
    /// case) differently from the URI the editor sent.
    pub fn get_diagnostics(&self, path: &str) -> Vec<Diagnostic> {
        if let Some(d) = self.diagnostics.get(path) {
            return d.clone();
        }
        let wanted = normalized_path_key(path);
        self.diagnostics
            .iter()
            .find(|(k, _)| normalized_path_key(k) == wanted)
            .map(|(_, d)| d.clone())
            .unwrap_or_default()
    }

    /// Request document symbols. Returns the request id.
    pub fn request_document_symbols(&mut self, uri: &str) -> u64 {
        let Some(inner) = &mut self.inner else {
            return 0;
        };
        let id = Self::next_id(inner);
        let _ = inner.transport.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/documentSymbol",
            "params": {
                "textDocument": { "uri": uri }
            }
        }));
        id
    }

    /// Parse a documentSymbol response into a list of DocumentSymbol entries.
    pub fn parse_document_symbols(response: &Value) -> Vec<DocumentSymbol> {
        let result = match response.get("result") {
            Some(r) if r.is_array() => r.as_array().unwrap(),
            _ => return vec![],
        };
        let mut symbols = Vec::new();
        for item in result {
            let name = item["name"].as_str().unwrap_or("").to_string();
            let kind_num = item["kind"].as_u64().unwrap_or(0);
            let kind = symbol_kind_name(kind_num).to_string();
            // DocumentSymbol format uses `range`, SymbolInformation uses `location.range`
            let line = item["range"]["start"]["line"]
                .as_u64()
                .or_else(|| item["location"]["range"]["start"]["line"].as_u64())
                .unwrap_or(0) as u32;
            if !name.is_empty() {
                symbols.push(DocumentSymbol { name, kind, line });
            }
        }
        symbols
    }

    /// Request `workspace/symbol` for `query`. Returns the request id.
    pub fn request_workspace_symbols(&mut self, query: &str) -> u64 {
        let Some(inner) = &mut self.inner else {
            return 0;
        };
        let id = Self::next_id(inner);
        let _ = inner.transport.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "workspace/symbol",
            "params": { "query": query }
        }));
        id
    }

    /// Parse a `workspace/symbol` response. Accepts both `SymbolInformation[]`
    /// (`location.range`) and `WorkspaceSymbol[]` (whose `location` may be just
    /// `{ uri }`, in which case the position defaults to the top of the file).
    pub fn parse_workspace_symbols(response: &Value) -> Vec<WorkspaceSymbol> {
        let Some(result) = response.get("result").and_then(|r| r.as_array()) else {
            return vec![];
        };
        result
            .iter()
            .filter_map(|item| {
                let name = item["name"].as_str().filter(|n| !n.is_empty())?;
                let uri = item["location"]["uri"].as_str()?;
                let start = &item["location"]["range"]["start"];
                Some(WorkspaceSymbol {
                    name: name.to_string(),
                    kind: symbol_kind_name(item["kind"].as_u64().unwrap_or(0)).to_string(),
                    container: item["containerName"]
                        .as_str()
                        .filter(|c| !c.is_empty())
                        .map(|c| c.to_string()),
                    path: uri_to_path(uri),
                    line: start["line"].as_u64().unwrap_or(0) as u32,
                    col: start["character"].as_u64().unwrap_or(0) as u32,
                })
            })
            .collect()
    }

    /// Request find-all-references. Returns the request id.
    pub fn request_references(&mut self, uri: &str, line: u32, character: u32) -> u64 {
        let Some(inner) = &mut self.inner else {
            return 0;
        };
        let id = Self::next_id(inner);
        let _ = inner.transport.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/references",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
                "context": { "includeDeclaration": true }
            }
        }));
        id
    }

    /// Parse a references response into a list of (file_path, line).
    pub fn parse_references(response: &Value) -> Vec<(std::path::PathBuf, u32)> {
        let result = match response.get("result") {
            Some(r) if r.is_array() => r.as_array().unwrap(),
            _ => return vec![],
        };
        let mut refs = Vec::new();
        for item in result {
            let uri = item["uri"].as_str().unwrap_or("");
            let line = item["range"]["start"]["line"].as_u64().unwrap_or(0) as u32;
            refs.push((uri_to_path(uri), line));
        }
        refs
    }

    /// Request rename. Returns the request id.
    pub fn request_rename(&mut self, uri: &str, line: u32, character: u32, new_name: &str) -> u64 {
        let Some(inner) = &mut self.inner else {
            return 0;
        };
        let id = Self::next_id(inner);
        let _ = inner.transport.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/rename",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character },
                "newName": new_name
            }
        }));
        id
    }

    /// Parse a rename response into a list of (file_path, edits).
    /// Each edit is (line, start_col, end_col, new_text).
    #[allow(clippy::type_complexity)]
    pub fn apply_rename(
        response: &Value,
    ) -> Vec<(std::path::PathBuf, Vec<(u32, u32, u32, String)>)> {
        let result = match response.get("result") {
            Some(r) => r,
            None => return vec![],
        };
        let changes = match result.get("changes") {
            Some(c) if c.is_object() => c.as_object().unwrap(),
            _ => return vec![],
        };
        let mut out = Vec::new();
        for (uri, edits_val) in changes {
            let path = uri_to_path(uri);
            let mut file_edits = Vec::new();
            if let Some(arr) = edits_val.as_array() {
                for edit in arr {
                    let line = edit["range"]["start"]["line"].as_u64().unwrap_or(0) as u32;
                    let start_col =
                        edit["range"]["start"]["character"].as_u64().unwrap_or(0) as u32;
                    let end_col = edit["range"]["end"]["character"].as_u64().unwrap_or(0) as u32;
                    let new_text = edit["newText"].as_str().unwrap_or("").to_string();
                    file_edits.push((line, start_col, end_col, new_text));
                }
            }
            out.push((path, file_edits));
        }
        out
    }

    /// Request code actions. Returns the request id.
    pub fn request_code_actions(
        &mut self,
        uri: &str,
        line: u32,
        character: u32,
        diag_messages: &[String],
    ) -> u64 {
        let Some(inner) = &mut self.inner else {
            return 0;
        };
        let id = Self::next_id(inner);
        let diagnostics_json: Vec<Value> = diag_messages
            .iter()
            .map(|msg| {
                json!({
                    "range": {
                        "start": { "line": line, "character": character },
                        "end": { "line": line, "character": character }
                    },
                    "message": msg
                })
            })
            .collect();
        let _ = inner.transport.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/codeAction",
            "params": {
                "textDocument": { "uri": uri },
                "range": {
                    "start": { "line": line, "character": character },
                    "end": { "line": line, "character": character }
                },
                "context": {
                    "diagnostics": diagnostics_json
                }
            }
        }));
        id
    }

    /// Parse a codeAction response.
    pub fn parse_code_actions(response: &Value) -> Vec<CodeAction> {
        let result = match response.get("result") {
            Some(r) if r.is_array() => r.as_array().unwrap(),
            _ => return vec![],
        };
        let mut actions = Vec::new();
        for item in result {
            let title = item["title"].as_str().unwrap_or("").to_string();
            if title.is_empty() {
                continue;
            }
            let kind = item["kind"].as_str().map(|s| s.to_string());
            let command = item["command"]["command"]
                .as_str()
                .or_else(|| item["command"].as_str())
                .map(|s| s.to_string());
            actions.push(CodeAction {
                title,
                kind,
                command,
            });
        }
        actions
    }

    /// Request signature help. Returns the request id.
    pub fn request_signature_help(&mut self, uri: &str, line: u32, character: u32) -> u64 {
        let Some(inner) = &mut self.inner else {
            return 0;
        };
        let id = Self::next_id(inner);
        let _ = inner.transport.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/signatureHelp",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }
        }));
        id
    }

    pub fn request_formatting(&mut self, uri: &str, tab_size: u32, insert_spaces: bool) -> u64 {
        let Some(inner) = &mut self.inner else {
            return 0;
        };
        let id = Self::next_id(inner);
        let _ = inner.transport.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/formatting",
            "params": {
                "textDocument": { "uri": uri },
                "options": {
                    "tabSize": tab_size,
                    "insertSpaces": insert_spaces
                }
            }
        }));
        id
    }

    /// Parse a textEdit list from a formatting response into a Vec of (range, newText).
    pub fn parse_text_edits(response: &Value) -> Vec<(u32, u32, u32, u32, String)> {
        let Some(edits) = response.get("result").and_then(|r| r.as_array()) else {
            return vec![];
        };
        edits
            .iter()
            .filter_map(|edit| {
                let range = edit.get("range")?;
                let start = range.get("start")?;
                let end = range.get("end")?;
                let new_text = edit.get("newText")?.as_str()?.to_string();
                Some((
                    start["line"].as_u64()? as u32,
                    start["character"].as_u64()? as u32,
                    end["line"].as_u64()? as u32,
                    end["character"].as_u64()? as u32,
                    new_text,
                ))
            })
            .collect()
    }

    /// Parse a signatureHelp response into a display string.
    pub fn parse_signature_help(response: &Value) -> Option<String> {
        let result = response.get("result")?;
        let signatures = result.get("signatures")?.as_array()?;
        let sig = signatures.first()?;
        let label = sig["label"].as_str()?;
        if label.is_empty() {
            return None;
        }
        // Optionally highlight the active parameter
        let active_param = result["activeParameter"]
            .as_u64()
            .or_else(|| sig["activeParameter"].as_u64());
        if let Some(param_idx) = active_param {
            if let Some(params) = sig["parameters"].as_array() {
                if let Some(param) = params.get(param_idx as usize) {
                    let param_label = param["label"].as_str().unwrap_or("");
                    if !param_label.is_empty() {
                        return Some(format!("{} [active: {}]", label, param_label));
                    }
                }
            }
        }
        Some(label.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);
    impl Write for Shared {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct Harness {
        client: LspClient,
        out: Shared,
        tx: Sender<Value>,
        alive: Arc<AtomicBool>,
    }

    impl Harness {
        fn sent(&self) -> Vec<Value> {
            let bytes = std::mem::take(&mut *self.out.0.lock().unwrap());
            let (tx, rx) = unbounded();
            crate::lsp::transport::read_messages(std::io::Cursor::new(bytes), &tx);
            drop(tx);
            rx.try_iter().collect()
        }
    }

    fn transport() -> (LspTransport, Shared, Sender<Value>, Arc<AtomicBool>) {
        let out = Shared::default();
        let (tx, rx) = unbounded();
        let alive = Arc::new(AtomicBool::new(true));
        let t = LspTransport::from_parts(Box::new(out.clone()), rx, alive.clone());
        (t, out, tx, alive)
    }

    fn connected() -> Harness {
        let (t, out, tx, alive) = transport();
        Harness {
            client: LspClient::connected_for_test(t),
            out,
            tx,
            alive,
        }
    }

    // ── Disconnected client ──────────────────────────────────────────────

    #[test]
    fn disconnected_client_requests_return_zero_and_noop() {
        let mut c = LspClient::new();
        assert!(!c.is_connected);
        assert!(!c.is_busy());
        c.did_open("file:///a.rs", "rust", "fn main() {}");
        c.did_change("file:///a.rs", 2, "x");
        assert_eq!(c.request_hover("u", 0, 0), 0);
        assert_eq!(c.request_completions("u", 0, 0), 0);
        assert_eq!(c.request_definition("u", 0, 0), 0);
        assert_eq!(c.request_document_symbols("u"), 0);
        assert_eq!(c.request_references("u", 0, 0), 0);
        assert_eq!(c.request_rename("u", 0, 0, "n"), 0);
        assert_eq!(c.request_code_actions("u", 0, 0, &[]), 0);
        assert_eq!(c.request_signature_help("u", 0, 0), 0);
        assert_eq!(c.request_formatting("u", 4, true), 0);
        assert!(c.poll().is_empty());
        assert!(!c.try_restart(), "no crash recorded → no restart");
        assert!(c.get_diagnostics("u").is_empty());
    }

    // ── Request building ─────────────────────────────────────────────────

    #[test]
    fn notifications_have_no_id() {
        let mut h = connected();
        h.client.did_open("file:///a.rs", "rust", "fn main() {}");
        h.client.did_change("file:///a.rs", 5, "fn main() { }");
        let sent = h.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0]["method"], "textDocument/didOpen");
        assert!(sent[0].get("id").is_none());
        assert_eq!(sent[0]["params"]["textDocument"]["languageId"], "rust");
        assert_eq!(sent[0]["params"]["textDocument"]["version"], 1);
        assert_eq!(sent[0]["params"]["textDocument"]["text"], "fn main() {}");
        assert_eq!(sent[1]["method"], "textDocument/didChange");
        assert_eq!(sent[1]["params"]["textDocument"]["version"], 5);
        assert_eq!(
            sent[1]["params"]["contentChanges"],
            json!([{"text": "fn main() { }"}])
        );
    }

    #[test]
    fn requests_get_increasing_ids_and_correct_params() {
        let mut h = connected();
        let ids = [
            h.client.request_hover("u", 1, 2),
            h.client.request_completions("u", 3, 4),
            h.client.request_definition("u", 5, 6),
            h.client.request_document_symbols("u"),
            h.client.request_references("u", 7, 8),
            h.client.request_rename("u", 9, 10, "renamed"),
            h.client
                .request_code_actions("u", 11, 12, &["unused".to_string()]),
            h.client.request_signature_help("u", 13, 14),
            h.client.request_formatting("u", 2, false),
        ];
        assert_eq!(ids, [2, 3, 4, 5, 6, 7, 8, 9, 10]);
        let sent = h.sent();
        let methods: Vec<&str> = sent.iter().map(|m| m["method"].as_str().unwrap()).collect();
        assert_eq!(
            methods,
            vec![
                "textDocument/hover",
                "textDocument/completion",
                "textDocument/definition",
                "textDocument/documentSymbol",
                "textDocument/references",
                "textDocument/rename",
                "textDocument/codeAction",
                "textDocument/signatureHelp",
                "textDocument/formatting",
            ]
        );
        for (m, id) in sent.iter().zip(ids) {
            assert_eq!(m["id"], id);
            assert_eq!(m["jsonrpc"], "2.0");
            assert_eq!(m["params"]["textDocument"]["uri"], "u");
        }
        assert_eq!(
            sent[0]["params"]["position"],
            json!({"line": 1, "character": 2})
        );
        assert_eq!(sent[4]["params"]["context"]["includeDeclaration"], true);
        assert_eq!(sent[5]["params"]["newName"], "renamed");
        let diags = &sent[6]["params"]["context"]["diagnostics"];
        assert_eq!(diags[0]["message"], "unused");
        assert_eq!(diags[0]["range"]["start"]["line"], 11);
        assert_eq!(sent[6]["params"]["range"]["end"]["character"], 12);
        assert_eq!(
            sent[8]["params"]["options"],
            json!({"tabSize": 2, "insertSpaces": false})
        );
    }

    // ── poll() ───────────────────────────────────────────────────────────

    #[test]
    fn poll_returns_responses_and_handles_notifications() {
        let mut h = connected();
        h.tx.send(json!({"jsonrpc": "2.0", "id": 2, "result": null}))
            .unwrap();
        h.tx.send(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {"uri": "file:///a.rs", "diagnostics": [
                {"message": "bad", "severity": 1,
                 "range": {"start": {"line": 3, "character": 4}, "end": {"line": 3, "character": 9}}},
                {"message": "warn", "severity": 2, "range": {}},
                {"message": "info", "severity": 3},
                {"message": "hint", "severity": 4},
                {}
            ]}
        }))
        .unwrap();
        h.tx.send(json!({"jsonrpc": "2.0", "method": "window/logMessage", "params": {}}))
            .unwrap();
        let res = h.client.poll();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].0, 2);

        let d = h.client.get_diagnostics("file:///a.rs");
        assert_eq!(d.len(), 5);
        assert_eq!(d[0].message, "bad");
        assert_eq!((d[0].line, d[0].col, d[0].end_col), (3, 4, 9));
        assert_eq!(d[0].severity, DiagSeverity::Error);
        assert_eq!(d[1].severity, DiagSeverity::Warning);
        assert_eq!(d[2].severity, DiagSeverity::Info);
        assert_eq!(d[3].severity, DiagSeverity::Hint);
        // Missing severity defaults to Error, missing fields to 0/"".
        assert_eq!(d[4].severity, DiagSeverity::Error);
        assert_eq!(d[4].message, "");
        assert!(h.sent().is_empty(), "no reply to notifications");
    }

    #[test]
    fn publish_diagnostics_replaces_previous_set() {
        let mut h = connected();
        let pd = |n: usize| {
            let diags: Vec<Value> = (0..n).map(|_| json!({"message": "m"})).collect();
            json!({"method": "textDocument/publishDiagnostics",
                "params": {"uri": "u", "diagnostics": diags}})
        };
        h.tx.send(pd(3)).unwrap();
        h.client.poll();
        assert_eq!(h.client.get_diagnostics("u").len(), 3);
        h.tx.send(pd(0)).unwrap();
        h.client.poll();
        assert!(h.client.get_diagnostics("u").is_empty());
    }

    #[test]
    fn server_requests_are_answered() {
        let mut h = connected();
        h.tx.send(json!({"id": 40, "method": "workspace/configuration",
            "params": {"items": [{}, {}, {}]}}))
            .unwrap();
        h.tx.send(json!({"id": 41, "method": "workspace/configuration", "params": {}}))
            .unwrap();
        h.tx.send(json!({"id": 42, "method": "workspace/configuration",
            "params": {"items": []}}))
            .unwrap();
        h.tx.send(json!({"id": 43, "method": "client/registerCapability"}))
            .unwrap();
        let res = h.client.poll();
        assert!(
            res.is_empty(),
            "server requests are not surfaced as responses"
        );
        let sent = h.sent();
        assert_eq!(sent.len(), 4);
        assert_eq!(
            sent[0],
            json!({"jsonrpc": "2.0", "id": 40, "result": [null, null, null]})
        );
        assert_eq!(sent[1]["result"], json!([null]));
        assert_eq!(sent[2]["result"], json!([null]), "at least one entry");
        assert_eq!(sent[3], json!({"jsonrpc": "2.0", "id": 43, "result": null}));
    }

    #[test]
    fn progress_notifications_track_busy_state() {
        let mut h = connected();
        let prog =
            |kind: &str| json!({"method": "$/progress", "params": {"value": {"kind": kind}}});
        h.tx.send(prog("begin")).unwrap();
        h.tx.send(prog("begin")).unwrap();
        h.tx.send(prog("report")).unwrap();
        h.client.poll();
        assert!(h.client.is_busy());
        h.tx.send(prog("end")).unwrap();
        h.client.poll();
        assert!(h.client.is_busy());
        h.tx.send(prog("end")).unwrap();
        h.tx.send(prog("end")).unwrap(); // extra end must not underflow
        h.client.poll();
        assert!(!h.client.is_busy());
    }

    #[test]
    fn crash_is_detected_and_restart_backs_off() {
        let mut h = connected();
        h.tx.send(json!({"method": "$/progress", "params": {"value": {"kind": "begin"}}}))
            .unwrap();
        h.client.poll();
        assert!(h.client.is_busy());

        h.alive.store(false, Ordering::Relaxed);
        h.client.poll();
        assert!(!h.client.is_connected);
        assert!(!h.client.is_busy(), "busy state reset on crash");
        assert!(h.client.last_crash_time.is_some());

        // Within the back-off window → no restart.
        h.client.restart_cmd = Some((
            "definitely-not-a-real-lsp-binary-xyz".into(),
            vec![],
            std::env::temp_dir(),
        ));
        assert!(!h.client.try_restart());
        assert_eq!(h.client.restart_attempts, 0);
        assert!(h.client.reconnect_rx.is_none());

        // After the back-off window a reconnect is scheduled.
        h.client.last_crash_time =
            std::time::Instant::now().checked_sub(std::time::Duration::from_secs(5));
        assert!(!h.client.try_restart());
        assert_eq!(h.client.restart_attempts, 1);
        assert!(h.client.reconnect_rx.is_some());

        // While a reconnect is pending, nothing else is scheduled.
        assert!(!h.client.try_restart());
        assert_eq!(h.client.restart_attempts, 1);
    }

    #[test]
    fn restart_without_command_does_nothing() {
        let mut c = LspClient::new();
        c.last_crash_time =
            std::time::Instant::now().checked_sub(std::time::Duration::from_secs(60));
        assert!(!c.try_restart());
        assert!(c.reconnect_rx.is_none());
    }

    #[test]
    fn connected_client_never_restarts() {
        let mut h = connected();
        h.client.last_crash_time = Some(std::time::Instant::now());
        assert!(!h.client.try_restart());
    }

    #[test]
    fn poll_installs_completed_reconnect() {
        let (t, out, tx, _alive) = transport();
        let mut c = LspClient::reconnecting_for_test(t);
        c.restart_attempts = 3;
        c.last_crash_time = Some(std::time::Instant::now());
        c.work_done_active = 2;
        tx.send(json!({"id": 9, "result": 1})).unwrap();
        let res = c.poll();
        assert!(c.is_connected);
        assert_eq!(c.restart_attempts, 0);
        assert!(c.last_crash_time.is_none());
        assert!(c.reconnect_rx.is_none());
        assert!(!c.is_busy());
        assert_eq!(res.len(), 1);
        // The new inner continues numbering from its own next_id.
        assert_eq!(c.request_hover("u", 0, 0), 2);
        drop(out);
    }

    #[test]
    fn start_records_restart_command_and_fails_quietly_for_missing_binary() {
        let mut c = LspClient::new();
        let dir = tempfile::tempdir().unwrap();
        c.start(
            "definitely-not-a-real-lsp-binary-xyz",
            &["--stdio"],
            dir.path(),
        )
        .unwrap();
        let (cmd, args, ws) = c.restart_cmd.clone().unwrap();
        assert_eq!(cmd, "definitely-not-a-real-lsp-binary-xyz");
        assert_eq!(args, vec!["--stdio".to_string()]);
        assert_eq!(ws, dir.path());
        assert!(c.reconnect_rx.is_some());
        // The handshake thread gives up; the client stays disconnected.
        for _ in 0..50 {
            c.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!c.is_connected);
    }

    // ── Response parsers ─────────────────────────────────────────────────

    #[test]
    fn parse_hover_variants() {
        assert_eq!(
            LspClient::parse_hover(&json!({"result": {"contents": "plain"}})),
            Some("plain".into())
        );
        assert_eq!(
            LspClient::parse_hover(
                &json!({"result": {"contents": {"kind": "markdown", "value": "**md**"}}})
            ),
            Some("**md**".into())
        );
        assert_eq!(
            LspClient::parse_hover(&json!({"result": {"contents": ["", "second"]}})),
            Some("second".into())
        );
        assert_eq!(
            LspClient::parse_hover(&json!({"result": {"contents": [
                {"language": "rust", "value": ""},
                {"language": "rust", "value": "fn x()"}
            ]}})),
            Some("fn x()".into())
        );
        assert_eq!(
            LspClient::parse_hover(&json!({"result": {"contents": [""]}})),
            None
        );
        assert_eq!(
            LspClient::parse_hover(&json!({"result": {"contents": {"kind": "x"}}})),
            None
        );
        assert_eq!(
            LspClient::parse_hover(&json!({"result": {"contents": 5}})),
            None
        );
        assert_eq!(LspClient::parse_hover(&json!({"result": null})), None);
        assert_eq!(LspClient::parse_hover(&json!({})), None);
    }

    #[test]
    fn parse_definition_variants() {
        let loc =
            json!({"uri": "file:///src/a.rs", "range": {"start": {"line": 4, "character": 0}}});
        assert_eq!(
            LspClient::parse_definition(&json!({"result": loc.clone()})),
            Some((PathBuf::from("/src/a.rs"), 4))
        );
        assert_eq!(
            LspClient::parse_definition(&json!({"result": [loc]})),
            Some((PathBuf::from("/src/a.rs"), 4))
        );
        // Non-file URIs are kept verbatim.
        assert_eq!(
            LspClient::parse_definition(&json!({"result": {"uri": "untitled:1",
                "range": {"start": {"line": 0}}}})),
            Some((PathBuf::from("untitled:1"), 0))
        );
        assert_eq!(LspClient::parse_definition(&json!({"result": []})), None);
        assert_eq!(LspClient::parse_definition(&json!({"result": null})), None);
        assert_eq!(
            LspClient::parse_definition(&json!({"result": {"uri": "file:///a"}})),
            None,
            "missing range"
        );
        assert_eq!(LspClient::parse_definition(&json!({})), None);
    }

    #[test]
    fn parse_definition_decodes_percent_encoded_uri() {
        let resp = json!({"result": {"uri": "file:///home/me/my%20proj/a.rs",
            "range": {"start": {"line": 1}}}});
        assert_eq!(
            LspClient::parse_definition(&resp),
            Some((PathBuf::from("/home/me/my proj/a.rs"), 1))
        );
    }

    #[test]
    fn uri_to_path_unix_forms() {
        let f = |u| uri_to_path_string(u, false);
        assert_eq!(f("file:///home/me/a.rs"), "/home/me/a.rs");
        assert_eq!(f("file:///home/me/my%20proj/a.rs"), "/home/me/my proj/a.rs");
        assert_eq!(f("file:///h/%C3%A9t%C3%A9.rs"), "/h/\u{e9}t\u{e9}.rs");
        assert_eq!(f("file:///a/100%25.rs"), "/a/100%.rs");
        assert_eq!(f("file://localhost/etc/x"), "/etc/x");
        assert_eq!(f("FILE:///x"), "/x");
        assert_eq!(f("file:///a.rs#frag"), "/a.rs");
        // Malformed / truncated escapes are kept literally.
        assert_eq!(f("file:///a%2"), "/a%2");
        assert_eq!(f("file:///a%zz"), "/a%zz");
        // Non-file URIs are returned verbatim.
        assert_eq!(f("untitled:1"), "untitled:1");
        assert_eq!(f("b.rs"), "b.rs");
    }

    #[test]
    fn uri_to_path_windows_forms() {
        let f = |u| uri_to_path_string(u, true);
        assert_eq!(f("file:///C:/x/a.rs"), r"C:\x\a.rs");
        assert_eq!(f("file:///c%3A/My%20Docs/a.rs"), r"c:\My Docs\a.rs");
        assert_eq!(f("file:///C|/x"), r"C:\x");
        assert_eq!(f("file://server/share/a.rs"), r"\\server\share\a.rs");
        // Legacy, non-conforming `file://C:\x` produced by old call sites.
        assert_eq!(f(r"file://C:\x\a.rs"), r"C:\x\a.rs");
    }

    #[test]
    fn path_to_uri_forms() {
        assert_eq!(
            path_to_uri_string("/home/me/a.rs", false),
            "file:///home/me/a.rs"
        );
        assert_eq!(
            path_to_uri_string("/home/me/my proj/#1%.rs", false),
            "file:///home/me/my%20proj/%231%25.rs"
        );
        assert_eq!(
            path_to_uri_string("/h/\u{e9}.rs", false),
            "file:///h/%C3%A9.rs"
        );
        assert_eq!(path_to_uri_string(r"/a\b", false), "file:///a%5Cb");
        assert_eq!(
            path_to_uri_string(r"C:\Users\me\my proj\a.rs", true),
            "file:///C:/Users/me/my%20proj/a.rs"
        );
        assert_eq!(path_to_uri_string(r"\\?\C:\x", true), "file:///C:/x");
        assert_eq!(
            path_to_uri_string(r"\\server\share\a.rs", true),
            "file://server/share/a.rs"
        );
        assert_eq!(
            path_to_uri_string(r"\\?\UNC\server\share\a", true),
            "file://server/share/a"
        );
    }

    #[test]
    fn uri_path_roundtrip() {
        for (p, win) in [
            ("/home/me/my proj/\u{e9}#%?.rs", false),
            (r"C:\Users\me\my proj\a b.rs", true),
            (r"\\server\share\x y", true),
        ] {
            assert_eq!(uri_to_path_string(&path_to_uri_string(p, win), win), p);
        }
        // Public helpers agree with the platform-specific inner functions.
        let here = std::env::temp_dir().join("my proj").join("a.rs");
        assert_eq!(uri_to_path(&path_to_uri(&here)), here);
    }

    #[test]
    fn references_and_rename_decode_uris() {
        let refs = LspClient::parse_references(&json!({"result": [
            {"uri": "file:///my%20dir/a.rs", "range": {"start": {"line": 3}}}
        ]}));
        assert_eq!(refs, vec![(PathBuf::from("/my dir/a.rs"), 3)]);
        let out = LspClient::apply_rename(&json!({"result": {"changes": {
            "file:///my%20dir/b.rs": []
        }}}));
        assert_eq!(out[0].0, PathBuf::from("/my dir/b.rs"));
    }

    #[test]
    fn diagnostics_lookup_tolerates_uri_encoding_differences() {
        let mut store = HashMap::new();
        LspClient::process_diagnostics_msg(
            &mut store,
            &json!({"params": {"uri": "file:///my%20dir/a.rs", "diagnostics": [
                {"message": "m", "range": {"start": {"line": 0, "character": 0},
                                           "end": {"line": 0, "character": 1}}}
            ]}}),
        );
        let mut c = LspClient::new();
        c.diagnostics = store;
        assert_eq!(c.get_diagnostics("file:///my%20dir/a.rs").len(), 1);
        assert_eq!(c.get_diagnostics("file:///my dir/a.rs").len(), 1);
        assert!(c.get_diagnostics("file:///other.rs").is_empty());
    }

    #[test]
    fn parse_completions_list_and_array_forms() {
        let items = json!([
            {"label": "a", "kind": 1}, {"label": "b", "kind": 2}, {"label": "c", "kind": 3},
            {"label": "d", "kind": 4}, {"label": "e", "kind": 5}, {"label": "f", "kind": 6},
            {"label": "g", "kind": 7}, {"label": "h", "kind": 8}, {"label": "i", "kind": 9},
            {"label": "j", "kind": 10}, {"label": "k", "kind": 14}, {"label": "l", "kind": 15},
            {"label": "m", "kind": 99, "detail": "d", "insertText": "m()"}, {}
        ]);
        let kinds: Vec<String> = LspClient::parse_completions(&json!({"result": items.clone()}))
            .into_iter()
            .map(|c| c.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                "Text",
                "Method",
                "Function",
                "Constructor",
                "Field",
                "Variable",
                "Class",
                "Interface",
                "Module",
                "Property",
                "Keyword",
                "Snippet",
                "Value",
                "Value"
            ]
        );
        let list = LspClient::parse_completions(
            &json!({"result": {"isIncomplete": false, "items": items}}),
        );
        assert_eq!(list.len(), 14);
        assert_eq!(list[12].detail.as_deref(), Some("d"));
        assert_eq!(list[12].insert_text.as_deref(), Some("m()"));
        assert_eq!(list[13].label, "");
        assert_eq!(list[13].detail, None);
    }

    #[test]
    fn parse_completions_caps_at_50_and_handles_null() {
        let many: Vec<Value> = (0..80).map(|i| json!({"label": i.to_string()})).collect();
        assert_eq!(
            LspClient::parse_completions(&json!({"result": many})).len(),
            50
        );
        assert!(LspClient::parse_completions(&json!({"result": null})).is_empty());
        assert!(LspClient::parse_completions(&json!({})).is_empty());
    }

    #[test]
    fn parse_document_symbols_both_formats() {
        let resp = json!({"result": [
            {"name": "File", "kind": 1, "range": {"start": {"line": 0}}},
            {"name": "m", "kind": 2, "range": {"start": {"line": 1}}},
            {"name": "C", "kind": 5, "range": {"start": {"line": 2}}},
            {"name": "meth", "kind": 6, "location": {"range": {"start": {"line": 3}}}},
            {"name": "p", "kind": 7}, {"name": "f", "kind": 8}, {"name": "ctor", "kind": 9},
            {"name": "E", "kind": 10}, {"name": "I", "kind": 11}, {"name": "fun", "kind": 12},
            {"name": "v", "kind": 13}, {"name": "K", "kind": 14}, {"name": "S", "kind": 23},
            {"name": "T", "kind": 26}, {"name": "other", "kind": 99},
            {"name": "", "kind": 12}
        ]});
        let syms = LspClient::parse_document_symbols(&resp);
        assert_eq!(syms.len(), 15, "nameless symbols are skipped");
        let kinds: Vec<&str> = syms.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                "File",
                "Module",
                "Class",
                "Method",
                "Property",
                "Field",
                "Constructor",
                "Enum",
                "Interface",
                "Function",
                "Variable",
                "Constant",
                "Struct",
                "TypeParameter",
                "Symbol"
            ]
        );
        assert_eq!(syms[2].line, 2);
        assert_eq!(syms[3].line, 3, "SymbolInformation location.range");
        assert_eq!(syms[4].line, 0, "missing range defaults to 0");
        assert!(LspClient::parse_document_symbols(&json!({"result": null})).is_empty());
        assert!(LspClient::parse_document_symbols(&json!({"result": {}})).is_empty());
    }

    #[test]
    fn workspace_symbols_request_and_disconnected_noop() {
        assert_eq!(LspClient::new().request_workspace_symbols("q"), 0);
        let mut h = connected();
        let id = h.client.request_workspace_symbols("Foo");
        assert_eq!(id, 2);
        let sent = h.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["method"], "workspace/symbol");
        assert_eq!(sent[0]["id"], 2);
        assert_eq!(sent[0]["params"], json!({"query": "Foo"}));
    }

    #[test]
    fn parse_workspace_symbols_both_formats() {
        let resp = json!({"result": [
            {"name": "Foo", "kind": 23, "containerName": "crate::a",
             "location": {"uri": "file:///src/a.rs",
                          "range": {"start": {"line": 4, "character": 11}}}},
            // WorkspaceSymbol with a location lacking a range.
            {"name": "bar", "kind": 12, "containerName": "",
             "location": {"uri": "file:///my%20dir/b.rs"}},
            {"name": "", "kind": 12, "location": {"uri": "file:///c.rs"}},
            {"name": "nouri", "kind": 12},
            {"name": "Odd", "kind": 999, "location": {"uri": "file:///d.rs"}}
        ]});
        let syms = LspClient::parse_workspace_symbols(&resp);
        assert_eq!(syms.len(), 3);
        assert_eq!(
            syms[0],
            WorkspaceSymbol {
                name: "Foo".into(),
                kind: "Struct".into(),
                container: Some("crate::a".into()),
                path: PathBuf::from("/src/a.rs"),
                line: 4,
                col: 11,
            }
        );
        assert_eq!(syms[1].kind, "Function");
        assert_eq!(syms[1].container, None, "empty container dropped");
        assert_eq!(syms[1].path, PathBuf::from("/my dir/b.rs"));
        assert_eq!((syms[1].line, syms[1].col), (0, 0));
        assert_eq!(syms[2].kind, "Symbol");
        assert!(LspClient::parse_workspace_symbols(&json!({"result": null})).is_empty());
        assert!(LspClient::parse_workspace_symbols(&json!({})).is_empty());
    }

    #[test]
    fn symbol_kind_names_cover_the_spec() {
        assert_eq!(symbol_kind_name(3), "Namespace");
        assert_eq!(symbol_kind_name(22), "EnumMember");
        assert_eq!(symbol_kind_name(26), "TypeParameter");
        assert_eq!(symbol_kind_name(0), "Symbol");
        for k in 1..=26 {
            assert_ne!(symbol_kind_name(k), "Symbol", "kind {k}");
        }
    }

    #[test]
    fn publish_diagnostics_bumps_generation() {
        let mut h = connected();
        assert_eq!(h.client.diagnostics_generation, 0);
        h.tx.send(json!({"method": "textDocument/publishDiagnostics",
            "params": {"uri": "u", "diagnostics": []}}))
            .unwrap();
        h.tx.send(json!({"method": "window/logMessage", "params": {}}))
            .unwrap();
        h.client.poll();
        assert_eq!(h.client.diagnostics_generation, 1);
        h.client.poll();
        assert_eq!(h.client.diagnostics_generation, 1, "no new publish");
    }

    #[test]
    fn parse_references_list() {
        let resp = json!({"result": [
            {"uri": "file:///a.rs", "range": {"start": {"line": 1}}},
            {"uri": "b.rs", "range": {"start": {"line": 2}}},
            {}
        ]});
        assert_eq!(
            LspClient::parse_references(&resp),
            vec![
                (PathBuf::from("/a.rs"), 1),
                (PathBuf::from("b.rs"), 2),
                (PathBuf::from(""), 0)
            ]
        );
        assert!(LspClient::parse_references(&json!({"result": null})).is_empty());
    }

    #[test]
    fn apply_rename_collects_edits_per_file() {
        let resp = json!({"result": {"changes": {
            "file:///a.rs": [
                {"range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 7}},
                 "newText": "bar"},
                {}
            ],
            "file:///b.rs": "not-an-array"
        }}});
        let mut out = LspClient::apply_rename(&resp);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, PathBuf::from("/a.rs"));
        assert_eq!(
            out[0].1,
            vec![(1, 4, 7, "bar".to_string()), (0, 0, 0, String::new())]
        );
        assert!(out[1].1.is_empty());
        assert!(LspClient::apply_rename(&json!({})).is_empty());
        assert!(LspClient::apply_rename(&json!({"result": {}})).is_empty());
        assert!(LspClient::apply_rename(&json!({"result": {"changes": []}})).is_empty());
    }

    #[test]
    fn parse_code_actions_variants() {
        let resp = json!({"result": [
            {"title": "Fix it", "kind": "quickfix", "command": {"command": "do.fix"}},
            {"title": "Legacy", "command": "legacy.cmd"},
            {"title": "Plain"},
            {"title": ""},
            {}
        ]});
        let acts = LspClient::parse_code_actions(&resp);
        assert_eq!(acts.len(), 3);
        assert_eq!(acts[0].kind.as_deref(), Some("quickfix"));
        assert_eq!(acts[0].command.as_deref(), Some("do.fix"));
        assert_eq!(acts[1].command.as_deref(), Some("legacy.cmd"));
        assert_eq!(acts[2].kind, None);
        assert_eq!(acts[2].command, None);
        assert!(LspClient::parse_code_actions(&json!({"result": null})).is_empty());
    }

    #[test]
    fn parse_text_edits_filters_malformed() {
        let resp = json!({"result": [
            {"range": {"start": {"line": 0, "character": 1}, "end": {"line": 2, "character": 3}},
             "newText": "x"},
            {"range": {"start": {"line": 0, "character": 1}}, "newText": "no end"},
            {"range": {"start": {"line": 0}, "end": {"line": 0, "character": 0}}, "newText": "y"},
            {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}}
        ]});
        assert_eq!(
            LspClient::parse_text_edits(&resp),
            vec![(0, 1, 2, 3, "x".to_string())]
        );
        assert!(LspClient::parse_text_edits(&json!({"result": null})).is_empty());
    }

    #[test]
    fn parse_signature_help_variants() {
        let sig = |active_top: Option<u64>, active_sig: Option<u64>| {
            let mut s = json!({"label": "fn f(a: i32, b: i32)",
                "parameters": [{"label": "a: i32"}, {"label": [12, 18]}]});
            if let Some(a) = active_sig {
                s["activeParameter"] = json!(a);
            }
            let mut r = json!({"signatures": [s]});
            if let Some(a) = active_top {
                r["activeParameter"] = json!(a);
            }
            json!({"result": r})
        };
        assert_eq!(
            LspClient::parse_signature_help(&sig(Some(0), None)),
            Some("fn f(a: i32, b: i32) [active: a: i32]".into())
        );
        assert_eq!(
            LspClient::parse_signature_help(&sig(None, Some(0))),
            Some("fn f(a: i32, b: i32) [active: a: i32]".into())
        );
        // Offset-style parameter label → plain signature.
        assert_eq!(
            LspClient::parse_signature_help(&sig(Some(1), None)),
            Some("fn f(a: i32, b: i32)".into())
        );
        // Out-of-range parameter.
        assert_eq!(
            LspClient::parse_signature_help(&sig(Some(5), None)),
            Some("fn f(a: i32, b: i32)".into())
        );
        assert_eq!(
            LspClient::parse_signature_help(&sig(None, None)),
            Some("fn f(a: i32, b: i32)".into())
        );
        assert_eq!(
            LspClient::parse_signature_help(
                &json!({"result": {"signatures": [{"label": "f()"}], "activeParameter": 0}})
            ),
            Some("f()".into())
        );
        assert_eq!(
            LspClient::parse_signature_help(&json!({"result": {"signatures": [{"label": ""}]}})),
            None
        );
        assert_eq!(
            LspClient::parse_signature_help(&json!({"result": {"signatures": []}})),
            None
        );
        assert_eq!(
            LspClient::parse_signature_help(&json!({"result": null})),
            None
        );
    }
}
