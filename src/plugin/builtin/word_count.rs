use crate::plugin::{Plugin, PluginCommand, PluginContext, PluginResponse, SidebarPanel};

pub struct WordCountPlugin {
    word_count: usize,
    line_count: usize,
    char_count: usize,
}

impl WordCountPlugin {
    pub fn new() -> Self {
        Self {
            word_count: 0,
            line_count: 0,
            char_count: 0,
        }
    }
}

impl Default for WordCountPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for WordCountPlugin {
    fn name(&self) -> &str {
        "Word Count"
    }

    fn commands(&self) -> Vec<PluginCommand> {
        vec![PluginCommand {
            id: "word-count.show".into(),
            title: "Word Count: Show Statistics".into(),
            keybinding: None,
        }]
    }

    fn update(&mut self, ctx: &PluginContext) -> PluginResponse {
        self.word_count = ctx.buffer_text.split_whitespace().count();
        self.line_count = ctx.buffer_text.lines().count();
        self.char_count = ctx.buffer_text.chars().count();
        PluginResponse {
            status_text: Some(format!(
                "{} words | {} lines",
                self.word_count, self.line_count
            )),
            ..Default::default()
        }
    }

    fn render_sidebar(&mut self, _panel_id: &str, ui: &mut egui::Ui) {
        ui.label(format!("Words: {}", self.word_count));
        ui.label(format!("Lines: {}", self.line_count));
        ui.label(format!("Characters: {}", self.char_count));
    }

    fn sidebar_panels(&self) -> Vec<SidebarPanel> {
        vec![SidebarPanel {
            id: "word-count.panel".into(),
            title: "Word Count".into(),
            icon: egui_phosphor::regular::CHART_BAR,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(text: &str) -> PluginContext<'_> {
        PluginContext {
            buffer_text: text,
            filename: None,
            cursor_row: 0,
            cursor_col: 0,
            is_modified: false,
            hovered_word: None,
        }
    }

    #[test]
    fn new_and_default_start_at_zero() {
        for p in [WordCountPlugin::new(), WordCountPlugin::default()] {
            assert_eq!((p.word_count, p.line_count, p.char_count), (0, 0, 0));
        }
    }

    #[test]
    fn metadata() {
        let p = WordCountPlugin::new();
        assert_eq!(p.name(), "Word Count");
        let cmds = p.commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].id, "word-count.show");
        let panels = p.sidebar_panels();
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].id, "word-count.panel");
    }

    #[test]
    fn update_counts_words_lines_and_chars() {
        let mut p = WordCountPlugin::new();
        let r = p.update(&ctx("hello  world\nsecond line here\n"));
        assert_eq!(r.status_text.as_deref(), Some("5 words | 2 lines"));
        assert!(r.notifications.is_empty());
        assert_eq!(p.char_count, 30);
    }

    #[test]
    fn update_counts_unicode_chars_not_bytes() {
        let mut p = WordCountPlugin::new();
        p.update(&ctx("héllo 🦄"));
        assert_eq!(p.char_count, 7);
        assert_eq!(p.word_count, 2);
        assert_eq!(p.line_count, 1);
    }

    #[test]
    fn update_empty_buffer() {
        let mut p = WordCountPlugin::new();
        p.update(&ctx("something"));
        let r = p.update(&ctx(""));
        assert_eq!(r.status_text.as_deref(), Some("0 words | 0 lines"));
        assert_eq!(p.char_count, 0);
    }

    #[test]
    fn render_sidebar_runs() {
        let mut p = WordCountPlugin::new();
        p.update(&ctx("a b c"));
        let ectx = egui::Context::default();
        let _ = ectx.run(egui::RawInput::default(), |c| {
            egui::CentralPanel::default().show(c, |ui| p.render_sidebar("word-count.panel", ui));
        });
    }
}
