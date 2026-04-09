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
