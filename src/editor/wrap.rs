//! Soft word wrap: splitting long lines into visual rows, the cached
//! line → visual-row layout, and mapping between buffer and screen positions.
//!
//! The editor font is monospace, so wrapping is done by character count: each
//! visual row holds at most `cols` chars. Rows break after the last whitespace
//! that fits, or hard-break mid-word when a word is longer than a row. A column
//! sitting exactly on a row boundary belongs to the row that starts there.

use super::buffer::Buffer;
use super::Editor;

/// Start column of every visual row of a line wrapped at `cols` chars (always
/// begins with 0; a line that fits yields `[0]`).
pub(super) fn wrap_starts(chars: &[char], cols: usize) -> Vec<usize> {
    let cols = cols.max(1);
    let mut starts = vec![0];
    let mut s = 0;
    while chars.len() - s > cols {
        // Break after the last whitespace within the row, else mid-word.
        let brk = (s + 1..=s + cols)
            .rev()
            .find(|&p| chars[p - 1].is_whitespace())
            .unwrap_or(s + cols);
        starts.push(brk);
        s = brk;
    }
    starts
}

/// Up/Down by one visual row: within a wrapped line first, then into the
/// neighbouring line's nearest row, keeping the column offset within the row
/// (clamped to the target row). With `select` the selection is extended.
pub(super) fn move_visual(
    cur: &mut super::cursor::Cursor,
    buffer: &Buffer,
    cols: usize,
    down: bool,
    select: bool,
) {
    if select {
        cur.start_selection();
    } else {
        cur.clear_selection();
    }
    let starts_of = |row: usize| -> (Vec<usize>, usize) {
        let chars: Vec<char> = buffer.line(row).chars().collect();
        (wrap_starts(&chars, cols), chars.len())
    };
    let (starts, _) = starts_of(cur.row);
    let seg = starts.partition_point(|&s| s <= cur.col) - 1;
    let vcol = cur.col - starts[seg];
    let (row, seg) = if down {
        if seg + 1 < starts.len() {
            (cur.row, seg + 1)
        } else if cur.row + 1 < buffer.num_lines() {
            (cur.row + 1, 0)
        } else {
            return;
        }
    } else if seg > 0 {
        (cur.row, seg - 1)
    } else if cur.row > 0 {
        (cur.row - 1, usize::MAX)
    } else {
        return;
    };
    let (starts, len) = starts_of(row);
    let seg = seg.min(starts.len() - 1);
    let limit = match starts.get(seg + 1) {
        Some(&next) => next - 1,
        None => len,
    };
    cur.row = row;
    cur.col = (starts[seg] + vcol).min(limit);
    cur.desired_col = cur.col;
}

/// Visual-row layout of the whole buffer, recomputed only when the content or
/// the wrap width changes.
#[derive(Default)]
pub(super) struct WrapLayout {
    /// Wrap width in chars; 0 = word wrap off.
    pub(super) cols: usize,
    /// (content_version, num_lines, rope_len) the layout was built for. The
    /// sizes guard against edits that don't bump the version.
    key: Option<(i32, usize, usize)>,
    /// `row_start[i]` = first visual row of line `i`; the last entry is the
    /// total row count.
    row_start: Vec<usize>,
}

impl WrapLayout {
    pub(super) fn active(&self) -> bool {
        self.cols > 0
    }

    pub(super) fn disable(&mut self) {
        self.cols = 0;
        self.key = None;
        self.row_start.clear();
    }

    pub(super) fn invalidate(&mut self) {
        self.key = None;
    }

    /// Rebuild the layout for `cols` if the buffer or width changed.
    pub(super) fn update(&mut self, buffer: &Buffer, cols: usize, version: i32) {
        let cols = cols.max(1);
        let key = (version, buffer.num_lines(), buffer.rope_len());
        if self.cols == cols && self.key == Some(key) {
            return;
        }
        self.cols = cols;
        self.key = Some(key);
        self.row_start.clear();
        self.row_start.reserve(key.1 + 1);
        let mut row = 0;
        for i in 0..key.1 {
            self.row_start.push(row);
            // Fast path: a line that fits (length incl. newline) is one row.
            row += if buffer.line_char_len_fast(i) <= cols {
                1
            } else {
                let chars: Vec<char> = buffer.line(i).chars().collect();
                wrap_starts(&chars, cols).len()
            };
        }
        self.row_start.push(row);
    }

    fn lines(&self) -> usize {
        self.row_start.len().saturating_sub(1)
    }

    /// First visual row of `line` (clamped to the end of the layout).
    pub(super) fn line_top(&self, line: usize) -> usize {
        match self.row_start.get(line) {
            Some(&r) => r,
            None => self.row_start.last().copied().unwrap_or(line),
        }
    }

    /// Number of visual rows `line` occupies.
    pub(super) fn rows(&self, line: usize) -> usize {
        match (self.row_start.get(line), self.row_start.get(line + 1)) {
            (Some(a), Some(b)) => b - a,
            _ => 1,
        }
    }

    pub(super) fn total_rows(&self) -> usize {
        self.row_start.last().copied().unwrap_or(0)
    }

    /// Logical line containing visual row `vrow`, and the row index within that
    /// line. Rows past the end map to the last line.
    pub(super) fn line_at_row(&self, vrow: usize) -> (usize, usize) {
        if self.lines() == 0 {
            return (0, 0);
        }
        let line = self
            .row_start
            .partition_point(|&r| r <= vrow)
            .saturating_sub(1)
            .min(self.lines() - 1);
        (line, vrow - self.row_start[line])
    }
}

impl Editor {
    /// Visual row (and column within it) where buffer position (row, col) is
    /// drawn. Without word wrap this is the identity.
    pub(super) fn visual_pos(&self, row: usize, col: usize) -> (usize, usize) {
        if !self.wrap.active() {
            return (row, col);
        }
        let chars: Vec<char> = self.buffer.line(row).chars().collect();
        let starts = wrap_starts(&chars, self.wrap.cols);
        let seg = starts.partition_point(|&s| s <= col) - 1;
        (self.wrap.line_top(row) + seg, col - starts[seg])
    }

    /// Buffer position under a point, given relative to the editor rect's
    /// top-left. Without wrap the column is not clamped to the line length
    /// (callers clamp it); with wrap it is clamped to the clicked visual row.
    pub(super) fn hit_test(
        &self,
        local: egui::Vec2,
        gutter_width: f32,
        line_height: f32,
        char_width: f32,
    ) -> (usize, usize) {
        let last = self.buffer.num_lines().saturating_sub(1);
        if !self.wrap.active() {
            let row = ((local.y + self.scroll_offset.y) / line_height) as usize;
            let x_in_text = (local.x - gutter_width + self.scroll_offset.x).max(0.0);
            return (row.min(last), (x_in_text / char_width).round() as usize);
        }
        let vrow = ((local.y + self.scroll_offset.y).max(0.0) / line_height) as usize;
        let (mut line, mut seg) = self.wrap.line_at_row(vrow);
        if line > last {
            (line, seg) = (last, usize::MAX);
        }
        let chars: Vec<char> = self.buffer.line(line).chars().collect();
        let starts = wrap_starts(&chars, self.wrap.cols);
        let seg = seg.min(starts.len() - 1);
        // Clicking past the end of a wrapped row lands before its last char, so
        // the caret stays on that row.
        let limit = match starts.get(seg + 1) {
            Some(&next) => next - 1,
            None => chars.len(),
        };
        let x_in_text = (local.x - gutter_width).max(0.0);
        let col = starts[seg] + (x_in_text / char_width).round() as usize;
        (line, col.min(limit))
    }
}

/// Frame-constant inputs for painting a wrapped line.
pub(super) struct WrapPaint<'a> {
    pub painter: &'a egui::Painter,
    pub plugin_manager: &'a crate::plugin::manager::PluginManager,
    pub font_id: egui::FontId,
    pub line_height: f32,
    pub char_width: f32,
    pub rect: egui::Rect,
    pub x_start: f32,
    pub fg_color: egui::Color32,
    pub cursor_color: egui::Color32,
    pub accent_color: egui::Color32,
    pub find_highlight: egui::Color32,
    pub find_highlight_active: egui::Color32,
    pub palette: crate::ui::theme::Palette,
    pub sel_range: Option<((usize, usize), (usize, usize))>,
    pub active_block: Option<(usize, usize, usize)>,
}

impl Editor {
    /// Paint the body of logical line `line_idx` (top visual row at `y`) split
    /// into visual rows: find/occurrence/selection highlights, cursors, indent
    /// guides, bracket match, text, Ctrl+hover underline and diagnostics. The
    /// gutter is painted by the caller, once, on the first row. Mirrors the
    /// unwrapped painting in `show`; there is no horizontal scroll in this mode.
    pub(super) fn paint_wrapped_line(
        &mut self,
        ui: &egui::Ui,
        p: &WrapPaint<'_>,
        line_idx: usize,
        line: &str,
        y: f32,
    ) {
        let lh = p.line_height;
        let chars: Vec<char> = line.chars().collect();
        let starts = wrap_starts(&chars, self.wrap.cols);
        let seg_end = |i: usize| starts.get(i + 1).copied().unwrap_or(chars.len());
        let seg_of = |col: usize| starts.partition_point(|&s| s <= col) - 1;
        let measure = |from: usize, to: usize| -> f32 {
            if to <= from {
                return 0.0;
            }
            ui.fonts(|f| {
                f.layout_no_wrap(
                    chars[from..to].iter().collect(),
                    p.font_id.clone(),
                    egui::Color32::WHITE,
                )
                .size()
                .x
            })
        };
        // Top-left of the caret slot at `col`.
        let caret = |col: usize| -> egui::Pos2 {
            let col = col.min(chars.len());
            let seg = seg_of(col);
            egui::pos2(p.x_start + measure(starts[seg], col), y + seg as f32 * lh)
        };
        // One rect per visual row covered by the char range [a, b).
        let spans = |a: usize, b: usize| -> Vec<egui::Rect> {
            let mut out = Vec::new();
            for (i, &s) in starts.iter().enumerate() {
                let (lo, hi) = (a.max(s), b.min(seg_end(i)));
                if lo < hi {
                    let ry = y + i as f32 * lh;
                    out.push(egui::Rect::from_min_max(
                        egui::pos2(p.x_start + measure(s, lo), ry),
                        egui::pos2(p.x_start + measure(s, hi), ry + lh),
                    ));
                }
            }
            out
        };
        let selection_cols = |range: ((usize, usize), (usize, usize))| {
            let ((sr, sc), (er, ec)) = range;
            if line_idx < sr || line_idx > er {
                return None;
            }
            let start = if line_idx == sr { sc } else { 0 };
            let end = if line_idx == er { ec } else { chars.len() };
            Some((start, end))
        };

        // Find matches (sorted by row, so this line's are one contiguous slice).
        let first = self.find_matches.partition_point(|m| m.row < line_idx);
        for (i, m) in self.find_matches[first..]
            .iter()
            .take_while(|m| m.row == line_idx)
            .enumerate()
        {
            let color = if first + i == self.find_current {
                p.find_highlight_active
            } else {
                p.find_highlight
            };
            for r in spans(m.start, m.end) {
                let r = egui::Rect::from_min_size(
                    egui::pos2(r.min.x, r.min.y + 1.0),
                    egui::vec2(r.width().max(4.0), lh - 2.0),
                );
                p.painter.rect_filled(r, 2.0, color);
            }
        }

        // Word occurrences
        for &(occ_row, occ_start, occ_end) in &self.word_occurrences {
            if occ_row == line_idx {
                for r in spans(occ_start, occ_end) {
                    p.painter.rect_filled(r, 2.0, p.palette.accent_muted);
                }
            }
        }

        // Selections (primary + extra cursors)
        let ranges = p.sel_range.into_iter().chain(
            self.extra_cursors
                .iter()
                .filter_map(|c| c.selection_range()),
        );
        for (start, end) in ranges.filter_map(selection_cols) {
            for r in spans(start, end) {
                p.painter.rect_filled(r, 0.0, p.palette.selection);
            }
        }

        // Primary cursor (blinking) + autocomplete anchor
        let (cur_row, cur_col) = self.cursor.position();
        if line_idx == cur_row {
            let c = caret(cur_col);
            let blink_ms = self.cursor_blink_epoch.elapsed().as_millis() % 1060;
            let cursor_visible = blink_ms < 530;
            if cursor_visible {
                p.painter.line_segment(
                    [c, egui::pos2(c.x, c.y + lh)],
                    egui::Stroke::new(2.0_f32, p.cursor_color),
                );
            }
            let next_transition = if cursor_visible {
                530 - blink_ms
            } else {
                1060 - blink_ms
            };
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(
                    next_transition as u64 + 1,
                ));
            self.autocomplete.cursor_screen_pos = egui::pos2(c.x, c.y + lh);
        }
        for extra in &self.extra_cursors {
            let (er, ec) = extra.position();
            if er == line_idx {
                let c = caret(ec);
                p.painter.line_segment(
                    [c, egui::pos2(c.x, c.y + lh)],
                    egui::Stroke::new(2.0_f32, p.accent_color),
                );
            }
        }

        // Indent guides (first visual row only — continuation rows aren't indented)
        let ind_size = self.detected_indent_size.max(1);
        let leading = if self.detected_indent_spaces {
            chars.iter().take_while(|&&c| c == ' ').count()
        } else {
            chars.iter().take_while(|&&c| c == '\t').count() * ind_size
        };
        for g in 1..=leading / ind_size {
            let gx = p.x_start + (g * ind_size) as f32 * p.char_width - p.char_width * 0.5 - 2.0;
            if gx < p.x_start || gx > p.rect.max.x {
                continue;
            }
            let guide_color = match p.active_block {
                Some((lvl, s, e)) if g == lvl && line_idx >= s && line_idx <= e => {
                    p.palette.accent_muted
                }
                _ => egui::Color32::from_rgba_unmultiplied(130, 130, 145, 50),
            };
            p.painter.line_segment(
                [egui::pos2(gx, y), egui::pos2(gx, y + lh)],
                egui::Stroke::new(1.0_f32, guide_color),
            );
        }

        // Bracket match
        if let Some((or, oc, cr, cc)) = self.bracket_match {
            for (br, bc) in [(or, oc), (cr, cc)] {
                if br == line_idx {
                    let r = egui::Rect::from_min_size(caret(bc), egui::vec2(p.char_width, lh));
                    p.painter.rect_filled(
                        r,
                        2.0,
                        egui::Color32::from_rgba_premultiplied(100, 160, 255, 50),
                    );
                    p.painter.rect_stroke(
                        r,
                        2.0,
                        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(100, 160, 255)),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }

        // Syntax-highlighted text, one galley per visual row.
        let tokens = self
            .highlighter
            .tokens_for_line(line_idx, line, Some(p.plugin_manager));
        let mut jobs: Vec<egui::text::LayoutJob> =
            starts.iter().map(|_| Default::default()).collect();
        let mut pos = 0;
        for tok in &tokens {
            let tchars: Vec<char> = tok.text.chars().collect();
            let mut i = 0;
            while i < tchars.len() {
                let seg = seg_of(pos);
                let take = seg_end(seg).saturating_sub(pos).clamp(1, tchars.len() - i);
                let piece: String = tchars[i..i + take].iter().collect();
                jobs[seg].append(
                    &piece,
                    0.0,
                    egui::TextFormat {
                        font_id: p.font_id.clone(),
                        color: tok.kind.color(),
                        ..Default::default()
                    },
                );
                i += take;
                pos += take;
            }
        }
        let text_clip = egui::Rect::from_min_max(
            egui::pos2(p.x_start, p.rect.min.y),
            egui::pos2(p.rect.max.x, p.rect.max.y),
        );
        for (i, job) in jobs.into_iter().enumerate() {
            let galley = ui.fonts(|f| f.layout_job(job));
            p.painter.with_clip_rect(text_clip).galley(
                egui::pos2(p.x_start, y + i as f32 * lh + lh * 0.15),
                galley,
                p.fg_color,
            );
        }

        // Ctrl+hover underline
        if let Some((hover_row, hover_start, hover_end)) = self.ctrl_hover_word_bounds {
            if hover_row == line_idx {
                for r in spans(hover_start, hover_end) {
                    p.painter.line_segment(
                        [
                            egui::pos2(r.min.x, r.max.y - 2.0),
                            egui::pos2(r.max.x, r.max.y - 2.0),
                        ],
                        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(100, 160, 255)),
                    );
                }
            }
        }

        // Diagnostic squiggles
        for diag in &self.diagnostics {
            if diag.line as usize != line_idx {
                continue;
            }
            let start = diag.col as usize;
            let end = if diag.end_col > diag.col {
                diag.end_col as usize
            } else {
                start + 1
            };
            let color = match diag.severity {
                crate::lsp::client::DiagSeverity::Error => p.palette.error,
                crate::lsp::client::DiagSeverity::Warning => p.palette.warning,
                _ => p.palette.info,
            };
            for r in spans(start, end) {
                let underline_y = r.max.y - 2.0;
                let (amp, period) = (1.5_f32, 4.0_f32);
                let mut x = r.min.x;
                while x < r.max.x {
                    let y1 = underline_y + amp * ((x / period * std::f32::consts::PI).sin());
                    let x2 = (x + period / 2.0).min(r.max.x);
                    p.painter.line_segment(
                        [egui::pos2(x, y1), egui::pos2(x2, underline_y - amp)],
                        egui::Stroke::new(1.0_f32, color),
                    );
                    x = x2;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts(s: &str, cols: usize) -> Vec<usize> {
        wrap_starts(&s.chars().collect::<Vec<_>>(), cols)
    }

    #[test]
    fn short_and_exact_fit_lines_are_one_row() {
        assert_eq!(starts("", 5), vec![0]);
        assert_eq!(starts("abc", 5), vec![0]);
        assert_eq!(starts("abcde", 5), vec![0]);
    }

    #[test]
    fn breaks_after_last_whitespace_that_fits() {
        // "aaa bbb ccc" at 8 cols: "aaa bbb " | "ccc"
        assert_eq!(starts("aaa bbb ccc", 8), vec![0, 8]);
        // "aaa bbb ccc" at 6 cols: "aaa " | "bbb " | "ccc"
        assert_eq!(starts("aaa bbb ccc", 6), vec![0, 4, 8]);
    }

    #[test]
    fn long_words_hard_break() {
        assert_eq!(starts("abcdefghij", 4), vec![0, 4, 8]);
        assert_eq!(starts("ab cdefghijk", 4), vec![0, 3, 7, 11]);
    }

    #[test]
    fn zero_cols_is_treated_as_one() {
        assert_eq!(starts("abc", 0), vec![0, 1, 2]);
    }

    #[test]
    fn move_visual_steps_through_rows_then_lines() {
        // Line 0 at 5 cols: "aaaa " | "bbbb " | "cc"; line 1: "xy".
        let buf = Buffer::from_str("aaaa bbbb cc\nxy");
        let mut c = super::super::cursor::Cursor::new();
        c.set_position(0, 2);
        move_visual(&mut c, &buf, 5, true, false);
        assert_eq!(c.position(), (0, 7));
        move_visual(&mut c, &buf, 5, true, false);
        assert_eq!(c.position(), (0, 12), "clamped to the short last row");
        move_visual(&mut c, &buf, 5, true, false);
        assert_eq!(c.position(), (1, 2));
        move_visual(&mut c, &buf, 5, true, false);
        assert_eq!(c.position(), (1, 2), "no row below");
        // Up from line 1 lands on line 0's last row.
        move_visual(&mut c, &buf, 5, false, false);
        assert_eq!(c.position(), (0, 12));
        // Shift extends the selection.
        c.set_position(0, 12);
        move_visual(&mut c, &buf, 5, false, true);
        assert_eq!(c.position(), (0, 7));
        assert_eq!(c.sel_anchor, Some((0, 12)));
        c.set_position(0, 4);
        move_visual(&mut c, &buf, 5, false, false);
        assert_eq!(c.position(), (0, 4), "no row above");
        assert!(c.sel_anchor.is_none());
        // Moving onto a shorter row clamps: to the line end on a last row, and
        // before the next row's start on a wrapped one.
        // "ab " | "cdefg" | "h"
        let buf = Buffer::from_str("ab cdefgh\nxyzw");
        c.set_position(1, 4);
        move_visual(&mut c, &buf, 5, false, false);
        assert_eq!(c.position(), (0, 9));
        c.set_position(0, 7);
        move_visual(&mut c, &buf, 5, false, false);
        assert_eq!(c.position(), (0, 2));
    }

    #[test]
    fn layout_counts_rows_and_maps_both_ways() {
        let buf = Buffer::from_str("short\naaaa bbbb cccc\n\nxyz");
        let mut w = WrapLayout::default();
        w.update(&buf, 5, 0);
        // Line 1 wraps into "aaaa " | "bbbb " | "cccc".
        assert_eq!(w.rows(0), 1);
        assert_eq!(w.rows(1), 3);
        assert_eq!(w.line_top(2), 4);
        assert_eq!(w.line_top(3), 5);
        assert_eq!(w.total_rows(), 6);
        assert_eq!(w.line_at_row(0), (0, 0));
        assert_eq!(w.line_at_row(2), (1, 1));
        assert_eq!(w.line_at_row(3), (1, 2));
        assert_eq!(w.line_at_row(5), (3, 0));
        assert_eq!(w.line_at_row(99), (3, 94), "past the end -> last line");
        // Out-of-range lookups stay in bounds.
        assert_eq!(w.line_top(50), 6);
        assert_eq!(w.rows(50), 1);
    }

    #[test]
    fn layout_rebuilds_on_size_change_even_without_version_bump() {
        let mut buf = Buffer::from_str("a");
        let mut w = WrapLayout::default();
        w.update(&buf, 4, 7);
        assert_eq!(w.total_rows(), 1);
        buf.insert_str(0, 1, "bcdefgh");
        w.update(&buf, 4, 7);
        assert_eq!(w.total_rows(), 2);
        w.update(&buf, 2, 7);
        assert_eq!(w.total_rows(), 4, "width change rebuilds");
        w.disable();
        assert!(!w.active());
        assert_eq!(w.total_rows(), 0);
        let empty = WrapLayout::default();
        assert_eq!(empty.line_at_row(3), (0, 0));
    }
}
