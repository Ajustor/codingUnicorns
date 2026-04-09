use crate::git::merge::{parse_conflict_file, HunkResolution};
use std::path::PathBuf;

pub struct MergeView {
    pub file_path: PathBuf,
    /// Original file content, stored once on open to avoid re-reading disk.
    original_content: String,
    pub ours_text: String,
    pub theirs_text: String,
    pub result_text: String,
    pub hunks: Vec<crate::git::merge::ConflictHunk>,
    pub is_active: bool,
}

pub enum MergeAction {
    None,
    SaveAndResolve,
    Cancel,
}

impl MergeView {
    pub fn open(file_path: PathBuf) -> Option<Self> {
        let content = std::fs::read_to_string(&file_path).ok()?;
        let parsed = parse_conflict_file(&content)?;
        Some(Self {
            file_path,
            original_content: content,
            ours_text: parsed.ours_content,
            theirs_text: parsed.theirs_content,
            result_text: parsed.result_content,
            hunks: parsed.hunks,
            is_active: true,
        })
    }

    pub fn accept_all_ours(&mut self) {
        for hunk in &mut self.hunks {
            hunk.resolution = HunkResolution::AcceptOurs;
        }
        self.rebuild_result();
    }

    pub fn accept_all_theirs(&mut self) {
        for hunk in &mut self.hunks {
            hunk.resolution = HunkResolution::AcceptTheirs;
        }
        self.rebuild_result();
    }

    pub fn accept_ours(&mut self, idx: usize) {
        if let Some(hunk) = self.hunks.get_mut(idx) {
            hunk.resolution = HunkResolution::AcceptOurs;
        }
        self.rebuild_result();
    }

    pub fn accept_theirs(&mut self, idx: usize) {
        if let Some(hunk) = self.hunks.get_mut(idx) {
            hunk.resolution = HunkResolution::AcceptTheirs;
        }
        self.rebuild_result();
    }

    /// Rebuild result from stored original content + current hunk resolutions.
    /// Uses parse_conflict_file to avoid duplicating parsing logic.
    fn rebuild_result(&mut self) {
        if let Some(parsed) = parse_conflict_file(&self.original_content) {
            let mut result: Vec<String> = vec![];
            let lines: Vec<&str> = self.original_content.lines().collect();
            let mut i = 0;
            let mut hunk_idx = 0;

            while i < lines.len() {
                if lines[i].starts_with("<<<<<<<") {
                    // Skip conflict markers, use resolution
                    i += 1;
                    while i < lines.len() && !lines[i].starts_with("=======") {
                        i += 1;
                    }
                    i += 1;
                    while i < lines.len() && !lines[i].starts_with(">>>>>>>") {
                        i += 1;
                    }
                    i += 1;

                    if let Some(hunk) = self.hunks.get(hunk_idx) {
                        if let Some(parsed_hunk) = parsed.hunks.get(hunk_idx) {
                            match hunk.resolution {
                                HunkResolution::AcceptTheirs => {
                                    result.extend(parsed_hunk.theirs.clone());
                                }
                                _ => {
                                    result.extend(parsed_hunk.ours.clone());
                                }
                            }
                        }
                    }
                    hunk_idx += 1;
                } else {
                    result.push(lines[i].to_string());
                    i += 1;
                }
            }
            self.result_text = result.join("\n");
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> MergeAction {
        let mut action = MergeAction::None;

        // Toolbar
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("MERGE CONFLICT").strong());
            ui.label(
                egui::RichText::new(
                    self.file_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                )
                .color(egui::Color32::YELLOW),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    action = MergeAction::Cancel;
                }
                if ui.button("Save & Resolve").clicked() {
                    action = MergeAction::SaveAndResolve;
                }
                if ui.button("Accept All Theirs").clicked() {
                    self.accept_all_theirs();
                }
                if ui.button("Accept All Ours").clicked() {
                    self.accept_all_ours();
                }
            });
        });
        ui.separator();

        // Per-hunk resolution buttons
        let mut accept_ours_idx: Option<usize> = None;
        let mut accept_theirs_idx: Option<usize> = None;
        for (idx, hunk) in self.hunks.iter().enumerate() {
            ui.horizontal(|ui| {
                let status = match hunk.resolution {
                    HunkResolution::Unresolved => "unresolved",
                    HunkResolution::AcceptOurs => "\u{2190} ours",
                    HunkResolution::AcceptTheirs => "theirs \u{2192}",
                };
                ui.label(
                    egui::RichText::new(format!("Conflict #{}: {}", idx + 1, status))
                        .small()
                        .color(egui::Color32::YELLOW),
                );
                if ui.small_button("\u{2190} Ours").clicked() {
                    accept_ours_idx = Some(idx);
                }
                if ui.small_button("Theirs \u{2192}").clicked() {
                    accept_theirs_idx = Some(idx);
                }
            });
        }
        if let Some(idx) = accept_ours_idx {
            self.accept_ours(idx);
        }
        if let Some(idx) = accept_theirs_idx {
            self.accept_theirs(idx);
        }

        ui.separator();

        // Three panels side-by-side
        let available = ui.available_size();
        let panel_width = (available.x - 12.0) / 3.0;

        ui.horizontal(|ui| {
            // Left: Ours (read-only) — use label to avoid cloning
            ui.vertical(|ui| {
                ui.set_width(panel_width);
                ui.label(
                    egui::RichText::new("OURS (current)")
                        .size(11.0)
                        .color(egui::Color32::from_gray(150)),
                );
                egui::ScrollArea::vertical()
                    .id_salt("merge_ours")
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.ours_text.as_str())
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        );
                    });
            });
            ui.separator();

            // Center: Result (editable)
            ui.vertical(|ui| {
                ui.set_width(panel_width);
                ui.label(
                    egui::RichText::new("RESULT")
                        .size(11.0)
                        .color(egui::Color32::from_gray(150)),
                );
                egui::ScrollArea::vertical()
                    .id_salt("merge_result")
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.result_text)
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        );
                    });
            });
            ui.separator();

            // Right: Theirs (read-only) — use &str to avoid cloning
            ui.vertical(|ui| {
                ui.set_width(panel_width);
                ui.label(
                    egui::RichText::new("THEIRS (incoming)")
                        .size(11.0)
                        .color(egui::Color32::from_gray(150)),
                );
                egui::ScrollArea::vertical()
                    .id_salt("merge_theirs")
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.theirs_text.as_str())
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        );
                    });
            });
        });

        action
    }
}
