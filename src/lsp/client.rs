use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use crossbeam_channel::Sender;
use serde_json::{json, Value};

use super::transport::LspTransport;

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
}

impl LspClient {
    pub fn new() -> Self {
        Self {
            diagnostics: HashMap::new(),
            completions: vec![],
            is_connected: false,
            inner: None,
            restart_cmd: None,
            last_crash_time: None,
            restart_attempts: 0,
            reconnect_rx: None,
        }
    }

    fn next_id(inner: &mut LspClientInner) -> u64 {
        let id = inner.next_id;
        inner.next_id += 1;
        id
    }

    /// Spawn the LSP server process and perform the initialize handshake.
    pub fn start(&mut self, command: &str, args: &[&str], workspace: &Path) -> anyhow::Result<()> {
        // Save restart info for auto-reconnect.
        self.restart_cmd = Some((
            command.to_string(),
            args.iter().map(|s| s.to_string()).collect(),
            workspace.to_path_buf(),
        ));
        let workspace_str = workspace.to_string_lossy();
        let mut transport = LspTransport::spawn(command, args, &workspace_str)?;

        let id = 1u64;
        transport.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "processId": std::process::id(),
                "rootUri": format!("file://{}", workspace.display()),
                "capabilities": {
                    "textDocument": {
                        "hover": { "contentFormat": ["plaintext", "markdown"] },
                        "completion": { "completionItem": { "snippetSupport": false } },
                        "publishDiagnostics": {}
                    }
                }
            }
        }))?;

        // Wait up to 5 s for the initialize response.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok(msg) = transport.receiver.try_recv() {
                if msg.get("id").and_then(|v| v.as_u64()) == Some(id) {
                    transport.send(&json!({
                        "jsonrpc": "2.0",
                        "method": "initialized",
                        "params": {}
                    }))?;
                    self.is_connected = true;
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        self.inner = Some(LspClientInner {
            transport,
            next_id: id + 1,
            pending: HashMap::new(),
        });
        Ok(())
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
            }
        }

        let Some(inner) = &mut self.inner else {
            return vec![];
        };
        let mut results = Vec::new();

        while let Ok(msg) = inner.transport.receiver.try_recv() {
            crate::lsp::transport::debug_log('<', &msg.to_string());
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
                }
                _ => {}
            }
        }

        // Detect server crash: alive flag cleared by the reader thread on EOF.
        use std::sync::atomic::Ordering;
        if self.is_connected && !inner.transport.is_alive.load(Ordering::Relaxed) {
            self.is_connected = false;
            self.last_crash_time = Some(std::time::Instant::now());
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
        let (tx, rx) = mpsc::channel();
        self.reconnect_rx = Some(rx);

        std::thread::spawn(move || {
            let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            let workspace_str = workspace.to_string_lossy().to_string();
            let Ok(mut transport) = LspTransport::spawn(&cmd, &args_ref, &workspace_str) else {
                return;
            };
            let id = 1u64;
            if transport
                .send(&json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": "initialize",
                    "params": {
                        "processId": std::process::id(),
                        "rootUri": format!("file://{}", workspace.display()),
                        "capabilities": {
                            "textDocument": {
                                "hover": { "contentFormat": ["plaintext", "markdown"] },
                                "completion": { "completionItem": { "snippetSupport": false } },
                                "publishDiagnostics": {}
                            }
                        }
                    }
                }))
                .is_err()
            {
                return;
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while std::time::Instant::now() < deadline {
                if let Ok(msg) = transport.receiver.try_recv() {
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
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });

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
                let items: Vec<Value> = std::iter::repeat(Value::Null).take(n).collect();
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
        let path = uri.strip_prefix("file://").unwrap_or(uri);
        Some((PathBuf::from(path), line))
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

    pub fn get_diagnostics(&self, path: &str) -> Vec<Diagnostic> {
        self.diagnostics.get(path).cloned().unwrap_or_default()
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
        let kind_str = |k: u64| match k {
            1 => "File",
            2 => "Module",
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
            23 => "Struct",
            26 => "TypeParameter",
            _ => "Symbol",
        };
        let mut symbols = Vec::new();
        for item in result {
            let name = item["name"].as_str().unwrap_or("").to_string();
            let kind_num = item["kind"].as_u64().unwrap_or(0);
            let kind = kind_str(kind_num).to_string();
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
            let path = uri.strip_prefix("file://").unwrap_or(uri);
            refs.push((std::path::PathBuf::from(path), line));
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
            let path_str = uri.strip_prefix("file://").unwrap_or(uri);
            let path = std::path::PathBuf::from(path_str);
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
