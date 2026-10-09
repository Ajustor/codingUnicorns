use crate::app::file_ops::is_image_file;
use crate::app::CodingUnicorns;
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
    let (palette, spacing) = crate::ui::theme::apply_theme(ctx, &app.config);
    app.palette = palette;
    app.spacing = spacing;

    // ── Auto-save (2-second inactivity) ──────────────────────────────────────
    if app.editor.content_version != app.last_edit_version_seen {
        app.last_edit_version_seen = app.editor.content_version;
        app.last_edit_instant = Some(std::time::Instant::now());
    }
    if let Some(t) = app.last_edit_instant {
        if t.elapsed().as_secs() >= 2 {
            if app.editor.is_modified {
                app.save_editor_guarded(false);
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
                ui.menu_button("Open Recent", |ui| {
                    // Entries whose folder vanished are dropped when the list is shown.
                    if app.config.prune_recent_workspaces() {
                        app.config.save();
                    }
                    if app.config.recent_workspaces.is_empty() {
                        ui.add_enabled(false, egui::Button::new("No recent folders"));
                    }
                    let mut picked = None;
                    for ws in &app.config.recent_workspaces {
                        if ui.button(ws.as_str()).clicked() {
                            picked = Some(std::path::PathBuf::from(ws));
                        }
                    }
                    ui.separator();
                    let clear = ui
                        .add_enabled(
                            !app.config.recent_workspaces.is_empty(),
                            egui::Button::new("Clear Recent"),
                        )
                        .clicked();
                    if let Some(path) = picked {
                        app.open_recent_workspace(path);
                        ui.close_menu();
                    } else if clear {
                        app.clear_recent_workspaces();
                        ui.close_menu();
                    }
                });
                if ui.button("Open File…  Ctrl+Shift+O").clicked() {
                    app.file_pending = Some(app.trigger_open_file());
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Save              Ctrl+S").clicked() {
                    if app.save_editor_guarded(true) {
                        app.toast("Saved");
                    }
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
                if ui.button("Markdown Preview  Ctrl+Shift+V").clicked() {
                    app.show_md_preview = !app.show_md_preview;
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
                // Markdown preview toggle — shown only when the current file can be
                // previewed. (Replaces the branch label here; the branch is still in
                // the bottom status bar.)
                let can_preview = app
                    .editor
                    .current_path
                    .as_ref()
                    .and_then(|p| p.extension())
                    .and_then(|e| e.to_str())
                    .map(|e| {
                        matches!(
                            e.to_lowercase().as_str(),
                            "md" | "markdown" | "mdown" | "mkd"
                        )
                    })
                    .unwrap_or(false);
                if can_preview {
                    let color = if app.show_md_preview {
                        egui::Color32::from_rgb(120, 170, 255)
                    } else {
                        egui::Color32::from_gray(200)
                    };
                    let btn =
                        egui::Button::new(egui::RichText::new("👁 Preview").small().color(color))
                            .frame(false);
                    if ui
                        .add(btn)
                        .on_hover_text("Toggle Markdown preview (Ctrl+Shift+V)")
                        .clicked()
                    {
                        app.show_md_preview = !app.show_md_preview;
                    }
                }
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
                    .as_deref()
                    .and_then(crate::language::language_key)
                    .unwrap_or_default();
                match app.lsp.get(&ext) {
                    None => LspStatus::Inactive,
                    Some(c) if !c.is_connected => LspStatus::Connecting,
                    Some(c) if c.is_busy() => LspStatus::Loading,
                    Some(_) => LspStatus::Ready,
                }
            };
            let counts = app.problems_panel.counts();
            let problems = (lsp_status != LspStatus::Inactive
                || counts.errors + counts.warnings > 0)
                .then_some((counts.errors, counts.warnings));
            if app.status_bar.show(
                ui,
                &app.editor,
                &app.git_status,
                lsp_status,
                problems,
                app.palette,
            ) {
                app.problems_panel.open = !app.problems_panel.open;
            }
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
                            egui::Stroke::new(2.0_f32, active_bar),
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

                // Drain EVERY terminal's PTY each frame (not just the visible one) so none
                // stalls waiting on an unanswered query like ESC[6n — fixes 2nd+ terminals.
                for term in app.terminals.iter_mut() {
                    term.update();
                }
                // Terminal content
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                if let Some(term) = app.terminals.get_mut(app.active_terminal) {
                    // PTY output wakes the UI by itself (`terminal::set_output_waker`),
                    // so it's drained promptly without redrawing every frame.
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
                            Stroke::new(2.0_f32, accent),
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
                                    app.pending_delete = Some(path);
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
                        app.extensions_panel.show(
                            ui,
                            &mut app.extension_registry,
                            &app.config.extensions.registry_url,
                        );
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
                        let action = app.debugger_panel.show(
                            ui,
                            &mut app.dap,
                            app.workspace_path.as_deref(),
                        );
                        if action.start_or_continue {
                            if app.dap.is_paused() {
                                if let Some(tid) = app.dap.paused_thread_id() {
                                    if let Some(sess) = app.dap.active_mut() {
                                        sess.continue_execution(tid);
                                    }
                                }
                            } else if app.dap.can_start() {
                                app.start_debug_session();
                            }
                        }
                        if action.stop {
                            app.dap.stop_session();
                        }
                        if action.step_over {
                            if let Some(tid) = app.dap.paused_thread_id() {
                                if let Some(sess) = app.dap.active_mut() {
                                    sess.next_step(tid);
                                }
                            }
                        }
                        if action.step_in {
                            if let Some(tid) = app.dap.paused_thread_id() {
                                if let Some(sess) = app.dap.active_mut() {
                                    sess.step_in(tid);
                                }
                            }
                        }
                        if action.step_out {
                            if let Some(tid) = app.dap.paused_thread_id() {
                                if let Some(sess) = app.dap.active_mut() {
                                    sess.step_out(tid);
                                }
                            }
                        }
                        if action.pause {
                            if let Some(sess) = app.dap.active_mut() {
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

    // Problems panel (bottom, Ctrl+Shift+M)
    if app.problems_panel.open {
        let target = TopBottomPanel::bottom("problems_panel")
            .resizable(true)
            .min_height(80.0)
            .default_height(180.0)
            .show(ctx, |ui| {
                app.problems_panel
                    .show(ui, app.workspace_path.as_deref(), app.palette)
            })
            .inner;
        if let Some((path, line, col)) = target {
            app.goto_location(path, line, col);
        }
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

    if app.show_claude {
        egui::SidePanel::right("claude_panel")
            .resizable(true)
            .default_width(360.0)
            .min_width(260.0)
            .show(ctx, |ui| {
                let pending = app.claude_pending.as_ref();
                let account = app.claude_account.as_deref();
                let action = app
                    .claude_panel
                    .show(ui, &app.claude_session, pending, account);
                match action {
                    claude::panel::ClaudeAction::Send(text) => {
                        if text.starts_with('/') {
                            app.handle_claude_slash(text);
                        } else {
                            app.start_claude_turn(text);
                        }
                    }
                    claude::panel::ClaudeAction::NewConversation => {
                        app.claude_session.reset();
                        app.claude_pending = None;
                        app.claude_turn = None;
                    }
                    claude::panel::ClaudeAction::Cancel => {
                        if let Some(turn) = &mut app.claude_turn {
                            turn.cancel();
                        }
                        app.claude_turn = None;
                        app.claude_session.running = false;
                        app.claude_pending = None;
                    }
                    claude::panel::ClaudeAction::Permission(decision) => {
                        if let Some(req) = app.claude_pending.take() {
                            let _ = req.reply.send(decision);
                        }
                    }
                    claude::panel::ClaudeAction::OpenInteractive => {
                        // Launch the real interactive `claude` in a terminal tab so
                        // its built-in commands work, rooted at the workspace.
                        let cwd = app.workspace_path.clone();
                        let term = crate::terminal::Terminal::new_command(
                            &app.config.claude_binary,
                            cwd.as_deref(),
                        );
                        app.terminals.push(term);
                        app.active_terminal = app.terminals.len() - 1;
                        app.show_terminal = true;
                    }
                    claude::panel::ClaudeAction::None => {}
                }
            });
    }

    // Markdown preview: rendered view on the right, source stays in the editor
    // (a source | preview split). Only for the current file when it's Markdown.
    let md_preview = app.show_md_preview
        && app
            .editor
            .current_path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(|e| {
                matches!(
                    e.to_lowercase().as_str(),
                    "md" | "markdown" | "mdown" | "mkd"
                )
            })
            .unwrap_or(false);
    if md_preview {
        SidePanel::right("md_preview")
            .resizable(true)
            .default_width(440.0)
            .min_width(240.0)
            .show(ctx, |ui| {
                let md = app.editor.buffer.to_string();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui_commonmark::CommonMarkViewer::new().show(ui, &mut app.md_cache, &md);
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
                        egui::Stroke::new(1.0_f32, accent_color),
                        egui::StrokeKind::Inside,
                    );
                } else {
                    ui.painter().rect_stroke(
                        right_rect,
                        0.0,
                        egui::Stroke::new(1.0_f32, accent_color),
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
                    // Horizontal breathing room on the settings content.
                    let settings_changed = egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(28, 0))
                        .show(&mut left_ui, |ui| {
                            app.settings_panel.show_inline(ui, &mut app.config)
                        })
                        .inner;
                    if settings_changed {
                        app.config.save();
                        if app.file_tree.show_gitignored != app.config.editor.show_gitignored {
                            app.file_tree.show_gitignored = app.config.editor.show_gitignored;
                            app.file_tree.reload_children();
                        }
                    }
                } else if app.editor.current_path.is_some() || app.editor.buffer.rope_len() > 0 {
                    // Breadcrumbs
                    let breadcrumb_path = app.editor.current_path.clone();
                    let breadcrumb_workspace = app.workspace_path.clone();
                    let breadcrumb_symbols = app.outline_symbols.clone();
                    let breadcrumb_line = app.editor.cursor.position().0 as u32;
                    crate::ui::breadcrumbs::render(
                        &mut left_ui,
                        app.palette,
                        breadcrumb_path.as_deref(),
                        breadcrumb_workspace.as_deref(),
                        &breadcrumb_symbols,
                        breadcrumb_line,
                    );
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
                        app.palette,
                        app.spacing,
                    );
                } else if let Some(ws) = welcome_screen(&mut left_ui, &app.config.recent_workspaces)
                {
                    app.open_recent_workspace(ws);
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
                    if let Some(ref mut e2) = app.editor2 {
                        let bp_lines: std::collections::HashSet<usize> = e2
                            .current_path
                            .as_ref()
                            .map(|p| {
                                app.dap
                                    .breakpoint_lines_for(p)
                                    .iter()
                                    .map(|l| l.saturating_sub(1)) // 1-based → 0-based
                                    .collect()
                            })
                            .unwrap_or_default();
                        e2.show(
                            &mut right_ui,
                            &app.config,
                            &app.plugin_manager,
                            None,
                            &bp_lines,
                            app.palette,
                            app.spacing,
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
                    let settings_changed = egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(28, 0))
                        .show(ui, |ui| app.settings_panel.show_inline(ui, &mut app.config))
                        .inner;
                    if settings_changed {
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
                } else if app.editor.current_path.is_some() || app.editor.buffer.rope_len() > 0 {
                    // ── Breadcrumbs bar ───────────────────────────────────────────
                    let breadcrumb_path = app.editor.current_path.clone();
                    let breadcrumb_workspace = app.workspace_path.clone();
                    let breadcrumb_symbols = app.outline_symbols.clone();
                    let breadcrumb_line = app.editor.cursor.position().0 as u32;
                    crate::ui::breadcrumbs::render(
                        ui,
                        app.palette,
                        breadcrumb_path.as_deref(),
                        breadcrumb_workspace.as_deref(),
                        &breadcrumb_symbols,
                        breadcrumb_line,
                    );

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
                    app.editor.show(
                        ui,
                        &app.config,
                        &app.plugin_manager,
                        lsp_hover,
                        &bp_lines,
                        app.palette,
                        app.spacing,
                    );
                } else if let Some(ws) = welcome_screen(ui, &app.config.recent_workspaces) {
                    app.open_recent_workspace(ws);
                }
            }
        });

    if let Some(path) = app.pending_delete.clone() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let kind = if path.is_dir() { "folder" } else { "file" };
        match crate::ui::widgets::confirm_dialog(
            ctx,
            app.palette,
            "Confirm delete",
            &format!("Delete {kind} \"{name}\"? This cannot be undone."),
        ) {
            Some(true) => {
                if path.is_dir() {
                    let _ = std::fs::remove_dir_all(&path);
                } else {
                    let _ = std::fs::remove_file(&path);
                }
                app.file_tree.reload_children();
                app.pending_delete = None;
            }
            Some(false) => app.pending_delete = None,
            None => {}
        }
    }

    // Toast on explicit Ctrl+S save (handled inside the editor, signalled via a flag).
    {
        let mut saved = false;
        if app.editor.just_saved {
            app.editor.just_saved = false;
            saved = true;
        }
        if let Some(e2) = app.editor2.as_mut() {
            if e2.just_saved {
                e2.just_saved = false;
                saved = true;
            }
        }
        if saved {
            app.toast("Saved");
        }
    }

    if crate::ui::widgets::render_toasts(ctx, app.palette, app.spacing, &mut app.toasts) {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

/// Welcome screen; lists existing recent workspaces as links and returns the
/// one the user clicked, if any.
fn welcome_screen(ui: &mut egui::Ui, recent: &[String]) -> Option<std::path::PathBuf> {
    let mut picked = None;
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
        let existing: Vec<&String> = recent
            .iter()
            .filter(|p| std::path::Path::new(p).is_dir())
            .collect();
        if !existing.is_empty() {
            ui.add_space(24.0);
            ui.label(egui::RichText::new("Recent").color(egui::Color32::GRAY));
            for ws in existing {
                if ui.link(ws.as_str()).clicked() {
                    picked = Some(std::path::PathBuf::from(ws));
                }
            }
        }
    });
    picked
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

#[cfg(test)]
mod tests {
    use egui::{Event, Pos2, Rect};

    /// The sidebar of [`super::render`], drawn headlessly with the app's fonts
    /// and theme.
    ///
    /// egui gives a side panel the width of its content and starts the next
    /// frame from it: a row sized from `available_width()` that overflows the
    /// panel widens it a little more every frame, until it fills the window.
    struct Sidebar {
        ctx: egui::Context,
        config: crate::config::Config,
        texts: Vec<(String, Rect)>,
    }

    impl Sidebar {
        /// Default of `CodingUnicorns::sidebar_width`.
        const WIDTH: f32 = 220.0;

        fn new() -> Self {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::ui::theme::app_fonts());
            Self {
                ctx,
                config: crate::config::Config::default(),
                texts: vec![],
            }
        }

        /// Runs one frame and returns the sidebar's width.
        fn frame(&mut self, events: Vec<Event>, content: &mut dyn FnMut(&mut egui::Ui)) -> f32 {
            let raw = egui::RawInput {
                // Tall enough to keep every Extensions section on screen.
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 2400.0))),
                events,
                ..Default::default()
            };
            let config = &self.config;
            let mut width = 0.0;
            let out = self.ctx.run(raw, |ctx| {
                crate::ui::theme::apply_theme(ctx, config);
                // Same panels as `render`.
                egui::SidePanel::left("activity_bar")
                    .exact_width(48.0)
                    .resizable(false)
                    .show(ctx, |_| {});
                width = egui::SidePanel::left("sidebar")
                    .resizable(true)
                    .min_width(150.0)
                    .default_width(Self::WIDTH)
                    .show(ctx, |ui| content(ui))
                    .response
                    .rect
                    .width();
                egui::CentralPanel::default().show(ctx, |_| {});
            });
            self.texts.clear();
            for clipped in &out.shapes {
                collect_texts(&clipped.shape, &mut self.texts);
            }
            width
        }

        /// Clicks the rendered text `label`, e.g. to open a collapsing section.
        fn click(&mut self, label: &str, content: &mut dyn FnMut(&mut egui::Ui)) {
            self.frame(vec![], content);
            let pos = self
                .texts
                .iter()
                .find(|(t, _)| t == label)
                .map(|(_, r)| r.center())
                .unwrap_or_else(|| panic!("{label:?} not rendered"));
            let button = |pressed| Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(pos), button(true)], content);
            self.frame(vec![button(false)], content);
            self.frame(vec![Event::PointerGone], content);
        }

        /// Shows `content` for a few frames: the sidebar must keep its width.
        fn assert_fits(&mut self, tab: &str, content: &mut dyn FnMut(&mut egui::Ui)) {
            let widths: Vec<f32> = (0..10).map(|_| self.frame(vec![], content)).collect();
            assert!(
                widths.iter().all(|&w| w <= Self::WIDTH),
                "{tab} widens the sidebar: {widths:?}"
            );
        }
    }

    fn collect_texts(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Text(t) => {
                out.push((t.galley.text().to_string(), t.visual_bounding_rect()));
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect_texts(s, out)),
            _ => {}
        }
    }

    #[test]
    fn sidebar_tabs_do_not_widen_the_sidebar() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();

        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "").unwrap();
        let mut tree = crate::filetree::FileTree::new();
        tree.load(root.clone());
        Sidebar::new().assert_fits("Explorer", &mut |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| tree.show(ui));
        });

        let mut search = crate::ui::search::WorkspaceSearch::new();
        search.show_replace = true;
        Sidebar::new().assert_fits("Search", &mut |ui| {
            search.show(ui, Some(&root));
        });

        let mut git_panel = crate::ui::git_panel::GitPanel::new();
        let mut git = crate::git::GitStatus::new();
        git.files.push(crate::git::FileStatus {
            path: "src/main.rs".into(),
            wt_status: crate::git::FileChangeKind::Modified,
            ..Default::default()
        });
        Sidebar::new().assert_fits("Git", &mut |ui| {
            git_panel.show(ui, &mut git);
        });

        let mut run_panel = crate::ui::run_panel::RunPanel::new();
        let mut runner = crate::runner::RunManager::new();
        runner.configs.push(crate::runner::RunConfig {
            name: "cargo run".into(),
            command: "cargo run".into(),
            cwd: "${workspaceRoot}".into(),
            env: vec![],
            args: vec![],
            debug: None,
        });
        Sidebar::new().assert_fits("Run", &mut |ui| {
            run_panel.show(ui, &mut runner, None, None, false);
        });

        let mut debugger = crate::ui::debugger::DebuggerPanel::new();
        let mut dap = crate::dap::manager::DapManager::new();
        dap.add_watch("x");
        Sidebar::new().assert_fits("Debug", &mut |ui| {
            debugger.show(ui, &mut dap, None);
        });

        // Extensions with every section open. The registry URL is left empty
        // so nothing is fetched: its browser is checked on its own below.
        let ext_dir = root.join("extensions");
        std::fs::create_dir_all(ext_dir.join("acme.python")).unwrap();
        std::fs::write(
            ext_dir.join("acme.python/manifest.toml"),
            "[extension]\nid = \"acme.python\"\nname = \"Python\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        let mut registry = crate::extension::registry::ExtensionRegistry::new_in(ext_dir);
        registry.load_installed();
        let mut extensions = crate::extension::ui::ExtensionsPanel::new();
        extensions.install_url = "https://github.com/user/extension".into();
        extensions.workspace_path = "/path/to/modules".into();
        extensions.workspace_log = vec!["⚙ Building workspace…".into()];
        extensions.git_group_url = "https://github.com/user/modules".into();
        extensions.git_group_log = vec!["✓ Done — 1/1 modules installed".into()];
        let mut sidebar = Sidebar::new();
        let mut show = |ui: &mut egui::Ui| extensions.show(ui, &mut registry, "");
        for section in [
            "INSTALLED",
            "INSTALL FROM GIT",
            "📁 Load from local folder",
            "📦 INSTALL FROM ZIP",
            "⚙ BUILD FROM SOURCES",
            "📦 INSTALL GROUP FROM GIT",
            "CREATE EXTENSION",
        ] {
            sidebar.click(section, &mut show);
        }
        sidebar.assert_fits("Extensions", &mut show);

        registry.remote_index = Some(
            crate::extension::remote_registry::parse_index(&format!(
                r#"{{"schema":1,"modules":[{{"id":"acme.rust","dir":"rust","name":"Rust",
                    "version":"1.0.0","description":"Rust support","languages":["rs"],
                    "assets":{{"{}":{{"url":"https://example.invalid/rust.zip"}}}}}}]}}"#,
                crate::extension::remote_registry::platform_key()
            ))
            .unwrap(),
        );
        let mut browser = crate::extension::registry_ui::RegistryBrowser::new();
        Sidebar::new().assert_fits("Extensions registry", &mut |ui| {
            browser.show(ui, &registry, "https://example.invalid/registry.json");
        });
    }
}
