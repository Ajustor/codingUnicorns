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
