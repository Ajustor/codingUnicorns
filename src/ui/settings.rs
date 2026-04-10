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

        ui.add_space(10.0);

        // Search bar
        let search_resp = ui.add(
            egui::TextEdit::singleline(&mut self.search_query)
                .hint_text("Search settings...")
                .desired_width(ui.available_width()),
        );
        if !self.search_focused {
            search_resp.request_focus();
            self.search_focused = true;
        }
        ui.add_space(8.0);

        let q = self.search_query.to_lowercase();

        egui::ScrollArea::vertical().show(ui, |ui| {
            // ═══════════════════════════════════════════════════════════════
            // EDITOR
            // ═══════════════════════════════════════════════════════════════
            if setting_matches(&q, &[
                "font", "size", "tab", "indent", "spaces", "tabs",
                "line numbers", "word wrap", "auto-save", "auto-close", "brackets",
                "gitignore", "hidden", "files", "editor", "minimap", "overview",
            ]) {
                section_heading(ui, "Editor");

                if setting_matches(&q, &["font", "size"]) {
                    ui.horizontal(|ui| {
                        ui.label("Font size");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let old = config.font.size;
                            ui.add(egui::Slider::new(&mut config.font.size, 8.0..=32.0).suffix(" px"));
                            if config.font.size != old { changed = true; }
                        });
                    });
                    ui.add_space(4.0);
                }

                if setting_matches(&q, &["tab", "size", "indent"]) {
                    ui.horizontal(|ui| {
                        ui.label("Tab size");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let old = config.editor.tab_size;
                            ui.add(egui::Slider::new(&mut config.editor.tab_size, 1..=8));
                            if config.editor.tab_size != old { changed = true; }
                        });
                    });
                    ui.add_space(4.0);
                }

                if setting_matches(&q, &["insert", "spaces", "tabs"]) {
                    changed |= checkbox(ui, &mut config.editor.insert_spaces, "Insert spaces instead of tabs");
                }

                if setting_matches(&q, &["line", "numbers"]) {
                    changed |= checkbox(ui, &mut config.editor.line_numbers, "Line numbers");
                }

                if setting_matches(&q, &["word", "wrap"]) {
                    changed |= checkbox(ui, &mut config.editor.word_wrap, "Word wrap");
                }

                if setting_matches(&q, &["auto", "save", "focus"]) {
                    changed |= checkbox(ui, &mut config.editor.auto_save, "Auto-save on focus loss");
                }

                if setting_matches(&q, &["auto", "close", "brackets", "pairs"]) {
                    changed |= checkbox(ui, &mut config.editor.auto_close_brackets, "Auto-close brackets and quotes");
                }

                if setting_matches(&q, &["minimap", "overview", "visualizer", "map"]) {
                    changed |= checkbox(ui, &mut config.editor.show_minimap, "Minimap");
                }

                if setting_matches(&q, &["gitignore", "ignored", "hidden", "files", "tree"]) {
                    changed |= checkbox(ui, &mut config.editor.show_gitignored, "Show gitignored files");
                    hint(ui, "Display files excluded by .gitignore in the file tree.");
                }

                ui.add_space(16.0);
            }

            // ═══════════════════════════════════════════════════════════════
            // TERMINAL
            // ═══════════════════════════════════════════════════════════════
            if setting_matches(&q, &["terminal", "shell", "pwsh", "powershell", "cmd", "bash", "zsh"]) {
                section_heading(ui, "Terminal");

                if setting_matches(&q, &["shell", "terminal", "pwsh", "powershell", "cmd", "bash", "zsh"]) {
                    let available = crate::terminal::list_available_shells();
                    ui.label("Shell");
                    ui.add_space(2.0);

                    // "Auto-detect" button
                    let is_auto = config.shell.is_empty();
                    let auto_label = if is_auto { "Auto-detect (current)" } else { "Auto-detect" };
                    if ui.selectable_label(is_auto, auto_label).clicked() && !is_auto {
                        config.shell.clear();
                        changed = true;
                    }

                    // One button per detected shell
                    for (name, path) in &available {
                        let is_selected = config.shell == *path;
                        let label = if is_selected {
                            format!("{name} (current)")
                        } else {
                            name.clone()
                        };
                        if ui.selectable_label(is_selected, label).clicked() && !is_selected {
                            config.shell = path.clone();
                            changed = true;
                        }
                        if ui.ctx().input(|i| i.pointer.hover_pos().is_some()) {
                            // Show full path on hover via the previous response
                        }
                    }

                    hint(ui, "Select a shell or choose Auto-detect. Restart the terminal to apply.");
                }

                ui.add_space(16.0);
            }

            // ═══════════════════════════════════════════════════════════════
            // THEME
            // ═══════════════════════════════════════════════════════════════
            if setting_matches(&q, &[
                "theme", "color", "background", "foreground", "accent",
                "dark", "monokai", "solarized", "preset",
            ]) {
                section_heading(ui, "Theme");

                if setting_matches(&q, &["theme", "preset", "dark", "monokai", "solarized"]) {
                    const PRESETS: &[ThemePreset] = &[
                        ThemePreset { name: "dark",           label: "Dark",      bg: [30,30,30],   fg: [212,212,212], accent: [0,122,204] },
                        ThemePreset { name: "monokai",        label: "Monokai",   bg: [39,40,34],   fg: [248,248,242], accent: [166,226,46] },
                        ThemePreset { name: "solarized-dark", label: "Solarized", bg: [0,43,54],    fg: [131,148,150], accent: [38,139,210] },
                        ThemePreset { name: "one-dark",       label: "One Dark",  bg: [40,44,52],   fg: [171,178,191], accent: [97,175,239] },
                    ];
                    ui.horizontal(|ui| {
                        for preset in PRESETS {
                            theme_button(ui, preset, config, &mut changed);
                        }
                    });
                    ui.add_space(8.0);
                }

                if setting_matches(&q, &["color", "background", "foreground", "accent", "custom"]) {
                    color_picker_row(ui, "Background", &mut config.theme.background, &mut changed);
                    color_picker_row(ui, "Foreground", &mut config.theme.foreground, &mut changed);
                    color_picker_row(ui, "Accent", &mut config.theme.accent, &mut changed);

                    // Preview
                    ui.add_space(6.0);
                    let size = egui::vec2(ui.available_width(), 36.0);
                    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                    let bg = egui::Color32::from_rgb(
                        config.theme.background[0], config.theme.background[1], config.theme.background[2]);
                    let fg = egui::Color32::from_rgb(
                        config.theme.foreground[0], config.theme.foreground[1], config.theme.foreground[2]);
                    let ac = egui::Color32::from_rgb(
                        config.theme.accent[0], config.theme.accent[1], config.theme.accent[2]);
                    ui.painter().rect_filled(rect, 4.0, bg);
                    ui.painter().text(
                        rect.center(), egui::Align2::CENTER_CENTER,
                        "fn main() { println!(\"hello\"); }",
                        egui::FontId::monospace(12.0), fg,
                    );
                    let bar = egui::Rect::from_min_size(
                        egui::pos2(rect.min.x, rect.max.y - 3.0), egui::vec2(rect.width(), 3.0));
                    ui.painter().rect_filled(bar, 0.0, ac);
                }

                ui.add_space(16.0);
            }

            // ═══════════════════════════════════════════════════════════════
            // KEYBINDINGS
            // ═══════════════════════════════════════════════════════════════
            if setting_matches(&q, &[
                "keybinding", "shortcut", "key", "bind",
                "new file", "open", "save", "close", "palette", "sidebar", "terminal",
                "find", "replace", "undo", "redo", "select", "indent",
                "comment", "delete", "duplicate", "cursor",
                "definition", "navigate", "references", "rename", "format", "blame",
                "debug", "breakpoint", "step",
            ]) {
                section_heading(ui, "Keybindings");

                let rebinding = &mut self.rebinding;
                let kb = &mut config.keybindings;

                keybinding_group(ui, "General", &mut [
                    ("New File",         &mut kb.new_file),
                    ("Open Folder",      &mut kb.open_folder),
                    ("Open File",        &mut kb.open_file),
                    ("Save",             &mut kb.save),
                    ("Close Tab",        &mut kb.close_tab),
                    ("Command Palette",  &mut kb.command_palette),
                    ("Settings",         &mut kb.settings),
                    ("Shortcuts Help",   &mut kb.shortcuts_help),
                    ("Toggle Sidebar",   &mut kb.toggle_sidebar),
                    ("Toggle Terminal",  &mut kb.toggle_terminal),
                    ("Toggle Split",     &mut kb.toggle_split),
                ], rebinding, &mut changed);

                keybinding_group(ui, "Editor", &mut [
                    ("Find",             &mut kb.find),
                    ("Find & Replace",   &mut kb.find_replace),
                    ("Go to Line",       &mut kb.go_to_line),
                    ("Undo",             &mut kb.undo),
                    ("Redo",             &mut kb.redo),
                    ("Select All",       &mut kb.select_all),
                    ("Indent",           &mut kb.indent),
                    ("Unindent",         &mut kb.unindent),
                    ("Toggle Comment",   &mut kb.toggle_comment),
                    ("Delete Line",      &mut kb.delete_line),
                    ("Duplicate Line",   &mut kb.duplicate_line),
                    ("Line Below",       &mut kb.insert_line_below),
                    ("Line Above",       &mut kb.insert_line_above),
                    ("Move Line Up",     &mut kb.move_line_up),
                    ("Move Line Down",   &mut kb.move_line_down),
                ], rebinding, &mut changed);

                keybinding_group(ui, "Multi-cursor", &mut [
                    ("Next Occurrence",  &mut kb.select_next_occurrence),
                    ("All Occurrences",  &mut kb.select_all_occurrences),
                    ("Cursor Above",     &mut kb.add_cursor_above),
                    ("Cursor Below",     &mut kb.add_cursor_below),
                ], rebinding, &mut changed);

                keybinding_group(ui, "Navigation", &mut [
                    ("Go to Definition", &mut kb.goto_definition),
                    ("Navigate Back",    &mut kb.navigate_back),
                    ("Navigate Forward", &mut kb.navigate_forward),
                ], rebinding, &mut changed);

                keybinding_group(ui, "Code", &mut [
                    ("Completion",       &mut kb.trigger_completion),
                    ("Find References",  &mut kb.find_references),
                    ("Rename Symbol",    &mut kb.rename_symbol),
                    ("Code Actions",     &mut kb.code_actions),
                    ("Format Document",  &mut kb.format_document),
                    ("Toggle Blame",     &mut kb.toggle_blame),
                ], rebinding, &mut changed);

                keybinding_group(ui, "Debug", &mut [
                    ("Start / Continue", &mut kb.debug_start),
                    ("Breakpoint",       &mut kb.debug_toggle_breakpoint),
                    ("Step Over",        &mut kb.debug_step_over),
                    ("Step Into",        &mut kb.debug_step_into),
                    ("Step Out",         &mut kb.debug_step_out),
                ], rebinding, &mut changed);

                ui.add_space(4.0);
                if ui
                    .add(egui::Button::new(
                        egui::RichText::new("Reset keybindings to defaults").size(11.0),
                    ))
                    .clicked()
                {
                    config.keybindings = crate::config::KeyBindings::default();
                    changed = true;
                }

                ui.add_space(16.0);
            }
        });

        changed
    }

    /// No-op — settings are now shown inline via the tab system.
    pub fn show(&mut self, _ctx: &egui::Context, _config: &mut crate::config::Config) -> bool {
        false
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn section_heading(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title).strong().size(15.0));
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(6.0);
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(11.0).color(egui::Color32::GRAY));
    ui.add_space(2.0);
}

/// Checkbox that returns true if the value changed.
fn checkbox(ui: &mut egui::Ui, value: &mut bool, label: &str) -> bool {
    let old = *value;
    ui.checkbox(value, label);
    ui.add_space(2.0);
    *value != old
}

struct ThemePreset {
    name: &'static str,
    label: &'static str,
    bg: [u8; 3],
    fg: [u8; 3],
    accent: [u8; 3],
}

fn theme_button(
    ui: &mut egui::Ui,
    preset: &ThemePreset,
    config: &mut crate::config::Config,
    changed: &mut bool,
) {
    let is_active = config.theme.name == preset.name;
    let btn = egui::Button::new(
        egui::RichText::new(preset.label).size(11.0).color(if is_active {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_rgb(180, 180, 180)
        }),
    )
    .fill(if is_active {
        egui::Color32::from_rgb(preset.accent[0], preset.accent[1], preset.accent[2])
    } else {
        egui::Color32::from_rgb(
            preset.bg[0].saturating_add(15),
            preset.bg[1].saturating_add(15),
            preset.bg[2].saturating_add(15),
        )
    });
    if ui.add(btn).clicked() {
        config.theme = crate::config::Theme {
            name: preset.name.into(),
            background: preset.bg,
            foreground: preset.fg,
            accent: preset.accent,
        };
        *changed = true;
    }
}

fn color_picker_row(ui: &mut egui::Ui, label: &str, color: &mut [u8; 3], changed: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut c = egui::Color32::from_rgb(color[0], color[1], color[2]);
            if ui.color_edit_button_srgba(&mut c).changed() {
                color[0] = c.r();
                color[1] = c.g();
                color[2] = c.b();
                *changed = true;
            }
        });
    });
    ui.add_space(2.0);
}

fn keybinding_group(
    ui: &mut egui::Ui,
    title: &str,
    bindings: &mut [(&str, &mut crate::config::KeyBinding)],
    rebinding: &mut Option<String>,
    changed: &mut bool,
) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(title).strong().size(12.0).color(egui::Color32::from_rgb(180, 180, 180)));
    ui.add_space(2.0);
    for (label, binding) in bindings.iter_mut() {
        keybinding_row(ui, label, binding, rebinding, changed);
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
        ui.label(egui::RichText::new(label).size(12.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let is_rebinding = rebinding.as_deref() == Some(label);
            if is_rebinding {
                ui.label(
                    egui::RichText::new("Press a key...")
                        .color(egui::Color32::from_rgb(255, 200, 0))
                        .monospace()
                        .size(11.0),
                );
                ui.input(|i| {
                    for event in &i.events {
                        if let egui::Event::Key { key, modifiers, pressed: true, .. } = event {
                            binding.key = format!("{key:?}");
                            binding.ctrl = modifiers.ctrl;
                            binding.shift = modifiers.shift;
                            binding.alt = modifiers.alt;
                            *rebinding = None;
                            *changed = true;
                        }
                    }
                });
                if ui.small_button("Cancel").clicked() {
                    *rebinding = None;
                }
            } else {
                let btn = egui::Button::new(
                    egui::RichText::new(binding.display()).monospace().size(10.0),
                )
                .min_size(egui::vec2(80.0, 0.0));
                if ui.add(btn).on_hover_text("Click to rebind").clicked() {
                    *rebinding = Some(label.to_string());
                }
            }
        });
    });
}

/// Returns true if any keyword matches the search query (empty query matches all).
fn setting_matches(query: &str, keywords: &[&str]) -> bool {
    if query.is_empty() {
        return true;
    }
    keywords.iter().any(|k| k.contains(query))
}
