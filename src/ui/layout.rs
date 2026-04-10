use crate::app::file_ops::is_image_file;
use crate::app::CodingUnicorns;
use crate::config::Config;
use crate::terminal::Terminal;
use crate::ui::run_panel::RunPanelAction;
use crate::ui::statusbar::LspStatus;
use egui::{
    Align, CentralPanel, Color32, Context, Layout, RichText, SidePanel, Stroke, TopBottomPanel,
};

#[derive(Debug, Clone, PartialEq, Default)]
pub enum SidebarTab {
    #[default]
    Explorer,
    Search,
    Git,
    Extensions,
    Run,
    Outline,
    Debug,
}

pub fn render(app: &mut CodingUnicorns, ctx: &Context) {
    ctx.set_visuals(dark_visuals(&app.config));

    // ── Auto-save (2-second inactivity) ──────────────────────────────────────
    if app.editor.content_version != app.last_edit_version_seen {
        app.last_edit_version_seen = app.editor.content_version;
        app.last_edit_instant = Some(std::time::Instant::now());
    }
    if let Some(t) = app.last_edit_instant {
        if t.elapsed().as_secs() >= 2 {
            if app.editor.is_modified {
                let _ = app.editor.save();
            }
            app.last_edit_instant = None;
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    // ── File tree refresh on window focus ────────────────────────────────────
    {
        let focused_now = ctx.input(|i| i.focused);
        static LAST_FOCUSED: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        let was_focused = LAST_FOCUSED.swap(focused_now, std::sync::atomic::Ordering::Relaxed);
        if focused_now && !was_focused {
            // Window just gained focus — reload file tree to pick up external changes.
            app.file_tree.reload_children();
        }
    }

    // ── Ctrl+Tab / Ctrl+Shift+Tab — cycle tabs ────────────────────────────────
    if ctx.input(|i| i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::Tab)) {
        app.cycle_tab_next();
    }
    if ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::Tab)) {
        app.cycle_tab_prev();
    }

    // Drain pending folder/file picked by dialog threads
    if let Some(rx) = app.folder_pending.take() {
        match rx.try_recv() {
            Ok(path) => app.open_folder(path),
            Err(std::sync::mpsc::TryRecvError::Empty) => app.folder_pending = Some(rx),
            Err(_) => {}
        }
    }
    if let Some(rx) = app.file_pending.take() {
        match rx.try_recv() {
            Ok(path) => app.open_file(path),
            Err(std::sync::mpsc::TryRecvError::Empty) => app.file_pending = Some(rx),
            Err(_) => {}
        }
    }

    // Sync editor modified state → active tab
    if let Some(active_id) = app.tab_manager.active_tab {
        if let Some(tab) = app.tab_manager.tabs.iter_mut().find(|t| t.id == active_id) {
            tab.is_modified = app.editor.is_modified;
        }
    }

    // Update window title to show modified indicator (● prefix)
    {
        let filename = app
            .editor
            .current_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Coding Unicorns".to_string());
        let title = if app.editor.is_modified {
            format!("● {} — Coding Unicorns", filename)
        } else {
            format!("{} — Coding Unicorns", filename)
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }

    TopBottomPanel::top("menu_bar").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New File          Ctrl+N").clicked() {
                    app.open_new_file();
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Open Folder…     Ctrl+O").clicked() {
                    app.folder_pending = Some(app.trigger_open_folder());
                    ui.close_menu();
                }
                if ui.button("Open File…  Ctrl+Shift+O").clicked() {
                    app.file_pending = Some(app.trigger_open_file());
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Save              Ctrl+S").clicked() {
                    let _ = app.editor.save();
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });

            ui.menu_button("View", |ui| {
                if ui.button("Toggle Sidebar  Ctrl+B").clicked() {
                    app.show_sidebar = !app.show_sidebar;
                    ui.close_menu();
                }
                if ui.button("Toggle Terminal  Ctrl+`").clicked() {
                    app.show_terminal = !app.show_terminal;
                    ui.close_menu();
                }
                if ui.button("Command Palette  Ctrl+P").clicked() {
                    app.command_palette.toggle();
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Keyboard Shortcuts  F1").clicked() {
                    app.shortcuts_help.toggle();
                    ui.close_menu();
                }
                if ui.button("Settings   Ctrl+,").clicked() {
                    app.tab_manager.open_settings();
                    app.settings_panel.open = true;
                    ui.close_menu();
                }
            });

            ui.menu_button("Git", |ui| {
                if ui.button("Refresh Status").clicked() {
                    app.git_status.refresh();
                    ui.close_menu();
                }
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("⎇ {}", app.git_status.branch))
                        .color(egui::Color32::from_rgb(150, 200, 150))
                        .small(),
                );
            });
        });
    });

    TopBottomPanel::bottom("status_bar")
        .exact_height(22.0)
        .show(ctx, |ui| {
            let lsp_status = {
                let ext = app
                    .editor
                    .current_path
                    .as_ref()
                    .and_then(|p| p.extension())
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                match app.lsp.get(ext) {
                    None => LspStatus::Inactive,
                    Some(c) if c.is_connected => LspStatus::Ready,
                    Some(_) => LspStatus::Connecting,
                }
            };
            app.status_bar
                .show(ui, &app.editor, &app.git_status, lsp_status);
        });

    if app.show_terminal {
        let panel_response = TopBottomPanel::bottom("terminal_panel")
            .resizable(true)
            .min_height(80.0)
            .default_height(app.terminal_height)
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(
                        app.config.theme.background[0],
                        app.config.theme.background[1],
                        app.config.theme.background[2],
                    ))
                    .inner_margin(egui::Margin::ZERO),
            )
            .show_separator_line(false)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;

                // Tab bar — one tab per terminal instance
                let tab_bg = egui::Color32::from_rgb(
                    app.config.theme.background[0].saturating_add(7),
                    app.config.theme.background[1].saturating_add(7),
                    app.config.theme.background[2].saturating_add(7),
                );
                let tab_height = 35.0;
                let (tab_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), tab_height),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(tab_rect, 0.0, tab_bg);

                let mut tab_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(tab_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );

                tab_ui.add_space(8.0);

                // Panel icon + label
                tab_ui.label(
                    egui::RichText::new(format!("{} TERMINAL", egui_phosphor::regular::TERMINAL))
                        .size(11.0)
                        .color(egui::Color32::from_gray(150)),
                );

                tab_ui.add_space(6.0);
                tab_ui.separator();
                tab_ui.add_space(4.0);

                // One clickable tab per terminal
                let mut close_idx: Option<usize> = None;
                for i in 0..app.terminals.len() {
                    let is_active = i == app.active_terminal;
                    let shell = app.terminals[i].shell_name.clone();
                    let tab_color = if is_active {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::from_gray(140)
                    };
                    let active_bar = egui::Color32::from_rgb(
                        app.config.theme.accent[0],
                        app.config.theme.accent[1],
                        app.config.theme.accent[2],
                    );

                    // Draw tab background if active
                    let tab_label_response = tab_ui.add(
                        egui::Button::new(
                            egui::RichText::new(format!(
                                "{} {}",
                                egui_phosphor::regular::TERMINAL,
                                shell
                            ))
                            .size(11.0)
                            .color(tab_color),
                        )
                        .frame(false)
                        .selected(is_active),
                    );
                    if tab_label_response.clicked() {
                        app.active_terminal = i;
                    }
                    // Draw active top-border indicator
                    if is_active {
                        let r = tab_label_response.rect;
                        tab_ui.painter().line_segment(
                            [r.left_top(), r.right_top()],
                            egui::Stroke::new(2.0, active_bar),
                        );
                    }

                    // Close button (only show if >1 terminal)
                    if app.terminals.len() > 1
                        && tab_ui
                            .add(
                                egui::Button::new(egui::RichText::new("×").size(12.0)).frame(false),
                            )
                            .clicked()
                    {
                        close_idx = Some(i);
                    }
                    tab_ui.add_space(4.0);
                }

                // Remove closed terminal after the loop
                if let Some(idx) = close_idx {
                    app.terminals.remove(idx);
                    if app.active_terminal >= app.terminals.len() {
                        app.active_terminal = app.terminals.len().saturating_sub(1);
                    }
                }

                // New terminal (+) button
                if tab_ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(egui_phosphor::regular::PLUS).size(14.0),
                        )
                        .frame(false),
                    )
                    .on_hover_text("New terminal")
                    .clicked()
                {
                    app.terminals.push(Terminal::new(&app.config.shell));
                    app.active_terminal = app.terminals.len() - 1;
                }

                // Right-aligned close-panel button
                let available = tab_ui.available_width();
                tab_ui.add_space((available - 30.0).max(0.0));
                if tab_ui
                    .add(egui::Button::new(egui::RichText::new("×").size(16.0)).frame(false))
                    .on_hover_text("Close terminal panel")
                    .clicked()
                {
                    app.show_terminal = false;
                }

                // Terminal content
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                if let Some(term) = app.terminals.get_mut(app.active_terminal) {
                    term.show_content(ui, &app.config);
                }
            });

        // Persist the panel height so it survives hide/show cycles and app restarts.
        let new_height = panel_response.response.rect.height();
        if (new_height - app.terminal_height).abs() > 0.5 {
            app.terminal_height = new_height;
            app.config.terminal_height = new_height;
            app.config.save();
        }
    }

    // Activity bar (always visible, far left)
    {
        let accent = Color32::from_rgb(
            app.config.theme.accent[0],
            app.config.theme.accent[1],
            app.config.theme.accent[2],
        );
        let bar_bg = Color32::from_rgb(
            app.config.theme.background[0].saturating_sub(5),
            app.config.theme.background[1].saturating_sub(5),
            app.config.theme.background[2].saturating_sub(5),
        );
        let hover_bg = Color32::from_rgb(
            app.config.theme.background[0].saturating_add(20),
            app.config.theme.background[1].saturating_add(20),
            app.config.theme.background[2].saturating_add(20),
        );

        SidePanel::left("activity_bar")
            .exact_width(48.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(bar_bg)
                    .inner_margin(egui::Margin::ZERO),
            )
            .show_separator_line(false)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;

                struct ActivityItem {
                    icon: &'static str,
                    tooltip: &'static str,
                    tab: SidebarTab,
                }

                let items = [
                    ActivityItem {
                        icon: egui_phosphor::regular::FILES,
                        tooltip: "Explorer",
                        tab: SidebarTab::Explorer,
                    },
                    ActivityItem {
                        icon: egui_phosphor::regular::MAGNIFYING_GLASS,
                        tooltip: "Search",
                        tab: SidebarTab::Search,
                    },
                    ActivityItem {
                        icon: egui_phosphor::regular::GIT_BRANCH,
                        tooltip: "Git",
                        tab: SidebarTab::Git,
                    },
                    ActivityItem {
                        icon: egui_phosphor::regular::PUZZLE_PIECE,
                        tooltip: "Extensions",
                        tab: SidebarTab::Extensions,
                    },
                    ActivityItem {
                        icon: egui_phosphor::regular::PLAY,
                        tooltip: "Run",
                        tab: SidebarTab::Run,
                    },
                    ActivityItem {
                        icon: egui_phosphor::regular::LIST,
                        tooltip: "Outline",
                        tab: SidebarTab::Outline,
                    },
                    ActivityItem {
                        icon: egui_phosphor::regular::BUG,
                        tooltip: "Debug",
                        tab: SidebarTab::Debug,
                    },
                ];

                for item in &items {
                    let is_active = app.show_sidebar && app.sidebar_tab == item.tab;

                    // Allocate space first, then paint bg, then icon on top
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::click());
                    let response = response.on_hover_text(item.tooltip);

                    let painter = ui.painter();

                    // Hover/active background (drawn first, under the icon)
                    if response.hovered() {
                        painter.rect_filled(rect, 0.0, hover_bg);
                    }

                    // Active left border
                    if is_active {
                        painter.line_segment(
                            [rect.left_top(), rect.left_bottom()],
                            Stroke::new(2.0, accent),
                        );
                    }

                    // Icon drawn on top
                    let icon_color = if is_active {
                        Color32::WHITE
                    } else if response.hovered() {
                        Color32::from_gray(220)
                    } else {
                        Color32::from_gray(160)
                    };
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        item.icon,
                        egui::FontId::proportional(22.0),
                        icon_color,
                    );

                    if response.clicked() {
                        if app.show_sidebar && app.sidebar_tab == item.tab {
                            app.show_sidebar = false;
                        } else {
                            app.show_sidebar = true;
                            app.sidebar_tab = item.tab.clone();
                        }
                    }
                }

                // Bottom-aligned settings gear
                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::click());
                    let response = response.on_hover_text("Settings");
                    let painter = ui.painter();
                    if response.hovered() {
                        painter.rect_filled(rect, 0.0, hover_bg);
                    }
                    let gear_color = if response.hovered() {
                        Color32::from_gray(220)
                    } else {
                        Color32::from_gray(160)
                    };
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        egui_phosphor::regular::GEAR,
                        egui::FontId::proportional(22.0),
                        gear_color,
                    );
                    if response.clicked() {
                        app.tab_manager.open_settings();
                        app.settings_panel.open = true;
                    }
                });
            });
    }

    if app.show_sidebar {
        SidePanel::left("sidebar")
            .resizable(true)
            .min_width(150.0)
            .default_width(app.sidebar_width)
            .show(ctx, |ui| {
                let section_title = match app.sidebar_tab {
                    SidebarTab::Explorer => "EXPLORER",
                    SidebarTab::Search => "SEARCH",
                    SidebarTab::Git => "GIT",
                    SidebarTab::Extensions => "EXTENSIONS",
                    SidebarTab::Run => "RUN",
                    SidebarTab::Outline => "OUTLINE",
                    SidebarTab::Debug => "DEBUG",
                };
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(section_title)
                            .size(11.0)
                            .color(Color32::from_gray(150))
                            .strong(),
                    );
                });
                ui.add_space(2.0);

                match app.sidebar_tab {
                    SidebarTab::Explorer => {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            if let Some(path) = app.file_tree.show(ui) {
                                if app.active_pane == 1 && app.editor2.is_some() {
                                    app.open_file_in_pane2(path);
                                } else {
                                    app.open_file(path);
                                }
                            }
                        });
                        // Handle context menu actions from the file tree
                        if let Some(action) = app.file_tree.context_action.take() {
                            use crate::filetree::FileTreeAction;
                            match action {
                                FileTreeAction::OpenFile(path) => app.open_file(path),
                                FileTreeAction::Delete(path) => {
                                    if path.is_dir() {
                                        let _ = std::fs::remove_dir_all(&path);
                                    } else {
                                        let _ = std::fs::remove_file(&path);
                                    }
                                    app.file_tree.reload_children();
                                }
                                FileTreeAction::Rename(old_path, new_name) => {
                                    if let Some(parent) = old_path.parent() {
                                        let new_path = parent.join(&new_name);
                                        let _ = std::fs::rename(&old_path, &new_path);
                                        app.file_tree.reload_children();
                                    }
                                }
                                FileTreeAction::NewFile(parent) => {
                                    // Create with a temp name then immediately enter inline rename
                                    let new_path = find_free_path(&parent, "untitled", false);
                                    let _ = std::fs::write(&new_path, "");
                                    app.file_tree.reload_children();
                                    let name = new_path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    app.file_tree.rename_state = Some((new_path.clone(), name));
                                    app.open_file(new_path);
                                }
                                FileTreeAction::NewFolder(parent) => {
                                    let new_path = find_free_path(&parent, "new_folder", true);
                                    let _ = std::fs::create_dir(&new_path);
                                    app.file_tree.reload_children();
                                    let name = new_path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    app.file_tree.rename_state = Some((new_path, name));
                                }
                                FileTreeAction::CopyPath(path) => {
                                    ctx.copy_text(path.to_string_lossy().to_string());
                                }
                                FileTreeAction::RevealInExplorer(path) => {
                                    let target = if path.is_dir() {
                                        path.clone()
                                    } else {
                                        path.parent().map(|p| p.to_path_buf()).unwrap_or(path)
                                    };
                                    let _ =
                                        std::process::Command::new("xdg-open").arg(&target).spawn();
                                }
                            }
                        }
                    }
                    SidebarTab::Search => {
                        if let Some((path, line)) =
                            app.workspace_search.show(ui, app.workspace_path.as_ref())
                        {
                            app.push_nav_and_goto(path, line);
                        }
                    }
                    SidebarTab::Git => {
                        let merge_file = app.git_panel.show(ui, &mut app.git_status);
                        if let Some(file_path) = merge_file {
                            if let Some(ws) = &app.workspace_path {
                                let full_path = ws.join(&file_path);
                                app.merge_view = crate::ui::merge_panel::MergeView::open(full_path);
                            }
                        }
                    }
                    SidebarTab::Extensions => {
                        app.extensions_panel.show(ui, &mut app.extension_registry);
                    }
                    SidebarTab::Run => {
                        let is_running = app.runner.is_running;
                        let action: RunPanelAction = app.run_panel.show(
                            ui,
                            &mut app.runner,
                            app.workspace_path.as_ref(),
                            app.editor.current_path.as_ref(),
                            is_running,
                        );
                        if action.run_clicked {
                            app.run_active_config();
                        }
                        if action.stop_clicked {
                            app.runner.is_running = false;
                            if let Some(term) = app.terminals.get_mut(app.active_terminal) {
                                term.send_input("\x03");
                            }
                        }
                    }
                    SidebarTab::Debug => {
                        let action = app.debugger_panel.show(ui, &app.dap);
                        if action.start_or_continue {
                            if app.dap.is_paused() {
                                if let Some(tid) = app.dap.paused_thread_id() {
                                    if let Some(sess) = &mut app.dap.session {
                                        sess.continue_execution(tid);
                                    }
                                }
                            } else if !app.dap.is_active() {
                                app.start_debug_session();
                            }
                        }
                        if action.stop {
                            app.dap.stop_session();
                        }
                        if action.step_over {
                            if let Some(tid) = app.dap.paused_thread_id() {
                                if let Some(sess) = &mut app.dap.session {
                                    sess.next_step(tid);
                                }
                            }
                        }
                        if action.step_in {
                            if let Some(tid) = app.dap.paused_thread_id() {
                                if let Some(sess) = &mut app.dap.session {
                                    sess.step_in(tid);
                                }
                            }
                        }
                        if action.step_out {
                            if let Some(tid) = app.dap.paused_thread_id() {
                                if let Some(sess) = &mut app.dap.session {
                                    sess.step_out(tid);
                                }
                            }
                        }
                        if action.pause {
                            if let Some(sess) = &mut app.dap.session {
                                sess.pause(1);
                            }
                        }
                        if let Some((path, line)) = action.navigate_to {
                            app.push_nav_and_goto(path, line);
                        }
                    }
                    SidebarTab::Outline => {
                        let current_path = app.editor.current_path.clone();
                        let symbols = app.outline_symbols.clone();
                        if symbols.is_empty() {
                            ui.label(
                                egui::RichText::new("No symbols found").color(egui::Color32::GRAY),
                            );
                        } else {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                let mut nav_to: Option<(std::path::PathBuf, usize)> = None;
                                for sym in &symbols {
                                    let icon = match sym.kind.as_str() {
                                        "Function" | "Method" => "ƒ",
                                        "Class" | "Struct" => "◻",
                                        "Enum" => "⊞",
                                        "Variable" | "Constant" => "≡",
                                        "Interface" => "Ι",
                                        _ => "•",
                                    };
                                    let label = format!("{} {} ({})", icon, sym.name, sym.kind);
                                    if ui.selectable_label(false, label).clicked() {
                                        if let Some(ref path) = current_path {
                                            nav_to = Some((path.clone(), sym.line as usize));
                                        }
                                    }
                                }
                                if let Some((path, line)) = nav_to {
                                    app.push_nav_and_goto(path, line);
                                }
                            });
                        }
                    }
                }
            });
    }

    // References panel (bottom)
    if app.show_references {
        TopBottomPanel::bottom("references_panel")
            .resizable(true)
            .min_height(80.0)
            .default_height(150.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("REFERENCES ({})", app.references_result.len()))
                            .size(11.0)
                            .color(Color32::from_gray(150))
                            .strong(),
                    );
                    if ui.button("✕").clicked() {
                        app.show_references = false;
                    }
                });
                ui.separator();
                let refs = app.references_result.clone();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut nav_to: Option<(std::path::PathBuf, usize)> = None;
                    for (path, line, preview) in &refs {
                        let filename = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let label = format!("{}:{} {}", filename, line + 1, preview);
                        if ui.selectable_label(false, label).clicked() {
                            nav_to = Some((path.clone(), *line as usize));
                        }
                    }
                    if let Some((path, line)) = nav_to {
                        app.push_nav_and_goto(path, line);
                    }
                });
            });
    }

    // Use a zero-margin frame so there's no gap/padding around the editor area
    CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(
                    app.config.theme.background[0],
                    app.config.theme.background[1],
                    app.config.theme.background[2],
                ))
                .inner_margin(egui::Margin::ZERO),
        )
        .show(ctx, |ui| {
            // Remove default item spacing to avoid gaps between tab bar and editor
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;

            // ── Merge tool takes over the editor area ───────────────────────
            if let Some(ref mut merge_view) = app.merge_view {
                let action = merge_view.show(ui);
                match action {
                    crate::ui::merge_panel::MergeAction::SaveAndResolve => {
                        let path = merge_view.file_path.clone();
                        let content = merge_view.result_text.clone();
                        let _ = std::fs::write(&path, &content);
                        // Stage the resolved file
                        if let Some(rel_path) = app
                            .workspace_path
                            .as_ref()
                            .and_then(|ws| path.strip_prefix(ws).ok())
                            .map(|p| p.to_string_lossy().to_string())
                        {
                            app.git_status.stage_file(&rel_path);
                        }
                        app.merge_view = None;
                    }
                    crate::ui::merge_panel::MergeAction::Cancel => {
                        app.merge_view = None;
                    }
                    crate::ui::merge_panel::MergeAction::None => {}
                }
                return; // Don't render normal editor when merge tool is active
            }

            let is_split = app.editor2.is_some();

            if is_split {
                // ── Split mode: left pane (full) + right pane (simplified) ──
                let available = ui.available_rect_before_wrap();
                let split_ratio = app.split_ratio;
                let left_width = (available.width() * split_ratio - 1.0).max(80.0);
                let right_width = (available.width() - left_width - 2.0).max(80.0);

                // Left pane rect
                let left_rect = egui::Rect::from_min_size(
                    available.min,
                    egui::vec2(left_width, available.height()),
                );
                // Separator rect (1px)
                let sep_rect = egui::Rect::from_min_size(
                    egui::pos2(available.min.x + left_width, available.min.y),
                    egui::vec2(2.0, available.height()),
                );
                // Right pane rect
                let right_rect = egui::Rect::from_min_size(
                    egui::pos2(available.min.x + left_width + 2.0, available.min.y),
                    egui::vec2(right_width, available.height()),
                );

                // Draw separator
                ui.painter().rect_filled(
                    sep_rect,
                    0.0,
                    egui::Color32::from_rgb(
                        app.config.theme.background[0].saturating_add(30),
                        app.config.theme.background[1].saturating_add(30),
                        app.config.theme.background[2].saturating_add(30),
                    ),
                );

                // Active pane highlight border
                let active_pane = app.active_pane;
                let accent_color = egui::Color32::from_rgb(
                    app.config.theme.accent[0],
                    app.config.theme.accent[1],
                    app.config.theme.accent[2],
                );
                if active_pane == 0 {
                    ui.painter().rect_stroke(
                        left_rect,
                        0.0,
                        egui::Stroke::new(1.0, accent_color),
                        egui::StrokeKind::Inside,
                    );
                } else {
                    ui.painter().rect_stroke(
                        right_rect,
                        0.0,
                        egui::Stroke::new(1.0, accent_color),
                        egui::StrokeKind::Inside,
                    );
                }

                // ── Left pane ──────────────────────────────────────────────
                let mut left_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(left_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                left_ui.spacing_mut().item_spacing = egui::Vec2::ZERO;

                // Click to focus left pane
                let left_sense = left_ui.interact(
                    left_rect,
                    left_ui.id().with("left_focus"),
                    egui::Sense::click(),
                );
                if left_sense.clicked() {
                    app.active_pane = 0;
                }

                if !app.tab_manager.tabs.is_empty() {
                    let (tab_rect, _) = left_ui.allocate_exact_size(
                        egui::vec2(left_ui.available_width(), 32.0),
                        egui::Sense::hover(),
                    );
                    left_ui.painter().rect_filled(
                        tab_rect,
                        0.0,
                        egui::Color32::from_rgb(
                            app.config.theme.background[0].saturating_add(7),
                            app.config.theme.background[1].saturating_add(7),
                            app.config.theme.background[2].saturating_add(7),
                        ),
                    );
                    let mut tab_ui = left_ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(tab_rect)
                            .layout(*left_ui.layout()),
                    );
                    tab_ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
                    if let Some(path) = app.tab_manager.show(&mut tab_ui) {
                        app.active_pane = 0;
                        app.open_file(path);
                    } else if app.tab_manager.tabs.is_empty() && app.editor.current_path.is_some() {
                        app.load_active_tab();
                    }
                    app.settings_panel.open = app.tab_manager.tabs.iter().any(|t| t.is_settings);
                }

                let active_is_settings = app
                    .tab_manager
                    .active_tab
                    .and_then(|id| app.tab_manager.tabs.iter().find(|t| t.id == id))
                    .map(|t| t.is_settings)
                    .unwrap_or(false);

                if active_is_settings {
                    if app
                        .settings_panel
                        .show_inline(&mut left_ui, &mut app.config)
                    {
                        app.config.save();
                        if app.file_tree.show_gitignored != app.config.editor.show_gitignored {
                            app.file_tree.show_gitignored = app.config.editor.show_gitignored;
                            app.file_tree.reload_children();
                        }
                    }
                } else if app.editor.current_path.is_some()
                    || !app.editor.buffer.to_string().is_empty()
                {
                    // Breadcrumbs
                    if let Some(ref path) = app.editor.current_path.clone() {
                        let crumb_height = 22.0;
                        let (crumb_rect, _) = left_ui.allocate_exact_size(
                            egui::vec2(left_ui.available_width(), crumb_height),
                            egui::Sense::hover(),
                        );
                        let crumb_bg = egui::Color32::from_rgb(
                            app.config.theme.background[0].saturating_add(12),
                            app.config.theme.background[1].saturating_add(12),
                            app.config.theme.background[2].saturating_add(12),
                        );
                        left_ui.painter().rect_filled(crumb_rect, 0.0, crumb_bg);
                        let mut crumb_ui = left_ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(crumb_rect)
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        crumb_ui.add_space(8.0);
                        let components: Vec<String> = path
                            .components()
                            .map(|c| c.as_os_str().to_string_lossy().to_string())
                            .filter(|s| !s.is_empty() && s != "/")
                            .collect();
                        let shown: Vec<&str> = components
                            .iter()
                            .rev()
                            .take(3)
                            .rev()
                            .map(|s| s.as_str())
                            .collect();
                        for (i, part) in shown.iter().enumerate() {
                            if i > 0 {
                                crumb_ui.label(
                                    egui::RichText::new(" › ")
                                        .color(egui::Color32::from_gray(90))
                                        .size(11.0),
                                );
                            }
                            crumb_ui.label(
                                egui::RichText::new(*part)
                                    .color(egui::Color32::from_gray(160))
                                    .size(11.0),
                            );
                        }
                        if let Some(ref sym) = app.editor.current_symbol.clone() {
                            crumb_ui.label(
                                egui::RichText::new(" › ")
                                    .color(egui::Color32::from_gray(90))
                                    .size(11.0),
                            );
                            crumb_ui.label(
                                egui::RichText::new(sym.as_str())
                                    .color(egui::Color32::from_rgb(180, 200, 255))
                                    .size(11.0),
                            );
                        }
                    }
                    {
                        let (cur_row, _) = app.editor.cursor.position();
                        app.editor.current_symbol = app
                            .outline_symbols
                            .iter()
                            .rfind(|s| s.line as usize <= cur_row)
                            .map(|s| s.name.clone());
                    }
                    app.editor.workspace_path = app.workspace_path.clone();
                    let lsp_hover = app.lsp_hover_result.take();
                    let bp_lines: std::collections::HashSet<usize> = app
                        .editor
                        .current_path
                        .as_ref()
                        .map(|p| {
                            app.dap
                                .breakpoint_lines_for(p)
                                .iter()
                                .map(|l| l.saturating_sub(1))
                                .collect()
                        })
                        .unwrap_or_default();
                    app.editor.show(
                        &mut left_ui,
                        &app.config,
                        &app.plugin_manager,
                        lsp_hover,
                        &bp_lines,
                    );
                } else {
                    welcome_screen(&mut left_ui);
                }

                // ── Right pane ─────────────────────────────────────────────
                let mut right_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(right_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                right_ui.spacing_mut().item_spacing = egui::Vec2::ZERO;

                // Click to focus right pane
                let right_sense = right_ui.interact(
                    right_rect,
                    right_ui.id().with("right_focus"),
                    egui::Sense::click(),
                );
                if right_sense.clicked() {
                    app.active_pane = 1;
                }

                // Tab bar for right pane
                let mut open_path_pane2: Option<std::path::PathBuf> = None;
                if let Some(ref mut tm2) = app.tab_manager2 {
                    if !tm2.tabs.is_empty() {
                        let (tab_rect2, _) = right_ui.allocate_exact_size(
                            egui::vec2(right_ui.available_width(), 32.0),
                            egui::Sense::hover(),
                        );
                        right_ui.painter().rect_filled(
                            tab_rect2,
                            0.0,
                            egui::Color32::from_rgb(
                                app.config.theme.background[0].saturating_add(7),
                                app.config.theme.background[1].saturating_add(7),
                                app.config.theme.background[2].saturating_add(7),
                            ),
                        );
                        let mut tab_ui2 = right_ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(tab_rect2)
                                .layout(*right_ui.layout()),
                        );
                        tab_ui2.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
                        if let Some(path) = tm2.show(&mut tab_ui2) {
                            app.active_pane = 1;
                            open_path_pane2 = Some(path);
                        }
                    }
                }
                // Load file into right editor if tab was clicked
                if let Some(path) = open_path_pane2 {
                    app.open_file_in_pane2(path);
                }
                // Close split if right pane has no tabs
                let pane2_empty = app
                    .tab_manager2
                    .as_ref()
                    .map(|tm| tm.tabs.is_empty())
                    .unwrap_or(true);
                if pane2_empty {
                    app.editor2 = None;
                    app.tab_manager2 = None;
                    app.active_pane = 0;
                } else {
                    // Render right editor
                    let no_bp: std::collections::HashSet<usize> = std::collections::HashSet::new();
                    if let Some(ref mut e2) = app.editor2 {
                        e2.show(
                            &mut right_ui,
                            &app.config,
                            &app.plugin_manager,
                            None,
                            &no_bp,
                        );
                    }
                }
            } else {
                // ── Single pane (existing behavior) ────────────────────────
                if !app.tab_manager.tabs.is_empty() {
                    // Draw the tab bar background explicitly to fill the full allocated height
                    let (tab_rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 32.0),
                        egui::Sense::hover(),
                    );
                    ui.painter().rect_filled(
                        tab_rect,
                        0.0,
                        egui::Color32::from_rgb(
                            app.config.theme.background[0].saturating_add(7),
                            app.config.theme.background[1].saturating_add(7),
                            app.config.theme.background[2].saturating_add(7),
                        ),
                    );
                    let mut tab_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(tab_rect)
                            .layout(*ui.layout()),
                    );
                    tab_ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
                    if let Some(path) = app.tab_manager.show(&mut tab_ui) {
                        app.open_file(path);
                    } else if app.tab_manager.tabs.is_empty() && app.editor.current_path.is_some() {
                        // Last tab was closed via × — clear the editor so the welcome screen appears.
                        app.load_active_tab();
                    }
                    // Keep settings_panel.open in sync with whether a settings tab exists
                    app.settings_panel.open = app.tab_manager.tabs.iter().any(|t| t.is_settings);
                }

                let active_is_settings = app
                    .tab_manager
                    .active_tab
                    .and_then(|id| app.tab_manager.tabs.iter().find(|t| t.id == id))
                    .map(|t| t.is_settings)
                    .unwrap_or(false);

                if active_is_settings {
                    if app.settings_panel.show_inline(ui, &mut app.config) {
                        app.config.save();
                        if app.file_tree.show_gitignored != app.config.editor.show_gitignored {
                            app.file_tree.show_gitignored = app.config.editor.show_gitignored;
                            app.file_tree.reload_children();
                        }
                    }
                } else if app
                    .editor
                    .current_path
                    .as_ref()
                    .map(|p| is_image_file(p))
                    .unwrap_or(false)
                {
                    // ── Image viewer ─────────────────────────────────────────────
                    // Create the egui texture from raw pixel data on the first frame.
                    if app.image_texture.is_none() {
                        if let Some(ref img_data) = app.pending_image {
                            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                [img_data.width as usize, img_data.height as usize],
                                &img_data.pixels,
                            );
                            let texture = ui.ctx().load_texture(
                                "image_preview",
                                color_image,
                                egui::TextureOptions::LINEAR,
                            );
                            let size = egui::vec2(img_data.width as f32, img_data.height as f32);
                            app.image_texture = Some((texture, size));
                        }
                    }

                    if let Some((ref texture, original_size)) = app.image_texture {
                        let available = ui.available_size();
                        let scale = (available.x / original_size.x)
                            .min(available.y / original_size.y)
                            .min(1.0);
                        let display_size =
                            egui::vec2(original_size.x * scale, original_size.y * scale);
                        ui.vertical_centered(|ui| {
                            ui.add_space(((available.y - display_size.y) / 2.0).max(0.0));
                            ui.image(egui::load::SizedTexture::new(texture.id(), display_size));
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "{}×{}",
                                    original_size.x as u32, original_size.y as u32
                                ))
                                .small()
                                .color(egui::Color32::GRAY),
                            );
                        });
                    } else {
                        // Image failed to load — show a placeholder.
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                egui::RichText::new("Unable to load image")
                                    .color(egui::Color32::GRAY),
                            );
                        });
                    }
                } else if app.editor.current_path.is_some()
                    || !app.editor.buffer.to_string().is_empty()
                {
                    // ── Breadcrumbs bar ───────────────────────────────────────────
                    if let Some(ref path) = app.editor.current_path.clone() {
                        let crumb_height = 22.0;
                        let (crumb_rect, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), crumb_height),
                            egui::Sense::hover(),
                        );
                        let crumb_bg = egui::Color32::from_rgb(
                            app.config.theme.background[0].saturating_add(12),
                            app.config.theme.background[1].saturating_add(12),
                            app.config.theme.background[2].saturating_add(12),
                        );
                        ui.painter().rect_filled(crumb_rect, 0.0, crumb_bg);
                        let mut crumb_ui = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(crumb_rect)
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        crumb_ui.add_space(8.0);
                        // Show up to last 3 path components
                        let components: Vec<String> = path
                            .components()
                            .map(|c| c.as_os_str().to_string_lossy().to_string())
                            .filter(|s| !s.is_empty() && s != "/")
                            .collect();
                        let shown: Vec<&str> = components
                            .iter()
                            .rev()
                            .take(3)
                            .rev()
                            .map(|s| s.as_str())
                            .collect();
                        for (i, part) in shown.iter().enumerate() {
                            if i > 0 {
                                crumb_ui.label(
                                    egui::RichText::new(" › ")
                                        .color(egui::Color32::from_gray(90))
                                        .size(11.0),
                                );
                            }
                            crumb_ui.label(
                                egui::RichText::new(*part)
                                    .color(egui::Color32::from_gray(160))
                                    .size(11.0),
                            );
                        }
                        // Current symbol
                        if let Some(ref sym) = app.editor.current_symbol.clone() {
                            crumb_ui.label(
                                egui::RichText::new(" › ")
                                    .color(egui::Color32::from_gray(90))
                                    .size(11.0),
                            );
                            crumb_ui.label(
                                egui::RichText::new(sym.as_str())
                                    .color(egui::Color32::from_rgb(180, 200, 255))
                                    .size(11.0),
                            );
                        }
                    }

                    // Update current symbol from outline
                    {
                        let (cur_row, _) = app.editor.cursor.position();
                        app.editor.current_symbol = app
                            .outline_symbols
                            .iter()
                            .rfind(|s| s.line as usize <= cur_row)
                            .map(|s| s.name.clone());
                    }

                    app.editor.workspace_path = app.workspace_path.clone();
                    let lsp_hover = app.lsp_hover_result.take();
                    // Compute breakpoint lines for the current file (1-based from DAP, 0-based for gutter).
                    let bp_lines: std::collections::HashSet<usize> = app
                        .editor
                        .current_path
                        .as_ref()
                        .map(|p| {
                            app.dap
                                .breakpoint_lines_for(p)
                                .iter()
                                .map(|l| l.saturating_sub(1)) // convert 1-based → 0-based
                                .collect()
                        })
                        .unwrap_or_default();
                    app.editor
                        .show(ui, &app.config, &app.plugin_manager, lsp_hover, &bp_lines);
                } else {
                    welcome_screen(ui);
                }
            }
        });
}

fn welcome_screen(ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space(80.0);
        ui.label(
            egui::RichText::new("🦄 Coding Unicorns")
                .size(32.0)
                .color(egui::Color32::from_rgb(180, 130, 255))
                .strong(),
        );
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("A lightweight IDE")
                .size(16.0)
                .color(egui::Color32::GRAY),
        );
        ui.add_space(40.0);
        ui.label(egui::RichText::new("Ctrl+P  — Command Palette").color(egui::Color32::GRAY));
        ui.label(egui::RichText::new("Ctrl+B  — Toggle Sidebar").color(egui::Color32::GRAY));
        ui.label(egui::RichText::new("Ctrl+`  — Toggle Terminal").color(egui::Color32::GRAY));
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("File → Open Folder to get started")
                .color(egui::Color32::from_rgb(150, 200, 150)),
        );
    });
}

/// Find a free path like `parent/base`, `parent/base1`, `parent/base2`, …
fn find_free_path(parent: &std::path::Path, base: &str, _is_dir: bool) -> std::path::PathBuf {
    let candidate = parent.join(base);
    if !candidate.exists() {
        return candidate;
    }
    for i in 1..=999 {
        let name = format!("{}{}", base, i);
        let c = parent.join(&name);
        if !c.exists() {
            return c;
        }
    }
    parent.join(base) // fallback
}

fn dark_visuals(config: &Config) -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    let bg = egui::Color32::from_rgb(
        config.theme.background[0],
        config.theme.background[1],
        config.theme.background[2],
    );
    let fg = egui::Color32::from_rgb(
        config.theme.foreground[0],
        config.theme.foreground[1],
        config.theme.foreground[2],
    );
    let accent = egui::Color32::from_rgb(
        config.theme.accent[0],
        config.theme.accent[1],
        config.theme.accent[2],
    );
    v.panel_fill = egui::Color32::from_rgb(
        config.theme.background[0].saturating_add(7),
        config.theme.background[1].saturating_add(7),
        config.theme.background[2].saturating_add(7),
    );
    v.window_fill = bg;
    v.override_text_color = Some(fg);
    v.selection.bg_fill =
        egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 80);
    v.selection.stroke = egui::Stroke::new(1.0, accent);
    v.hyperlink_color = accent;
    v.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(
        config.theme.background[0].saturating_add(15),
        config.theme.background[1].saturating_add(15),
        config.theme.background[2].saturating_add(15),
    );
    v.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(
        config.theme.background[0].saturating_add(30),
        config.theme.background[1].saturating_add(30),
        config.theme.background[2].saturating_add(30),
    );
    v
}
