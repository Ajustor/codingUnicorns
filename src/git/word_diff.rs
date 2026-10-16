//! Which words changed between two versions of a line, for the highlights of
//! the pull request diff.

use std::ops::Range;

/// Lines longer than this (in bytes) get no word highlights.
pub const MAX_LINE: usize = 1000;

// The LCS length is bounded by the token count, which is bounded by the line
// length, so `u16` cannot overflow.
const _: () = assert!(MAX_LINE <= u16::MAX as usize);

/// Below this share of the shorter line's non-whitespace characters, the two lines
/// are considered unrelated (a stray `(` or `;` in common is not worth
/// highlighting) and get no word highlights, only their line colour.
const MIN_SHARED: f32 = 0.5;

/// Byte ranges of a line pair that are not part of their longest common
/// word subsequence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WordChanges {
    pub old: Vec<Range<usize>>,
    pub new: Vec<Range<usize>>,
}

/// The changed words of `old` → `new`; `None` when a line is too long to
/// compare (more than [`MAX_LINE`] bytes, not characters).
///
/// No ranges (`WordChanges::default()`) are returned for lines that share too little to be worth
/// highlighting. That includes a blank line paired with code; two lines with
/// only whitespace are still compared.
///
/// Guarantees for each side: the ranges are non-empty, sorted, disjoint and
/// never touching (adjacent changes are merged), and both ends of each range
/// are on char boundaries of the line.
pub fn changed_ranges(old: &str, new: &str) -> Option<WordChanges> {
    if old.len() > MAX_LINE || new.len() > MAX_LINE {
        return None;
    }
    let (a, b) = (tokens(old), tokens(new));
    let pa: Vec<&str> = a.iter().map(|r| &old[r.clone()]).collect();
    let pb: Vec<&str> = b.iter().map(|r| &new[r.clone()]).collect();
    let (n, m) = (pa.len(), pb.len());

    // Common leading and trailing tokens are kept; only the middle needs the DP.
    let pre = pa.iter().zip(&pb).take_while(|(x, y)| x == y).count();
    let suf = pa[pre..]
        .iter()
        .rev()
        .zip(pb[pre..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (mid_a, mid_b) = (&pa[pre..n - suf], &pb[pre..m - suf]);
    let (na, nb) = (mid_a.len(), mid_b.len());

    // lcs[i * w + j]: length of the longest common subsequence of
    // mid_a[i..] and mid_b[j..].
    let w = nb + 1;
    let mut lcs = vec![0u16; (na + 1) * w];
    for i in (0..na).rev() {
        for j in (0..nb).rev() {
            lcs[i * w + j] = if mid_a[i] == mid_b[j] {
                lcs[(i + 1) * w + j + 1] + 1
            } else {
                lcs[(i + 1) * w + j].max(lcs[i * w + j + 1])
            };
        }
    }
    let (mut keep_a, mut keep_b) = (vec![false; n], vec![false; m]);
    keep_a[..pre].fill(true);
    keep_b[..pre].fill(true);
    keep_a[n - suf..].fill(true);
    keep_b[m - suf..].fill(true);
    let (mut i, mut j) = (0, 0);
    while i < na && j < nb {
        if mid_a[i] == mid_b[j] {
            keep_a[pre + i] = true;
            keep_b[pre + j] = true;
            i += 1;
            j += 1;
        } else if lcs[(i + 1) * w + j] >= lcs[i * w + j + 1] {
            // On a tie the old token is dropped first. Both choices leave a
            // common subsequence of the same length, so only which of two
            // equally good alignments gets highlighted differs.
            i += 1;
        } else {
            j += 1;
        }
    }

    let shared: usize = pa
        .iter()
        .zip(&keep_a)
        .filter(|(_, &kept)| kept)
        .map(|(t, _)| non_whitespace_chars(t))
        .sum();
    let (len_old, len_new) = (non_whitespace_chars(old), non_whitespace_chars(new));
    let (shorter, longer) = (len_old.min(len_new), len_old.max(len_new));
    if longer > 0 && (shared as f32) < MIN_SHARED * shorter.max(1) as f32 {
        return Some(WordChanges::default());
    }

    bridge_spaces(&pa, &mut keep_a);
    bridge_spaces(&pb, &mut keep_b);
    Some(WordChanges {
        old: merge(&a, &keep_a),
        new: merge(&b, &keep_b),
    })
}

fn non_whitespace_chars(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

/// A kept whitespace token between two changed tokens becomes changed, so
/// `[old_name] [other_thing]` is one range rather than two.
fn bridge_spaces(tokens: &[&str], keep: &mut [bool]) {
    for i in 1..tokens.len().saturating_sub(1) {
        if keep[i] && tokens[i].chars().all(char::is_whitespace) && !keep[i - 1] && !keep[i + 1] {
            keep[i] = false;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Other,
}

fn class(c: char) -> Class {
    if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else if c.is_whitespace() {
        Class::Space
    } else {
        Class::Other
    }
}

/// Runs of word characters, runs of spaces, and single other characters.
fn tokens(s: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut chars = s.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let kind = class(c);
        let mut end = start + c.len_utf8();
        if kind != Class::Other {
            while let Some(&(i, d)) = chars.peek() {
                if class(d) != kind {
                    break;
                }
                end = i + d.len_utf8();
                chars.next();
            }
        }
        out.push(start..end);
    }
    out
}

/// The tokens not kept, adjacent ones merged.
fn merge(tokens: &[Range<usize>], keep: &[bool]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for (t, &kept) in tokens.iter().zip(keep) {
        if kept {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.end == t.start => last.end = t.end,
            _ => out.push(t.clone()),
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_word_in_the_middle() {
        let w = changed_ranges("let x = 1;", "let y = 1;").unwrap();
        assert_eq!((w.old, w.new), (vec![4..5], vec![4..5]));
    }

    #[test]
    fn identical_lines_have_no_changes() {
        assert_eq!(
            changed_ranges("a b", "a b").unwrap(),
            WordChanges::default()
        );
    }

    #[test]
    fn appended_words_merge_into_one_range() {
        let w = changed_ranges("a b", "a b c").unwrap();
        assert_eq!((w.old, w.new), (vec![], vec![3..5]));
    }

    #[test]
    fn ranges_are_byte_offsets_of_whole_characters() {
        let w = changed_ranges("café x", "café y").unwrap();
        assert_eq!((w.old, w.new), (vec![6..7], vec![6..7]));
    }

    #[test]
    fn a_changed_multi_byte_word() {
        let w = changed_ranges("let café = 1;", "let cafe = 1;").unwrap();
        assert_eq!((w.old, w.new), (vec![4..9], vec![4..8]));
    }

    #[test]
    fn long_lines_are_not_compared() {
        assert_eq!(changed_ranges(&"x".repeat(MAX_LINE + 1), "x"), None);
        assert_eq!(changed_ranges("x", &"x".repeat(MAX_LINE + 1)), None);
    }

    #[test]
    fn a_line_of_exactly_max_line_bytes_is_compared() {
        let line = "x".repeat(MAX_LINE);
        assert!(changed_ranges(&line, "x").is_some());
        assert!(changed_ranges("x", &line).is_some());
    }

    #[test]
    fn tokens_split_words_spaces_and_other_characters() {
        assert_eq!(tokens("a_b  c("), vec![0..3, 3..5, 5..6, 6..7]);
    }

    #[test]
    fn a_blank_line_paired_with_code_gets_no_highlights() {
        assert_eq!(
            changed_ranges("", "    let x = 1;").unwrap(),
            WordChanges::default()
        );
        assert_eq!(changed_ranges("abc", "").unwrap(), WordChanges::default());
    }

    #[test]
    fn whitespace_only_lines_are_still_highlighted() {
        let w = changed_ranges("    ", "").unwrap();
        assert_eq!((w.old, w.new), (vec![0..4], vec![]));
    }

    #[test]
    fn a_changed_cjk_word_is_highlighted() {
        let w = changed_ranges("x = \"日本語\"", "x = \"中文\"").unwrap();
        assert_eq!((w.old, w.new), (vec![5..14], vec![5..11]));
    }

    #[test]
    fn a_changed_phrase_is_one_range_through_its_spaces() {
        let w = changed_ranges(
            "The quick brown fox jumps over the dog.",
            "The slow red fox jumps over the dog.",
        )
        .unwrap();
        assert_eq!((w.old, w.new), (vec![4..15], vec![4..12]));
    }

    #[test]
    fn an_indentation_only_change_is_highlighted() {
        let w = changed_ranges("    foo", "  foo").unwrap();
        assert_eq!((w.old, w.new), (vec![0..4], vec![0..2]));
    }

    #[test]
    fn unrelated_lines_get_no_highlights() {
        assert_eq!(
            changed_ranges("}", "let x = 1;").unwrap(),
            WordChanges::default()
        );
        assert_eq!(
            changed_ranges("return self.cache.get(key);", "let value = compute(input);").unwrap(),
            WordChanges::default()
        );
    }

    #[test]
    fn a_renamed_identifier_is_still_highlighted() {
        let w = changed_ranges("let x = 1;", "let completely_different_name = 1;").unwrap();
        assert_eq!((w.old, w.new), (vec![4..5], vec![4..29]));
    }

    #[test]
    fn a_removed_call_is_one_range() {
        let w = changed_ranges("self.items.push(item.clone());", "self.items.push(item);").unwrap();
        assert_eq!((w.old, w.new), (vec![20..28], vec![]));
    }

    #[test]
    fn inserted_words_are_one_range() {
        let w = changed_ranges(
            r#"println!("Hello world")"#,
            r#"println!("Goodbye cruel world")"#,
        )
        .unwrap();
        assert_eq!(w.old, vec![10..15]);
        assert_eq!(w.new, vec![10..23]);
    }

    #[test]
    fn ranges_are_sorted_disjoint_and_on_char_boundaries() {
        let pairs = [
            ("let x = 1;", "let y = 2;"),
            ("café au lait", "cafe au  lait noir"),
            ("a b c d e", "a b x d e"),
            ("    foo(bar, baz)", "  foo(baz, bar, qux)"),
            ("naïve → x", "naïve ← y z"),
            (
                "alpha beta old_name other_thing gamma delta epsilon",
                "alpha beta new_name another_thing gamma delta epsilon",
            ),
            ("x = \"日本語\"", "x = \"中文\""),
        ];
        for (old, new) in pairs {
            let w = changed_ranges(old, new).unwrap();
            for (line, ranges) in [(old, &w.old), (new, &w.new)] {
                for r in ranges {
                    assert!(r.start < r.end, "{line:?}: empty range {r:?}");
                    assert!(line.is_char_boundary(r.start), "{line:?}: {r:?}");
                    assert!(line.is_char_boundary(r.end), "{line:?}: {r:?}");
                }
                for pair in ranges.windows(2) {
                    assert!(pair[0].end < pair[1].start, "{line:?}: {pair:?}");
                }
            }
        }
    }
}
