//! Problems panel: every LSP diagnostic in the workspace, grouped by file.

use std::path::{Path, PathBuf};

use crate::lsp::client::{DiagSeverity, Diagnostic};
use crate::lsp::manager::FileDiagnostics;

/// Error / warning / info+hint totals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProblemCounts {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
}

impl ProblemCounts {
    pub fn of(files: &[FileDiagnostics]) -> Self {
        let mut c = Self::default();
        for d in files.iter().flat_map(|(_, d)| d) {
            match d.severity {
                DiagSeverity::Error => c.errors += 1,
                DiagSeverity::Warning => c.warnings += 1,
                DiagSeverity::Info | DiagSeverity::Hint => c.infos += 1,
            }
        }
        c
    }
}

pub struct ProblemsPanel {
    pub open: bool,
    pub show_errors: bool,
    pub show_warnings: bool,
    /// Info and hint diagnostics.
    pub show_infos: bool,
    /// Cached `LspManager::workspace_diagnostics()` result.
    files: Vec<FileDiagnostics>,
    counts: ProblemCounts,
    /// `LspManager::diagnostics_revision()` the cache was built from.
    revision: Option<u64>,
}

impl ProblemsPanel {
    pub fn new() -> Self {
        Self {
            open: false,
            show_errors: true,
            show_warnings: true,
            show_infos: true,
            files: Vec::new(),
            counts: ProblemCounts::default(),
            revision: None,
        }
    }

    /// Rebuild the cache from `fetch` only when `revision` changed.
    pub fn sync(&mut self, revision: u64, fetch: impl FnOnce() -> Vec<FileDiagnostics>) {
        if self.revision != Some(revision) {
            self.files = fetch();
            self.counts = ProblemCounts::of(&self.files);
            self.revision = Some(revision);
        }
    }

    pub fn counts(&self) -> ProblemCounts {
        self.counts
    }

    fn wants(&self, sev: &DiagSeverity) -> bool {
        match sev {
            DiagSeverity::Error => self.show_errors,
            DiagSeverity::Warning => self.show_warnings,
            DiagSeverity::Info | DiagSeverity::Hint => self.show_infos,
        }
    }

    /// Files and their diagnostics that pass the severity filter.
    pub fn visible(&self) -> Vec<(&Path, Vec<&Diagnostic>)> {
        self.files
            .iter()
            .filter_map(|(p, diags)| {
                let shown: Vec<&Diagnostic> =
                    diags.iter().filter(|d| self.wants(&d.severity)).collect();
                (!shown.is_empty()).then_some((p.as_path(), shown))
            })
            .collect()
    }

    /// Render the panel body. Returns `(path, line, col)` (0-based) of a
    /// clicked entry, or `None`. Sets `open = false` when closed.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        workspace: Option<&Path>,
        palette: crate::ui::theme::Palette,
    ) -> Option<(PathBuf, usize, usize)> {
        let mut target = None;
        let c = self.counts;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("PROBLEMS ({})", c.errors + c.warnings + c.infos))
                    .size(11.0)
                    .color(egui::Color32::from_gray(150))
                    .strong(),
            );
            ui.add_space(8.0);
            ui.toggle_value(
                &mut self.show_errors,
                egui::RichText::new(format!(
                    "{} {}",
                    severity_icon(&DiagSeverity::Error),
                    c.errors
                ))
                .color(palette.error),
            )
            .on_hover_text("Show errors");
            ui.toggle_value(
                &mut self.show_warnings,
                egui::RichText::new(format!(
                    "{} {}",
                    severity_icon(&DiagSeverity::Warning),
                    c.warnings
                ))
                .color(palette.warning),
            )
            .on_hover_text("Show warnings");
            ui.toggle_value(
                &mut self.show_infos,
                egui::RichText::new(format!(
                    "{} {}",
                    severity_icon(&DiagSeverity::Info),
                    c.infos
                ))
                .color(palette.accent),
            )
            .on_hover_text("Show infos and hints");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("✕")
                    .on_hover_text("Close (Ctrl+Shift+M)")
                    .clicked()
                {
                    self.open = false;
                }
            });
        });
        ui.separator();

        let visible = self.visible();
        if visible.is_empty() {
            ui.label(
                egui::RichText::new(if self.files.is_empty() {
                    "No problems have been detected in the workspace."
                } else {
                    "No problems match the current filter."
                })
                .color(egui::Color32::GRAY),
            );
            return None;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (path, diags) in &visible {
                    let rel = display_path(path, workspace);
                    egui::CollapsingHeader::new(
                        egui::RichText::new(format!("{}  ({})", rel, diags.len())).size(12.0),
                    )
                    .id_salt(path)
                    .default_open(true)
                    .show(ui, |ui| {
                        for d in diags {
                            let color = match d.severity {
                                DiagSeverity::Error => palette.error,
                                DiagSeverity::Warning => palette.warning,
                                _ => palette.accent,
                            };
                            let first_line = d.message.lines().next().unwrap_or("");
                            let mut job = egui::text::LayoutJob::default();
                            let fmt = |c: egui::Color32| egui::TextFormat {
                                color: c,
                                font_id: egui::FontId::proportional(12.0),
                                ..Default::default()
                            };
                            job.append(
                                &format!("{}  ", severity_icon(&d.severity)),
                                0.0,
                                fmt(color),
                            );
                            job.append(first_line, 0.0, fmt(ui.visuals().text_color()));
                            job.append(
                                &format!("  [{}:{}]", d.line + 1, d.col + 1),
                                0.0,
                                fmt(egui::Color32::GRAY),
                            );
                            let resp = ui.selectable_label(false, job);
                            let resp = if d.message.contains('\n') {
                                resp.on_hover_text(&d.message)
                            } else {
                                resp
                            };
                            if resp.clicked() {
                                target =
                                    Some((path.to_path_buf(), d.line as usize, d.col as usize));
                            }
                        }
                    });
                }
            });
        target
    }
}

impl Default for ProblemsPanel {
    fn default() -> Self {
        Self::new()
    }
}

pub fn severity_icon(sev: &DiagSeverity) -> &'static str {
    match sev {
        DiagSeverity::Error => "⊗",
        DiagSeverity::Warning => "⚠",
        DiagSeverity::Info => "ℹ",
        DiagSeverity::Hint => "💡",
    }
}

/// `path` relative to the workspace (forward slashes), else the full path.
fn display_path(path: &Path, workspace: Option<&Path>) -> String {
    let rel = workspace
        .and_then(|ws| path.strip_prefix(ws).ok())
        .unwrap_or(path);
    rel.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect};

    fn d(line: u32, col: u32, severity: DiagSeverity, message: &str) -> Diagnostic {
        Diagnostic {
            message: message.into(),
            line,
            col,
            end_col: col + 1,
            severity,
        }
    }

    fn sample() -> Vec<FileDiagnostics> {
        vec![
            (
                PathBuf::from("ws").join("a.rs"),
                vec![
                    d(0, 1, DiagSeverity::Error, "bad"),
                    d(2, 0, DiagSeverity::Warning, "meh"),
                ],
            ),
            (
                PathBuf::from("ws").join("b.rs"),
                vec![
                    d(4, 2, DiagSeverity::Hint, "hint"),
                    d(5, 0, DiagSeverity::Info, "info"),
                ],
            ),
        ]
    }

    #[test]
    fn counts_by_severity() {
        assert_eq!(
            ProblemCounts::of(&sample()),
            ProblemCounts {
                errors: 1,
                warnings: 1,
                infos: 2
            }
        );
        assert_eq!(ProblemCounts::of(&[]), ProblemCounts::default());
    }

    #[test]
    fn sync_only_refetches_on_new_revision() {
        let mut p = ProblemsPanel::new();
        let mut calls = 0;
        p.sync(1, || {
            calls += 1;
            sample()
        });
        p.sync(1, || {
            calls += 1;
            vec![]
        });
        assert_eq!(calls, 1);
        assert_eq!(p.counts().errors, 1);
        p.sync(2, Vec::new);
        assert_eq!(p.counts(), ProblemCounts::default());
    }

    #[test]
    fn severity_filter_hides_entries_and_empty_files() {
        let mut p = ProblemsPanel::new();
        p.sync(0, sample);
        assert_eq!(p.visible().len(), 2);
        p.show_infos = false;
        let v = p.visible();
        assert_eq!(v.len(), 1, "b.rs has only info/hint");
        assert_eq!(v[0].1.len(), 2);
        p.show_errors = false;
        let v = p.visible();
        assert_eq!(v[0].1.len(), 1);
        assert_eq!(v[0].1[0].message, "meh");
        p.show_warnings = false;
        assert!(p.visible().is_empty());
        // Counts are unfiltered totals.
        assert_eq!(p.counts().errors, 1);
    }

    #[test]
    fn display_path_is_workspace_relative() {
        let ws = PathBuf::from("ws");
        assert_eq!(
            display_path(&ws.join("src").join("a.rs"), Some(&ws)),
            "src/a.rs"
        );
        assert_eq!(
            display_path(Path::new("other/x.rs"), Some(&ws)),
            "other/x.rs"
        );
        assert_eq!(display_path(Path::new("x.rs"), None), "x.rs");
    }

    #[test]
    fn icons_differ_per_severity() {
        let icons: std::collections::HashSet<&str> = [
            DiagSeverity::Error,
            DiagSeverity::Warning,
            DiagSeverity::Info,
            DiagSeverity::Hint,
        ]
        .iter()
        .map(severity_icon)
        .collect();
        assert_eq!(icons.len(), 4);
    }

    /// Headless render; returns the clicked target and every clickable row rect.
    fn frame(
        ctx: &egui::Context,
        p: &mut ProblemsPanel,
        events: Vec<Event>,
    ) -> (Option<(PathBuf, usize, usize)>, Vec<Rect>) {
        let palette =
            crate::ui::theme::Palette::from_theme(&crate::config::Config::default().theme);
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut res = None;
        let ws = PathBuf::from("ws");
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                res = p.show(ui, Some(&ws), palette);
            });
        });
        let rows = ctx.viewport(|vp| {
            let w = &vp.prev_pass.widgets;
            w.layer_ids()
                .flat_map(|l| w.get_layer(l))
                .filter(|r| r.sense.senses_click() && r.rect.height() < 30.0)
                .map(|r| r.rect)
                .collect::<Vec<_>>()
        });
        (res, rows)
    }

    #[test]
    fn clicking_an_entry_returns_its_location() {
        let ctx = egui::Context::default();
        let mut p = ProblemsPanel::new();
        p.show_errors = false; // only "meh" (a.rs) + b.rs entries
        p.show_infos = false;
        p.sync(0, sample);
        frame(&ctx, &mut p, vec![]);
        let (_, rows) = frame(&ctx, &mut p, vec![]);
        // The lowest clickable row is the single visible diagnostic.
        let row = rows
            .iter()
            .max_by(|a, b| a.top().partial_cmp(&b.top()).unwrap())
            .copied()
            .unwrap();
        let pos = row.center();
        let press = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(&ctx, &mut p, vec![Event::PointerMoved(pos), press(true)]);
        let (res, _) = frame(&ctx, &mut p, vec![press(false)]);
        assert_eq!(res, Some((PathBuf::from("ws").join("a.rs"), 2, 0)));
    }
}
