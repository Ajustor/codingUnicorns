use super::buffer::Buffer;

/// Find the matching bracket for the char at `(row, col)` in the buffer.
/// Returns `(open_row, open_col, close_row, close_col)` or `None`.
pub(super) fn find_matching_bracket(
    buffer: &Buffer,
    row: usize,
    col: usize,
) -> Option<(usize, usize, usize, usize)> {
    let line = buffer.line(row);
    let chars: Vec<char> = line.chars().collect();

    for check_col in [col, col.wrapping_sub(1)] {
        if check_col >= chars.len() {
            continue;
        }
        let ch = chars[check_col];
        match ch {
            '{' | '(' | '[' => {
                let close = match ch {
                    '{' => '}',
                    '(' => ')',
                    _ => ']',
                };
                let mut depth = 1usize;
                let mut r = row;
                let mut c = check_col + 1;
                loop {
                    let line_chars: Vec<char> = buffer.line(r).chars().collect();
                    while c < line_chars.len() {
                        if line_chars[c] == ch {
                            depth += 1;
                        } else if line_chars[c] == close {
                            depth -= 1;
                            if depth == 0 {
                                return Some((row, check_col, r, c));
                            }
                        }
                        c += 1;
                    }
                    r += 1;
                    if r >= buffer.num_lines() {
                        break;
                    }
                    c = 0;
                }
            }
            '}' | ')' | ']' => {
                let open = match ch {
                    '}' => '{',
                    ')' => '(',
                    _ => '[',
                };
                let mut depth = 1usize;
                let mut r = row;
                let mut c = check_col;
                loop {
                    let line_chars: Vec<char> = buffer.line(r).chars().collect();
                    let start = if r == row { c } else { line_chars.len() };
                    let scan_range: Vec<usize> = (0..start).rev().collect();
                    for sc in scan_range {
                        if line_chars[sc] == ch {
                            depth += 1;
                        } else if line_chars[sc] == open {
                            depth -= 1;
                            if depth == 0 {
                                return Some((r, sc, row, check_col));
                            }
                        }
                    }
                    if r == 0 {
                        break;
                    }
                    r -= 1;
                    c = buffer.line(r).chars().count();
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(src: &str, row: usize, col: usize) -> Option<(usize, usize, usize, usize)> {
        find_matching_bracket(&Buffer::from_str(src), row, col)
    }

    #[test]
    fn matches_from_opening_bracket_on_same_line() {
        assert_eq!(find("fn a() {", 0, 4), Some((0, 4, 0, 5)));
        assert_eq!(find("x[1]", 0, 1), Some((0, 1, 0, 3)));
    }

    #[test]
    fn matches_from_closing_bracket_on_same_line() {
        assert_eq!(find("fn a() {", 0, 5), Some((0, 4, 0, 5)));
        assert_eq!(find("x[1]", 0, 3), Some((0, 1, 0, 3)));
    }

    #[test]
    fn matches_bracket_just_before_cursor() {
        // Cursor sits after ')' on a space.
        assert_eq!(find("a() b", 0, 3), Some((0, 1, 0, 2)));
        // Cursor at end of line, after '}'.
        assert_eq!(find("{}", 0, 2), Some((0, 0, 0, 1)));
    }

    #[test]
    fn bracket_under_cursor_takes_precedence() {
        // col 2 is '(' (under cursor) and col 1 is ')' (before cursor).
        assert_eq!(find("()(x)", 0, 2), Some((0, 2, 0, 4)));
    }

    #[test]
    fn nested_brackets_of_same_kind() {
        assert_eq!(find("((a)(b))", 0, 0), Some((0, 0, 0, 7)));
        assert_eq!(find("((a)(b))", 0, 7), Some((0, 0, 0, 7)));
        assert_eq!(find("((a)(b))", 0, 4), Some((0, 4, 0, 6)));
    }

    #[test]
    fn matches_across_lines_in_both_directions() {
        let src = "{\n  {}\n  (x)\n}";
        assert_eq!(find(src, 0, 0), Some((0, 0, 3, 0)));
        assert_eq!(find(src, 3, 0), Some((0, 0, 3, 0)));
        assert_eq!(find(src, 3, 1), Some((0, 0, 3, 0)));
    }

    #[test]
    fn unmatched_brackets_return_none() {
        assert_eq!(find("(abc", 0, 0), None);
        assert_eq!(find("(\nabc\n", 0, 0), None);
        assert_eq!(find("abc)", 0, 3), None);
        assert_eq!(find("a\nb]", 1, 1), None);
    }

    #[test]
    fn non_bracket_positions_return_none() {
        assert_eq!(find("abc", 0, 0), None);
        assert_eq!(find("abc", 0, 1), None);
        assert_eq!(find("", 0, 0), None);
        assert_eq!(find("(x)", 5, 0), None);
    }
}
