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
