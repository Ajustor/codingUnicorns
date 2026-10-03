use crate::permission::{Decision, PermissionRequest};
use crate::session::{ClaudeSession, Role};

#[derive(Default)]
pub struct ClaudePanel {
    pub input: String,
}

/// What the panel asks the app to do after a frame.
pub enum ClaudeAction {
    None,
    Send(String),
    NewConversation,
    Cancel,
    Permission(Decision),
    /// Open the real interactive `claude` CLI in a terminal (for built-in
    /// commands like /usage, /cost, /context that don't exist in headless mode).
    OpenInteractive,
}

impl ClaudePanel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &ClaudeSession,
        pending: Option<&PermissionRequest>,
        account: Option<&str>,
    ) -> ClaudeAction {
        let mut action = ClaudeAction::None;

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Claude")
                    .strong()
                    .color(egui::Color32::WHITE),
            );
            if let Some(acc) = account {
                ui.label(
                    egui::RichText::new(acc)
                        .small()
                        .color(egui::Color32::from_gray(140)),
                )
                .on_hover_text("Active Claude account (from `claude auth status`)");
            }
            if session.running {
                ui.spinner();
                if ui.small_button("Cancel").clicked() {
                    action = ClaudeAction::Cancel;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("New").clicked() {
                    action = ClaudeAction::NewConversation;
                }
                if ui
                    .small_button("Interactive")
                    .on_hover_text(
                        "Open the real interactive Claude in a terminal — for built-in \
                         commands like /usage, /cost, /context",
                    )
                    .clicked()
                {
                    action = ClaudeAction::OpenInteractive;
                }
                if session.cost_usd > 0.0 {
                    ui.label(
                        egui::RichText::new(format!("${:.4}", session.cost_usd))
                            .small()
                            .color(egui::Color32::from_gray(150)),
                    );
                }
            });
        });
        ui.separator();

        // Transcript (scrolls, sticks to bottom).
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .max_height(ui.available_height() - 80.0)
            .show(ui, |ui| {
                for msg in &session.transcript {
                    let (prefix, color) = match msg.role {
                        Role::User => ("you", egui::Color32::from_rgb(120, 170, 255)),
                        Role::Assistant => ("claude", egui::Color32::from_rgb(180, 230, 180)),
                        Role::Tool => ("tool", egui::Color32::from_gray(150)),
                    };
                    ui.label(egui::RichText::new(prefix).small().color(color));
                    ui.label(egui::RichText::new(&msg.text).color(egui::Color32::from_gray(220)));
                    ui.add_space(4.0);
                }
            });

        // Inline permission dialog.
        if let Some(req) = pending {
            ui.separator();
            ui.label(
                egui::RichText::new(format!("Allow tool: {}?", req.tool))
                    .strong()
                    .color(egui::Color32::from_rgb(255, 200, 60)),
            );
            ui.label(
                egui::RichText::new(req.input.to_string())
                    .small()
                    .color(egui::Color32::from_gray(180)),
            );
            ui.horizontal(|ui| {
                if ui.button("Allow").clicked() {
                    action = ClaudeAction::Permission(Decision::Allow);
                }
                if ui.button("Deny").clicked() {
                    action = ClaudeAction::Permission(Decision::Deny);
                }
            });
        }

        // Input box. Kept ENABLED even while a turn runs, so pressing Enter to send
        // doesn't disable the widget and steal focus; sending is just gated on idle.
        ui.separator();
        let resp = ui.add(
            egui::TextEdit::multiline(&mut self.input)
                .desired_rows(2)
                .hint_text("Ask Claude…  (Enter to send, Shift+Enter for newline)")
                .desired_width(f32::INFINITY),
        );
        let enter =
            resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
        if enter && !session.running {
            let text = self.input.trim().to_string();
            if !text.is_empty() {
                self.input.clear();
                action = ClaudeAction::Send(text);
            }
        }
        // Keep the caret in the box across send (the Enter would otherwise leave a
        // stray newline / the next frame's rebuild could drop focus).
        if enter {
            resp.request_focus();
        }

        action
    }
}
