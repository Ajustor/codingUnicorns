use crate::lsp::client::DocumentSymbol;
use crate::ui::theme::Palette;

/// The deepest document symbol whose start line is at or above `cursor_line`
/// (symbols assumed ordered by line). Returns its name, or None.
pub(crate) fn enclosing_symbol(symbols: &[DocumentSymbol], cursor_line: u32) -> Option<&str> {
    symbols
        .iter()
        .filter(|s| s.line <= cursor_line)
        .max_by_key(|s| s.line)
        .map(|s| s.name.as_str())
}

/// Render a thin breadcrumb bar: workspace-relative path › enclosing symbol. Display-only.
pub fn render(
    ui: &mut egui::Ui,
    palette: Palette,
    path: Option<&std::path::Path>,
    workspace: Option<&std::path::Path>,
    symbols: &[DocumentSymbol],
    cursor_line: u32,
) {
    let Some(path) = path else { return };
    egui::Frame::new()
        .fill(palette.surface)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let rel = workspace
                    .and_then(|w| path.strip_prefix(w).ok())
                    .unwrap_or(path);
                let mut first = true;
                for comp in rel.components() {
                    if !first {
                        ui.label(egui::RichText::new("›").color(palette.text_faint).small());
                    }
                    first = false;
                    ui.label(
                        egui::RichText::new(comp.as_os_str().to_string_lossy())
                            .color(palette.text_muted)
                            .small(),
                    );
                }
                if let Some(name) = enclosing_symbol(symbols, cursor_line) {
                    ui.label(egui::RichText::new("›").color(palette.text_faint).small());
                    ui.label(egui::RichText::new(name).color(palette.text).small());
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::client::DocumentSymbol;

    fn sym(name: &str, line: u32) -> DocumentSymbol {
        DocumentSymbol {
            name: name.to_string(),
            kind: "Function".into(),
            line,
        }
    }

    #[test]
    fn picks_nearest_symbol_at_or_above_cursor() {
        let syms = vec![sym("alpha", 0), sym("beta", 10), sym("gamma", 20)];
        assert_eq!(enclosing_symbol(&syms, 14), Some("beta"));
        assert_eq!(enclosing_symbol(&syms, 0), Some("alpha"));
        assert_eq!(enclosing_symbol(&[], 5), None);
    }

    #[test]
    fn cursor_above_all_symbols_has_none() {
        let syms = vec![sym("late", 10)];
        assert_eq!(enclosing_symbol(&syms, 9), None);
        assert_eq!(enclosing_symbol(&syms, 10), Some("late"));
        assert_eq!(enclosing_symbol(&syms, u32::MAX), Some("late"));
    }

    #[test]
    fn unordered_symbols_still_pick_closest_preceding() {
        let syms = vec![sym("gamma", 20), sym("alpha", 0), sym("beta", 10)];
        assert_eq!(enclosing_symbol(&syms, 15), Some("beta"));
        assert_eq!(enclosing_symbol(&syms, 25), Some("gamma"));
    }

    fn palette() -> Palette {
        Palette::from_theme(&crate::config::Config::default().theme)
    }

    /// Render the breadcrumb bar headlessly and return every text run painted.
    fn rendered(
        path: Option<&std::path::Path>,
        workspace: Option<&std::path::Path>,
        symbols: &[DocumentSymbol],
        cursor_line: u32,
    ) -> Vec<String> {
        let ctx = egui::Context::default();
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                render(ui, palette(), path, workspace, symbols, cursor_line);
            });
        });
        let mut texts = Vec::new();
        for clipped in &out.shapes {
            collect_texts(&clipped.shape, &mut texts);
        }
        texts
    }

    fn collect_texts(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect_texts(s, out)),
            _ => {}
        }
    }

    #[test]
    fn no_path_renders_nothing() {
        assert!(rendered(None, None, &[sym("f", 0)], 3).is_empty());
    }

    #[test]
    fn workspace_relative_path_and_symbol() {
        let ws = std::path::PathBuf::from("ws");
        let file = ws.join("src").join("main.rs");
        let texts = rendered(Some(&file), Some(&ws), &[sym("main", 0)], 4);
        assert_eq!(texts, ["src", "›", "main.rs", "›", "main"]);
    }

    #[test]
    fn path_outside_workspace_is_shown_in_full_without_symbol() {
        let file = std::path::PathBuf::from("other").join("lib.rs");
        let ws = std::path::PathBuf::from("ws");
        let texts = rendered(Some(&file), Some(&ws), &[sym("later", 50)], 4);
        assert_eq!(texts, ["other", "›", "lib.rs"]);
        let texts = rendered(Some(&file), None, &[], 0);
        assert_eq!(texts, ["other", "›", "lib.rs"]);
    }
}
