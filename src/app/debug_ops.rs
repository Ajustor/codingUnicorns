use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crossbeam_channel::Sender;
use serde_json::Value;

use super::CodingUnicorns;
use crate::dap::adapters::DebugAdapter;
use crate::dap::launcher::LaunchPlan;
use crate::runner::{vscode_tasks, DebugLaunch};
use crate::ui::layout::SidebarTab;

/// Language server commands asked for by debug launches (sessions started by
/// the language server, e.g. Java's), waiting for their server or its answer.
#[derive(Default)]
pub struct DebugLspBridge {
    /// Not sent yet: the server is starting (or being downloaded).
    queued: Vec<crate::dap::lsp_session::LspCommand>,
    /// Sent: (language, request id) → where the answer goes.
    sent: HashMap<(String, u64), Sender<Result<Value, String>>>,
}

impl CodingUnicorns {
    /// Send the debug launch's language server commands to their server
    /// (started if needed). Call every frame.
    pub(crate) fn serve_debug_lsp_commands(&mut self) {
        let mut pending = std::mem::take(&mut self.debug_lsp.queued);
        pending.extend(self.dap.take_lsp_commands());
        for c in pending {
            self.ensure_lsp_for_language(&c.language);
            match self.lsp.get_mut(&c.language) {
                Some(client) if client.is_connected => {
                    let id = client.request_execute_command(&c.command, c.args);
                    self.debug_lsp.sent.insert((c.language, id), c.reply);
                }
                // Starting: sent once it is up (the launcher gives up after
                // a while).
                Some(_) => self.debug_lsp.queued.push(c),
                None if self.lsp_downloads.is_running() => self.debug_lsp.queued.push(c),
                None => {
                    let _ = c
                        .reply
                        .send(Err(format!("no language server for .{} files", c.language)));
                }
            }
        }
    }

    /// Route the answer to a debug launch's command. True when consumed.
    pub(crate) fn handle_debug_lsp_response(
        &mut self,
        ext: &str,
        id: u64,
        response: &Value,
    ) -> bool {
        let Some(reply) = self.debug_lsp.sent.remove(&(ext.to_string(), id)) else {
            return false;
        };
        let answer = match response.get("error") {
            Some(e) => Err(e["message"].as_str().unwrap_or("error").to_string()),
            None => Ok(response.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = reply.send(answer);
        true
    }

    /// Start a DAP debug session. Uses the active run configuration's debug
    /// settings (imported from `.vscode/launch.json`) when it has some, else
    /// the first one handled by the current file's module, else the module's
    /// defaults. The debug adapter always comes from a language module
    /// (`[debugger]` in its manifest); the configuration's `preLaunchTask`
    /// runs first.
    pub fn start_debug_session(&mut self) {
        let current_file = self.editor.current_path.clone();
        let workspace = self.workspace_path.clone().unwrap_or_else(|| {
            current_file
                .as_ref()
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf())
                .unwrap_or_default()
        });
        match self.debug_plan(current_file.as_deref(), &workspace) {
            Ok(plan) => self.dap.launch(plan, current_file.as_deref()),
            Err(e) => self.dap.launch_failed(e),
        }
        // Switch to debugger panel.
        self.show_sidebar = true;
        self.sidebar_tab = SidebarTab::Debug;
    }

    /// The debug settings to use: the active run configuration's, else the
    /// first configuration of a type handled by `file_adapter`.
    fn debug_launch(&self, file_adapter: Option<&DebugAdapter>) -> Option<&DebugLaunch> {
        self.runner
            .active_config()
            .and_then(|c| c.debug.as_ref())
            .or_else(|| {
                let adapter = file_adapter?;
                self.runner
                    .configs
                    .iter()
                    .filter_map(|c| c.debug.as_ref())
                    .find(|d| adapter.handles_type(&d.adapter_type))
            })
    }

    /// Pick the adapter, launch arguments and tasks for a new debug session.
    fn debug_plan(
        &self,
        current_file: Option<&Path>,
        workspace: &Path,
    ) -> Result<LaunchPlan, String> {
        let current_ext = current_file
            .and_then(crate::language::language_key)
            .unwrap_or_default();
        let current_ext = current_ext.as_str();
        let installed = &self.extension_registry.installed;
        let file_adapter = DebugAdapter::find(installed, None, current_ext);
        let debug = self.debug_launch(file_adapter.as_ref());
        let adapter_type = debug
            .map(|d| d.adapter_type.as_str())
            .filter(|t| !t.is_empty());
        let adapter = match adapter_type {
            Some(t) => DebugAdapter::find(installed, Some(t), current_ext).ok_or_else(|| {
                format!("No installed module provides a debugger for configurations of type `{t}`.")
            })?,
            None => file_adapter.ok_or_else(|| {
                if current_ext.is_empty() {
                    "Open a source file or pick a run configuration with debug settings (.vscode/launch.json).".to_string()
                } else {
                    format!("No installed module provides a debugger for .{current_ext} files.")
                }
            })?,
        };
        let launch = debug
            .map(|d| d.launch_args.clone())
            .or_else(|| adapter.default_launch())
            .ok_or_else(|| {
                format!(
                    "The {} debugger needs a launch configuration telling which program to run: add one to .vscode/launch.json.",
                    adapter.module
                )
            })?;
        let tasks = match launch["preLaunchTask"].as_str() {
            Some(label) => vscode_tasks::plan_for_workspace(workspace, label, current_file)
                .map_err(|e| format!("preLaunchTask `{label}`: {e}"))?,
            None => vec![],
        };
        // The language whose server starts an LSP-started session: the
        // current file's when the module handles it, else the module's first.
        let language = if adapter.languages.iter().any(|l| l == current_ext) {
            current_ext.to_string()
        } else {
            adapter.languages.first().cloned().unwrap_or_default()
        };
        Ok(LaunchPlan {
            config: adapter.config(PathBuf::new(), launch),
            adapter: Some(adapter),
            tasks,
            workspace: workspace.to_path_buf(),
            file: current_file.map(Path::to_path_buf),
            language,
        })
    }

    /// Apply breakpoints toggled by a click in either editor's gutter.
    pub fn apply_gutter_breakpoint_clicks(&mut self) {
        let mut clicks = Vec::new();
        for ed in std::iter::once(&mut self.editor).chain(self.editor2.as_mut()) {
            if let (Some(row), Some(path)) =
                (ed.breakpoint_toggle_request.take(), ed.current_path.clone())
            {
                clicks.push((path, row));
            }
        }
        for (path, row) in clicks {
            // Breakpoints are 1-based in DAP.
            self.dap.toggle_breakpoint(&path, row + 1);
        }
    }

    /// Toggle a breakpoint at the current cursor line.
    pub fn toggle_breakpoint_at_cursor(&mut self) {
        let Some(path) = self.editor.current_path.clone() else {
            return;
        };
        let (row, _) = self.editor.cursor.position();
        // Breakpoints are 1-based in DAP.
        self.dap.toggle_breakpoint(&path, row + 1);
    }
}
