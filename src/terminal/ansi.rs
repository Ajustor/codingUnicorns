use super::screen_buffer::ScreenBuffer;
use vte::{Params, Perform};

pub(super) struct AnsiPerformer {
    pub(super) buf: ScreenBuffer,
    /// Bytes to write back to the PTY in answer to terminal queries (e.g. the
    /// cursor-position report that replies to `ESC[6n`). Drained by `Terminal::update`.
    pub(super) responses: Vec<u8>,
}

impl AnsiPerformer {
    pub(super) fn new() -> Self {
        Self {
            buf: ScreenBuffer::new(200, 50),
            responses: Vec::new(),
        }
    }
}

impl Perform for AnsiPerformer {
    fn print(&mut self, c: char) {
        self.buf.write_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => self.buf.line_feed(),
            b'\r' => self.buf.carriage_return(),
            b'\x08' if self.buf.cursor_col > 0 => {
                self.buf.cursor_col -= 1;
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, _: &[u8], _: bool, action: char) {
        let ns: Vec<u16> = params
            .iter()
            .map(|p| p.first().copied().unwrap_or(0))
            .collect();
        let n0 = ns.first().copied().unwrap_or(0);
        let n1 = ns.get(1).copied().unwrap_or(0);
        // TEMP DEBUG — log only DSR queries (ESC[5n / ESC[6n) to keep noise low.
        if action == 'n' {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(std::env::temp_dir().join("cu_csi.txt"))
            {
                use std::io::Write;
                let _ = writeln!(f, "DSR query 'n' {ns:?}");
            }
        }
        match action {
            'A' => self.buf.move_cursor('A', n0.max(1) as usize),
            'B' => self.buf.move_cursor('B', n0.max(1) as usize),
            'C' => self.buf.move_cursor('C', n0.max(1) as usize),
            'D' => self.buf.move_cursor('D', n0.max(1) as usize),
            'H' | 'f' => self.buf.set_cursor_pos(n0 as usize, n1 as usize),
            'J' => self.buf.erase_display(n0),
            'K' => self.buf.erase_line(n0),
            'm' => self.buf.set_sgr(&ns),
            'n' => {
                // Device Status Report. ConPTY sends `ESC[6n` during startup and waits for a
                // cursor-position reply; without it, shells like PowerShell never print a prompt.
                if n0 == 6 {
                    let row = self.buf.cursor_row + 1;
                    let col = self.buf.cursor_col + 1;
                    self.responses
                        .extend_from_slice(format!("\x1b[{row};{col}R").as_bytes());
                } else if n0 == 5 {
                    self.responses.extend_from_slice(b"\x1b[0n");
                }
            }
            'l' | 'h' => {}
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _: &[u8], _: bool, byte: u8) {
        if byte == b'M' && self.buf.cursor_row > 0 {
            self.buf.cursor_row -= 1;
        }
    }

    fn hook(&mut self, _: &Params, _: &[u8], _: bool, _: char) {}
    fn put(&mut self, _: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, _: &[&[u8]], _: bool) {}
}

#[cfg(test)]
mod tests {
    use super::super::colors::{ansi_color, color_256};
    use super::super::screen_buffer::DEFAULT_FG;
    use super::*;
    use vte::Parser;

    struct Term {
        parser: Parser,
        p: AnsiPerformer,
    }

    impl Term {
        fn new() -> Self {
            Self {
                parser: Parser::new(),
                p: AnsiPerformer::new(),
            }
        }
        fn feed(&mut self, bytes: &[u8]) -> &mut Self {
            for &b in bytes {
                self.parser.advance(&mut self.p, b);
            }
            self
        }
        fn row(&self, r: usize) -> String {
            self.p.buf.rows[r]
                .iter()
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .to_string()
        }
        fn cursor(&self) -> (usize, usize) {
            (self.p.buf.cursor_row, self.p.buf.cursor_col)
        }
    }

    #[test]
    fn performer_starts_with_200x50_screen() {
        let p = AnsiPerformer::new();
        assert_eq!(p.buf.rows.len(), 50);
        assert_eq!(p.buf.cols, 200);
        assert!(p.responses.is_empty());
    }

    #[test]
    fn prints_text_and_handles_crlf() {
        let mut t = Term::new();
        t.feed(b"hello\r\nworld");
        assert_eq!(t.row(0), "hello");
        assert_eq!(t.row(1), "world");
        assert_eq!(t.cursor(), (1, 5));
    }

    #[test]
    fn bare_lf_keeps_column() {
        let mut t = Term::new();
        t.feed(b"ab\ncd");
        assert_eq!(t.row(0), "ab");
        assert_eq!(t.row(1), "  cd");
    }

    #[test]
    fn carriage_return_overwrites_line() {
        let mut t = Term::new();
        t.feed(b"12345\rab");
        assert_eq!(t.row(0), "ab345");
    }

    #[test]
    fn backspace_moves_left_but_not_past_zero() {
        let mut t = Term::new();
        t.feed(b"abc\x08\x08X");
        assert_eq!(t.row(0), "aXc");
        t.feed(b"\r\x08\x08Y");
        assert_eq!(t.row(0), "YXc");
    }

    #[test]
    fn other_control_bytes_are_ignored() {
        let mut t = Term::new();
        t.feed(b"a\x07\tb\x00c");
        assert_eq!(t.row(0), "abc");
    }

    #[test]
    fn utf8_split_across_chunks() {
        let mut t = Term::new();
        let s = "é€🦄".as_bytes();
        // Feed one byte at a time as if each arrived in its own PTY chunk.
        for b in s {
            t.feed(&[*b]);
        }
        assert_eq!(t.row(0), "é€🦄");
        assert_eq!(t.cursor(), (0, 3));
    }

    #[test]
    fn cursor_movement_sequences() {
        let mut t = Term::new();
        t.feed(b"\x1b[5;10H");
        assert_eq!(t.cursor(), (4, 9));
        t.feed(b"\x1b[A");
        assert_eq!(t.cursor(), (3, 9));
        t.feed(b"\x1b[2B");
        assert_eq!(t.cursor(), (5, 9));
        t.feed(b"\x1b[3C");
        assert_eq!(t.cursor(), (5, 12));
        t.feed(b"\x1b[0D"); // 0 is treated as 1
        assert_eq!(t.cursor(), (5, 11));
        t.feed(b"\x1b[H");
        assert_eq!(t.cursor(), (0, 0));
        t.feed(b"\x1b[2;3f");
        assert_eq!(t.cursor(), (1, 2));
        t.feed(b"\x1b[999;999H");
        assert_eq!(t.cursor(), (49, 199));
    }

    #[test]
    fn erase_sequences() {
        let mut t = Term::new();
        t.feed(b"line0\r\nline1\r\nline2");
        t.feed(b"\x1b[2;3H\x1b[K");
        assert_eq!(t.row(1), "li");
        t.feed(b"\x1b[1K");
        assert_eq!(t.row(1), "");
        t.feed(b"\x1b[J");
        assert_eq!(t.row(2), "");
        assert_eq!(t.row(0), "line0");
        t.feed(b"\x1b[2J");
        assert_eq!(t.row(0), "");
        assert_eq!(t.cursor(), (0, 0));
    }

    #[test]
    fn erase_line_full() {
        let mut t = Term::new();
        t.feed(b"abc\x1b[2K");
        assert_eq!(t.row(0), "");
        assert_eq!(t.cursor(), (0, 3));
    }

    #[test]
    fn sgr_sequences_color_cells() {
        let mut t = Term::new();
        t.feed(b"\x1b[1;31mR\x1b[0mN\x1b[38;5;46mG\x1b[92mB");
        let cells = &t.p.buf.rows[0];
        assert_eq!(cells[0].fg, ansi_color(1, false));
        assert!(cells[0].bold);
        assert_eq!(cells[1].fg, DEFAULT_FG);
        assert!(!cells[1].bold);
        assert_eq!(cells[2].fg, color_256(46));
        assert_eq!(cells[3].fg, ansi_color(2, true));
    }

    #[test]
    fn sgr_with_no_params_resets() {
        let mut t = Term::new();
        t.feed(b"\x1b[1;33m\x1b[mX");
        assert_eq!(t.p.buf.rows[0][0].fg, DEFAULT_FG);
        assert!(!t.p.buf.rows[0][0].bold);
    }

    #[test]
    #[ignore = "BUG: 24-bit SGR (38;2;r;g;b) unsupported; components misread as SGR codes"]
    fn sgr_truecolor_sequence() {
        let mut t = Term::new();
        t.feed(b"\x1b[38;2;10;31;0mX");
        assert_eq!(t.p.buf.rows[0][0].fg, egui::Color32::from_rgb(10, 31, 0));
    }

    #[test]
    fn wraps_long_lines_at_200_columns() {
        let mut t = Term::new();
        let line = "x".repeat(205);
        t.feed(line.as_bytes());
        assert_eq!(t.row(0).len(), 200);
        assert_eq!(t.row(1), "xxxxx");
        assert_eq!(t.cursor(), (1, 5));
    }

    #[test]
    fn output_beyond_screen_scrolls() {
        let mut t = Term::new();
        for i in 0..55 {
            t.feed(format!("{i}\r\n").as_bytes());
        }
        assert_eq!(t.p.buf.scrollback.len(), 6);
        assert_eq!(t.p.buf.scrollback[0][0].ch, '0');
        assert_eq!(t.row(0), "6");
        assert_eq!(t.row(48), "54");
        assert_eq!(t.cursor(), (49, 0));
    }

    #[test]
    fn reverse_index_moves_up_but_not_above_top() {
        let mut t = Term::new();
        t.feed(b"\x1b[3;1H\x1bM");
        assert_eq!(t.cursor(), (1, 0));
        t.feed(b"\x1bM\x1bM\x1bM");
        assert_eq!(t.cursor(), (0, 0));
        // Other ESC finals are ignored.
        t.feed(b"\x1b7\x1b8");
        assert_eq!(t.cursor(), (0, 0));
    }

    #[test]
    fn device_status_reports_produce_responses() {
        let mut t = Term::new();
        t.feed(b"\x1b[4;7H\x1b[6n");
        assert_eq!(t.p.responses, b"\x1b[4;7R");
        t.p.responses.clear();
        t.feed(b"\x1b[5n");
        assert_eq!(t.p.responses, b"\x1b[0n");
        t.p.responses.clear();
        t.feed(b"\x1b[n\x1b[99n");
        assert!(t.p.responses.is_empty());
    }

    #[test]
    fn mode_set_reset_osc_and_dcs_are_ignored() {
        let mut t = Term::new();
        t.feed(b"\x1b[?25l\x1b[?1049h\x1b]0;title\x07\x1bP1$qm\x1b\\\x1b[5Sok");
        assert_eq!(t.row(0), "ok");
        assert_eq!(t.cursor(), (0, 2));
        assert!(t.p.responses.is_empty());
    }
}
