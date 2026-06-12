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
        DocumentSymbol { name: name.to_string(), kind: "Function".into(), line }
    }

    #[test]
    fn picks_nearest_symbol_at_or_above_cursor() {
        let syms = vec![sym("alpha", 0), sym("beta", 10), sym("gamma", 20)];
        assert_eq!(enclosing_symbol(&syms, 14), Some("beta"));
        assert_eq!(enclosing_symbol(&syms, 0), Some("alpha"));
        assert_eq!(enclosing_symbol(&[], 5), None);
    }
}
