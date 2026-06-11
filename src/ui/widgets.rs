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
    let mut result = None;
    egui::Area::new(egui::Id::new("confirm_scrim"))
        .order(egui::Order::Background)
        .show(ctx, |ui| {
            let r = ctx.screen_rect();
            ui.painter().rect_filled(r, 0.0, palette.overlay);
        });
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(message).color(palette.text));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    result = Some(false);
                }
                if ui
                    .add(egui::Button::new(egui::RichText::new("Delete").color(palette.on_accent))
                        .fill(palette.error))
                    .clicked()
                {
                    result = Some(true);
                }
            });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        result = Some(false);
    }
    result
}
