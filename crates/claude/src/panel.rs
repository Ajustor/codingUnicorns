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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Message;
    use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Sense};

    /// Headless harness: one egui context + the panel and its inputs.
    struct Harness {
        ctx: egui::Context,
        panel: ClaudePanel,
        session: ClaudeSession,
        pending: Option<PermissionRequest>,
        account: Option<String>,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                ctx: egui::Context::default(),
                panel: ClaudePanel::new(),
                session: ClaudeSession::new(),
                pending: None,
                account: None,
            }
        }

        fn frame(&mut self, events: Vec<Event>) -> ClaudeAction {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 400.0))),
                // Held modifiers mirror those of any key event (as a real backend does).
                modifiers: events
                    .iter()
                    .find_map(|e| match e {
                        Event::Key { modifiers, .. } => Some(*modifiers),
                        _ => None,
                    })
                    .unwrap_or_default(),
                events,
                ..Default::default()
            };
            let mut action = ClaudeAction::None;
            let Self {
                ctx,
                panel,
                session,
                pending,
                account,
            } = self;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    action = panel.show(ui, session, pending.as_ref(), account.as_deref());
                });
            });
            action
        }

        /// Focusable widgets of the last frame as (rect, sense).
        fn focusables(&self) -> Vec<(Rect, Sense)> {
            self.ctx.viewport(|vp| {
                let w = &vp.prev_pass.widgets;
                w.layer_ids()
                    .flat_map(|l| w.get_layer(l))
                    .filter(|r| r.sense.is_focusable())
                    .map(|r| (r.rect, r.sense))
                    .collect()
            })
        }

        /// Buttons (click, no drag) sorted top-to-bottom then left-to-right.
        fn buttons(&self) -> Vec<Rect> {
            let mut b: Vec<Rect> = self
                .focusables()
                .into_iter()
                .filter(|(_, s)| s.senses_click() && !s.senses_drag())
                .map(|(r, _)| r)
                .collect();
            b.sort_by(|a, b| {
                (a.top().round(), a.left())
                    .partial_cmp(&(b.top().round(), b.left()))
                    .unwrap()
            });
            b
        }

        fn text_edit(&self) -> Rect {
            self.focusables()
                .into_iter()
                .find(|(_, s)| s.senses_drag())
                .map(|(r, _)| r)
                .expect("text edit present")
        }

        /// Press in one frame, release in the next; returns the release frame's action.
        fn click(&mut self, rect: Rect) -> ClaudeAction {
            let pos = rect.center();
            self.frame(vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ]);
            self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }])
        }

        fn focus_input(&mut self) {
            self.frame(vec![]);
            let r = self.text_edit();
            self.click(r);
        }

        fn type_text(&mut self, s: &str) -> ClaudeAction {
            self.frame(vec![Event::Text(s.into())])
        }

        fn press_enter(&mut self, modifiers: Modifiers) -> ClaudeAction {
            self.frame(vec![Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }])
        }
    }

    fn request() -> (PermissionRequest, std::sync::mpsc::Receiver<Decision>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (
            PermissionRequest {
                tool: "Edit".into(),
                input: serde_json::json!({"file":"a.rs"}),
                reply: tx,
            },
            rx,
        )
    }

    #[test]
    fn idle_frame_does_nothing() {
        let mut h = Harness::new();
        assert!(matches!(h.frame(vec![]), ClaudeAction::None));
        assert!(h.panel.input.is_empty());
    }

    #[test]
    fn enter_sends_trimmed_input_and_clears() {
        let mut h = Harness::new();
        h.focus_input();
        h.type_text("  explain this  ");
        assert_eq!(h.panel.input, "  explain this  ");
        match h.press_enter(Modifiers::NONE) {
            ClaudeAction::Send(t) => assert_eq!(t, "explain this"),
            _ => panic!("expected Send"),
        }
        assert!(h.panel.input.is_empty());
    }

    #[test]
    fn shift_enter_does_not_send() {
        let mut h = Harness::new();
        h.focus_input();
        h.type_text("line");
        let a = h.press_enter(Modifiers::SHIFT);
        assert!(!matches!(a, ClaudeAction::Send(_)));
        assert!(h.panel.input.starts_with("line"), "{:?}", h.panel.input);
    }

    #[test]
    fn enter_with_blank_input_does_not_send() {
        let mut h = Harness::new();
        h.focus_input();
        h.type_text("   ");
        assert!(!matches!(
            h.press_enter(Modifiers::NONE),
            ClaudeAction::Send(_)
        ));
    }

    #[test]
    fn enter_while_running_keeps_input() {
        let mut h = Harness::new();
        h.session.running = true;
        h.focus_input();
        h.type_text("wait");
        assert!(!matches!(
            h.press_enter(Modifiers::NONE),
            ClaudeAction::Send(_)
        ));
        assert!(h.panel.input.contains("wait"));
    }

    #[test]
    fn enter_without_focus_does_not_send() {
        let mut h = Harness::new();
        h.panel.input = "queued".into();
        h.frame(vec![]);
        assert!(matches!(h.press_enter(Modifiers::NONE), ClaudeAction::None));
        assert_eq!(h.panel.input, "queued");
    }

    #[test]
    fn header_buttons_when_idle() {
        let mut h = Harness::new();
        h.frame(vec![]);
        // Idle header: [Interactive, New] (right-to-left layout).
        let b = h.buttons();
        assert_eq!(b.len(), 2);
        assert!(matches!(h.click(b[1]), ClaudeAction::NewConversation));
        let b = h.buttons();
        assert!(matches!(h.click(b[0]), ClaudeAction::OpenInteractive));
    }

    #[test]
    fn cancel_button_when_running() {
        let mut h = Harness::new();
        h.session.running = true;
        h.session.cost_usd = 0.1234;
        h.account = Some("me@example.com · pro".into());
        h.frame(vec![]);
        let b = h.buttons();
        assert_eq!(b.len(), 3, "Cancel + Interactive + New");
        assert!(matches!(h.click(b[0]), ClaudeAction::Cancel));
    }

    #[test]
    fn permission_buttons() {
        let mut h = Harness::new();
        let (req, _rx) = request();
        h.pending = Some(req);
        h.frame(vec![]);
        let b = h.buttons();
        assert_eq!(b.len(), 4, "Interactive, New, Allow, Deny");
        assert!(matches!(
            h.click(b[2]),
            ClaudeAction::Permission(Decision::Allow)
        ));
        let b = h.buttons();
        assert!(matches!(
            h.click(b[3]),
            ClaudeAction::Permission(Decision::Deny)
        ));
    }

    #[test]
    fn renders_full_transcript() {
        let mut h = Harness::new();
        for (role, text) in [
            (Role::User, "q"),
            (Role::Assistant, "a"),
            (Role::Tool, "Edit {}"),
        ] {
            h.session.transcript.push(Message {
                role,
                text: text.into(),
            });
        }
        assert!(matches!(h.frame(vec![]), ClaudeAction::None));
    }
}
