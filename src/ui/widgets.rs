use crate::ui::theme::{Palette, Spacing};

/// The shared material for every floating element (autocomplete, hover, palette, find).
/// Gives all popups the same fill / border / rounding / shadow.
pub fn popup_frame(palette: Palette, spacing: Spacing) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.surface_raised)
        .stroke(egui::Stroke::new(1.0, palette.border))
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
pub fn render_toasts(ctx: &egui::Context, palette: Palette, spacing: Spacing, toasts: &mut Vec<Toast>) -> bool {
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
                    ui.label(
                        egui::RichText::new(&t.message)
                            .color(egui::Color32::from_rgba_unmultiplied(
                                palette.text.r(),
                                palette.text.g(),
                                palette.text.b(),
                                a,
                            )),
                    );
                });
                ui.add_space(spacing.sm);
            }
        });
    true
}
