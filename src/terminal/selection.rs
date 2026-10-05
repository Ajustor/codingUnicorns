use super::screen_buffer::Cell;

/// A cell position in the terminal's line space: scrollback lines first, then the
/// visible screen rows. Indices are stable while output scrolls (a screen row that
/// moves into scrollback keeps its index).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct GridPos {
    pub(super) line: usize,
    pub(super) col: usize,
}

impl GridPos {
    pub(super) fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

/// A stream (line-wrapping) selection; both ends are inclusive cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Selection {
    pub(super) anchor: GridPos,
    pub(super) head: GridPos,
}

impl Selection {
    pub(super) fn new(anchor: GridPos, head: GridPos) -> Self {
        Self { anchor, head }
    }

    fn ordered(&self) -> (GridPos, GridPos) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// The selected `[start, end)` cell range on `line`, a row of `width` cells.
    pub(super) fn cols_on_line(&self, line: usize, width: usize) -> Option<(usize, usize)> {
        let (s, e) = self.ordered();
        if line < s.line || line > e.line {
            return None;
        }
        let start = if line == s.line { s.col } else { 0 };
        let end = if line == e.line {
            e.col.saturating_add(1)
        } else {
            width
        };
        let (start, end) = (start.min(width), end.min(width));
        (start < end).then_some((start, end))
    }

    /// The selected text: each line's trailing spaces trimmed, lines joined by `\n`.
    /// `line_at` maps a line index to its cells.
    pub(super) fn text<'a>(&self, line_at: impl Fn(usize) -> Option<&'a [Cell]>) -> String {
        let (s, e) = self.ordered();
        let mut lines = Vec::with_capacity(e.line - s.line + 1);
        for line in s.line..=e.line {
            let Some(row) = line_at(line) else { break };
            let text: String = match self.cols_on_line(line, row.len()) {
                Some((a, b)) => row[a..b].iter().map(|c| c.ch).collect(),
                None => String::new(),
            };
            lines.push(text.trim_end().to_string());
        }
        lines.join("\n")
    }
}

#[derive(PartialEq, Eq)]
enum CharClass {
    Space,
    Word,
    Delimiter,
}

fn char_class(ch: char) -> CharClass {
    if ch.is_whitespace() {
        CharClass::Space
    } else if "()[]{}<>\"'`,;|".contains(ch) {
        CharClass::Delimiter
    } else {
        CharClass::Word
    }
}

/// The `[start, end)` cell range of the word under `col` (used for double-click).
/// Paths and URLs count as one word; a delimiter selects just itself; a run of
/// spaces selects the whole run.
pub(super) fn word_at(row: &[Cell], col: usize) -> (usize, usize) {
    let Some(cell) = row.get(col) else {
        return (col, col + 1);
    };
    let class = char_class(cell.ch);
    if class == CharClass::Delimiter {
        return (col, col + 1);
    }
    let same = |c: &Cell| char_class(c.ch) == class;
    let start = row[..col]
        .iter()
        .rposition(|c| !same(c))
        .map_or(0, |i| i + 1);
    let end = row[col..]
        .iter()
        .position(|c| !same(c))
        .map_or(row.len(), |i| col + i);
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(s: &str) -> Vec<Cell> {
        s.chars()
            .map(|ch| Cell {
                ch,
                ..Cell::default()
            })
            .collect()
    }

    fn sel(a: (usize, usize), b: (usize, usize)) -> Selection {
        Selection::new(GridPos::new(a.0, a.1), GridPos::new(b.0, b.1))
    }

    #[test]
    fn cols_on_line_single_and_multi_line() {
        let s = sel((1, 2), (1, 4));
        assert_eq!(s.cols_on_line(1, 10), Some((2, 5)));
        assert_eq!(s.cols_on_line(0, 10), None);
        assert_eq!(s.cols_on_line(2, 10), None);
        // Reversed drag gives the same range.
        let s = sel((3, 1), (1, 6));
        assert_eq!(s.cols_on_line(1, 10), Some((6, 10)));
        assert_eq!(s.cols_on_line(2, 10), Some((0, 10)));
        assert_eq!(s.cols_on_line(3, 10), Some((0, 2)));
        // Past the row's width.
        assert_eq!(sel((0, 12), (0, 15)).cols_on_line(0, 10), None);
        assert_eq!(sel((0, 8), (0, 15)).cols_on_line(0, 10), Some((8, 10)));
    }

    #[test]
    fn text_trims_trailing_spaces_and_joins_lines() {
        let rows = [
            cells("hello world   "),
            cells("   "),
            cells("second line  "),
        ];
        let at = |i: usize| rows.get(i).map(Vec::as_slice);
        assert_eq!(sel((0, 6), (2, 5)).text(at), "world\n\nsecond");
        assert_eq!(sel((2, 3), (0, 0)).text(at), "hello world\n\nseco");
        // Lines beyond the buffer are ignored.
        assert_eq!(sel((2, 7), (9, 0)).text(at), "line");
    }

    #[test]
    fn word_at_selects_words_paths_and_runs() {
        let row = cells("ls C:\\dev\\x.rs  (ok)");
        assert_eq!(word_at(&row, 0), (0, 2));
        assert_eq!(word_at(&row, 1), (0, 2));
        assert_eq!(word_at(&row, 5), (3, 14), "a path is one word");
        assert_eq!(word_at(&row, 14), (14, 16), "space run");
        assert_eq!(word_at(&row, 16), (16, 17), "delimiter alone");
        assert_eq!(word_at(&row, 17), (17, 19));
        assert_eq!(word_at(&row, 50), (50, 51), "past the end");
    }
}
