#[derive(Debug, Clone)]
pub struct BlameEntry {
    pub commit_short: String,
    pub author: String,
    pub line: usize,
}

/// Run git blame on `path` using git2 and return per-line blame entries.
pub fn blame_file(path: &std::path::Path) -> Vec<BlameEntry> {
    let repo = match git2::Repository::discover(path) {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    let workdir = match repo.workdir() {
        Some(w) => w,
        None => return vec![],
    };
    let rel = match super::relative_to_workdir(path, workdir) {
        Some(r) => r,
        None => return vec![],
    };
    let blame = match repo.blame_file(&rel, None) {
        Ok(b) => b,
        Err(_) => return vec![],
    };
    let mut entries = Vec::new();
    for hunk in blame.iter() {
        let commit_id = hunk.final_commit_id();
        let short: String = commit_id.to_string().chars().take(7).collect();
        let author = hunk.final_signature().name().unwrap_or("?").to_string();
        let start_line = hunk.final_start_line(); // 1-indexed
        let lines_in_hunk = hunk.lines_in_hunk();
        for i in 0..lines_in_hunk {
            entries.push(BlameEntry {
                commit_short: short.clone(),
                author: author.clone(),
                line: start_line + i - 1, // convert to 0-indexed
            });
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::TestRepo;

    /// Absolute path of `rel` as libgit2 sees the workdir (avoids 8.3 /
    /// symlinked temp-dir mismatches).
    fn abs(r: &TestRepo, rel: &str) -> std::path::PathBuf {
        r.repo.workdir().unwrap().join(rel)
    }

    #[test]
    fn blame_attributes_each_line_to_its_last_commit() {
        let r = TestRepo::new();
        r.write("f.txt", "one\ntwo\nthree\n");
        let c1 = r.commit_as("first", "Alice");
        r.write("f.txt", "one\nTWO\nthree\nfour\n");
        let c2 = r.commit_as("second", "Bob");

        let mut entries = blame_file(&abs(&r, "f.txt"));
        entries.sort_by_key(|e| e.line);
        let lines: Vec<usize> = entries.iter().map(|e| e.line).collect();
        assert_eq!(lines, [0, 1, 2, 3], "0-indexed, one entry per line");
        let short1 = c1.to_string()[..7].to_string();
        let short2 = c2.to_string()[..7].to_string();
        let who: Vec<(&str, &str)> = entries
            .iter()
            .map(|e| (e.commit_short.as_str(), e.author.as_str()))
            .collect();
        assert_eq!(
            who,
            [
                (short1.as_str(), "Alice"),
                (short2.as_str(), "Bob"),
                (short1.as_str(), "Alice"),
                (short2.as_str(), "Bob"),
            ]
        );
    }

    #[test]
    fn blame_in_subdirectory() {
        let r = TestRepo::new();
        r.write("src/lib.rs", "a\nb\n");
        r.commit_as("init", "Carol");
        let entries = blame_file(&abs(&r, "src/lib.rs"));
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.author == "Carol"));
    }

    #[test]
    fn untracked_or_missing_file_yields_nothing() {
        let r = TestRepo::new();
        r.write("a.txt", "x\n");
        r.commit_all("init");
        r.write("new.txt", "untracked\n");
        assert!(blame_file(&abs(&r, "new.txt")).is_empty());
        assert!(blame_file(&abs(&r, "missing.txt")).is_empty());
    }

    #[test]
    fn relative_path_outside_workdir_yields_nothing() {
        let r = TestRepo::new();
        r.write("a.txt", "x\n");
        r.commit_all("init");
        // Discoverable from the process CWD only if the CWD is a repo; a
        // relative path never strips the absolute workdir prefix.
        assert!(blame_file(std::path::Path::new("a.txt")).is_empty());
    }

    #[test]
    fn non_repo_or_bare_repo_yields_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "x").unwrap();
        if git2::Repository::discover(dir.path()).is_err() {
            assert!(blame_file(&dir.path().join("f.txt")).is_empty());
        }
        let (bare, _) = crate::git::tests::bare_origin();
        assert!(blame_file(bare.path()).is_empty());
    }
}
