use super::{Plugin, PluginCommand, PluginContext, PluginResponse, SidebarPanel};

pub struct PluginManager {
    plugins: Vec<Box<dyn Plugin>>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self { plugins: vec![] }
    }

    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        self.plugins.push(plugin);
    }

    /// Run all plugins' `update()` and collect responses.
    pub fn update_all(&mut self, ctx: &PluginContext) -> Vec<PluginResponse> {
        self.plugins.iter_mut().map(|p| p.update(ctx)).collect()
    }

    /// Execute a command by id, finding which plugin owns it.
    pub fn execute_command(
        &mut self,
        command_id: &str,
        ctx: &PluginContext,
    ) -> Option<PluginResponse> {
        for plugin in &mut self.plugins {
            if plugin.commands().iter().any(|c| c.id == command_id) {
                return Some(plugin.execute_command(command_id, ctx));
            }
        }
        None
    }

    /// Collect all commands from all plugins (for command palette).
    /// Returns `(plugin_name, command)` pairs.
    pub fn all_commands(&self) -> Vec<(String, PluginCommand)> {
        self.plugins
            .iter()
            .flat_map(|p| {
                let name = p.name().to_string();
                p.commands().into_iter().map(move |cmd| (name.clone(), cmd))
            })
            .collect()
    }

    /// Render a sidebar panel by id, delegating to the owning plugin.
    pub fn render_panel(&mut self, panel_id: &str, ui: &mut egui::Ui) {
        for plugin in &mut self.plugins {
            if plugin.sidebar_panels().iter().any(|p| p.id == panel_id) {
                plugin.render_sidebar(panel_id, ui);
                return;
            }
        }
    }

    /// List all sidebar panels from all plugins.
    /// Returns `(plugin_name, panel)` pairs.
    pub fn sidebar_panels(&self) -> Vec<(String, SidebarPanel)> {
        self.plugins
            .iter()
            .flat_map(|p| {
                let name = p.name().to_string();
                p.sidebar_panels()
                    .into_iter()
                    .map(move |panel| (name.clone(), panel))
            })
            .collect()
    }

    pub fn tokenize_line(
        &self,
        lang: &str,
        line: &str,
    ) -> Option<Vec<crate::editor::highlight::Token>> {
        self.plugins
            .iter()
            .find_map(|p| p.tokenize_line(lang, line))
    }

    /// Tokenize an entire document via a plugin's document-level tokenizer.
    pub fn tokenize_document(
        &self,
        lang: &str,
        text: &str,
    ) -> Option<Vec<Vec<crate::editor::highlight::Token>>> {
        self.plugins
            .iter()
            .find_map(|p| p.tokenize_document(lang, text))
    }

    /// Unload all plugins whose file extensions match those of the given extension ID.
    /// This drops the `Library` handle, unlocking the DLL on Windows.
    pub fn unload_by_extensions(&mut self, extensions: &[String]) {
        self.plugins.retain(|p| {
            let exts = p.file_extensions();
            !extensions.iter().any(|e| exts.contains(&e.as_str()))
        });
    }

    /// Reset multi-line tokenizer state for all plugins that handle `lang`.
    pub fn reset_tokenizer(&self, lang: &str) {
        for plugin in &self.plugins {
            if plugin.file_extensions().contains(&lang) {
                plugin.reset_tokenizer();
            }
        }
    }

    /// Query all plugins for hover documentation for `word` in a file of type `lang`.
    /// Returns the first non-empty result, or `None`.
    pub fn hover_info(&self, lang: &str, word: &str, file_content: &str) -> Option<String> {
        for plugin in &self.plugins {
            if let Some(info) = plugin.hover_info(lang, word, file_content) {
                if !info.is_empty() {
                    return Some(info);
                }
            }
        }
        None
    }

    /// Return the DAP configuration for the plugin that handles the given file extension.
    pub fn dap_config_for_ext(&self, ext: &str) -> Option<crate::dap::types::DapConfig> {
        for plugin in &self.plugins {
            if plugin.file_extensions().contains(&ext) {
                if let Some(cfg) = plugin.dap_config() {
                    return Some(cfg);
                }
            }
        }
        None
    }

    /// Return the LSP server command for the plugin that handles the given file extension.
    /// Returns the first plugin whose `file_extensions()` includes `ext`, or `None`.
    pub fn lsp_server_for_ext(&self, ext: &str) -> Option<(String, Vec<String>)> {
        for plugin in &self.plugins {
            if plugin.file_extensions().contains(&ext) {
                if let Some(cmd) = plugin.lsp_server_command() {
                    return Some(cmd);
                }
            }
        }
        None
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}
