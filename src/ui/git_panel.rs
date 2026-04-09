use crate::git::{BranchInfo, FileChangeKind, GitStatus};

pub struct GitPanel {
    pub commit_message: String,
    pub new_branch_name: String,
    pub show_new_branch_dialog: bool,
    pub new_branch_from: String,
    pub rename_branch_name: String,
    pub rename_branch_old: String,
    pub show_rename_dialog: bool,
    /// Cached conflict file paths to avoid reading files every frame.
    cached_conflict_files: Vec<String>,
    /// Number of files when cache was last computed.
    conflict_cache_file_count: usize,
}

impl GitPanel {
    pub fn new() -> Self {
        Self {
            commit_message: String::new(),
            new_branch_name: String::new(),
            show_new_branch_dialog: false,
            new_branch_from: String::new(),
            rename_branch_name: String::new(),
            rename_branch_old: String::new(),
            show_rename_dialog: false,
            cached_conflict_files: vec![],
            conflict_cache_file_count: 0,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, git: &mut GitStatus) -> Option<String> {
        // Branch + ahead/behind
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("⎇ {}", git.branch)).strong());
            if git.ahead > 0 || git.behind > 0 {
                ui.label(
                    egui::RichText::new(format!("↑{} ↓{}", git.ahead, git.behind))
                        .color(egui::Color32::from_rgb(180, 180, 80))
                        .small(),
                );
            }
        });
        ui.separator();

        // Commit message + buttons
        ui.label("Commit message:");
        ui.add(
            egui::TextEdit::multiline(&mut self.commit_message)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text("Enter commit message…"),
        );
        ui.horizontal(|ui| {
            let can_commit = git.has_staged_files() && !self.commit_message.trim().is_empty();
            if ui
                .add_enabled(can_commit, egui::Button::new("Commit"))
                .clicked()
            {
                let msg = self.commit_message.trim().to_string();
                match git.commit(&msg) {
                    Ok(()) => {
                        self.commit_message.clear();
                        git.last_error = None;
                    }
                    Err(e) => git.last_error = Some(e),
                }
            }
            if ui.button("Push").clicked() {
                if let Err(e) = git.push() {
                    git.last_error = Some(e);
                } else {
                    git.last_error = None;
                }
            }
            if ui.button("Pull").clicked() {
                if let Err(e) = git.pull() {
                    git.last_error = Some(e);
                } else {
                    git.last_error = None;
                }
            }
        });

        // Error display
        if let Some(err) = &git.last_error.clone() {
            ui.separator();
            ui.label(
                egui::RichText::new(format!("Error: {err}"))
                    .color(egui::Color32::from_rgb(220, 80, 80))
                    .small(),
            );
        }

        ui.separator();
        self.show_branches(ui, git);
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            // --- STAGED CHANGES ---
            let staged: Vec<String> = git
                .files
                .iter()
                .filter(|f| f.index_status != FileChangeKind::None)
                .map(|f| f.path.clone())
                .collect();

            if !staged.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("STAGED CHANGES").strong().small());
                    if ui.small_button("Unstage All").clicked() {
                        git.unstage_all();
                    }
                });
                let mut unstage_path: Option<String> = None;
                for path in &staged {
                    let kind = git
                        .files
                        .iter()
                        .find(|f| &f.path == path)
                        .map(|f| &f.index_status)
                        .cloned()
                        .unwrap_or(FileChangeKind::None);
                    let (icon, color) = file_kind_icon(&kind);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(icon)
                                .color(color)
                                .monospace()
                                .small(),
                        );
                        ui.label(egui::RichText::new(path).small());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button(egui::RichText::new("−").color(egui::Color32::RED))
                                .on_hover_text("Unstage")
                                .clicked()
                            {
                                unstage_path = Some(path.clone());
                            }
                        });
                    });
                }
                if let Some(p) = unstage_path {
                    git.unstage_file(&p);
                }
                ui.add_space(4.0);
            }

            // --- UNSTAGED CHANGES ---
            let unstaged: Vec<String> = git
                .files
                .iter()
                .filter(|f| f.wt_status != FileChangeKind::None)
                .map(|f| f.path.clone())
                .collect();

            if !unstaged.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("CHANGES").strong().small());
                    if ui.small_button("Stage All").clicked() {
                        git.stage_all();
                    }
                });
                let mut stage_path: Option<String> = None;
                for path in &unstaged {
                    let kind = git
                        .files
                        .iter()
                        .find(|f| &f.path == path)
                        .map(|f| &f.wt_status)
                        .cloned()
                        .unwrap_or(FileChangeKind::None);
                    let (icon, color) = file_kind_icon(&kind);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(icon)
                                .color(color)
                                .monospace()
                                .small(),
                        );
                        ui.label(egui::RichText::new(path).small());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button(
                                    egui::RichText::new("+").color(egui::Color32::GREEN),
                                )
                                .on_hover_text("Stage")
                                .clicked()
                            {
                                stage_path = Some(path.clone());
                            }
                        });
                    });
                }
                if let Some(p) = stage_path {
                    git.stage_file(&p);
                }
            }

            if staged.is_empty() && unstaged.is_empty() {
                ui.label(
                    egui::RichText::new("No changes")
                        .color(egui::Color32::GRAY)
                        .small(),
                );
            }
        });

        // Show conflicts section and return any clicked conflict file path
        self.show_conflicts(ui, git)
    }

    fn show_branches(&mut self, ui: &mut egui::Ui, git: &mut GitStatus) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("BRANCHES").strong().small());
        });

        // Inline dialogs
        if self.show_new_branch_dialog {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("New branch:").small());
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_branch_name)
                        .desired_width(120.0)
                        .hint_text("branch-name"),
                );
                let can_create = !self.new_branch_name.trim().is_empty();
                if ui.add_enabled(can_create, egui::Button::new("Create").small()).clicked() {
                    let name = self.new_branch_name.trim().to_string();
                    let from = self.new_branch_from.clone();
                    match git.create_branch(&name, &from) {
                        Ok(()) => {
                            git.last_error = None;
                        }
                        Err(e) => git.last_error = Some(e),
                    }
                    self.new_branch_name.clear();
                    self.show_new_branch_dialog = false;
                }
                if ui.button(egui::RichText::new("✕").small()).clicked() {
                    self.show_new_branch_dialog = false;
                    self.new_branch_name.clear();
                }
            });
        }

        if self.show_rename_dialog {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("Rename '{}':", self.rename_branch_old)).small());
                ui.add(
                    egui::TextEdit::singleline(&mut self.rename_branch_name)
                        .desired_width(120.0)
                        .hint_text("new-name"),
                );
                let can_rename = !self.rename_branch_name.trim().is_empty();
                if ui.add_enabled(can_rename, egui::Button::new("Rename").small()).clicked() {
                    let old = self.rename_branch_old.clone();
                    let new_name = self.rename_branch_name.trim().to_string();
                    match git.rename_branch(&old, &new_name) {
                        Ok(()) => {
                            git.last_error = None;
                        }
                        Err(e) => git.last_error = Some(e),
                    }
                    self.rename_branch_name.clear();
                    self.show_rename_dialog = false;
                }
                if ui.button(egui::RichText::new("✕").small()).clicked() {
                    self.show_rename_dialog = false;
                    self.rename_branch_name.clear();
                }
            });
        }

        egui::ScrollArea::vertical()
            .id_salt("branches")
            .max_height(250.0)
            .show(ui, |ui| {
                // --- Commit graph section ---
                let graph_entries = git.graph_entries.clone();
                if !graph_entries.is_empty() {
                    egui::CollapsingHeader::new(egui::RichText::new("Graph").small())
                        .default_open(true)
                        .show(ui, |ui| {
                            let row_height = 18.0;
                            let dot_x = 10.0;
                            let line_x = dot_x;

                            for (i, entry) in graph_entries.iter().enumerate() {
                                let (rect, _response) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), row_height),
                                    egui::Sense::hover(),
                                );

                                if ui.is_rect_visible(rect) {
                                    let painter = ui.painter();

                                    // Dot color
                                    let dot_color = if entry.is_head {
                                        egui::Color32::from_rgb(80, 200, 80)
                                    } else if !entry.branches.is_empty() {
                                        egui::Color32::from_rgb(80, 140, 220)
                                    } else {
                                        egui::Color32::from_gray(130)
                                    };

                                    let dot_center = egui::pos2(
                                        rect.left() + dot_x,
                                        rect.center().y,
                                    );

                                    // Draw vertical line (skip last entry)
                                    if i + 1 < graph_entries.len() {
                                        painter.line_segment(
                                            [
                                                egui::pos2(rect.left() + line_x, dot_center.y + 4.0),
                                                egui::pos2(rect.left() + line_x, rect.bottom()),
                                            ],
                                            egui::Stroke::new(1.5, egui::Color32::from_gray(100)),
                                        );
                                    }

                                    // Draw dot
                                    painter.circle_filled(dot_center, 4.0, dot_color);
                                    painter.circle_stroke(
                                        dot_center,
                                        4.0,
                                        egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
                                    );

                                    // Text: hash + message + branch tags
                                    let text_x = rect.left() + dot_x * 2.0 + 6.0;
                                    let mut text = format!("{} {}", entry.short_hash, entry.message);
                                    if !entry.branches.is_empty() {
                                        let tags: Vec<String> = entry
                                            .branches
                                            .iter()
                                            .map(|b| format!("[{}]", b))
                                            .collect();
                                        text = format!("{} {}", text, tags.join(" "));
                                    }
                                    painter.text(
                                        egui::pos2(text_x, rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        text,
                                        egui::FontId::monospace(10.0),
                                        egui::Color32::from_gray(200),
                                    );
                                }
                            }
                        });
                }

                // Clone branch data upfront to avoid borrow conflicts during mutation
                let local: Vec<BranchInfo> =
                    git.branches.iter().filter(|b| !b.is_remote).cloned().collect();
                let remote: Vec<BranchInfo> =
                    git.branches.iter().filter(|b| b.is_remote).cloned().collect();

                // Pending actions collected during UI to execute after loop
                let mut checkout_name: Option<String> = None;
                let mut merge_name: Option<String> = None;
                let mut delete_name: Option<String> = None;
                let mut create_from: Option<String> = None;
                let mut rename_old: Option<String> = None;

                // Local branches
                if !local.is_empty() {
                    egui::CollapsingHeader::new(egui::RichText::new("Local").small())
                        .default_open(true)
                        .show(ui, |ui| {
                            for branch in &local {
                                let icon = if branch.is_current { "●" } else { "○" };
                                let color = if branch.is_current {
                                    egui::Color32::GREEN
                                } else {
                                    egui::Color32::from_rgb(80, 140, 220)
                                };
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(icon).color(color).small());
                                    let label = egui::RichText::new(&branch.name)
                                        .small()
                                        .color(color);
                                    let response = ui.selectable_label(branch.is_current, label);
                                    if response.clicked() && !branch.is_current {
                                        checkout_name = Some(branch.name.clone());
                                    }
                                    let branch_name = branch.name.clone();
                                    let is_current = branch.is_current;
                                    response.context_menu(|ui| {
                                        if !is_current {
                                            if ui.button("Checkout").clicked() {
                                                checkout_name = Some(branch_name.clone());
                                                ui.close_menu();
                                            }
                                            if ui.button("Merge into current").clicked() {
                                                merge_name = Some(branch_name.clone());
                                                ui.close_menu();
                                            }
                                        }
                                        if ui.button("Create new branch from here").clicked() {
                                            create_from = Some(branch_name.clone());
                                            ui.close_menu();
                                        }
                                        if !is_current {
                                            if ui.button("Rename").clicked() {
                                                rename_old = Some(branch_name.clone());
                                                ui.close_menu();
                                            }
                                            ui.separator();
                                            if ui
                                                .button(
                                                    egui::RichText::new("Delete")
                                                        .color(egui::Color32::from_rgb(220, 80, 80)),
                                                )
                                                .clicked()
                                            {
                                                delete_name = Some(branch_name.clone());
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                });
                            }
                        });
                }

                // Remote branches
                if !remote.is_empty() {
                    egui::CollapsingHeader::new(egui::RichText::new("Remote").small())
                        .default_open(false)
                        .show(ui, |ui| {
                            for branch in &remote {
                                let color = egui::Color32::from_gray(140);
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("○").color(color).small());
                                    let response = ui.label(
                                        egui::RichText::new(&branch.name).small().color(color),
                                    );
                                    response.context_menu(|ui| {
                                        if ui.button("Create new branch from here").clicked() {
                                            create_from = Some(branch.name.clone());
                                            ui.close_menu();
                                        }
                                    });
                                });
                            }
                        });
                }

                if local.is_empty() && remote.is_empty() {
                    ui.label(
                        egui::RichText::new("No branches")
                            .color(egui::Color32::GRAY)
                            .small(),
                    );
                }

                // Execute pending actions
                if let Some(name) = checkout_name {
                    if let Err(e) = git.checkout_branch(&name) {
                        git.last_error = Some(e);
                    }
                }
                if let Some(name) = merge_name {
                    if let Err(e) = git.merge_branch(&name) {
                        git.last_error = Some(e);
                    }
                }
                if let Some(name) = delete_name {
                    if let Err(e) = git.delete_branch(&name) {
                        git.last_error = Some(e);
                    }
                }
                if let Some(from) = create_from {
                    self.new_branch_from = from;
                    self.show_new_branch_dialog = true;
                    self.new_branch_name.clear();
                }
                if let Some(old) = rename_old {
                    self.rename_branch_old = old;
                    self.show_rename_dialog = true;
                    self.rename_branch_name.clear();
                }
            });
    }

    /// Check whether a file (given by relative path) contains git conflict markers.
    fn has_conflict_markers(repo_path: Option<&std::path::PathBuf>, rel_path: &str) -> bool {
        if let Some(root) = repo_path {
            let full = root.join(rel_path);
            if let Ok(content) = std::fs::read_to_string(&full) {
                return content.contains("<<<<<<<");
            }
        }
        false
    }

    pub fn show_conflicts(
        &mut self,
        ui: &mut egui::Ui,
        git: &mut GitStatus,
    ) -> Option<String> {
        // Only rescan for conflict markers when the file list changes
        let file_count = git.files.len();
        if file_count != self.conflict_cache_file_count {
            self.conflict_cache_file_count = file_count;
            self.cached_conflict_files = git
                .files
                .iter()
                .filter(|f| Self::has_conflict_markers(git.repo_path.as_ref(), &f.path))
                .map(|f| f.path.clone())
                .collect();
        }
        let conflict_files = &self.cached_conflict_files;

        if conflict_files.is_empty() {
            return None;
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("CONFLICTS")
                    .strong()
                    .small()
                    .color(egui::Color32::from_rgb(220, 80, 80)),
            );
        });

        let mut open_path: Option<String> = None;
        for path in conflict_files {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("!")
                        .color(egui::Color32::from_rgb(220, 80, 80))
                        .monospace()
                        .small(),
                );
                if ui
                    .selectable_label(false, egui::RichText::new(path).small())
                    .clicked()
                {
                    open_path = Some(path.clone());
                }
            });
        }
        open_path
    }
}

fn file_kind_icon(kind: &FileChangeKind) -> (&'static str, egui::Color32) {
    match kind {
        FileChangeKind::Modified => ("M", egui::Color32::YELLOW),
        FileChangeKind::Added => ("A", egui::Color32::GREEN),
        FileChangeKind::Deleted => ("D", egui::Color32::RED),
        FileChangeKind::Renamed => ("R", egui::Color32::from_rgb(100, 150, 255)),
        FileChangeKind::Untracked => ("U", egui::Color32::from_rgb(100, 200, 100)),
        FileChangeKind::None => ("·", egui::Color32::GRAY),
    }
}
