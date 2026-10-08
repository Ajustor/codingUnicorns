pub mod builtin;
pub mod manager;
pub mod types;

pub use types::{PluginCommand, PluginContext, PluginResponse, SidebarPanel};

use crate::editor::highlight::Token;

/// The Plugin trait — all plugins implement this.
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str {
        "0.1.0"
    }
    fn commands(&self) -> Vec<PluginCommand> {
        vec![]
    }
    fn sidebar_panels(&self) -> Vec<SidebarPanel> {
        vec![]
    }

    /// Called every frame with current editor state.
    fn update(&mut self, _ctx: &PluginContext) -> PluginResponse {
        PluginResponse::default()
    }

    /// Called when one of this plugin's commands is executed.
    fn execute_command(&mut self, _command_id: &str, _ctx: &PluginContext) -> PluginResponse {
        PluginResponse::default()
    }

    /// Called to render this plugin's sidebar panel (if any).
    fn render_sidebar(&mut self, _panel_id: &str, _ui: &mut egui::Ui) {}

    /// Provide syntax tokens for a line (optional — for language plugins).
    fn tokenize_line(&self, _lang: &str, _line: &str) -> Option<Vec<Token>> {
        None
    }

    /// Return hover documentation or a signature string for `word` in the given file.
    /// `lang` is the file extension (e.g. `"rs"`, `"ts"`, `"js"`).
    /// `file_content` is the full text of the current buffer.
    /// Returns a formatted string (e.g. a code-fenced signature), or `None` if not found.
    fn hover_info(&self, _lang: &str, _word: &str, _file_content: &str) -> Option<String> {
        None
    }

    /// Tokenize an entire document at once (tree-sitter based).
    /// Returns per-line token vectors, or None if not supported.
    fn tokenize_document(
        &self,
        _lang: &str,
        _text: &str,
    ) -> Option<Vec<Vec<crate::editor::highlight::Token>>> {
        None
    }

    /// Reset multi-line tokenizer state before tokenizing a new document.
    fn reset_tokenizer(&self) {}

    /// File extensions handled by this plugin (e.g. `&["rs"]`, `&["ts", "tsx"]`).
    /// Used to match the correct LSP server to a file.
    fn file_extensions(&self) -> &[&str] {
        &[]
    }

    /// Return the LSP server command for this language plugin.
    /// E.g. `Some(("rust-analyzer", vec![]))` for Rust.
    /// Return `None` if this plugin doesn't provide language server support.
    fn lsp_server_command(&self) -> Option<(String, Vec<String>)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Minimal;
    impl Plugin for Minimal {
        fn name(&self) -> &str {
            "minimal"
        }
    }

    fn ctx() -> PluginContext<'static> {
        PluginContext {
            buffer_text: "some text",
            filename: Some("a.rs"),
            cursor_row: 0,
            cursor_col: 0,
            is_modified: false,
            hovered_word: None,
        }
    }

    #[test]
    fn trait_defaults_are_inert() {
        let mut p = Minimal;
        assert_eq!(p.name(), "minimal");
        assert_eq!(p.version(), "0.1.0");
        assert!(p.commands().is_empty());
        assert!(p.sidebar_panels().is_empty());
        let r = p.update(&ctx());
        assert!(r.status_text.is_none() && r.notifications.is_empty());
        let r = p.execute_command("x", &ctx());
        assert!(r.status_text.is_none() && r.notifications.is_empty());
        assert!(p.tokenize_line("rs", "fn x").is_none());
        assert!(p.hover_info("rs", "x", "").is_none());
        assert!(p.tokenize_document("rs", "fn x").is_none());
        p.reset_tokenizer();
        assert!(p.file_extensions().is_empty());
        assert!(p.lsp_server_command().is_none());
    }

    #[test]
    fn default_render_sidebar_draws_nothing() {
        let ctx = egui::Context::default();
        let mut p = Minimal;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| p.render_sidebar("panel", ui));
        });
    }
}
