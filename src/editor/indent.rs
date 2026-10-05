/// Detect whether a file uses spaces or tabs and the indent unit size.
/// Returns `(use_spaces, indent_size)`.
pub(super) fn detect_indent(content: &str) -> (bool, usize) {
    let mut space_lines = 0usize;
    let mut tab_lines = 0usize;
    let mut size_votes: std::collections::HashMap<usize, usize> = Default::default();
    for line in content.lines().take(200) {
        if line.starts_with('\t') {
            tab_lines += 1;
        } else if line.starts_with("  ") {
            let n = line.chars().take_while(|&c| c == ' ').count();
            space_lines += 1;
            if n > 0 {
                *size_votes.entry(n).or_default() += 1;
            }
        }
    }
    let use_spaces = space_lines >= tab_lines;
    let size = if use_spaces {
        [2usize, 4, 3, 8]
            .iter()
            .find(|&&s| size_votes.contains_key(&s))
            .copied()
            .unwrap_or(4)
    } else {
        4
    };
    (use_spaces, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_four_space_indent() {
        assert_eq!(detect_indent("fn a() {\n    x;\n        y;\n}"), (true, 4));
    }

    #[test]
    fn detects_two_space_indent_even_with_deeper_lines() {
        assert_eq!(detect_indent("a:\n  b:\n    c: 1\n  d: 2"), (true, 2));
    }

    #[test]
    fn detects_three_and_eight_space_indent() {
        assert_eq!(detect_indent("a\n   b\n      c"), (true, 3));
        assert_eq!(detect_indent("a\n        b"), (true, 8));
    }

    #[test]
    fn unusual_space_width_falls_back_to_four() {
        assert_eq!(detect_indent("a\n      b"), (true, 4));
    }

    #[test]
    fn tabs_win_when_more_frequent() {
        assert_eq!(detect_indent("\ta\n\t\tb\n  c"), (false, 4));
    }

    #[test]
    fn ties_and_empty_content_prefer_spaces() {
        assert_eq!(detect_indent(""), (true, 4));
        assert_eq!(detect_indent("no indent\nat all"), (true, 4));
        assert_eq!(detect_indent("\ta\n  b"), (true, 2));
    }

    #[test]
    fn single_leading_space_is_ignored() {
        assert_eq!(detect_indent("\ta\n b\n c"), (false, 4));
    }

    #[test]
    fn only_first_200_lines_are_sampled() {
        let mut src = "\tx\n".repeat(200);
        src.push_str(&"  y\n".repeat(300));
        assert_eq!(detect_indent(&src), (false, 4));
    }
}
