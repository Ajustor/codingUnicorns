use super::buffer::Buffer;

#[derive(Clone)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    pub desired_col: usize,
    pub sel_anchor: Option<(usize, usize)>,
}

impl Cursor {
    pub fn new() -> Self {
        Self {
            row: 0,
            col: 0,
            desired_col: 0,
            sel_anchor: None,
        }
    }

    pub fn position(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    pub fn set_position(&mut self, row: usize, col: usize) {
        self.row = row;
        self.col = col;
        self.desired_col = col;
    }

    pub fn start_selection(&mut self) {
        if self.sel_anchor.is_none() {
            self.sel_anchor = Some((self.row, self.col));
        }
    }

    pub fn clear_selection(&mut self) {
        self.sel_anchor = None;
    }

    pub fn has_selection(&self) -> bool {
        self.sel_anchor.is_some()
    }

    /// Returns normalized (start, end) in (row, col) order.
    pub fn selection_range(&self) -> Option<((usize, usize), (usize, usize))> {
        let anchor = self.sel_anchor?;
        let cursor = (self.row, self.col);
        if anchor <= cursor {
            Some((anchor, cursor))
        } else {
            Some((cursor, anchor))
        }
    }

    pub fn move_left(&mut self, buf: &Buffer) {
        self.clear_selection();
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = buf.line_len(self.row);
        }
        self.desired_col = self.col;
    }

    pub fn move_right(&mut self, buf: &Buffer) {
        self.clear_selection();
        let line_len = buf.line_len(self.row);
        if self.col < line_len {
            self.col += 1;
        } else if self.row + 1 < buf.num_lines() {
            self.row += 1;
            self.col = 0;
        }
        self.desired_col = self.col;
    }

    pub fn move_up(&mut self, buf: &Buffer) {
        self.clear_selection();
        if self.row > 0 {
            self.row -= 1;
            self.col = self.desired_col.min(buf.line_len(self.row));
        }
    }

    pub fn move_down(&mut self, buf: &Buffer) {
        self.clear_selection();
        if self.row + 1 < buf.num_lines() {
            self.row += 1;
            self.col = self.desired_col.min(buf.line_len(self.row));
        }
    }

    pub fn move_left_select(&mut self, buf: &Buffer) {
        self.start_selection();
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = buf.line_len(self.row);
        }
        self.desired_col = self.col;
    }

    pub fn move_right_select(&mut self, buf: &Buffer) {
        self.start_selection();
        let line_len = buf.line_len(self.row);
        if self.col < line_len {
            self.col += 1;
        } else if self.row + 1 < buf.num_lines() {
            self.row += 1;
            self.col = 0;
        }
        self.desired_col = self.col;
    }

    pub fn move_up_select(&mut self, buf: &Buffer) {
        self.start_selection();
        if self.row > 0 {
            self.row -= 1;
            self.col = self.desired_col.min(buf.line_len(self.row));
        }
    }

    pub fn move_down_select(&mut self, buf: &Buffer) {
        self.start_selection();
        if self.row + 1 < buf.num_lines() {
            self.row += 1;
            self.col = self.desired_col.min(buf.line_len(self.row));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(row: usize, col: usize) -> Cursor {
        let mut c = Cursor::new();
        c.set_position(row, col);
        c
    }

    #[test]
    fn new_cursor_is_at_origin_without_selection() {
        let c = Cursor::new();
        assert_eq!(c.position(), (0, 0));
        assert_eq!(c.desired_col, 0);
        assert!(!c.has_selection());
        assert!(c.selection_range().is_none());
    }

    #[test]
    fn set_position_updates_desired_col() {
        let c = at(3, 7);
        assert_eq!(c.position(), (3, 7));
        assert_eq!(c.desired_col, 7);
    }

    #[test]
    fn start_selection_keeps_first_anchor() {
        let mut c = at(1, 2);
        c.start_selection();
        c.set_position(4, 0);
        c.start_selection();
        assert_eq!(c.sel_anchor, Some((1, 2)));
        assert!(c.has_selection());
        c.clear_selection();
        assert!(!c.has_selection());
    }

    #[test]
    fn selection_range_is_normalized() {
        let mut c = at(2, 5);
        c.start_selection();
        c.set_position(1, 3);
        assert_eq!(c.selection_range(), Some(((1, 3), (2, 5))));
        c.set_position(2, 9);
        assert_eq!(c.selection_range(), Some(((2, 5), (2, 9))));
    }

    #[test]
    fn move_left_wraps_to_previous_line_end() {
        let buf = Buffer::from_str("abc\nde");
        let mut c = at(1, 1);
        c.start_selection();
        c.move_left(&buf);
        assert_eq!(c.position(), (1, 0));
        assert!(!c.has_selection(), "plain movement clears selection");
        c.move_left(&buf);
        assert_eq!(c.position(), (0, 3));
        assert_eq!(c.desired_col, 3);
        c.set_position(0, 0);
        c.move_left(&buf);
        assert_eq!(c.position(), (0, 0));
    }

    #[test]
    fn move_right_wraps_to_next_line_start() {
        let buf = Buffer::from_str("ab\nc");
        let mut c = at(0, 1);
        c.move_right(&buf);
        assert_eq!(c.position(), (0, 2));
        c.move_right(&buf);
        assert_eq!(c.position(), (1, 0));
        c.move_right(&buf);
        assert_eq!(c.position(), (1, 1));
        // End of buffer: stays put.
        c.move_right(&buf);
        assert_eq!(c.position(), (1, 1));
        assert_eq!(c.desired_col, 1);
    }

    #[test]
    fn vertical_movement_remembers_desired_column() {
        let buf = Buffer::from_str("long line\nab\nanother line");
        let mut c = at(0, 7);
        c.move_down(&buf);
        assert_eq!(c.position(), (1, 2), "clamped to short line");
        c.move_down(&buf);
        assert_eq!(c.position(), (2, 7), "desired column restored");
        c.move_down(&buf);
        assert_eq!(c.position(), (2, 7), "no line below");
        c.move_up(&buf);
        assert_eq!(c.position(), (1, 2));
        c.move_up(&buf);
        assert_eq!(c.position(), (0, 7));
        c.move_up(&buf);
        assert_eq!(c.position(), (0, 7), "no line above");
    }

    #[test]
    fn vertical_movement_clears_selection() {
        let buf = Buffer::from_str("a\nb");
        let mut c = at(0, 0);
        c.start_selection();
        c.move_down(&buf);
        assert!(!c.has_selection());
        c.start_selection();
        c.move_up(&buf);
        assert!(!c.has_selection());
    }

    #[test]
    fn select_left_and_right_extend_selection_across_lines() {
        let buf = Buffer::from_str("ab\ncd");
        let mut c = at(1, 0);
        c.move_left_select(&buf);
        assert_eq!(c.position(), (0, 2));
        assert_eq!(c.selection_range(), Some(((0, 2), (1, 0))));
        c.move_left_select(&buf);
        assert_eq!(c.selection_range(), Some(((0, 1), (1, 0))));

        let mut c = at(0, 2);
        c.move_right_select(&buf);
        assert_eq!(c.position(), (1, 0));
        c.move_right_select(&buf);
        assert_eq!(c.selection_range(), Some(((0, 2), (1, 1))));
        assert_eq!(c.desired_col, 1);
    }

    #[test]
    fn select_left_right_stop_at_buffer_edges() {
        let buf = Buffer::from_str("ab");
        let mut c = at(0, 0);
        c.move_left_select(&buf);
        assert_eq!(c.position(), (0, 0));
        assert_eq!(c.sel_anchor, Some((0, 0)));
        let mut c = at(0, 2);
        c.move_right_select(&buf);
        assert_eq!(c.position(), (0, 2));
    }

    #[test]
    fn select_up_and_down_extend_selection() {
        let buf = Buffer::from_str("hello\nhi\nworld");
        let mut c = at(0, 4);
        c.move_down_select(&buf);
        assert_eq!(c.position(), (1, 2));
        c.move_down_select(&buf);
        assert_eq!(c.position(), (2, 4));
        c.move_down_select(&buf);
        assert_eq!(c.position(), (2, 4));
        assert_eq!(c.selection_range(), Some(((0, 4), (2, 4))));

        c.move_up_select(&buf);
        assert_eq!(c.position(), (1, 2));
        c.move_up_select(&buf);
        c.move_up_select(&buf);
        assert_eq!(c.position(), (0, 4));
        assert_eq!(c.sel_anchor, Some((0, 4)));
    }
}
