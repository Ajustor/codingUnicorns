use crate::ui::theme::{Palette, Spacing};

/// The shared material for every floating element (autocomplete, hover, palette, find).
/// Gives all popups the same fill / border / rounding / shadow.
pub fn popup_frame(palette: Palette, spacing: Spacing) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.surface_raised)
        .stroke(egui::Stroke::new(1.0_f32, palette.border))
        .corner_radius(egui::CornerRadius::same(spacing.round_md as u8))
        .inner_margin(egui::Margin::same(spacing.xs as i8))
        .shadow(egui::epaint::Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: egui::Color32::from_black_alpha(60),
        })
}

/// A small centred modal. Returns `Some(true)` if confirmed, `Some(false)` if cancelled,
/// `None` while still open. Caller clears its trigger state on a `Some(_)`.
pub fn confirm_dialog(
    ctx: &egui::Context,
    palette: Palette,
    title: &str,
    message: &str,
) -> Option<bool> {
    // egui::Modal renders a dimming, input-blocking backdrop, so clicks/keys can't
    // reach the editor or file tree behind it — a real modal, unlike a bare Window.
    let modal = egui::Modal::new(egui::Id::new("confirm_modal")).show(ctx, |ui| {
        ui.set_max_width(360.0);
        ui.heading(title);
        ui.add_space(8.0);
        ui.label(egui::RichText::new(message).color(palette.text));
        ui.add_space(12.0);
        let mut choice: Option<bool> = None;
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                choice = Some(false);
            }
            // White on the red `error` fill stays readable on every theme
            // (on_accent would be near-black on light-accent presets).
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("Delete").color(egui::Color32::WHITE))
                        .fill(palette.error),
                )
                .clicked()
            {
                choice = Some(true);
            }
        });
        choice
    });
    let mut result = modal.inner;
    // Backdrop click or Escape → treat as cancel.
    if result.is_none() && modal.should_close() {
        result = Some(false);
    }
    result
}

pub struct Toast {
    pub message: String,
    pub born: std::time::Instant,
}

const TOAST_TTL_MS: u128 = 2000;
const TOAST_FADE_MS: u128 = 400;

/// Draw active toasts bottom-centre and drop expired ones. Returns true if any remain
/// (so the caller can request a repaint).
pub fn render_toasts(
    ctx: &egui::Context,
    palette: Palette,
    spacing: Spacing,
    toasts: &mut Vec<Toast>,
) -> bool {
    toasts.retain(|t| t.born.elapsed().as_millis() < TOAST_TTL_MS);
    if toasts.is_empty() {
        return false;
    }
    egui::Area::new(egui::Id::new("toasts"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -48.0])
        .show(ctx, |ui| {
            for t in toasts.iter() {
                let elapsed = t.born.elapsed().as_millis();
                let remaining = TOAST_TTL_MS.saturating_sub(elapsed);
                let a = if remaining < TOAST_FADE_MS {
                    (remaining as f32 / TOAST_FADE_MS as f32 * 255.0) as u8
                } else {
                    255
                };
                popup_frame(palette, spacing).show(ui, |ui| {
                    ui.label(egui::RichText::new(&t.message).color(
                        egui::Color32::from_rgba_unmultiplied(
                            palette.text.r(),
                            palette.text.g(),
                            palette.text.b(),
                            a,
                        ),
                    ));
                });
                ui.add_space(spacing.sm);
            }
        });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect};
    use std::time::{Duration, Instant};

    fn palette() -> Palette {
        Palette::from_theme(&crate::config::Config::default().theme)
    }

    fn input(events: Vec<Event>) -> RawInput {
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            ..Default::default()
        }
    }

    fn collect_texts(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect_texts(s, out)),
            _ => {}
        }
    }

    fn texts(out: &egui::FullOutput) -> Vec<String> {
        let mut t = Vec::new();
        for c in &out.shapes {
            collect_texts(&c.shape, &mut t);
        }
        t
    }

    fn toast(msg: &str, age: Duration) -> Toast {
        Toast {
            message: msg.into(),
            born: Instant::now().checked_sub(age).expect("monotonic clock"),
        }
    }

    #[test]
    fn popup_frame_uses_palette_and_spacing() {
        let p = palette();
        let s = Spacing::default();
        let f = popup_frame(p, s);
        assert_eq!(f.fill, p.surface_raised);
        assert_eq!(f.stroke, egui::Stroke::new(1.0_f32, p.border));
        assert_eq!(f.corner_radius, egui::CornerRadius::same(s.round_md as u8));
        assert_eq!(f.inner_margin, egui::Margin::same(s.xs as i8));
        assert_eq!(f.shadow.offset, [0, 2]);
        assert_eq!(f.shadow.blur, 8);
        assert_eq!(f.shadow.color, egui::Color32::from_black_alpha(60));
    }

    /// Run `render_toasts` for two frames (a new Area is invisible on its first frame);
    /// returns the second frame's (result, painted texts).
    fn run_toasts(toasts: &mut Vec<Toast>) -> (bool, Vec<String>) {
        let ctx = egui::Context::default();
        let mut result = (false, Vec::new());
        for _ in 0..2 {
            let mut any = false;
            let out = ctx.run(input(vec![]), |ctx| {
                any = render_toasts(ctx, palette(), Spacing::default(), toasts);
            });
            result = (any, texts(&out));
        }
        result
    }

    #[test]
    fn no_toasts_returns_false() {
        let mut toasts = Vec::new();
        let (any, painted) = run_toasts(&mut toasts);
        assert!(!any);
        assert!(painted.is_empty());
    }

    #[test]
    fn fresh_toasts_render_and_request_repaint() {
        let mut toasts = vec![
            toast("Saved", Duration::ZERO),
            toast("Copied", Duration::ZERO),
        ];
        let (any, painted) = run_toasts(&mut toasts);
        assert!(any);
        assert_eq!(toasts.len(), 2);
        assert_eq!(painted, ["Saved", "Copied"]);
    }

    #[test]
    fn expired_toasts_are_dropped() {
        let mut toasts = vec![
            toast("old", Duration::from_millis(2500)),
            toast("new", Duration::ZERO),
            toast("ancient", Duration::from_secs(60)),
        ];
        let (any, painted) = run_toasts(&mut toasts);
        assert!(any);
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].message, "new");
        assert_eq!(painted, ["new"]);

        let mut only_old = vec![toast("old", Duration::from_millis(TOAST_TTL_MS as u64))];
        let (any, painted) = run_toasts(&mut only_old);
        assert!(!any, "a toast is gone exactly at its TTL");
        assert!(only_old.is_empty());
        assert!(painted.is_empty());
    }

    #[test]
    fn fading_toast_is_still_shown() {
        // Within the last TOAST_FADE_MS of its life.
        let mut toasts = vec![toast("fading", Duration::from_millis(1800))];
        let (any, painted) = run_toasts(&mut toasts);
        assert!(any);
        assert_eq!(painted, ["fading"]);
    }

    /// Drives `confirm_dialog` frame by frame on one context.
    struct Dialog {
        ctx: egui::Context,
    }

    impl Dialog {
        fn frame(&self, events: Vec<Event>) -> (Option<bool>, Vec<String>) {
            let mut r = None;
            let out = self.ctx.run(input(events), |ctx| {
                r = confirm_dialog(ctx, palette(), "Delete file?", "This cannot be undone.");
            });
            (r, texts(&out))
        }

        /// Focusable click-only widgets (the buttons), left to right.
        fn buttons(&self) -> Vec<Rect> {
            let mut b: Vec<Rect> = self.ctx.viewport(|vp| {
                let w = &vp.prev_pass.widgets;
                w.layer_ids()
                    .flat_map(|l| w.get_layer(l))
                    .filter(|r| {
                        r.sense.is_focusable() && r.sense.senses_click() && !r.sense.senses_drag()
                    })
                    .map(|r| r.rect)
                    .collect()
            });
            b.sort_by(|a, b| a.left().partial_cmp(&b.left()).unwrap());
            b
        }

        fn click(&self, pos: Pos2) -> Option<bool> {
            let press = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            let (a, _) = self.frame(vec![Event::PointerMoved(pos), press(true)]);
            assert!(a.is_none(), "press alone must not decide");
            self.frame(vec![press(false)]).0
        }
    }

    fn dialog() -> Dialog {
        let d = Dialog {
            ctx: egui::Context::default(),
        };
        // Modal fades in; let it settle so its widgets are laid out.
        for _ in 0..3 {
            let (r, _) = d.frame(vec![]);
            assert!(r.is_none(), "stays open with no input");
        }
        d
    }

    #[test]
    fn confirm_dialog_shows_title_message_and_buttons() {
        let d = dialog();
        let (r, painted) = d.frame(vec![]);
        assert!(r.is_none());
        for t in ["Delete file?", "This cannot be undone.", "Cancel", "Delete"] {
            assert!(
                painted.iter().any(|p| p == t),
                "missing {t:?} in {painted:?}"
            );
        }
        assert_eq!(d.buttons().len(), 2);
    }

    #[test]
    fn confirm_dialog_delete_confirms() {
        let d = dialog();
        let b = d.buttons();
        assert_eq!(d.click(b[1].center()), Some(true));
    }

    #[test]
    fn confirm_dialog_cancel_button_cancels() {
        let d = dialog();
        let b = d.buttons();
        assert_eq!(d.click(b[0].center()), Some(false));
    }

    #[test]
    fn confirm_dialog_escape_cancels() {
        let d = dialog();
        let (r, _) = d.frame(vec![Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        assert_eq!(r, Some(false));
    }

    #[test]
    fn confirm_dialog_backdrop_click_cancels() {
        let d = dialog();
        assert_eq!(d.click(Pos2::new(5.0, 5.0)), Some(false));
    }
}
