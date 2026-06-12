use egui::Color32;

/// Per-channel linear interpolation. `t` is clamped to [0,1].
pub(crate) fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Perceptual relative luminance in [0,1].
pub(crate) fn luminance(c: Color32) -> f32 {
    (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32) / 255.0
}

/// True if a colour is dark enough to want light text/surfaces lifted toward white.
pub(crate) fn is_dark(c: Color32) -> bool {
    luminance(c) < 0.5
}

/// Pick a readable text colour to draw *on top of* `c` (near-black on light fills, white on dark).
pub(crate) fn on(c: Color32) -> Color32 {
    if luminance(c) > 0.55 {
        Color32::from_rgb(20, 20, 20)
    } else {
        Color32::WHITE
    }
}

/// Semantic colours derived from the 3-colour `config::Theme` seed.
/// Single source of truth for UI rendering. `Copy` so renderers can take it by value.
#[derive(Clone, Copy)]
pub struct Palette {
    // Surfaces (depth hierarchy)
    pub bg: Color32,
    pub surface: Color32,
    pub surface_raised: Color32,
    pub overlay: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    // Text
    pub text: Color32,
    pub text_muted: Color32,
    pub text_faint: Color32,
    // Accent
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_muted: Color32,
    pub on_accent: Color32,
    // Semantic
    pub error: Color32,
    pub warning: Color32,
    pub info: Color32,
    pub success: Color32,
    pub hint: Color32,
    // Git
    pub git_added: Color32,
    pub git_modified: Color32,
    pub git_removed: Color32,
    // Editor
    pub line_highlight: Color32,
    pub selection: Color32,
    pub selection_inactive: Color32,
}

impl Palette {
    pub fn from_theme(theme: &crate::config::Theme) -> Self {
        let bg = Color32::from_rgb(theme.background[0], theme.background[1], theme.background[2]);
        let fg = Color32::from_rgb(theme.foreground[0], theme.foreground[1], theme.foreground[2]);
        let accent = Color32::from_rgb(theme.accent[0], theme.accent[1], theme.accent[2]);
        let toward = if is_dark(bg) { Color32::WHITE } else { Color32::BLACK };
        let alpha = |c: Color32, a: u8| Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a);
        Self {
            bg,
            surface: mix(bg, toward, 0.04),
            surface_raised: mix(bg, toward, 0.08),
            overlay: Color32::from_rgba_unmultiplied(0, 0, 0, 150),
            border: mix(bg, toward, 0.14),
            border_strong: mix(bg, toward, 0.25),
            text: fg,
            text_muted: mix(fg, bg, 0.40),
            text_faint: mix(fg, bg, 0.65),
            accent,
            accent_hover: mix(accent, toward, 0.15),
            accent_muted: alpha(accent, 48),
            on_accent: on(accent),
            error: Color32::from_rgb(229, 83, 75),
            warning: Color32::from_rgb(229, 181, 67),
            info: Color32::from_rgb(86, 156, 214),
            success: Color32::from_rgb(80, 200, 120),
            hint: mix(fg, bg, 0.40),
            git_added: Color32::from_rgb(80, 200, 80),
            git_modified: Color32::from_rgb(80, 150, 255),
            git_removed: Color32::from_rgb(220, 80, 80),
            line_highlight: mix(bg, toward, 0.06),
            selection: alpha(accent, 95),
            selection_inactive: alpha(accent, 45),
        }
    }
}

/// Spacing + corner-rounding scale. Replaces scattered magic numbers. `Copy`.
#[derive(Clone, Copy)]
pub struct Spacing {
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub round_sm: f32,
    pub round_md: f32,
    pub round_lg: f32,
}

impl Default for Spacing {
    fn default() -> Self {
        Self { xs: 2.0, sm: 4.0, md: 8.0, lg: 12.0, round_sm: 4.0, round_md: 6.0, round_lg: 8.0 }
    }
}

/// Rebuild the palette + spacing from config, push the matching egui `Visuals`
/// and UI `text_styles` into the context, and return them for renderers to read.
/// Called once per frame at the top of `layout::render`; cost is negligible.
pub fn apply_theme(ctx: &egui::Context, config: &crate::config::Config) -> (Palette, Spacing) {
    let p = Palette::from_theme(&config.theme);
    let spacing = Spacing::default();

    let mut v = if is_dark(p.bg) { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.surface;
    v.window_fill = p.surface_raised;
    v.extreme_bg_color = p.bg;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.selection;
    v.selection.stroke = egui::Stroke::new(1.0, p.accent);
    v.widgets.inactive.weak_bg_fill = p.surface;
    v.widgets.hovered.weak_bg_fill = p.border;
    v.widgets.active.weak_bg_fill = p.border_strong;
    v.window_stroke = egui::Stroke::new(1.0, p.border);
    ctx.set_visuals(v);

    ctx.style_mut(|style| {
        use egui::{FontFamily, FontId, TextStyle};
        style.text_styles.insert(TextStyle::Small, FontId::new(12.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Button, FontId::new(13.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Heading, FontId::new(16.0, FontFamily::Proportional));
    });

    (p, spacing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_midpoint_is_grey() {
        let m = mix(Color32::BLACK, Color32::WHITE, 0.5);
        assert!((127..=128).contains(&m.r()));
    }

    #[test]
    fn luminance_bounds() {
        assert!(luminance(Color32::WHITE) > 0.99);
        assert!(luminance(Color32::BLACK) < 0.01);
    }

    #[test]
    fn dark_detection() {
        assert!(is_dark(Color32::from_rgb(30, 30, 30)));
        assert!(!is_dark(Color32::from_rgb(246, 246, 246)));
    }

    #[test]
    fn on_picks_contrasting_text() {
        assert_eq!(on(Color32::from_rgb(0, 122, 204)), Color32::WHITE);
        assert_eq!(on(Color32::from_rgb(166, 226, 46)), Color32::from_rgb(20, 20, 20));
    }

    fn dark_theme() -> crate::config::Theme {
        crate::config::Theme {
            name: "dark".into(),
            background: [30, 30, 30],
            foreground: [212, 212, 212],
            accent: [0, 122, 204],
        }
    }

    #[test]
    fn dark_palette_preserves_current_look() {
        let p = Palette::from_theme(&dark_theme());
        assert_eq!(p.surface.r(), 39);
        assert_eq!(p.text_muted.r(), 139);
        assert_eq!(p.on_accent, Color32::WHITE);
        assert_eq!(p.selection.a(), 95);
    }

    #[test]
    fn light_background_lifts_surfaces_downward() {
        let light = crate::config::Theme {
            name: "light".into(),
            background: [246, 246, 246],
            foreground: [40, 40, 40],
            accent: [0, 103, 184],
        };
        let p = Palette::from_theme(&light);
        assert!(p.surface.r() < p.bg.r());
        assert!(p.border.r() < p.surface.r());
    }
}
