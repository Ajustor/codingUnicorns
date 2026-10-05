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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dap::types::DapConfig;
    use crate::editor::highlight::{Token, TokenKind};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Configurable fake plugin that records calls.
    #[derive(Default)]
    struct Fake {
        name: &'static str,
        exts: &'static [&'static str],
        cmds: Vec<&'static str>,
        panels: Vec<&'static str>,
        tokenizes: bool,
        hover: Option<&'static str>,
        lsp: Option<&'static str>,
        dap: bool,
        resets: Arc<AtomicUsize>,
        rendered: Arc<AtomicUsize>,
        executed: Arc<AtomicUsize>,
    }

    impl Plugin for Fake {
        fn name(&self) -> &str {
            self.name
        }
        fn commands(&self) -> Vec<PluginCommand> {
            self.cmds
                .iter()
                .map(|c| PluginCommand {
                    id: c.to_string(),
                    title: format!("{}: {c}", self.name),
                    keybinding: None,
                })
                .collect()
        }
        fn sidebar_panels(&self) -> Vec<SidebarPanel> {
            self.panels
                .iter()
                .map(|p| SidebarPanel {
                    id: p.to_string(),
                    title: p.to_string(),
                    icon: "*",
                })
                .collect()
        }
        fn update(&mut self, _ctx: &PluginContext) -> PluginResponse {
            PluginResponse {
                status_text: Some(self.name.to_string()),
                notifications: vec![],
            }
        }
        fn execute_command(&mut self, id: &str, _ctx: &PluginContext) -> PluginResponse {
            self.executed.fetch_add(1, Ordering::SeqCst);
            PluginResponse {
                status_text: Some(format!("{} ran {id}", self.name)),
                notifications: vec![],
            }
        }
        fn render_sidebar(&mut self, _panel_id: &str, ui: &mut egui::Ui) {
            self.rendered.fetch_add(1, Ordering::SeqCst);
            ui.label(self.name);
        }
        fn tokenize_line(&self, _lang: &str, line: &str) -> Option<Vec<Token>> {
            self.tokenizes.then(|| {
                vec![Token {
                    text: format!("{}:{line}", self.name),
                    kind: TokenKind::Normal,
                }]
            })
        }
        fn tokenize_document(&self, _lang: &str, text: &str) -> Option<Vec<Vec<Token>>> {
            self.tokenizes.then(|| {
                vec![vec![Token {
                    text: format!("{}:{text}", self.name),
                    kind: TokenKind::Normal,
                }]]
            })
        }
        fn hover_info(&self, _lang: &str, _word: &str, _c: &str) -> Option<String> {
            self.hover.map(|s| s.to_string())
        }
        fn reset_tokenizer(&self) {
            self.resets.fetch_add(1, Ordering::SeqCst);
        }
        fn file_extensions(&self) -> &[&str] {
            self.exts
        }
        fn lsp_server_command(&self) -> Option<(String, Vec<String>)> {
            self.lsp
                .map(|s| (s.to_string(), vec!["--stdio".to_string()]))
        }
        fn dap_config(&self) -> Option<DapConfig> {
            self.dap.then(|| DapConfig {
                adapter_cmd: format!("{}-dap", self.name),
                adapter_args: vec![],
                launch_config: serde_json::json!({}),
            })
        }
    }

    fn ctx() -> PluginContext<'static> {
        PluginContext {
            buffer_text: "",
            filename: None,
            cursor_row: 0,
            cursor_col: 0,
            is_modified: false,
            hovered_word: None,
        }
    }

    #[test]
    fn empty_manager() {
        let mut m = PluginManager::default();
        assert!(m.update_all(&ctx()).is_empty());
        assert!(m.execute_command("x", &ctx()).is_none());
        assert!(m.all_commands().is_empty());
        assert!(m.sidebar_panels().is_empty());
        assert!(m.tokenize_line("rs", "x").is_none());
        assert!(m.tokenize_document("rs", "x").is_none());
        assert!(m.hover_info("rs", "x", "").is_none());
        assert!(m.dap_config_for_ext("rs").is_none());
        assert!(m.lsp_server_for_ext("rs").is_none());
    }

    #[test]
    fn update_all_collects_one_response_per_plugin() {
        let mut m = PluginManager::new();
        m.register(Box::new(Fake {
            name: "a",
            ..Default::default()
        }));
        m.register(Box::new(Fake {
            name: "b",
            ..Default::default()
        }));
        let r: Vec<_> = m
            .update_all(&ctx())
            .into_iter()
            .map(|r| r.status_text.unwrap())
            .collect();
        assert_eq!(r, vec!["a", "b"]);
    }

    #[test]
    fn execute_command_routes_to_owner() {
        let mut m = PluginManager::new();
        let a_exec = Arc::new(AtomicUsize::new(0));
        let b_exec = Arc::new(AtomicUsize::new(0));
        m.register(Box::new(Fake {
            name: "a",
            cmds: vec!["a.one"],
            executed: a_exec.clone(),
            ..Default::default()
        }));
        m.register(Box::new(Fake {
            name: "b",
            cmds: vec!["b.one", "b.two"],
            executed: b_exec.clone(),
            ..Default::default()
        }));
        let r = m.execute_command("b.two", &ctx()).unwrap();
        assert_eq!(r.status_text.as_deref(), Some("b ran b.two"));
        assert_eq!(a_exec.load(Ordering::SeqCst), 0);
        assert_eq!(b_exec.load(Ordering::SeqCst), 1);
        assert!(m.execute_command("missing", &ctx()).is_none());

        let all: Vec<(String, String)> = m
            .all_commands()
            .into_iter()
            .map(|(n, c)| (n, c.id))
            .collect();
        assert_eq!(
            all,
            vec![
                ("a".into(), "a.one".into()),
                ("b".into(), "b.one".into()),
                ("b".into(), "b.two".into())
            ]
        );
    }

    #[test]
    fn sidebar_panels_and_render_delegation() {
        let mut m = PluginManager::new();
        let a_r = Arc::new(AtomicUsize::new(0));
        let b_r = Arc::new(AtomicUsize::new(0));
        m.register(Box::new(Fake {
            name: "a",
            panels: vec!["a.panel"],
            rendered: a_r.clone(),
            ..Default::default()
        }));
        m.register(Box::new(Fake {
            name: "b",
            panels: vec!["b.panel"],
            rendered: b_r.clone(),
            ..Default::default()
        }));
        let panels: Vec<(String, String)> = m
            .sidebar_panels()
            .into_iter()
            .map(|(n, p)| (n, p.id))
            .collect();
        assert_eq!(
            panels,
            vec![
                ("a".into(), "a.panel".into()),
                ("b".into(), "b.panel".into())
            ]
        );

        let ectx = egui::Context::default();
        let _ = ectx.run(egui::RawInput::default(), |c| {
            egui::CentralPanel::default().show(c, |ui| {
                m.render_panel("b.panel", ui);
                m.render_panel("nope", ui);
            });
        });
        assert_eq!(a_r.load(Ordering::SeqCst), 0);
        assert_eq!(b_r.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn tokenizers_use_first_plugin_that_answers() {
        let mut m = PluginManager::new();
        m.register(Box::new(Fake {
            name: "silent",
            ..Default::default()
        }));
        m.register(Box::new(Fake {
            name: "first",
            tokenizes: true,
            ..Default::default()
        }));
        m.register(Box::new(Fake {
            name: "second",
            tokenizes: true,
            ..Default::default()
        }));
        assert_eq!(m.tokenize_line("rs", "x").unwrap()[0].text, "first:x");
        assert_eq!(
            m.tokenize_document("rs", "y").unwrap()[0][0].text,
            "first:y"
        );
    }

    #[test]
    fn hover_info_skips_empty_results() {
        let mut m = PluginManager::new();
        m.register(Box::new(Fake {
            name: "none",
            ..Default::default()
        }));
        m.register(Box::new(Fake {
            name: "empty",
            hover: Some(""),
            ..Default::default()
        }));
        m.register(Box::new(Fake {
            name: "real",
            hover: Some("fn x()"),
            ..Default::default()
        }));
        assert_eq!(m.hover_info("rs", "x", "").as_deref(), Some("fn x()"));
    }

    #[test]
    fn lsp_and_dap_lookup_by_extension() {
        let mut m = PluginManager::new();
        m.register(Box::new(Fake {
            name: "py-nolsp",
            exts: &["py"],
            ..Default::default()
        }));
        m.register(Box::new(Fake {
            name: "py",
            exts: &["py", "pyw"],
            lsp: Some("pylsp"),
            dap: true,
            ..Default::default()
        }));
        m.register(Box::new(Fake {
            name: "rs",
            exts: &["rs"],
            lsp: Some("rust-analyzer"),
            ..Default::default()
        }));
        assert_eq!(
            m.lsp_server_for_ext("pyw"),
            Some(("pylsp".to_string(), vec!["--stdio".to_string()]))
        );
        assert_eq!(m.lsp_server_for_ext("rs").unwrap().0, "rust-analyzer");
        assert!(m.lsp_server_for_ext("go").is_none());
        assert_eq!(m.dap_config_for_ext("py").unwrap().adapter_cmd, "py-dap");
        assert!(m.dap_config_for_ext("rs").is_none());
    }

    #[test]
    fn reset_tokenizer_only_targets_matching_language() {
        let mut m = PluginManager::new();
        let rs = Arc::new(AtomicUsize::new(0));
        let py = Arc::new(AtomicUsize::new(0));
        m.register(Box::new(Fake {
            name: "rs",
            exts: &["rs"],
            resets: rs.clone(),
            ..Default::default()
        }));
        m.register(Box::new(Fake {
            name: "py",
            exts: &["py"],
            resets: py.clone(),
            ..Default::default()
        }));
        m.reset_tokenizer("rs");
        m.reset_tokenizer("rs");
        assert_eq!(rs.load(Ordering::SeqCst), 2);
        assert_eq!(py.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn unload_by_extensions_removes_matching_plugins() {
        let mut m = PluginManager::new();
        m.register(Box::new(Fake {
            name: "rs",
            exts: &["rs"],
            lsp: Some("ra"),
            ..Default::default()
        }));
        m.register(Box::new(Fake {
            name: "ts",
            exts: &["ts", "tsx"],
            lsp: Some("tsserver"),
            ..Default::default()
        }));
        m.register(Box::new(Fake {
            name: "builtin",
            ..Default::default()
        }));
        m.unload_by_extensions(&["tsx".to_string()]);
        assert!(m.lsp_server_for_ext("ts").is_none());
        assert!(m.lsp_server_for_ext("rs").is_some());
        assert_eq!(m.update_all(&ctx()).len(), 2);
        m.unload_by_extensions(&[]);
        assert_eq!(m.update_all(&ctx()).len(), 2);
    }
}
