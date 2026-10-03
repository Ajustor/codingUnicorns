use egui::Color32;

pub(super) fn ansi_color(idx: u16, bright: bool) -> Color32 {
    match (idx, bright) {
        (0, false) => Color32::from_rgb(0, 0, 0),
        (1, false) => Color32::from_rgb(205, 49, 49),
        (2, false) => Color32::from_rgb(13, 188, 121),
        (3, false) => Color32::from_rgb(229, 229, 16),
        (4, false) => Color32::from_rgb(36, 114, 200),
        (5, false) => Color32::from_rgb(188, 63, 188),
        (6, false) => Color32::from_rgb(17, 168, 205),
        (7, false) => Color32::from_rgb(229, 229, 229),
        (0, true) => Color32::from_rgb(102, 102, 102),
        (1, true) => Color32::from_rgb(241, 76, 76),
        (2, true) => Color32::from_rgb(35, 209, 139),
        (3, true) => Color32::from_rgb(245, 245, 67),
        (4, true) => Color32::from_rgb(59, 142, 234),
        (5, true) => Color32::from_rgb(214, 112, 214),
        (6, true) => Color32::from_rgb(41, 184, 219),
        (7, true) => Color32::from_rgb(229, 229, 229),
        _ => Color32::from_rgb(212, 212, 212),
    }
}

pub(super) fn color_256(n: u16) -> Color32 {
    match n {
        0..=7 => ansi_color(n, false),
        8..=15 => ansi_color(n - 8, true),
        16..=231 => {
            let n = n - 16;
            let b = n % 6;
            let g = (n / 6) % 6;
            let r = n / 36;
            let c = |x: u16| -> u8 {
                if x == 0 {
                    0
                } else {
                    (55 + x * 40) as u8
                }
            };
            Color32::from_rgb(c(r), c(g), c(b))
        }
        232..=255 => {
            let v = (8 + (n - 232) * 10) as u8;
            Color32::from_rgb(v, v, v)
        }
        _ => Color32::from_rgb(212, 212, 212),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_and_bright_palette() {
        assert_eq!(ansi_color(0, false), Color32::from_rgb(0, 0, 0));
        assert_eq!(ansi_color(1, false), Color32::from_rgb(205, 49, 49));
        assert_eq!(ansi_color(2, false), Color32::from_rgb(13, 188, 121));
        assert_eq!(ansi_color(3, false), Color32::from_rgb(229, 229, 16));
        assert_eq!(ansi_color(4, false), Color32::from_rgb(36, 114, 200));
        assert_eq!(ansi_color(5, false), Color32::from_rgb(188, 63, 188));
        assert_eq!(ansi_color(6, false), Color32::from_rgb(17, 168, 205));
        assert_eq!(ansi_color(7, false), Color32::from_rgb(229, 229, 229));
        assert_eq!(ansi_color(0, true), Color32::from_rgb(102, 102, 102));
        assert_eq!(ansi_color(1, true), Color32::from_rgb(241, 76, 76));
        assert_eq!(ansi_color(2, true), Color32::from_rgb(35, 209, 139));
        assert_eq!(ansi_color(3, true), Color32::from_rgb(245, 245, 67));
        assert_eq!(ansi_color(4, true), Color32::from_rgb(59, 142, 234));
        assert_eq!(ansi_color(5, true), Color32::from_rgb(214, 112, 214));
        assert_eq!(ansi_color(6, true), Color32::from_rgb(41, 184, 219));
        assert_eq!(ansi_color(7, true), Color32::from_rgb(229, 229, 229));
    }

    #[test]
    fn out_of_range_index_falls_back_to_default_fg() {
        assert_eq!(ansi_color(8, false), Color32::from_rgb(212, 212, 212));
        assert_eq!(ansi_color(42, true), Color32::from_rgb(212, 212, 212));
    }

    #[test]
    fn color_256_low_entries_map_to_palette() {
        for i in 0..8 {
            assert_eq!(color_256(i), ansi_color(i, false));
            assert_eq!(color_256(i + 8), ansi_color(i, true));
        }
    }

    #[test]
    fn color_256_cube() {
        assert_eq!(color_256(16), Color32::from_rgb(0, 0, 0));
        assert_eq!(color_256(196), Color32::from_rgb(255, 0, 0));
        assert_eq!(color_256(46), Color32::from_rgb(0, 255, 0));
        assert_eq!(color_256(21), Color32::from_rgb(0, 0, 255));
        assert_eq!(color_256(231), Color32::from_rgb(255, 255, 255));
        // 16 + 36*1 + 6*2 + 3 = 67 → (95, 135, 175)
        assert_eq!(color_256(67), Color32::from_rgb(95, 135, 175));
    }

    #[test]
    fn color_256_grayscale_ramp() {
        assert_eq!(color_256(232), Color32::from_rgb(8, 8, 8));
        assert_eq!(color_256(244), Color32::from_rgb(128, 128, 128));
        assert_eq!(color_256(255), Color32::from_rgb(238, 238, 238));
    }

    #[test]
    fn color_256_out_of_range_is_default() {
        assert_eq!(color_256(256), Color32::from_rgb(212, 212, 212));
        assert_eq!(color_256(u16::MAX), Color32::from_rgb(212, 212, 212));
    }
}
