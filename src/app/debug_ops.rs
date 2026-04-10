use super::CodingUnicorns;
use crate::ui::layout::SidebarTab;

impl CodingUnicorns {
    /// Start a DAP debug session using the language plugin for the current file.
    pub fn start_debug_session(&mut self) {
        let Some(path) = self.editor.current_path.clone() else {
            return;
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();
        let Some(cfg) = self.plugin_manager.dap_config_for_ext(&ext) else {
            return;
        };
        let workspace = self
            .workspace_path
            .clone()
            .unwrap_or_else(|| path.parent().map(|p| p.to_path_buf()).unwrap_or_default());
        if let Err(e) = self.dap.start_session(&cfg, &workspace, Some(&path)) {
            self.show_terminal = true;
            if let Some(term) = self.terminals.get_mut(self.active_terminal) {
                term.send_input(&format!("echo 'DAP error: {e}'\n"));
            }
        }
        // Switch to debugger panel.
        self.show_sidebar = true;
        self.sidebar_tab = SidebarTab::Debug;
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
