pub struct SettingsPanel {
    pub open: bool,
    rebinding: Option<String>,
}

impl SettingsPanel {
    pub fn new() -> Self {
        Self {
            open: false,
            rebinding: None,
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Render settings content inline into the provided ui.
    /// Returns true if config was changed.
    pub fn show_inline(&mut self, ui: &mut egui::Ui, config: &mut crate::config::Config) -> bool {
        let mut changed = false;

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.heading("⚙  Settings");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new("Changes are saved automatically")
                        .size(11.0)
                        .color(egui::Color32::GRAY),
                );
            });
        });
        ui.separator();
        ui.add_space(4.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            // === EDITOR section ===
            ui.heading("Editor");
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Font size:");
                let old = config.font.size;
                ui.add(egui::Slider::new(&mut config.font.size, 8.0..=32.0).suffix("px"));
                if config.font.size != old {
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label("Tab size:");
                let old = config.editor.tab_size;
                ui.add(egui::Slider::new(&mut config.editor.tab_size, 1..=8));
                if config.editor.tab_size != old {
                    changed = true;
                }
            });

            let old = config.editor.insert_spaces;
            ui.checkbox(&mut config.editor.insert_spaces, "Insert spaces (not tabs)");
            if config.editor.insert_spaces != old {
                changed = true;
            }

            let old = config.editor.line_numbers;
            ui.checkbox(&mut config.editor.line_numbers, "Show line numbers");
            if config.editor.line_numbers != old {
                changed = true;
            }

            let old = config.editor.word_wrap;
            ui.checkbox(&mut config.editor.word_wrap, "Word wrap");
            if config.editor.word_wrap != old {
                changed = true;
            }

            let old = config.editor.auto_save;
            ui.checkbox(&mut config.editor.auto_save, "Auto-save on focus loss");
            if config.editor.auto_save != old {
                changed = true;
            }

            let old = config.editor.auto_close_brackets;
            ui.checkbox(&mut config.editor.auto_close_brackets, "Auto-close brackets");
            if config.editor.auto_close_brackets != old {
                changed = true;
            }

            ui.add_space(12.0);

            // === THEME section ===
            ui.heading("Theme");
            ui.separator();

            ui.label("Preset themes:");
            ui.horizontal(|ui| {
                if ui.button("Dark (default)").clicked() {
                    config.theme = crate::config::Theme {
                        name: "dark".into(),
                        background: [30, 30, 30],
                        foreground: [212, 212, 212],
                        accent: [0, 122, 204],
                    };
                    changed = true;
                }
                if ui.button("Monokai").clicked() {
                    config.theme = crate::config::Theme {
                        name: "monokai".into(),
                        background: [39, 40, 34],
                        foreground: [248, 248, 242],
                        accent: [166, 226, 46],
                    };
                    changed = true;
                }
                if ui.button("Solarized Dark").clicked() {
                    config.theme = crate::config::Theme {
                        name: "solarized-dark".into(),
                        background: [0, 43, 54],
                        foreground: [131, 148, 150],
                        accent: [38, 139, 210],
                    };
                    changed = true;
                }
                if ui.button("One Dark").clicked() {
                    config.theme = crate::config::Theme {
                        name: "one-dark".into(),
                        background: [40, 44, 52],
                        foreground: [171, 178, 191],
                        accent: [97, 175, 239],
                    };
                    changed = true;
                }
            });

            ui.add_space(8.0);
            ui.label("Custom colors:");
            color_picker_row(
                ui,
                "Background:",
                &mut config.theme.background,
                &mut changed,
            );
            color_picker_row(
                ui,
                "Foreground:",
                &mut config.theme.foreground,
                &mut changed,
            );
            color_picker_row(ui, "Accent:", &mut config.theme.accent, &mut changed);

            // Color preview swatch
            ui.add_space(8.0);
            let preview_rect_size = egui::vec2(ui.available_width(), 40.0);
            let (preview_rect, _) = ui.allocate_exact_size(preview_rect_size, egui::Sense::hover());
            ui.painter().rect_filled(
                preview_rect,
                4.0,
                egui::Color32::from_rgb(
                    config.theme.background[0],
                    config.theme.background[1],
                    config.theme.background[2],
                ),
            );
            ui.painter().text(
                preview_rect.center(),
                egui::Align2::CENTER_CENTER,
                "The quick brown fox — sample text",
                egui::FontId::proportional(13.0),
                egui::Color32::from_rgb(
                    config.theme.foreground[0],
                    config.theme.foreground[1],
                    config.theme.foreground[2],
                ),
            );
            let accent_bar = egui::Rect::from_min_size(
                egui::pos2(preview_rect.min.x, preview_rect.max.y - 4.0),
                egui::vec2(preview_rect.width(), 4.0),
            );
            ui.painter().rect_filled(
                accent_bar,
                0.0,
                egui::Color32::from_rgb(
                    config.theme.accent[0],
                    config.theme.accent[1],
                    config.theme.accent[2],
                ),
            );

            ui.add_space(12.0);

            // === KEYBINDINGS section ===
            ui.collapsing("⌨ Keybindings", |ui| {
                let rebinding = &mut self.rebinding;
                keybinding_row(
                    ui,
                    "New File",
                    &mut config.keybindings.new_file,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Open Folder",
                    &mut config.keybindings.open_folder,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Open File",
                    &mut config.keybindings.open_file,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Save",
                    &mut config.keybindings.save,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Command Palette",
                    &mut config.keybindings.command_palette,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Toggle Sidebar",
                    &mut config.keybindings.toggle_sidebar,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Toggle Terminal",
                    &mut config.keybindings.toggle_terminal,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Shortcuts Help",
                    &mut config.keybindings.shortcuts_help,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Settings",
                    &mut config.keybindings.settings,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Find in File",
                    &mut config.keybindings.find,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Close Tab",
                    &mut config.keybindings.close_tab,
                    rebinding,
                    &mut changed,
                );

                if ui.button("Reset to defaults").clicked() {
                    config.keybindings = crate::config::KeyBindings::default();
                    changed = true;
                }
            });

            ui.add_space(12.0);
        });

        changed
    }

    /// No-op — settings are now shown inline via the tab system.
    pub fn show(&mut self, _ctx: &egui::Context, _config: &mut crate::config::Config) -> bool {
        false
    }
}

fn keybinding_row(
    ui: &mut egui::Ui,
    label: &str,
    binding: &mut crate::config::KeyBinding,
    rebinding: &mut Option<String>,
    changed: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let is_rebinding = rebinding.as_deref() == Some(label);

            if is_rebinding {
                ui.label(
                    egui::RichText::new("Press a key...")
                        .color(egui::Color32::from_rgb(255, 200, 0))
                        .monospace(),
                );

                ui.input(|i| {
                    for event in &i.events {
                        if let egui::Event::Key {
                            key,
                            modifiers,
                            pressed: true,
                            ..
                        } = event
                        {
                            binding.key = format!("{key:?}");
                            binding.ctrl = modifiers.ctrl;
                            binding.shift = modifiers.shift;
                            binding.alt = modifiers.alt;
                            *rebinding = None;
                            *changed = true;
                        }
                    }
                });

                if ui.button("Cancel").clicked() {
                    *rebinding = None;
                }
            } else {
                let btn = egui::Button::new(
                    egui::RichText::new(binding.display())
                        .monospace()
                        .size(11.0),
                );
                if ui.add(btn).on_hover_text("Click to rebind").clicked() {
                    *rebinding = Some(label.to_string());
                }
            }
        });
    });
    ui.add_space(2.0);
}

fn color_picker_row(ui: &mut egui::Ui, label: &str, color: &mut [u8; 3], changed: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut egui_color = egui::Color32::from_rgb(color[0], color[1], color[2]);
        if ui.color_edit_button_srgba(&mut egui_color).changed() {
            color[0] = egui_color.r();
            color[1] = egui_color.g();
            color[2] = egui_color.b();
            *changed = true;
        }
    });
}
