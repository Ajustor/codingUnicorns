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

    /// Render the bar. `problems` is the workspace `(errors, warnings)` count,
    /// shown when `Some`. Returns true when that indicator was clicked.
    pub fn show(
        &self,
        ui: &mut egui::Ui,
        editor: &Editor,
        git: &GitStatus,
        lsp_status: LspStatus,
        problems: Option<(usize, usize)>,
        palette: crate::ui::theme::Palette,
    ) -> bool {
        let bg = palette.accent;
        let mut problems_clicked = false;
        egui::Frame::new().fill(bg).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("⎇ {}", git.branch))
                        .color(palette.on_accent)
                        .small(),
                );
                ui.separator();

                if let Some((errors, warnings)) = problems {
                    let btn = egui::Button::new(
                        egui::RichText::new(format!("⊗ {errors}  ⚠ {warnings}"))
                            .color(palette.on_accent)
                            .small(),
                    )
                    .frame(false);
                    if ui
                        .add(btn)
                        .on_hover_text("Problems (Ctrl+Shift+M)")
                        .clicked()
                    {
                        problems_clicked = true;
                    }
                    ui.separator();
                }

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

                    let ext = crate::language::language_key(path).unwrap_or_else(|| "txt".into());
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

                    // On-disk line ending / encoding (re-applied on save)
                    let format = editor.text_format;
                    let encoding = if format.bom { "UTF-8 BOM" } else { "UTF-8" };
                    ui.label(
                        egui::RichText::new(format.line_ending.label())
                            .color(palette.on_accent)
                            .small(),
                    );
                    ui.separator();
                    if editor.decoded_lossy {
                        ui.label(
                            egui::RichText::new("⚠ Invalid UTF-8")
                                .color(palette.on_accent)
                                .small(),
                        )
                        .on_hover_text(
                            "The file was not valid UTF-8: invalid bytes are shown as \u{FFFD} \
                             and saving will replace them.",
                        );
                    } else {
                        ui.label(
                            egui::RichText::new(encoding)
                                .color(palette.on_accent)
                                .small(),
                        );
                    }
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
        problems_clicked
    }
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn collect_texts(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect_texts(s, out)),
            _ => {}
        }
    }

    /// Render the status bar headlessly and return every text run painted.
    fn rendered(editor: &Editor, branch: &str, lsp: LspStatus) -> Vec<String> {
        rendered_with(editor, branch, lsp, None)
    }

    fn rendered_with(
        editor: &Editor,
        branch: &str,
        lsp: LspStatus,
        problems: Option<(usize, usize)>,
    ) -> Vec<String> {
        let mut git = GitStatus::new();
        git.branch = branch.into();
        let palette =
            crate::ui::theme::Palette::from_theme(&crate::config::Config::default().theme);
        let ctx = egui::Context::default();
        let bar = StatusBar::default();
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                bar.show(ui, editor, &git, lsp.clone(), problems, palette);
            });
        });
        let mut texts = Vec::new();
        for c in &out.shapes {
            collect_texts(&c.shape, &mut texts);
        }
        texts
    }

    fn has(texts: &[String], s: &str) -> bool {
        texts.iter().any(|t| t == s)
    }

    #[test]
    fn no_file_shows_branch_and_cursor_only() {
        let editor = Editor::new();
        let t = rendered(&editor, "main", LspStatus::Inactive);
        assert_eq!(t.len(), 2, "{t:?}");
        assert!(has(&t, "⎇ main"));
        assert!(has(&t, "Ln 1, Col 1"));
    }

    #[test]
    fn file_details_and_one_based_cursor() {
        let mut editor = Editor::new();
        editor.current_path = Some(PathBuf::from("src").join("main.rs"));
        editor.cursor.row = 9;
        editor.cursor.col = 4;
        editor.detected_indent_spaces = true;
        editor.detected_indent_size = 2;
        let t = rendered(&editor, "feature/x", LspStatus::Inactive);
        assert!(has(&t, "⎇ feature/x"));
        assert!(has(&t, "main.rs"), "{t:?}");
        assert!(has(&t, "RS"));
        assert!(has(&t, "Spaces: 2"));
        assert!(has(&t, "LF"));
        assert!(has(&t, "UTF-8"));
        assert!(has(&t, "Ln 10, Col 5"));
        assert!(!t.iter().any(|s| s.contains("LSP")));
    }

    #[test]
    fn line_ending_bom_and_lossy_decoding_indicators() {
        let mut editor = Editor::new();
        editor.set_content("\u{FEFF}a\r\nb".to_string(), Some(PathBuf::from("win.txt")));
        let t = rendered(&editor, "main", LspStatus::Inactive);
        assert!(has(&t, "CRLF"), "{t:?}");
        assert!(has(&t, "UTF-8 BOM"), "{t:?}");

        editor.decoded_lossy = true;
        let t = rendered(&editor, "main", LspStatus::Inactive);
        assert!(has(&t, "⚠ Invalid UTF-8"), "{t:?}");
        assert!(!has(&t, "UTF-8 BOM"));
    }

    #[test]
    fn modified_marker_tabs_and_missing_extension() {
        let mut editor = Editor::new();
        editor.current_path = Some(PathBuf::from("README"));
        editor.is_modified = true;
        editor.detected_indent_spaces = false;
        let t = rendered(&editor, "main", LspStatus::Inactive);
        assert!(has(&t, "README ●"), "{t:?}");
        assert!(has(&t, "TXT"), "extension defaults to txt");
        editor.current_path = Some(PathBuf::from("Dockerfile"));
        let t = rendered(&editor, "main", LspStatus::Inactive);
        assert!(has(&t, "DOCKERFILE"), "named files show their language");
        assert!(has(&t, "Tabs"));
    }

    #[test]
    fn lsp_status_indicator() {
        let editor = Editor::new();
        for (status, label) in [
            (LspStatus::Connecting, "⬤ LSP"),
            (LspStatus::Loading, "⬤ LSP loading…"),
            (LspStatus::Ready, "⬤ LSP"),
            (LspStatus::Error, "⬤ LSP"),
        ] {
            let t = rendered(&editor, "main", status.clone());
            assert!(has(&t, label), "{status:?}: {t:?}");
            assert_eq!(t.len(), 3, "{status:?}: {t:?}");
        }
    }

    #[test]
    fn problem_counts_shown_when_provided() {
        let editor = Editor::new();
        let t = rendered_with(&editor, "main", LspStatus::Inactive, Some((2, 7)));
        assert!(has(&t, "⊗ 2  ⚠ 7"), "{t:?}");
        assert_eq!(t.len(), 3, "{t:?}");
        let t = rendered(&editor, "main", LspStatus::Inactive);
        assert!(!t.iter().any(|s| s.contains('⚠')));
    }

    #[test]
    fn clicking_problem_counts_reports_it() {
        let editor = Editor::new();
        let git = GitStatus::new();
        let palette =
            crate::ui::theme::Palette::from_theme(&crate::config::Config::default().theme);
        let ctx = egui::Context::default();
        let bar = StatusBar::default();
        let run = |events: Vec<egui::Event>| {
            let mut clicked = false;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 100.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    clicked = bar.show(ui, &editor, &git, LspStatus::Ready, Some((1, 0)), palette);
                });
            });
            clicked
        };
        assert!(!run(vec![]));
        let btn = ctx.viewport(|vp| {
            let w = &vp.prev_pass.widgets;
            w.layer_ids()
                .flat_map(|l| w.get_layer(l))
                .filter(|r| {
                    r.sense.senses_click() && r.sense.is_focusable() && !r.sense.senses_drag()
                })
                .map(|r| r.rect)
                .next()
                .unwrap()
        });
        let pos = btn.center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        assert!(!run(vec![egui::Event::PointerMoved(pos), press(true)]));
        assert!(run(vec![press(false)]));
    }
}
