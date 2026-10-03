use super::buffer::Buffer;

/// Compute foldable regions from buffer by indent level.
/// Returns list of `(start_line, end_line)`.
pub(super) fn compute_fold_regions(buffer: &Buffer) -> Vec<(usize, usize)> {
    let total = buffer.num_lines();
    let indent = |i: usize| -> usize {
        let line = buffer.line(i);
        if line.trim().is_empty() {
            return usize::MAX;
        }
        let tabs = line.chars().take_while(|&c| c == '\t').count();
        let spaces = line.chars().take_while(|&c| c == ' ').count();
        tabs.max(spaces / 2)
    };
    let mut regions = Vec::new();
    let mut i = 0;
    while i + 1 < total {
        let cur_ind = indent(i);
        if cur_ind == usize::MAX {
            i += 1;
            continue;
        }
        let mut end = i + 1;
        while end < total {
            let next_ind = indent(end);
            if next_ind == usize::MAX || next_ind > cur_ind {
                end += 1;
            } else {
                break;
            }
        }
        end -= 1;
        if end > i + 1 {
            regions.push((i, end));
        }
        i += 1;
    }
    regions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions(src: &str) -> Vec<(usize, usize)> {
        compute_fold_regions(&Buffer::from_str(src))
    }

    #[test]
    fn simple_block_folds_its_body() {
        assert_eq!(regions("fn a() {\n    x();\n    y();\n}"), vec![(0, 2)]);
    }

    #[test]
    fn nested_blocks_produce_nested_regions() {
        let src = "fn a() {\n    if x {\n        y();\n        z();\n    }\n}";
        assert_eq!(regions(src), vec![(0, 4), (1, 3)]);
    }

    #[test]
    fn single_nested_line_is_not_foldable() {
        assert!(regions("a\n  b\nc").is_empty());
    }

    #[test]
    fn blank_lines_are_skipped_and_do_not_end_a_region() {
        assert_eq!(regions("\nfn\n    a\n\n    b\nend"), vec![(1, 4)]);
    }

    #[test]
    fn tabs_count_as_one_level_and_single_spaces_do_not() {
        assert_eq!(regions("a\n\tb\n\tc"), vec![(0, 2)]);
        assert!(
            regions("a\n b\n c").is_empty(),
            "1 space < one indent level"
        );
        assert_eq!(regions("a\n  b\n  c"), vec![(0, 2)]);
    }

    #[test]
    fn trivial_buffers_have_no_regions() {
        assert!(regions("").is_empty());
        assert!(regions("one line").is_empty());
        assert!(regions("a\nb\nc").is_empty());
    }
}
