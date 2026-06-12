use super::CodingUnicorns;
use crate::ui::layout::SidebarTab;

impl CodingUnicorns {
    /// Démarre une session DAP. Privilégie la `DebugSpec` de la config de run
    /// active (importée de `.vscode/launch.json`) ; sinon, retombe sur le
    /// plugin associé à l'extension du fichier courant.
    pub fn start_debug_session(&mut self) {
        let current_file = self.editor.current_path.clone();
        let workspace = self.workspace_path.clone().unwrap_or_else(|| {
            current_file
                .as_ref()
                .and_then(|p| p.parent().map(|x| x.to_path_buf()))
                .unwrap_or_default()
        });

        // 1. DebugSpec de la config active, le cas échéant.
        let cfg = self
            .runner
            .active_config()
            .and_then(|c| self.runner.debug_spec_for(&c.name))
            .map(|spec| crate::dap::types::DapConfig {
                adapter_cmd: spec.adapter_cmd.clone(),
                adapter_args: spec.adapter_args.clone(),
                launch_config: spec.launch_config.clone(),
            });

        // 2. Fallback : plugin par extension du fichier courant.
        let cfg = cfg.or_else(|| {
            let ext = current_file
                .as_ref()
                .and_then(|p| p.extension())
                .and_then(|e| e.to_str())
                .unwrap_or("");
            self.plugin_manager.dap_config_for_ext(ext)
        });

        let Some(cfg) = cfg else {
            return;
        };

        if let Err(e) = self.dap.start_session(&cfg, &workspace, current_file.as_deref()) {
            self.show_terminal = true;
            if let Some(term) = self.terminals.get_mut(self.active_terminal) {
                term.send_input(&format!("echo 'DAP error: {e}'\n"));
            }
        }
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
