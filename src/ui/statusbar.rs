use crate::editor::Editor;
use crate::git::GitStatus;

#[derive(Debug, Clone, PartialEq)]
pub enum LspStatus {
    Inactive,
    Connecting,
    /// Connected but the server is still doing background work (e.g. csharp-ls
    /// loading the solution) — not yet able to answer fully.
    Loading,
    Ready,
    Error,
}

pub struct StatusBar {}

impl StatusBar {
    pub fn new() -> Self {
        Self {}
    }

    pub fn show(
        &self,
        ui: &mut egui::Ui,
        editor: &Editor,
        git: &GitStatus,
        lsp_status: LspStatus,
        palette: crate::ui::theme::Palette,
    ) {
        let bg = palette.accent;
        egui::Frame::new().fill(bg).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("⎇ {}", git.branch))
                        .color(palette.on_accent)
                        .small(),
                );
                ui.separator();

                if let Some(path) = &editor.current_path {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let modified = if editor.is_modified { " ●" } else { "" };
                    ui.label(
                        egui::RichText::new(format!("{}{}", name, modified))
                            .color(palette.on_accent)
                            .small(),
                    );
                    ui.separator();

                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("txt");
                    ui.label(
                        egui::RichText::new(ext.to_uppercase())
                            .color(palette.on_accent)
                            .small(),
                    );
                    ui.separator();

                    // Indent style indicator
                    let indent_label = if editor.detected_indent_spaces {
                        format!("Spaces: {}", editor.detected_indent_size)
                    } else {
                        "Tabs".to_string()
                    };
                    ui.label(
                        egui::RichText::new(indent_label)
                            .color(palette.on_accent)
                            .small(),
                    );
                    ui.separator();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (row, col) = editor.cursor.position();
                    ui.label(
                        egui::RichText::new(format!("Ln {}, Col {}", row + 1, col + 1))
                            .color(palette.on_accent)
                            .small(),
                    );
                    ui.separator();

                    // LSP status dot
                    match lsp_status {
                        LspStatus::Inactive => {}
                        LspStatus::Connecting => {
                            ui.label(egui::RichText::new("⬤ LSP").color(palette.warning).small())
                                .on_hover_text("Connecting to language server…");
                            ui.separator();
                        }
                        LspStatus::Loading => {
                            ui.label(
                                egui::RichText::new("⬤ LSP loading…")
                                    .color(palette.warning)
                                    .small(),
                            )
                            .on_hover_text("Language server is loading the project (indexing)…");
                            ui.separator();
                        }
                        LspStatus::Ready => {
                            ui.label(egui::RichText::new("⬤ LSP").color(palette.success).small());
                            ui.separator();
                        }
                        LspStatus::Error => {
                            ui.label(egui::RichText::new("⬤ LSP").color(palette.error).small());
                            ui.separator();
                        }
                    }
                });
            });
        });
    }
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}
