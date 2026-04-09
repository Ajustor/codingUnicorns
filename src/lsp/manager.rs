use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use super::client::LspClient;

/// Maps file extension to a running LSP client.
pub struct LspManager {
    clients: HashMap<String, LspClient>,
}

impl LspManager {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
        }
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
    }

    pub fn get_mut(&mut self, ext: &str) -> Option<&mut LspClient> {
        self.clients.get_mut(ext)
    }

    pub fn get(&self, ext: &str) -> Option<&LspClient> {
        self.clients.get(ext)
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
            let msgs = client.poll();
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
