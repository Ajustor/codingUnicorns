/// Parse a file with git conflict markers into separate ours/theirs/result versions.

#[derive(Debug, Clone)]
pub struct ConflictHunk {
    pub result_line_start: usize,
    pub result_line_end: usize,
    pub ours: Vec<String>,
    pub theirs: Vec<String>,
    pub resolution: HunkResolution,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HunkResolution {
    Unresolved,
    AcceptOurs,
    AcceptTheirs,
}

pub struct ParsedConflict {
    pub ours_content: String,
    pub theirs_content: String,
    pub result_content: String,
    pub hunks: Vec<ConflictHunk>,
}

/// Parse a file with conflict markers (<<<<<<<, =======, >>>>>>>) into three versions.
pub fn parse_conflict_file(content: &str) -> Option<ParsedConflict> {
    let lines: Vec<&str> = content.lines().collect();
    let mut ours_lines: Vec<String> = vec![];
    let mut theirs_lines: Vec<String> = vec![];
    let mut result_lines: Vec<String> = vec![];
    let mut hunks: Vec<ConflictHunk> = vec![];
    let mut i = 0;
    let mut found_conflict = false;

    while i < lines.len() {
        if lines[i].starts_with("<<<<<<<") {
            found_conflict = true;
            let mut ours: Vec<String> = vec![];
            let mut theirs: Vec<String> = vec![];
            i += 1;
            while i < lines.len() && !lines[i].starts_with("=======") {
                ours.push(lines[i].to_string());
                i += 1;
            }
            i += 1; // skip =======
            while i < lines.len() && !lines[i].starts_with(">>>>>>>") {
                theirs.push(lines[i].to_string());
                i += 1;
            }
            i += 1; // skip >>>>>>>

            let result_start = result_lines.len();
            for line in &ours {
                result_lines.push(line.clone());
            }
            let result_end = result_lines.len();
            ours_lines.extend(ours.clone());
            theirs_lines.extend(theirs.clone());

            hunks.push(ConflictHunk {
                result_line_start: result_start,
                result_line_end: result_end,
                ours,
                theirs,
                resolution: HunkResolution::Unresolved,
            });
        } else {
            ours_lines.push(lines[i].to_string());
            theirs_lines.push(lines[i].to_string());
            result_lines.push(lines[i].to_string());
            i += 1;
        }
    }

    if !found_conflict {
        return None;
    }

    Some(ParsedConflict {
        ours_content: ours_lines.join("\n"),
        theirs_content: theirs_lines.join("\n"),
        result_content: result_lines.join("\n"),
        hunks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_without_markers_is_not_a_conflict() {
        assert!(parse_conflict_file("").is_none());
        assert!(parse_conflict_file("a\nb\n=======\nc").is_none());
    }

    #[test]
    fn single_hunk_splits_into_ours_theirs_and_result() {
        let src = "head\n<<<<<<< HEAD\nmine 1\nmine 2\n=======\ntheirs\n>>>>>>> feature\ntail\n";
        let p = parse_conflict_file(src).unwrap();
        assert_eq!(p.ours_content, "head\nmine 1\nmine 2\ntail");
        assert_eq!(p.theirs_content, "head\ntheirs\ntail");
        // The initial result takes "ours" for every hunk.
        assert_eq!(p.result_content, "head\nmine 1\nmine 2\ntail");
        assert_eq!(p.hunks.len(), 1);
        let h = &p.hunks[0];
        assert_eq!(h.ours, ["mine 1", "mine 2"]);
        assert_eq!(h.theirs, ["theirs"]);
        assert_eq!((h.result_line_start, h.result_line_end), (1, 3));
        assert_eq!(h.resolution, HunkResolution::Unresolved);
    }

    #[test]
    fn multiple_hunks_track_result_line_ranges() {
        let src = "\
<<<<<<< ours
a
=======
b
>>>>>>> theirs
mid
<<<<<<< ours
=======
x
y
>>>>>>> theirs";
        let p = parse_conflict_file(src).unwrap();
        assert_eq!(p.hunks.len(), 2);
        assert_eq!(
            (p.hunks[0].result_line_start, p.hunks[0].result_line_end),
            (0, 1)
        );
        // Second hunk has an empty "ours" side: zero-width range after "mid".
        assert!(p.hunks[1].ours.is_empty());
        assert_eq!(p.hunks[1].theirs, ["x", "y"]);
        assert_eq!(
            (p.hunks[1].result_line_start, p.hunks[1].result_line_end),
            (2, 2)
        );
        assert_eq!(p.result_content, "a\nmid");
        assert_eq!(p.theirs_content, "b\nmid\nx\ny");
    }

    #[test]
    fn unterminated_conflict_consumes_rest_of_file() {
        let p = parse_conflict_file("pre\n<<<<<<< HEAD\nours\n=======\ntheirs").unwrap();
        assert_eq!(p.hunks[0].ours, ["ours"]);
        assert_eq!(p.hunks[0].theirs, ["theirs"]);
        assert_eq!(p.theirs_content, "pre\ntheirs");

        let p = parse_conflict_file("<<<<<<< HEAD\nonly ours").unwrap();
        assert_eq!(p.hunks[0].ours, ["only ours"]);
        assert!(p.hunks[0].theirs.is_empty());
    }

    #[test]
    fn crlf_line_endings_are_handled() {
        let p = parse_conflict_file("<<<<<<< HEAD\r\na\r\n=======\r\nb\r\n>>>>>>> x\r\n").unwrap();
        assert_eq!(p.hunks[0].ours, ["a"]);
        assert_eq!(p.hunks[0].theirs, ["b"]);
    }

    #[test]
    fn resolution_enum_is_comparable() {
        assert_ne!(HunkResolution::AcceptOurs, HunkResolution::AcceptTheirs);
        assert_ne!(HunkResolution::Unresolved, HunkResolution::AcceptOurs);
    }
}
