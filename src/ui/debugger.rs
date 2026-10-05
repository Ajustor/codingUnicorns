use std::path::PathBuf;

use egui::{Color32, RichText, ScrollArea};

use crate::dap::manager::DapManager;
use crate::dap::types::DebugSessionState;

pub struct DebuggerPanel {
    pub open: bool,
}

impl DebuggerPanel {
    pub fn new() -> Self {
        Self { open: false }
    }
}

/// Action requested by the debugger panel UI.
#[derive(Default)]
pub struct DebugPanelAction {
    /// User clicked "Start" or "Continue".
    pub start_or_continue: bool,
    /// User clicked "Stop".
    pub stop: bool,
    /// User clicked "Step Over".
    pub step_over: bool,
    /// User clicked "Step In".
    pub step_in: bool,
    /// User clicked "Step Out".
    pub step_out: bool,
    /// User clicked "Pause".
    pub pause: bool,
    /// Navigate to this file/line (e.g. from call stack click).
    pub navigate_to: Option<(PathBuf, usize)>,
}

impl DebuggerPanel {
    pub fn show(&mut self, ui: &mut egui::Ui, dap: &mut DapManager) -> DebugPanelAction {
        let mut action = DebugPanelAction::default();
        let state = dap.session_state();
        let is_active = dap.is_active();
        let is_paused = dap.is_paused();
        let is_running = dap.is_running();

        // ── Toolbar ──────────────────────────────────────────────────────────
        ui.horizontal(|ui| {
            let start_label = if is_paused {
                "▶ Continue (F5)"
            } else {
                "▶ Start (F5)"
            };
            let start_enabled = !is_active || is_paused;
            if ui
                .add_enabled(start_enabled, egui::Button::new(start_label))
                .clicked()
            {
                action.start_or_continue = true;
            }
            if ui
                .add_enabled(is_active, egui::Button::new("⏸ Pause"))
                .clicked()
            {
                action.pause = true;
            }
            if ui
                .add_enabled(is_paused, egui::Button::new("⤵ Over (F10)"))
                .clicked()
            {
                action.step_over = true;
            }
            if ui
                .add_enabled(is_paused, egui::Button::new("↓ In (F11)"))
                .clicked()
            {
                action.step_in = true;
            }
            if ui
                .add_enabled(is_paused, egui::Button::new("↑ Out (⇧F11)"))
                .clicked()
            {
                action.step_out = true;
            }
            if ui
                .add_enabled(is_active, egui::Button::new("■ Stop"))
                .clicked()
            {
                action.stop = true;
            }
        });

        ui.separator();

        // ── Status ────────────────────────────────────────────────────────────
        let status_text = match &state {
            DebugSessionState::Idle => "Idle — press F5 to start",
            DebugSessionState::Launching => "Launching…",
            DebugSessionState::Running => "Running",
            DebugSessionState::Paused { .. } => "Paused",
            DebugSessionState::Terminated => "Terminated",
        };
        let status_color = match &state {
            DebugSessionState::Running => Color32::from_rgb(80, 200, 80),
            DebugSessionState::Paused { .. } => Color32::from_rgb(255, 200, 50),
            DebugSessionState::Terminated => Color32::from_rgb(200, 80, 80),
            _ => Color32::GRAY,
        };
        ui.label(RichText::new(status_text).color(status_color).size(11.0));
        ui.separator();

        // ── Call Stack ────────────────────────────────────────────────────────
        let frames = dap.call_stack();
        let selected = dap.selected_frame();
        let mut select_frame = None;
        if !frames.is_empty() {
            section_label(ui, "CALL STACK");
            ScrollArea::vertical()
                .id_salt("dap_call_stack")
                .max_height(120.0)
                .show(ui, |ui| {
                    for (i, frame) in frames.iter().enumerate() {
                        let label = format!(
                            "{}  {}:{}",
                            frame.name,
                            frame
                                .file
                                .as_ref()
                                .and_then(|f| f.file_name())
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default(),
                            frame.line
                        );
                        let is_selected = i == selected;
                        let response = ui.selectable_label(
                            is_selected,
                            RichText::new(&label).size(11.0).color(if is_selected {
                                Color32::WHITE
                            } else {
                                Color32::from_gray(180)
                            }),
                        );
                        if response.clicked() {
                            if i != selected {
                                select_frame = Some(i);
                            }
                            if let Some(ref file) = frame.file {
                                action.navigate_to =
                                    Some((file.clone(), frame.line.saturating_sub(1)));
                            }
                        }
                    }
                });
            ui.separator();
        }
        if let Some(i) = select_frame {
            dap.select_frame(i);
        }

        // ── Variables ────────────────────────────────────────────────────────
        // Rendering only borrows `dap`; expansions that need data are collected
        // and requested afterwards.
        let mut to_fetch: Vec<i64> = Vec::new();
        let scopes = dap.scopes();
        if !scopes.is_empty() {
            section_label(ui, "VARIABLES");
            ScrollArea::vertical()
                .id_salt("dap_variables")
                .max_height(220.0)
                .show(ui, |ui| {
                    for scope in scopes {
                        let id = ui.id().with(("dap_scope", &scope.name));
                        egui::CollapsingHeader::new(RichText::new(&scope.name).size(11.0).strong())
                            .id_salt(id)
                            .default_open(!scope.expensive)
                            .show(ui, |ui| {
                                show_children(
                                    ui,
                                    dap,
                                    scope.variables_reference,
                                    id,
                                    0,
                                    &mut to_fetch,
                                );
                            });
                    }
                });
            ui.separator();
        }
        for r in to_fetch {
            dap.request_variables(r);
        }

        // ── Output log ────────────────────────────────────────────────────────
        let log = dap.output_log();
        if !log.is_empty() || is_active {
            ui.label(
                RichText::new("OUTPUT")
                    .size(10.0)
                    .color(Color32::from_gray(130))
                    .strong(),
            );
            ScrollArea::vertical()
                .id_salt("dap_output")
                .stick_to_bottom(true)
                .max_height(150.0)
                .show(ui, |ui| {
                    for line in log.iter().rev().take(200).collect::<Vec<_>>().iter().rev() {
                        ui.label(RichText::new(line.as_str()).size(11.0).monospace());
                    }
                });
        }

        if !is_active && !is_running {
            ui.add_space(8.0);
            ui.label(
                RichText::new("Set breakpoints by clicking in the gutter (F9),\nthen press F5 to start debugging.")
                    .color(Color32::GRAY)
                    .size(11.0),
            );
        }

        action
    }
}

/// Nesting limit for structured variables (guards against cyclic object graphs).
const MAX_VARIABLE_DEPTH: usize = 12;

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(10.0)
            .color(Color32::from_gray(130))
            .strong(),
    );
}

/// `name: value (type)` — the type hint is omitted when unknown.
fn variable_label(name: &str, value: &str, var_type: Option<&str>) -> String {
    match var_type.filter(|t| !t.is_empty()) {
        Some(t) => format!("{name}: {value} ({t})"),
        None => format!("{name}: {value}"),
    }
}

/// Render the children of `variables_reference`. Only runs for expanded
/// nodes, so not-yet-fetched references are queued in `to_fetch` (lazy load).
fn show_children(
    ui: &mut egui::Ui,
    dap: &DapManager,
    variables_reference: i64,
    parent_id: egui::Id,
    depth: usize,
    to_fetch: &mut Vec<i64>,
) {
    let muted = |ui: &mut egui::Ui, text: &str| {
        ui.label(
            RichText::new(text)
                .size(11.0)
                .italics()
                .color(Color32::GRAY),
        );
    };
    if variables_reference <= 0 {
        muted(ui, "No variables");
        return;
    }
    let Some(vars) = dap.children(variables_reference) else {
        if !dap.is_loading(variables_reference) {
            to_fetch.push(variables_reference);
        }
        muted(ui, "Loading…");
        return;
    };
    if vars.is_empty() {
        muted(ui, "No variables");
    }
    for v in vars {
        let label = RichText::new(variable_label(&v.name, &v.value, v.var_type.as_deref()))
            .size(11.0)
            .monospace();
        if v.variables_reference > 0 && depth < MAX_VARIABLE_DEPTH {
            // Keyed by name path (not reference) so expansions survive steps.
            let id = parent_id.with(&v.name);
            egui::CollapsingHeader::new(label)
                .id_salt(id)
                .default_open(false)
                .show(ui, |ui| {
                    show_children(ui, dap, v.variables_reference, id, depth + 1, to_fetch);
                });
        } else {
            ui.label(label);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_label_includes_type_only_when_known() {
        assert_eq!(variable_label("x", "1", Some("int")), "x: 1 (int)");
        assert_eq!(variable_label("x", "1", Some("")), "x: 1");
        assert_eq!(variable_label("x", "1", None), "x: 1");
    }
}
