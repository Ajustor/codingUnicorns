use super::colors::{ansi_color, color_256};
use egui::Color32;

pub(super) const DEFAULT_FG: Color32 = Color32::from_rgb(212, 212, 212);

#[derive(Clone)]
pub(super) struct Cell {
    pub(super) ch: char,
    pub(super) fg: Color32,
    pub(super) bold: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: DEFAULT_FG,
            bold: false,
        }
    }
}

pub(super) struct ScreenBuffer {
    /// Visible rows — fixed to terminal dimensions.
    pub(super) rows: Vec<Vec<Cell>>,
    /// Lines that have scrolled off the top.
    pub(super) scrollback: Vec<Vec<Cell>>,
    pub(super) cursor_row: usize,
    pub(super) cursor_col: usize,
    pub(super) cols: usize,
    term_rows: usize,
    pub(super) current_fg: Color32,
    pub(super) current_bold: bool,
    max_scrollback: usize,
}

impl ScreenBuffer {
    pub(super) fn new(cols: usize, rows: usize) -> Self {
        Self {
            rows: (0..rows).map(|_| vec![Cell::default(); cols]).collect(),
            scrollback: Vec::new(),
            cursor_row: 0,
            cursor_col: 0,
            cols,
            term_rows: rows,
            current_fg: DEFAULT_FG,
            current_bold: false,
            max_scrollback: 10_000,
        }
    }

    pub(super) fn write_char(&mut self, ch: char) {
        if self.cursor_col >= self.cols {
            self.line_feed();
            self.cursor_col = 0;
        }
        // Clamp cursor_row defensively in case external sequences put it out of range.
        let row_idx = self.cursor_row.min(self.rows.len().saturating_sub(1));
        if let Some(row) = self.rows.get_mut(row_idx) {
            if let Some(cell) = row.get_mut(self.cursor_col) {
                *cell = Cell {
                    ch,
                    fg: self.current_fg,
                    bold: self.current_bold,
                };
            }
        }
        self.cursor_col += 1;
    }

    pub(super) fn carriage_return(&mut self) {
        self.cursor_col = 0;
    }

    pub(super) fn line_feed(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        if self.cursor_row + 1 >= self.term_rows {
            let top = self.rows.remove(0);
            self.scrollback.push(top);
            if self.scrollback.len() > self.max_scrollback {
                let excess = self.scrollback.len() - self.max_scrollback;
                self.scrollback.drain(0..excess);
            }
            self.rows.push(vec![Cell::default(); self.cols]);
            // Keep cursor_row in bounds after scroll.
            self.cursor_row = self.cursor_row.min(self.rows.len().saturating_sub(1));
        } else {
            self.cursor_row += 1;
        }
    }

    pub(super) fn move_cursor(&mut self, dir: char, n: usize) {
        match dir {
            'A' => self.cursor_row = self.cursor_row.saturating_sub(n),
            'B' => {
                self.cursor_row = (self.cursor_row + n).min(self.term_rows.saturating_sub(1));
            }
            'C' => {
                self.cursor_col = (self.cursor_col + n).min(self.cols.saturating_sub(1));
            }
            'D' => self.cursor_col = self.cursor_col.saturating_sub(n),
            _ => {}
        }
    }

    pub(super) fn set_cursor_pos(&mut self, row: usize, col: usize) {
        let r = row.max(1) - 1;
        let c = col.max(1) - 1;
        self.cursor_row = r.min(self.term_rows.saturating_sub(1));
        self.cursor_col = c.min(self.cols.saturating_sub(1));
    }

    pub(super) fn erase_display(&mut self, param: u16) {
        let crow = self.cursor_row.min(self.rows.len().saturating_sub(1));
        let ccol = self.cursor_col.min(self.cols.saturating_sub(1));
        match param {
            0 => {
                if let Some(row) = self.rows.get_mut(crow) {
                    for cell in &mut row[ccol..] {
                        *cell = Cell::default();
                    }
                }
                for r in (crow + 1)..self.rows.len() {
                    self.rows[r] = vec![Cell::default(); self.cols];
                }
            }
            1 => {
                for r in 0..crow.min(self.rows.len()) {
                    self.rows[r] = vec![Cell::default(); self.cols];
                }
                if let Some(row) = self.rows.get_mut(crow) {
                    let end = ccol.min(row.len().saturating_sub(1));
                    for cell in &mut row[..=end] {
                        *cell = Cell::default();
                    }
                }
            }
            2 | 3 => {
                for row in &mut self.rows {
                    *row = vec![Cell::default(); self.cols];
                }
                self.cursor_row = 0;
                self.cursor_col = 0;
            }
            _ => {}
        }
    }

    pub(super) fn erase_line(&mut self, param: u16) {
        let crow = self.cursor_row.min(self.rows.len().saturating_sub(1));
        let ccol = self.cursor_col.min(self.cols.saturating_sub(1));
        let Some(row) = self.rows.get_mut(crow) else {
            return;
        };
        match param {
            0 => {
                for cell in &mut row[ccol..] {
                    *cell = Cell::default();
                }
            }
            1 => {
                let end = ccol.min(row.len().saturating_sub(1));
                for cell in &mut row[..=end] {
                    *cell = Cell::default();
                }
            }
            2 => {
                *row = vec![Cell::default(); self.cols];
            }
            _ => {}
        }
    }

    pub(super) fn set_sgr(&mut self, params: &[u16]) {
        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => {
                    self.current_fg = DEFAULT_FG;
                    self.current_bold = false;
                }
                1 => self.current_bold = true,
                22 => self.current_bold = false,
                39 => self.current_fg = DEFAULT_FG,
                30..=37 => self.current_fg = ansi_color(params[i] - 30, false),
                90..=97 => self.current_fg = ansi_color(params[i] - 90, true),
                38 if params.get(i + 1) == Some(&5) => {
                    if let Some(&n) = params.get(i + 2) {
                        self.current_fg = color_256(n);
                        i += 2;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_text(b: &ScreenBuffer, r: usize) -> String {
        b.rows[r].iter().map(|c| c.ch).collect::<String>()
    }

    fn write_str(b: &mut ScreenBuffer, s: &str) {
        for ch in s.chars() {
            b.write_char(ch);
        }
    }

    #[test]
    fn new_buffer_is_blank() {
        let b = ScreenBuffer::new(4, 3);
        assert_eq!(b.rows.len(), 3);
        assert!(b.rows.iter().all(|r| r.len() == 4));
        assert!(b.rows.iter().flatten().all(|c| c.ch == ' ' && !c.bold));
        assert_eq!((b.cursor_row, b.cursor_col), (0, 0));
        assert!(b.scrollback.is_empty());
        assert_eq!(b.current_fg, DEFAULT_FG);
    }

    #[test]
    fn write_char_stores_attributes_and_advances() {
        let mut b = ScreenBuffer::new(5, 2);
        b.current_fg = Color32::RED;
        b.current_bold = true;
        b.write_char('x');
        assert_eq!(b.rows[0][0].ch, 'x');
        assert_eq!(b.rows[0][0].fg, Color32::RED);
        assert!(b.rows[0][0].bold);
        assert_eq!(b.cursor_col, 1);
    }

    #[test]
    fn writing_past_last_column_wraps_to_next_line() {
        let mut b = ScreenBuffer::new(3, 3);
        write_str(&mut b, "abcde");
        assert_eq!(row_text(&b, 0), "abc");
        assert_eq!(row_text(&b, 1), "de ");
        assert_eq!((b.cursor_row, b.cursor_col), (1, 2));
    }

    #[test]
    fn cursor_sits_past_end_until_next_char() {
        let mut b = ScreenBuffer::new(3, 2);
        write_str(&mut b, "abc");
        // Pending-wrap state: no line feed until another character arrives.
        assert_eq!((b.cursor_row, b.cursor_col), (0, 3));
        b.carriage_return();
        assert_eq!(b.cursor_col, 0);
        b.write_char('z');
        assert_eq!(row_text(&b, 0), "zbc");
    }

    #[test]
    fn line_feed_scrolls_into_scrollback_at_bottom() {
        let mut b = ScreenBuffer::new(2, 2);
        b.write_char('1');
        b.line_feed();
        b.carriage_return();
        b.write_char('2');
        b.line_feed();
        b.carriage_return();
        b.write_char('3');
        assert_eq!(b.scrollback.len(), 1);
        assert_eq!(b.scrollback[0][0].ch, '1');
        assert_eq!(row_text(&b, 0), "2 ");
        assert_eq!(row_text(&b, 1), "3 ");
        assert_eq!(b.cursor_row, 1);
        assert_eq!(b.rows.len(), 2, "visible height is constant");
    }

    #[test]
    fn scrollback_is_trimmed_to_max() {
        let mut b = ScreenBuffer::new(1, 1);
        b.max_scrollback = 3;
        for ch in ['a', 'b', 'c', 'd', 'e'] {
            b.carriage_return();
            b.write_char(ch);
            b.line_feed();
        }
        let sb: String = b.scrollback.iter().map(|r| r[0].ch).collect();
        assert_eq!(sb, "cde");
    }

    #[test]
    fn line_feed_on_empty_buffer_is_noop() {
        let mut b = ScreenBuffer::new(0, 0);
        b.line_feed();
        b.write_char('x');
        b.erase_display(0);
        b.erase_line(0);
        assert!(b.rows.is_empty());
        assert!(b.scrollback.is_empty());
    }

    #[test]
    fn write_char_with_out_of_range_row_is_clamped() {
        let mut b = ScreenBuffer::new(3, 2);
        b.cursor_row = 10;
        b.write_char('q');
        assert_eq!(b.rows[1][0].ch, 'q');
    }

    #[test]
    fn move_cursor_clamps_in_every_direction() {
        let mut b = ScreenBuffer::new(10, 5);
        b.move_cursor('B', 2);
        assert_eq!(b.cursor_row, 2);
        b.move_cursor('B', 100);
        assert_eq!(b.cursor_row, 4);
        b.move_cursor('A', 1);
        assert_eq!(b.cursor_row, 3);
        b.move_cursor('A', 100);
        assert_eq!(b.cursor_row, 0);
        b.move_cursor('C', 3);
        assert_eq!(b.cursor_col, 3);
        b.move_cursor('C', 100);
        assert_eq!(b.cursor_col, 9);
        b.move_cursor('D', 4);
        assert_eq!(b.cursor_col, 5);
        b.move_cursor('D', 100);
        assert_eq!(b.cursor_col, 0);
        b.move_cursor('Z', 3);
        assert_eq!((b.cursor_row, b.cursor_col), (0, 0));
    }

    #[test]
    fn set_cursor_pos_is_one_based_and_clamped() {
        let mut b = ScreenBuffer::new(10, 5);
        b.set_cursor_pos(3, 4);
        assert_eq!((b.cursor_row, b.cursor_col), (2, 3));
        b.set_cursor_pos(0, 0);
        assert_eq!((b.cursor_row, b.cursor_col), (0, 0));
        b.set_cursor_pos(99, 99);
        assert_eq!((b.cursor_row, b.cursor_col), (4, 9));
    }

    fn filled(cols: usize, rows: usize) -> ScreenBuffer {
        let mut b = ScreenBuffer::new(cols, rows);
        for r in 0..rows {
            for c in 0..cols {
                b.rows[r][c].ch = 'x';
            }
        }
        b
    }

    #[test]
    fn erase_display_below() {
        let mut b = filled(4, 3);
        b.set_cursor_pos(2, 3);
        b.erase_display(0);
        assert_eq!(row_text(&b, 0), "xxxx");
        assert_eq!(row_text(&b, 1), "xx  ");
        assert_eq!(row_text(&b, 2), "    ");
        assert_eq!((b.cursor_row, b.cursor_col), (1, 2), "cursor unchanged");
    }

    #[test]
    fn erase_display_above() {
        let mut b = filled(4, 3);
        b.set_cursor_pos(2, 3);
        b.erase_display(1);
        assert_eq!(row_text(&b, 0), "    ");
        assert_eq!(row_text(&b, 1), "   x");
        assert_eq!(row_text(&b, 2), "xxxx");
    }

    #[test]
    fn erase_display_all_homes_cursor() {
        for mode in [2, 3] {
            let mut b = filled(4, 3);
            b.set_cursor_pos(3, 3);
            b.erase_display(mode);
            assert!(b.rows.iter().flatten().all(|c| c.ch == ' '));
            assert_eq!((b.cursor_row, b.cursor_col), (0, 0));
        }
    }

    #[test]
    fn erase_display_unknown_mode_is_ignored() {
        let mut b = filled(2, 2);
        b.erase_display(7);
        assert_eq!(row_text(&b, 0), "xx");
    }

    #[test]
    fn erase_display_with_pending_wrap_cursor() {
        let mut b = filled(3, 2);
        b.cursor_col = 3; // past the end after writing the last column
        b.erase_display(0);
        assert_eq!(row_text(&b, 0), "xx ");
        assert_eq!(row_text(&b, 1), "   ");
    }

    #[test]
    fn erase_line_modes() {
        let mut b = filled(5, 2);
        b.set_cursor_pos(1, 3);
        b.erase_line(0);
        assert_eq!(row_text(&b, 0), "xx   ");

        let mut b = filled(5, 2);
        b.set_cursor_pos(1, 3);
        b.erase_line(1);
        assert_eq!(row_text(&b, 0), "   xx");

        let mut b = filled(5, 2);
        b.set_cursor_pos(2, 3);
        b.erase_line(2);
        assert_eq!(row_text(&b, 1), "     ");
        assert_eq!(row_text(&b, 0), "xxxxx", "other rows untouched");

        let mut b = filled(5, 2);
        b.erase_line(9);
        assert_eq!(row_text(&b, 0), "xxxxx");
    }

    #[test]
    fn erase_resets_attributes() {
        let mut b = ScreenBuffer::new(3, 1);
        b.current_fg = Color32::RED;
        b.current_bold = true;
        write_str(&mut b, "abc");
        b.cursor_col = 0;
        b.erase_line(2);
        assert!(b.rows[0].iter().all(|c| c.fg == DEFAULT_FG && !c.bold));
    }

    #[test]
    fn sgr_basic_colors_and_bold() {
        let mut b = ScreenBuffer::new(1, 1);
        b.set_sgr(&[1, 31]);
        assert!(b.current_bold);
        assert_eq!(b.current_fg, ansi_color(1, false));
        b.set_sgr(&[94]);
        assert_eq!(b.current_fg, ansi_color(4, true));
        b.set_sgr(&[22]);
        assert!(!b.current_bold);
        b.set_sgr(&[39]);
        assert_eq!(b.current_fg, DEFAULT_FG);
        b.set_sgr(&[37, 1]);
        b.set_sgr(&[0]);
        assert_eq!(b.current_fg, DEFAULT_FG);
        assert!(!b.current_bold);
        b.set_sgr(&[30]);
        assert_eq!(b.current_fg, ansi_color(0, false));
        b.set_sgr(&[90]);
        assert_eq!(b.current_fg, ansi_color(0, true));
        b.set_sgr(&[97]);
        assert_eq!(b.current_fg, ansi_color(7, true));
    }

    #[test]
    fn sgr_256_color() {
        let mut b = ScreenBuffer::new(1, 1);
        b.set_sgr(&[38, 5, 196]);
        assert_eq!(b.current_fg, color_256(196));
        // Params after the 256-color triple are still processed.
        b.set_sgr(&[38, 5, 21, 1]);
        assert_eq!(b.current_fg, color_256(21));
        assert!(b.current_bold);
    }

    #[test]
    fn sgr_truncated_256_color_is_ignored() {
        let mut b = ScreenBuffer::new(1, 1);
        b.set_sgr(&[38, 5]);
        assert_eq!(b.current_fg, DEFAULT_FG);
        b.set_sgr(&[38]);
        assert_eq!(b.current_fg, DEFAULT_FG);
    }

    #[test]
    fn sgr_unknown_codes_are_ignored() {
        let mut b = ScreenBuffer::new(1, 1);
        b.set_sgr(&[4, 7, 49, 100]);
        assert_eq!(b.current_fg, DEFAULT_FG);
        assert!(!b.current_bold);
        b.set_sgr(&[]);
        assert_eq!(b.current_fg, DEFAULT_FG);
    }

    #[test]
    #[ignore = "BUG: 24-bit SGR (38;2;r;g;b) unsupported; r/g/b are misread as standalone SGR codes"]
    fn sgr_truecolor() {
        let mut b = ScreenBuffer::new(1, 1);
        b.set_sgr(&[38, 2, 10, 31, 0]);
        assert_eq!(b.current_fg, Color32::from_rgb(10, 31, 0));
    }

    #[test]
    #[ignore = "BUG: background SGR 48;5;n / 48;2;r;g;b leaks n/r/g/b into the foreground color"]
    fn sgr_background_256_does_not_change_foreground() {
        let mut b = ScreenBuffer::new(1, 1);
        b.set_sgr(&[48, 5, 31]);
        assert_eq!(b.current_fg, DEFAULT_FG);
    }

    #[test]
    fn cell_default_is_blank() {
        let c = Cell::default();
        assert_eq!(c.ch, ' ');
        assert_eq!(c.fg, DEFAULT_FG);
        assert!(!c.bold);
        assert_eq!(c.clone().ch, ' ');
    }
}
