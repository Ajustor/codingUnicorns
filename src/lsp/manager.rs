use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::client::{normalized_path_key, uri_to_path, Diagnostic, LspClient};

/// Diagnostics for one file, merged across every running server.
pub type FileDiagnostics = (PathBuf, Vec<Diagnostic>);

/// Maps file extension to a running LSP client.
pub struct LspManager {
    clients: HashMap<String, LspClient>,
    /// Bumped whenever any client's diagnostics change (or clients go away),
    /// so the workspace-wide Problems view only rebuilds when needed.
    diag_revision: u64,
}

impl LspManager {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
            diag_revision: 0,
        }
    }

    /// Changes whenever the result of [`Self::workspace_diagnostics`] may have.
    pub fn diagnostics_revision(&self) -> u64 {
        self.diag_revision
    }

    /// Every published diagnostic in the workspace, from all servers, grouped
    /// by file (sorted by path; files without diagnostics are omitted). The same
    /// file reported under different URI spellings or by several servers is
    /// merged, and exact duplicates are dropped.
    pub fn workspace_diagnostics(&self) -> Vec<FileDiagnostics> {
        let mut by_key: HashMap<String, FileDiagnostics> = HashMap::new();
        let mut exts: Vec<&String> = self.clients.keys().collect();
        exts.sort();
        for ext in exts {
            for (uri, diags) in &self.clients[ext].diagnostics {
                if diags.is_empty() {
                    continue;
                }
                let entry = by_key
                    .entry(normalized_path_key(uri))
                    .or_insert_with(|| (uri_to_path(uri), Vec::new()));
                for d in diags {
                    let dup = entry.1.iter().any(|e| {
                        e.line == d.line
                            && e.col == d.col
                            && e.severity == d.severity
                            && e.message == d.message
                    });
                    if !dup {
                        entry.1.push(d.clone());
                    }
                }
            }
        }
        let mut out: Vec<FileDiagnostics> = by_key.into_values().collect();
        for (_, diags) in &mut out {
            diags.sort_by_key(|d| (d.line, d.col));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// No-op: LSP servers are only started when an extension provides a command
    /// via `ensure_started_with_cmd()`.
    pub fn ensure_started(&mut self, _ext: &str, _workspace: &Path) {
        // Language support comes exclusively from installable extensions.
    }

    /// Ensure an LSP server is running using an explicit command from the plugin system.
    pub fn ensure_started_with_cmd(
        &mut self,
        ext: &str,
        cmd: &str,
        args: &[String],
        workspace: &Path,
    ) {
        if self.clients.contains_key(ext) {
            return;
        }
        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.start_client(ext, cmd, &args_ref, workspace);
    }

    fn start_client(&mut self, ext: &str, cmd: &str, args: &[&str], workspace: &Path) {
        if self.clients.contains_key(ext) {
            return;
        }
        let mut client = LspClient::new();
        if client.start(cmd, args, workspace).is_ok() {
            self.clients.insert(ext.to_string(), client);
        }
    }

    /// Stop and remove all LSP clients. They will be restarted on the next
    /// file interaction if an extension provides the command.
    pub fn restart_all(&mut self) {
        self.clients.clear();
        self.diag_revision = self.diag_revision.wrapping_add(1);
    }

    pub fn get_mut(&mut self, ext: &str) -> Option<&mut LspClient> {
        self.clients.get_mut(ext)
    }

    pub fn get(&self, ext: &str) -> Option<&LspClient> {
        self.clients.get(ext)
    }

    /// True if any client is doing background work (drives a periodic repaint so
    /// the "loading" status updates without user interaction).
    pub fn any_busy(&self) -> bool {
        self.clients.values().any(|c| c.is_busy())
    }

    /// Poll all active clients and return their pending responses.
    /// Also drives crash detection + auto-restart for disconnected clients.
    /// Returns `(responses, reconnected_exts)` where `reconnected_exts` is the
    /// list of file extensions whose LSP server just reconnected this frame.
    #[allow(clippy::type_complexity)]
    pub fn poll_all(&mut self) -> (HashMap<String, Vec<(u64, Value)>>, Vec<String>) {
        let mut results = HashMap::new();
        let mut reconnected = Vec::new();
        for (ext, client) in &mut self.clients {
            let was_connected = client.is_connected;
            let generation = client.diagnostics_generation;
            let msgs = client.poll();
            if client.diagnostics_generation != generation {
                self.diag_revision = self.diag_revision.wrapping_add(1);
            }
            if !msgs.is_empty() {
                results.insert(ext.clone(), msgs);
            }
            // Attempt non-blocking restart if the client just crashed.
            if !client.is_connected {
                client.try_restart();
            }
            // Detect successful reconnect this frame.
            if !was_connected && client.is_connected {
                reconnected.push(ext.clone());
            }
        }
        (results, reconnected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::transport::LspTransport;
    use crossbeam_channel::{unbounded, Sender};
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    fn transport() -> (LspTransport, Sender<Value>, Arc<AtomicBool>) {
        let (tx, rx) = unbounded();
        let alive = Arc::new(AtomicBool::new(true));
        let t = LspTransport::from_parts(Box::new(std::io::sink()), rx, alive.clone());
        (t, tx, alive)
    }

    #[test]
    fn new_manager_is_empty() {
        let mut m = LspManager::new();
        m.ensure_started("rs", Path::new("."));
        assert!(m.get("rs").is_none());
        assert!(m.get_mut("rs").is_none());
        assert!(!m.any_busy());
        let (res, rec) = m.poll_all();
        assert!(res.is_empty() && rec.is_empty());
    }

    #[test]
    fn ensure_started_with_cmd_registers_once() {
        let mut m = LspManager::new();
        let dir = tempfile::tempdir().unwrap();
        m.ensure_started_with_cmd(
            "rs",
            "definitely-not-a-real-lsp-binary-xyz",
            &["--stdio".to_string()],
            dir.path(),
        );
        assert!(m.get("rs").is_some());
        assert!(!m.get("rs").unwrap().is_connected);
        // Second call for the same extension is a no-op (does not replace the client).
        m.get_mut("rs")
            .unwrap()
            .completions
            .push(crate::lsp::client::CompletionItem {
                label: "marker".into(),
                detail: None,
                kind: "Text".into(),
                insert_text: None,
            });
        m.ensure_started_with_cmd("rs", "other", &[], dir.path());
        assert_eq!(m.get("rs").unwrap().completions.len(), 1);
        m.start_client("rs", "other", &[], dir.path());
        assert_eq!(m.get("rs").unwrap().completions.len(), 1);

        m.restart_all();
        assert!(m.get("rs").is_none());
    }

    #[test]
    fn poll_all_groups_responses_by_extension_and_reports_busy() {
        let mut m = LspManager::new();
        let (t, tx, _alive) = transport();
        m.clients
            .insert("rs".into(), LspClient::connected_for_test(t));
        tx.send(json!({"id": 5, "result": "ok"})).unwrap();
        tx.send(json!({"method": "$/progress", "params": {"value": {"kind": "begin"}}}))
            .unwrap();
        let (res, rec) = m.poll_all();
        assert!(rec.is_empty(), "already connected → not a reconnect");
        assert_eq!(res["rs"].len(), 1);
        assert_eq!(res["rs"][0].0, 5);
        assert!(m.any_busy());

        // Nothing new → no entry for the extension.
        let (res, _) = m.poll_all();
        assert!(!res.contains_key("rs"));
    }

    fn publish(uri: &str, diags: Value) -> Value {
        json!({"method": "textDocument/publishDiagnostics",
            "params": {"uri": uri, "diagnostics": diags}})
    }

    fn diag(line: u32, col: u32, sev: u64, msg: &str) -> Value {
        json!({"message": msg, "severity": sev,
            "range": {"start": {"line": line, "character": col},
                      "end": {"line": line, "character": col + 1}}})
    }

    #[test]
    fn workspace_diagnostics_merge_all_servers() {
        let mut m = LspManager::new();
        let (t1, tx1, _a1) = transport();
        let (t2, tx2, _a2) = transport();
        m.clients
            .insert("rs".into(), LspClient::connected_for_test(t1));
        m.clients
            .insert("ts".into(), LspClient::connected_for_test(t2));
        let rev0 = m.diagnostics_revision();
        tx1.send(publish(
            "file:///w/b.rs",
            json!([diag(5, 0, 2, "later"), diag(1, 2, 1, "first")]),
        ))
        .unwrap();
        tx1.send(publish("file:///w/empty.rs", json!([]))).unwrap();
        tx2.send(publish("file:///w/a.ts", json!([diag(0, 0, 1, "ts err")])))
            .unwrap();
        // Same file from another server under a different spelling + a duplicate.
        tx2.send(publish(
            "file:///w/%62.rs",
            json!([diag(1, 2, 1, "first"), diag(3, 0, 3, "info")]),
        ))
        .unwrap();
        m.poll_all();
        assert_ne!(m.diagnostics_revision(), rev0);

        let all = m.workspace_diagnostics();
        let paths: Vec<PathBuf> = all.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(
            paths,
            vec![
                crate::lsp::client::uri_to_path("file:///w/a.ts"),
                crate::lsp::client::uri_to_path("file:///w/b.rs"),
            ],
            "sorted by path, empty files omitted"
        );
        let b: Vec<(u32, &str)> = all[1]
            .1
            .iter()
            .map(|d| (d.line, d.message.as_str()))
            .collect();
        assert_eq!(b, vec![(1, "first"), (3, "info"), (5, "later")]);

        // Nothing new → revision unchanged; restart clears everything.
        let rev = m.diagnostics_revision();
        m.poll_all();
        assert_eq!(m.diagnostics_revision(), rev);
        m.restart_all();
        assert_ne!(m.diagnostics_revision(), rev);
        assert!(m.workspace_diagnostics().is_empty());
    }

    #[test]
    fn poll_all_reports_reconnects() {
        let mut m = LspManager::new();
        let (t, _tx, _alive) = transport();
        m.clients
            .insert("py".into(), LspClient::reconnecting_for_test(t));
        let (_, rec) = m.poll_all();
        assert_eq!(rec, vec!["py".to_string()]);
        assert!(m.get("py").unwrap().is_connected);
        let (_, rec) = m.poll_all();
        assert!(rec.is_empty());
    }

    #[test]
    fn poll_all_detects_crash() {
        let mut m = LspManager::new();
        let (t, _tx, alive) = transport();
        m.clients
            .insert("ts".into(), LspClient::connected_for_test(t));
        alive.store(false, Ordering::Relaxed);
        let (_, rec) = m.poll_all();
        assert!(rec.is_empty());
        assert!(!m.get("ts").unwrap().is_connected);
    }
}
