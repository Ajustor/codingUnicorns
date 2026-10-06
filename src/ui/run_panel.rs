use crate::runner::{RunConfig, RunManager};
use std::path::PathBuf;

pub struct RunPanelAction {
    pub run_clicked: bool,
    pub stop_clicked: bool,
}

pub struct RunPanel {
    pub show_add_config: bool,
    pub new_config_name: String,
    pub new_config_command: String,
    pub new_config_cwd: String,
    pub edit_idx: Option<usize>,
}

impl RunPanel {
    pub fn new() -> Self {
        Self {
            show_add_config: false,
            new_config_name: String::new(),
            new_config_command: String::new(),
            new_config_cwd: "${workspaceRoot}".to_string(),
            edit_idx: None,
        }
    }

    /// Returns a `RunPanelAction` describing what the user clicked.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        runner: &mut RunManager,
        workspace: Option<&PathBuf>,
        current_file: Option<&PathBuf>,
        is_running: bool,
    ) -> RunPanelAction {
        let mut run_clicked = false;
        let mut stop_clicked = false;

        // ── HEADER with config selector and Run/Stop button ────────────
        ui.horizontal(|ui| {
            let active_name = runner
                .active_config()
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "No configuration".to_string());

            egui::ComboBox::from_id_salt("run_config_selector")
                .selected_text(&active_name)
                .width(ui.available_width() - 56.0)
                .show_ui(ui, |ui| {
                    for (i, config) in runner.configs.iter().enumerate() {
                        let selected = runner.active_config == i;
                        if ui.selectable_label(selected, &config.name).clicked() {
                            runner.active_config = i;
                        }
                    }
                });

            let (btn_text, btn_color) = if is_running {
                ("■", egui::Color32::from_rgb(200, 80, 80))
            } else {
                ("▶", egui::Color32::from_rgb(80, 200, 80))
            };

            if ui
                .add(
                    egui::Button::new(egui::RichText::new(btn_text).size(16.0).color(btn_color))
                        .min_size(egui::vec2(40.0, 28.0)),
                )
                .clicked()
            {
                if is_running {
                    stop_clicked = true;
                } else {
                    run_clicked = true;
                }
            }
        });

        ui.add_space(4.0);

        // ── CONFIGURATIONS LIST ─────────────────────────────────────────
        ui.label(
            egui::RichText::new("CONFIGURATIONS")
                .size(10.0)
                .color(egui::Color32::GRAY)
                .strong(),
        );
        ui.add_space(2.0);

        let mut to_remove: Option<usize> = None;
        let mut to_run: Option<usize> = None;

        // Collect config data before mutably borrowing runner later
        let configs_snapshot: Vec<(String, String, bool)> = runner
            .configs
            .iter()
            .enumerate()
            .map(|(i, c)| (c.name.clone(), c.command.clone(), runner.active_config == i))
            .collect();

        for (i, (name, command, is_active)) in configs_snapshot.iter().enumerate() {
            egui::Frame::new()
                .fill(if *is_active {
                    egui::Color32::from_rgba_unmultiplied(100, 160, 255, 20)
                } else {
                    egui::Color32::TRANSPARENT
                })
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if *is_active {
                            ui.label(
                                egui::RichText::new("▶")
                                    .size(10.0)
                                    .color(egui::Color32::from_rgb(80, 200, 80)),
                            );
                        } else {
                            ui.label(egui::RichText::new("  ").size(10.0));
                        }

                        let resp = ui.add(
                            egui::Label::new(egui::RichText::new(name).size(12.0).color(
                                if *is_active {
                                    egui::Color32::WHITE
                                } else {
                                    egui::Color32::from_rgb(180, 180, 180)
                                },
                            ))
                            .sense(egui::Sense::click()),
                        );
                        if resp.clicked() {
                            runner.active_config = i;
                        }
                        if resp.double_clicked() {
                            to_run = Some(i);
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✕").on_hover_text("Remove").clicked() {
                                to_remove = Some(i);
                            }
                        });
                    });

                    ui.label(
                        egui::RichText::new(format!("  {}", command))
                            .size(10.0)
                            .color(egui::Color32::GRAY)
                            .monospace(),
                    );
                });
        }

        if let Some(i) = to_remove {
            runner.remove_config(i);
            runner.save();
        }
        if let Some(i) = to_run {
            runner.active_config = i;
            run_clicked = true;
        }

        ui.add_space(8.0);

        // ── ADD CONFIGURATION ───────────────────────────────────────────
        if ui.button("+ Add Configuration").clicked() {
            self.show_add_config = true;
            self.new_config_name.clear();
            self.new_config_command.clear();
            self.new_config_cwd = "${workspaceRoot}".to_string();
        }

        if self.show_add_config {
            ui.add_space(4.0);
            ui.separator();
            ui.label(egui::RichText::new("New Configuration").size(12.0).strong());

            egui::Grid::new("new_config_grid")
                .num_columns(2)
                .spacing([4.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Name:");
                    ui.text_edit_singleline(&mut self.new_config_name);
                    ui.end_row();

                    ui.label("Command:");
                    ui.text_edit_singleline(&mut self.new_config_command);
                    ui.end_row();

                    ui.label("Working dir:");
                    ui.text_edit_singleline(&mut self.new_config_cwd);
                    ui.end_row();
                });

            ui.label(
                egui::RichText::new(
                    "Variables: ${workspaceRoot}, ${file}, ${fileDir}, ${fileName}",
                )
                .size(10.0)
                .color(egui::Color32::GRAY),
            );

            ui.horizontal(|ui| {
                let can_add =
                    !self.new_config_name.is_empty() && !self.new_config_command.is_empty();
                if ui.add_enabled(can_add, egui::Button::new("Add")).clicked() {
                    runner.add_config(RunConfig {
                        name: self.new_config_name.clone(),
                        command: self.new_config_command.clone(),
                        cwd: self.new_config_cwd.clone(),
                        env: vec![],
                        args: vec![],
                        debug: None,
                    });
                    runner.save();
                    self.show_add_config = false;
                }
                if ui.button("Cancel").clicked() {
                    self.show_add_config = false;
                }
            });
        }

        ui.add_space(8.0);
        ui.separator();

        // ── HINTS ───────────────────────────────────────────────────────
        ui.label(
            egui::RichText::new("F5 — Run active configuration\nCtrl+F5 — Run without stopping")
                .size(10.0)
                .color(egui::Color32::GRAY),
        );

        if runner.configs.is_empty() {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(
                    "No run configurations found.\nOpen a workspace to auto-detect.",
                )
                .size(11.0)
                .color(egui::Color32::GRAY),
            );
        }

        // Keep the workspace/current_file params used (suppress dead-code lint)
        let _ = workspace;
        let _ = current_file;

        RunPanelAction {
            run_clicked,
            stop_clicked,
        }
    }
}

impl Default for RunPanel {
    fn default() -> Self {
        Self::new()
    }
}
