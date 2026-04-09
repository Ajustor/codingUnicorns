use crate::git::merge::{parse_conflict_file, HunkResolution};
use std::path::PathBuf;

pub struct MergeView {
    pub file_path: PathBuf,
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

    fn rebuild_result(&mut self) {
        if let Ok(content) = std::fs::read_to_string(&self.file_path) {
            let lines: Vec<&str> = content.lines().collect();
            let mut result: Vec<String> = vec![];
            let mut i = 0;
            let mut hunk_idx = 0;

            while i < lines.len() {
                if lines[i].starts_with("<<<<<<<") {
                    let mut ours: Vec<String> = vec![];
                    let mut theirs: Vec<String> = vec![];
                    i += 1;
                    while i < lines.len() && !lines[i].starts_with("=======") {
                        ours.push(lines[i].to_string());
                        i += 1;
                    }
                    i += 1;
                    while i < lines.len() && !lines[i].starts_with(">>>>>>>") {
                        theirs.push(lines[i].to_string());
                        i += 1;
                    }
                    i += 1;

                    if let Some(hunk) = self.hunks.get(hunk_idx) {
                        match hunk.resolution {
                            HunkResolution::AcceptTheirs => result.extend(theirs),
                            _ => result.extend(ours),
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
                    self.file_path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()
                ).color(egui::Color32::YELLOW),
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
                    HunkResolution::AcceptOurs => "← ours",
                    HunkResolution::AcceptTheirs => "theirs →",
                };
                ui.label(egui::RichText::new(format!("Conflict #{}: {}", idx + 1, status))
                    .small().color(egui::Color32::YELLOW));
                if ui.small_button("← Ours").clicked() {
                    accept_ours_idx = Some(idx);
                }
                if ui.small_button("Theirs →").clicked() {
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
            // Left: Ours (read-only)
            ui.vertical(|ui| {
                ui.set_width(panel_width);
                ui.label(egui::RichText::new("OURS (current)").size(11.0).color(egui::Color32::from_gray(150)));
                egui::ScrollArea::vertical().id_salt("merge_ours").show(ui, |ui| {
                    let mut text = self.ours_text.clone();
                    ui.add(egui::TextEdit::multiline(&mut text)
                        .code_editor().desired_width(f32::INFINITY).interactive(false));
                });
            });
            ui.separator();

            // Center: Result (editable)
            ui.vertical(|ui| {
                ui.set_width(panel_width);
                ui.label(egui::RichText::new("RESULT").size(11.0).color(egui::Color32::from_gray(150)));
                egui::ScrollArea::vertical().id_salt("merge_result").show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut self.result_text)
                        .code_editor().desired_width(f32::INFINITY));
                });
            });
            ui.separator();

            // Right: Theirs (read-only)
            ui.vertical(|ui| {
                ui.set_width(panel_width);
                ui.label(egui::RichText::new("THEIRS (incoming)").size(11.0).color(egui::Color32::from_gray(150)));
                egui::ScrollArea::vertical().id_salt("merge_theirs").show(ui, |ui| {
                    let mut text = self.theirs_text.clone();
                    ui.add(egui::TextEdit::multiline(&mut text)
                        .code_editor().desired_width(f32::INFINITY).interactive(false));
                });
            });
        });

        action
    }
}
