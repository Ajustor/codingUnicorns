use super::CodingUnicorns;
use crate::dap::types::DapConfig;
use crate::runner::vscode_launch;
use crate::ui::layout::SidebarTab;

impl CodingUnicorns {
    /// Start a DAP debug session. Uses the active run configuration's debug
    /// settings (imported from `.vscode/launch.json`) when it has some, else
    /// the language plugin for the current file.
    pub fn start_debug_session(&mut self) {
        let current_file = self.editor.current_path.clone();
        let Some(cfg) = self.debug_config(current_file.as_deref()) else {
            return;
        };
        let workspace = self.workspace_path.clone().unwrap_or_else(|| {
            current_file
                .as_ref()
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf())
                .unwrap_or_default()
        });
        if let Err(e) = self
            .dap
            .start_session(&cfg, &workspace, current_file.as_deref())
        {
            self.show_terminal = true;
            if let Some(term) = self.terminals.get_mut(self.active_terminal) {
                term.send_input(&format!("echo 'DAP error: {e}'\n"));
            }
        }
        // Switch to debugger panel.
        self.show_sidebar = true;
        self.sidebar_tab = SidebarTab::Debug;
    }

    /// Pick the adapter and launch arguments for a new debug session.
    fn debug_config(&self, current_file: Option<&std::path::Path>) -> Option<DapConfig> {
        let current_ext = current_file
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let Some(debug) = self.runner.active_config().and_then(|c| c.debug.as_ref()) else {
            return self.plugin_manager.dap_config_for_ext(current_ext);
        };
        // The adapter comes from the plugin of a language matching the VS Code
        // debug `type`, falling back to the current file's language.
        let mut cfg = vscode_launch::extensions_for_type(&debug.adapter_type)
            .iter()
            .copied()
            .chain(std::iter::once(current_ext))
            .find_map(|ext| self.plugin_manager.dap_config_for_ext(ext))?;
        cfg.launch_config = debug.launch_args.clone();
        Some(cfg)
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
