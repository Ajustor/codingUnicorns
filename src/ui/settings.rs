pub struct SettingsPanel {
    pub open: bool,
    rebinding: Option<String>,
    search_query: String,
    search_focused: bool,
}

impl SettingsPanel {
    pub fn new() -> Self {
        Self {
            open: false,
            rebinding: None,
            search_query: String::new(),
            search_focused: false,
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if !self.open {
            self.search_focused = false;
        }
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

        // Search bar
        ui.add_space(4.0);
        let search_resp = ui.add(
            egui::TextEdit::singleline(&mut self.search_query)
                .hint_text("Search settings...")
                .desired_width(ui.available_width()),
        );
        // Auto-focus search field once when settings tab is first opened.
        if !self.search_focused {
            search_resp.request_focus();
            self.search_focused = true;
        }
        ui.separator();
        ui.add_space(4.0);

        let q = self.search_query.to_lowercase();

        egui::ScrollArea::vertical().show(ui, |ui| {
            // === EDITOR section ===
            if setting_matches(&q, &[
                "font size", "tab size", "insert spaces", "tabs",
                "line numbers", "word wrap", "auto-save", "auto-close brackets",
                "show gitignored", "files", "editor",
            ]) {
                ui.heading("Editor");
                ui.separator();

                if setting_matches(&q, &["font", "size"]) {
                    ui.horizontal(|ui| {
                        ui.label("Font size:");
                        let old = config.font.size;
                        ui.add(egui::Slider::new(&mut config.font.size, 8.0..=32.0).suffix("px"));
                        if config.font.size != old {
                            changed = true;
                        }
                    });
                }

                if setting_matches(&q, &["tab", "size", "indent"]) {
                    ui.horizontal(|ui| {
                        ui.label("Tab size:");
                        let old = config.editor.tab_size;
                        ui.add(egui::Slider::new(&mut config.editor.tab_size, 1..=8));
                        if config.editor.tab_size != old {
                            changed = true;
                        }
                    });
                }

                if setting_matches(&q, &["insert", "spaces", "tabs"]) {
                    let old = config.editor.insert_spaces;
                    ui.checkbox(&mut config.editor.insert_spaces, "Insert spaces (not tabs)");
                    if config.editor.insert_spaces != old {
                        changed = true;
                    }
                }

                if setting_matches(&q, &["line", "numbers"]) {
                    let old = config.editor.line_numbers;
                    ui.checkbox(&mut config.editor.line_numbers, "Show line numbers");
                    if config.editor.line_numbers != old {
                        changed = true;
                    }
                }

                if setting_matches(&q, &["word", "wrap"]) {
                    let old = config.editor.word_wrap;
                    ui.checkbox(&mut config.editor.word_wrap, "Word wrap");
                    if config.editor.word_wrap != old {
                        changed = true;
                    }
                }

                if setting_matches(&q, &["auto", "save", "focus"]) {
                    let old = config.editor.auto_save;
                    ui.checkbox(&mut config.editor.auto_save, "Auto-save on focus loss");
                    if config.editor.auto_save != old {
                        changed = true;
                    }
                }

                if setting_matches(&q, &["auto", "close", "brackets", "pairs"]) {
                    let old = config.editor.auto_close_brackets;
                    ui.checkbox(
                        &mut config.editor.auto_close_brackets,
                        "Auto-close brackets",
                    );
                    if config.editor.auto_close_brackets != old {
                        changed = true;
                    }
                }

                if setting_matches(&q, &["gitignore", "ignored", "hidden", "files", "tree"]) {
                    let old = config.editor.show_gitignored;
                    ui.checkbox(
                        &mut config.editor.show_gitignored,
                        "Show gitignored files in file tree",
                    );
                    if config.editor.show_gitignored != old {
                        changed = true;
                    }
                }

                ui.add_space(12.0);
            }

            // === THEME section ===
            if setting_matches(&q, &[
                "theme", "color", "background", "foreground", "accent",
                "dark", "monokai", "solarized", "preset",
            ]) {
                ui.heading("Theme");
                ui.separator();

                if setting_matches(&q, &["theme", "preset", "dark", "monokai", "solarized"]) {
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
                }

                if setting_matches(&q, &["color", "background", "foreground", "accent", "custom"]) {
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
                    let (preview_rect, _) =
                        ui.allocate_exact_size(preview_rect_size, egui::Sense::hover());
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
                }

                ui.add_space(12.0);
            }

            // === KEYBINDINGS section ===
            if setting_matches(&q, &[
                "keybinding", "shortcut", "key", "bind",
                "new file", "open", "save", "close", "palette", "sidebar", "terminal",
                "find", "replace", "undo", "redo", "select", "indent",
                "comment", "delete", "duplicate", "cursor",
                "definition", "navigate", "references", "rename", "format", "blame",
                "debug", "breakpoint", "step",
            ]) {
            ui.collapsing("⌨ Keybindings", |ui| {
                let rebinding = &mut self.rebinding;
                let kb = &mut config.keybindings;

                // ── General ──
                ui.label(egui::RichText::new("General").strong().size(12.0));
                keybinding_row(ui, "New File", &mut kb.new_file, rebinding, &mut changed);
                keybinding_row(
                    ui,
                    "Open Folder",
                    &mut kb.open_folder,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(ui, "Open File", &mut kb.open_file, rebinding, &mut changed);
                keybinding_row(ui, "Save", &mut kb.save, rebinding, &mut changed);
                keybinding_row(ui, "Close Tab", &mut kb.close_tab, rebinding, &mut changed);
                keybinding_row(
                    ui,
                    "Command Palette",
                    &mut kb.command_palette,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(ui, "Settings", &mut kb.settings, rebinding, &mut changed);
                keybinding_row(
                    ui,
                    "Shortcuts Help",
                    &mut kb.shortcuts_help,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Toggle Sidebar",
                    &mut kb.toggle_sidebar,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Toggle Terminal",
                    &mut kb.toggle_terminal,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Toggle Split",
                    &mut kb.toggle_split,
                    rebinding,
                    &mut changed,
                );

                ui.add_space(8.0);

                // ── Editor ──
                ui.label(egui::RichText::new("Editor").strong().size(12.0));
                keybinding_row(ui, "Find in File", &mut kb.find, rebinding, &mut changed);
                keybinding_row(
                    ui,
                    "Find & Replace",
                    &mut kb.find_replace,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Go to Line",
                    &mut kb.go_to_line,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(ui, "Undo", &mut kb.undo, rebinding, &mut changed);
                keybinding_row(ui, "Redo", &mut kb.redo, rebinding, &mut changed);
                keybinding_row(
                    ui,
                    "Select All",
                    &mut kb.select_all,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(ui, "Indent", &mut kb.indent, rebinding, &mut changed);
                keybinding_row(ui, "Unindent", &mut kb.unindent, rebinding, &mut changed);
                keybinding_row(
                    ui,
                    "Toggle Comment",
                    &mut kb.toggle_comment,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Delete Line",
                    &mut kb.delete_line,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Duplicate Line",
                    &mut kb.duplicate_line,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Insert Line Below",
                    &mut kb.insert_line_below,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Insert Line Above",
                    &mut kb.insert_line_above,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Move Line Up",
                    &mut kb.move_line_up,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Move Line Down",
                    &mut kb.move_line_down,
                    rebinding,
                    &mut changed,
                );

                ui.add_space(8.0);

                // ── Multi-cursor ──
                ui.label(egui::RichText::new("Multi-cursor").strong().size(12.0));
                keybinding_row(
                    ui,
                    "Select Next Occurrence",
                    &mut kb.select_next_occurrence,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Select All Occurrences",
                    &mut kb.select_all_occurrences,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Add Cursor Above",
                    &mut kb.add_cursor_above,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Add Cursor Below",
                    &mut kb.add_cursor_below,
                    rebinding,
                    &mut changed,
                );

                ui.add_space(8.0);

                // ── Navigation ──
                ui.label(egui::RichText::new("Navigation").strong().size(12.0));
                keybinding_row(
                    ui,
                    "Go to Definition",
                    &mut kb.goto_definition,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Navigate Back",
                    &mut kb.navigate_back,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Navigate Forward",
                    &mut kb.navigate_forward,
                    rebinding,
                    &mut changed,
                );

                ui.add_space(8.0);

                // ── Code Actions ──
                ui.label(egui::RichText::new("Code Actions").strong().size(12.0));
                keybinding_row(
                    ui,
                    "Trigger Completion",
                    &mut kb.trigger_completion,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Find References",
                    &mut kb.find_references,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Rename Symbol",
                    &mut kb.rename_symbol,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Code Actions",
                    &mut kb.code_actions,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Format Document",
                    &mut kb.format_document,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Toggle Blame",
                    &mut kb.toggle_blame,
                    rebinding,
                    &mut changed,
                );

                ui.add_space(8.0);

                // ── Debug ──
                ui.label(egui::RichText::new("Debug").strong().size(12.0));
                keybinding_row(
                    ui,
                    "Start / Continue",
                    &mut kb.debug_start,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Toggle Breakpoint",
                    &mut kb.debug_toggle_breakpoint,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Step Over",
                    &mut kb.debug_step_over,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Step Into",
                    &mut kb.debug_step_into,
                    rebinding,
                    &mut changed,
                );
                keybinding_row(
                    ui,
                    "Step Out",
                    &mut kb.debug_step_out,
                    rebinding,
                    &mut changed,
                );

                ui.add_space(8.0);
                if ui.button("Reset to defaults").clicked() {
                    config.keybindings = crate::config::KeyBindings::default();
                    changed = true;
                }
            });
            } // end keybindings setting_matches

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

/// Returns true if any keyword matches the search query (empty query matches all).
fn setting_matches(query: &str, keywords: &[&str]) -> bool {
    if query.is_empty() {
        return true;
    }
    keywords.iter().any(|k| k.contains(query))
}
