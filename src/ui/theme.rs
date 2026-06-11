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
}
