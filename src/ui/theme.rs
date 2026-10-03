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
        let bg = Color32::from_rgb(
            theme.background[0],
            theme.background[1],
            theme.background[2],
        );
        let fg = Color32::from_rgb(
            theme.foreground[0],
            theme.foreground[1],
            theme.foreground[2],
        );
        let accent = Color32::from_rgb(theme.accent[0], theme.accent[1], theme.accent[2]);
        let toward = if is_dark(bg) {
            Color32::WHITE
        } else {
            Color32::BLACK
        };
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
        Self {
            xs: 2.0,
            sm: 4.0,
            md: 8.0,
            lg: 12.0,
            round_sm: 4.0,
            round_md: 6.0,
            round_lg: 8.0,
        }
    }
}

/// Rebuild the palette + spacing from config, push the matching egui `Visuals`
/// and UI `text_styles` into the context, and return them for renderers to read.
/// Called once per frame at the top of `layout::render`; cost is negligible.
pub fn apply_theme(ctx: &egui::Context, config: &crate::config::Config) -> (Palette, Spacing) {
    let p = Palette::from_theme(&config.theme);
    let spacing = Spacing::default();

    let mut v = if is_dark(p.bg) {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.surface;
    v.window_fill = p.surface_raised;
    v.extreme_bg_color = p.bg;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.selection;
    v.selection.stroke = egui::Stroke::new(1.0_f32, p.accent);
    v.widgets.inactive.weak_bg_fill = p.surface;
    v.widgets.hovered.weak_bg_fill = p.border;
    v.widgets.active.weak_bg_fill = p.border_strong;
    v.window_stroke = egui::Stroke::new(1.0_f32, p.border);
    ctx.set_visuals(v);

    ctx.style_mut(|style| {
        use egui::{FontFamily, FontId, TextStyle};
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        );
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(13.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(16.0, FontFamily::Proportional),
        );
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
        assert_eq!(
            on(Color32::from_rgb(166, 226, 46)),
            Color32::from_rgb(20, 20, 20)
        );
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

    fn light_theme() -> crate::config::Theme {
        crate::config::Theme {
            name: "light".into(),
            background: [246, 246, 246],
            foreground: [40, 40, 40],
            accent: [166, 226, 46],
        }
    }

    #[test]
    fn mix_clamps_t_and_hits_endpoints() {
        let a = Color32::from_rgb(10, 20, 30);
        let b = Color32::from_rgb(200, 100, 0);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        assert_eq!(mix(a, b, -3.0), a);
        assert_eq!(mix(a, b, 7.0), b);
        // Per-channel, rounded, alpha dropped to opaque.
        assert_eq!(mix(a, b, 0.25), Color32::from_rgb(58, 40, 23));
        let translucent = Color32::from_rgba_unmultiplied(100, 100, 100, 10);
        assert_eq!(mix(translucent, translucent, 0.5).a(), 255);
    }

    #[test]
    fn luminance_weights_green_most() {
        let r = luminance(Color32::from_rgb(255, 0, 0));
        let g = luminance(Color32::from_rgb(0, 255, 0));
        let b = luminance(Color32::from_rgb(0, 0, 255));
        assert!(g > r && r > b);
        assert!((r + g + b - 1.0).abs() < 1e-4);
    }

    #[test]
    fn dark_and_on_thresholds() {
        // luminance exactly 0.5 is not dark
        let mid = Color32::from_gray(128);
        assert!(!is_dark(mid));
        assert!(is_dark(Color32::from_gray(127)));
        // `on` switches at 0.55
        assert_eq!(on(Color32::from_gray(140)), Color32::WHITE);
        assert_eq!(on(Color32::from_gray(141)), Color32::from_rgb(20, 20, 20));
    }

    #[test]
    fn palette_copies_seed_colours() {
        let t = dark_theme();
        let p = Palette::from_theme(&t);
        assert_eq!(p.bg, Color32::from_rgb(30, 30, 30));
        assert_eq!(p.text, Color32::from_rgb(212, 212, 212));
        assert_eq!(p.accent, Color32::from_rgb(0, 122, 204));
    }

    #[test]
    fn dark_palette_lifts_toward_white_in_depth_order() {
        let p = Palette::from_theme(&dark_theme());
        assert!(p.bg.r() < p.surface.r());
        assert!(p.surface.r() < p.line_highlight.r());
        assert!(p.line_highlight.r() < p.surface_raised.r());
        assert!(p.surface_raised.r() < p.border.r());
        assert!(p.border.r() < p.border_strong.r());
        // accent hover is lighter than accent on dark themes
        assert!(p.accent_hover.g() > p.accent.g());
    }

    #[test]
    fn light_palette_darkens_and_uses_dark_text_on_bright_accent() {
        let p = Palette::from_theme(&light_theme());
        assert!(p.surface_raised.r() < p.surface.r());
        assert!(p.border_strong.r() < p.border.r());
        assert!(p.accent_hover.g() < p.accent.g());
        assert_eq!(p.on_accent, Color32::from_rgb(20, 20, 20));
    }

    #[test]
    fn muted_text_sits_between_fg_and_bg() {
        let p = Palette::from_theme(&dark_theme());
        assert!(p.text.r() > p.text_muted.r());
        assert!(p.text_muted.r() > p.text_faint.r());
        assert!(p.text_faint.r() > p.bg.r());
        assert_eq!(p.hint, p.text_muted);
    }

    #[test]
    fn translucent_accents_and_fixed_semantics() {
        let p = Palette::from_theme(&dark_theme());
        for (c, a) in [
            (p.accent_muted, 48),
            (p.selection, 95),
            (p.selection_inactive, 45),
        ] {
            assert_eq!(c.a(), a);
        }
        assert_eq!(p.overlay.a(), 150);
        // Semantic colours do not depend on the theme.
        let l = Palette::from_theme(&light_theme());
        assert_eq!(p.error, l.error);
        assert_eq!(p.warning, l.warning);
        assert_eq!(p.info, l.info);
        assert_eq!(p.success, l.success);
        assert_eq!(p.git_added, l.git_added);
        assert_eq!(p.git_modified, l.git_modified);
        assert_eq!(p.git_removed, l.git_removed);
    }

    #[test]
    fn spacing_scale_is_monotonic() {
        let s = Spacing::default();
        assert!(s.xs < s.sm && s.sm < s.md && s.md < s.lg);
        assert!(s.round_sm < s.round_md && s.round_md < s.round_lg);
        assert_eq!(s.md, 8.0);
    }

    #[test]
    fn apply_theme_dark_config_sets_visuals_and_fonts() {
        let ctx = egui::Context::default();
        let config = crate::config::Config::default();
        let (p, s) = apply_theme(&ctx, &config);
        assert_eq!(p.bg, Color32::from_rgb(30, 30, 30));
        assert_eq!(s.md, Spacing::default().md);
        let style = ctx.style();
        let v = &style.visuals;
        assert!(v.dark_mode);
        assert_eq!(v.panel_fill, p.surface);
        assert_eq!(v.window_fill, p.surface_raised);
        assert_eq!(v.extreme_bg_color, p.bg);
        assert_eq!(v.override_text_color, Some(p.text));
        assert_eq!(v.hyperlink_color, p.accent);
        assert_eq!(v.selection.bg_fill, p.selection);
        assert_eq!(v.selection.stroke.color, p.accent);
        assert_eq!(v.widgets.inactive.weak_bg_fill, p.surface);
        assert_eq!(v.widgets.hovered.weak_bg_fill, p.border);
        assert_eq!(v.widgets.active.weak_bg_fill, p.border_strong);
        assert_eq!(v.window_stroke.color, p.border);
        use egui::TextStyle;
        let size = |ts: TextStyle| style.text_styles[&ts].size;
        assert_eq!(size(TextStyle::Small), 12.0);
        assert_eq!(size(TextStyle::Body), 13.0);
        assert_eq!(size(TextStyle::Button), 13.0);
        assert_eq!(size(TextStyle::Heading), 16.0);
    }

    #[test]
    fn apply_theme_light_config_uses_light_visuals() {
        let ctx = egui::Context::default();
        let config = crate::config::Config {
            theme: light_theme(),
            ..Default::default()
        };
        let (p, _) = apply_theme(&ctx, &config);
        let style = ctx.style();
        assert!(!style.visuals.dark_mode);
        assert_eq!(style.visuals.panel_fill, p.surface);
    }
}
