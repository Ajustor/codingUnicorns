pub(super) const DIFF_UNCHANGED: u8 = 0;
pub(super) const DIFF_ADDED: u8 = 1;
pub(super) const DIFF_MODIFIED: u8 = 2;
pub(super) const DIFF_DELETED_ABOVE: u8 = 3;

/// Compare current file content against HEAD and return per-line diff status.
pub(super) fn compute_line_diff(path: &std::path::Path, num_lines: usize) -> Vec<u8> {
    let mut result = vec![DIFF_UNCHANGED; num_lines];
    let repo = match git2::Repository::discover(path) {
        Ok(r) => r,
        Err(_) => return result,
    };
    let workdir = match repo.workdir() {
        Some(w) => w.to_path_buf(),
        None => return result,
    };
    let rel = match crate::git::relative_to_workdir(path, &workdir) {
        Some(r) => r,
        None => return result,
    };
    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => {
            for s in result.iter_mut() {
                *s = DIFF_ADDED;
            }
            return result;
        }
    };
    let tree = match head.peel_to_tree() {
        Ok(t) => t,
        Err(_) => return result,
    };
    let entry = match tree.get_path(&rel) {
        Ok(e) => e,
        Err(_) => {
            for s in result.iter_mut() {
                *s = DIFF_ADDED;
            }
            return result;
        }
    };
    let blob = match repo.find_blob(entry.id()) {
        Ok(b) => b,
        Err(_) => return result,
    };
    let old_content = match std::str::from_utf8(blob.content()) {
        Ok(s) => s.to_string(),
        Err(_) => return result,
    };
    let current = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return result,
    };
    diff_statuses(&old_content, &current, num_lines).unwrap_or(result)
}

/// Per-line status of `new` against `old` (`num_lines` entries), from a real
/// line diff: a line inserted at the top doesn't mark the rest as modified.
/// In each changed block, the first new lines pair with the removed ones
/// (modified), the extra ones are added; a block that only removes lines marks
/// the line below the gap as `DIFF_DELETED_ABOVE`.
fn diff_statuses(old: &str, new: &str, num_lines: usize) -> Option<Vec<u8>> {
    let mut result = vec![DIFF_UNCHANGED; num_lines];
    // CRLF and LF compare equal: the buffer is LF-only.
    let (old, new) = (old.replace("\r\n", "\n"), new.replace("\r\n", "\n"));
    let mut opts = git2::DiffOptions::new();
    opts.context_lines(0);
    let patch =
        git2::Patch::from_buffers(old.as_bytes(), None, new.as_bytes(), None, Some(&mut opts))
            .ok()?;
    for h in 0..patch.num_hunks() {
        let (hunk, _) = patch.hunk(h).ok()?;
        let (removed, added) = (hunk.old_lines() as usize, hunk.new_lines() as usize);
        if added == 0 {
            // Pure deletion: `new_start` is the line before the gap (0 = top).
            let below = (hunk.new_start() as usize).min(num_lines.saturating_sub(1));
            if let Some(s) = result.get_mut(below) {
                if *s == DIFF_UNCHANGED {
                    *s = DIFF_DELETED_ABOVE;
                }
            }
            continue;
        }
        let first = (hunk.new_start() as usize).saturating_sub(1);
        for (k, s) in result.iter_mut().skip(first).take(added).enumerate() {
            *s = if k < removed {
                DIFF_MODIFIED
            } else {
                DIFF_ADDED
            };
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Commit `files` (relative path, bytes) to a new repository at `root`.
    fn commit(root: &Path, files: &[(&str, &[u8])]) -> git2::Repository {
        let repo = git2::Repository::init(root).unwrap();
        {
            let mut index = repo.index().unwrap();
            for (rel, content) in files {
                let path = root.join(rel);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, content).unwrap();
                index.add_path(Path::new(rel)).unwrap();
            }
            index.write().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            let sig = git2::Signature::now("test", "test@example.com").unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .unwrap();
        }
        repo
    }

    #[test]
    fn classifies_unchanged_deleted_and_new_lines() {
        let dir = tempfile::tempdir().unwrap();
        commit(dir.path(), &[("f.txt", b"alpha\nbeta\ngamma\n")]);
        let path = dir.path().join("f.txt");
        // "beta" removed (above "gamma", which is unchanged), "NEW" added.
        std::fs::write(&path, "alpha\ngamma\nNEW\n").unwrap();
        assert_eq!(
            compute_line_diff(&path, 3),
            vec![DIFF_UNCHANGED, DIFF_DELETED_ABOVE, DIFF_ADDED]
        );
    }

    #[test]
    fn a_line_inserted_at_the_top_marks_only_that_line() {
        let old = "a\nb\nc\nd\n";
        assert_eq!(
            diff_statuses(old, "new\na\nb\nc\nd\n", 5).unwrap(),
            vec![
                DIFF_ADDED,
                DIFF_UNCHANGED,
                DIFF_UNCHANGED,
                DIFF_UNCHANGED,
                DIFF_UNCHANGED
            ]
        );
    }

    #[test]
    fn changed_lines_are_modified_and_extra_ones_added() {
        let old = "a\nb\nc\n";
        assert_eq!(
            diff_statuses(old, "a\nB\nB2\nc\n", 4).unwrap(),
            vec![DIFF_UNCHANGED, DIFF_MODIFIED, DIFF_ADDED, DIFF_UNCHANGED]
        );
        // CRLF on disk vs LF in HEAD is not a change.
        assert_eq!(
            diff_statuses(old, "a\r\nb\r\nc\r\n", 3).unwrap(),
            vec![DIFF_UNCHANGED; 3]
        );
    }

    #[test]
    fn removed_lines_mark_the_line_below_the_gap() {
        assert_eq!(
            diff_statuses("a\nb\nc\n", "b\nc\n", 2).unwrap(),
            vec![DIFF_DELETED_ABOVE, DIFF_UNCHANGED],
            "removed at the top"
        );
        assert_eq!(
            diff_statuses("a\nb\nc\n", "a\nb\n", 2).unwrap(),
            vec![DIFF_UNCHANGED, DIFF_DELETED_ABOVE],
            "removed at the end: shown on the last line"
        );
    }

    #[test]
    fn unmodified_file_is_all_unchanged_and_extra_lines_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        commit(dir.path(), &[("f.txt", b"a\nb\n")]);
        let path = dir.path().join("f.txt");
        // Requested size larger than the file: trailing entries stay unchanged.
        assert_eq!(compute_line_diff(&path, 4), vec![DIFF_UNCHANGED; 4]);
        // Requested size smaller than the file: extra file lines are ignored.
        std::fs::write(&path, "x\ny\nz\n").unwrap();
        assert_eq!(compute_line_diff(&path, 1), vec![DIFF_MODIFIED]);
    }

    #[test]
    fn untracked_file_is_all_added() {
        let dir = tempfile::tempdir().unwrap();
        commit(dir.path(), &[("tracked.txt", b"t\n")]);
        let path = dir.path().join("untracked.txt");
        std::fs::write(&path, "1\n2\n").unwrap();
        assert_eq!(compute_line_diff(&path, 2), vec![DIFF_ADDED; 2]);
    }

    #[test]
    fn repository_without_commits_marks_all_added() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        let path = dir.path().join("f.txt");
        std::fs::write(&path, "x\n").unwrap();
        assert_eq!(compute_line_diff(&path, 3), vec![DIFF_ADDED; 3]);
    }

    #[test]
    fn missing_path_outside_any_repository_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope").join("missing.rs");
        assert_eq!(compute_line_diff(&path, 2), vec![DIFF_UNCHANGED; 2]);
    }

    #[test]
    fn bare_repository_has_no_workdir() {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init_bare(dir.path()).unwrap();
        let path = dir.path().join("config");
        assert_eq!(compute_line_diff(&path, 2), vec![DIFF_UNCHANGED; 2]);
    }

    #[test]
    fn non_utf8_blob_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        commit(dir.path(), &[("bin.dat", &[0xff, 0xfe, 0x00, 0x81])]);
        let path = dir.path().join("bin.dat");
        assert_eq!(compute_line_diff(&path, 1), vec![DIFF_UNCHANGED]);
    }

    #[test]
    fn tracked_path_that_is_a_directory_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        commit(dir.path(), &[("sub/inner.txt", b"x\n")]);
        let path = dir.path().join("sub");
        assert_eq!(compute_line_diff(&path, 1), vec![DIFF_UNCHANGED]);
    }

    #[test]
    fn unreadable_working_file_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        commit(dir.path(), &[("gone.txt", b"a\n")]);
        let path = dir.path().join("gone.txt");
        // Replace the file with a directory: HEAD lookup succeeds, reading fails.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(compute_line_diff(&path, 1), vec![DIFF_UNCHANGED]);
    }

    #[test]
    fn diff_status_codes_are_distinct() {
        let codes = [
            DIFF_UNCHANGED,
            DIFF_ADDED,
            DIFF_MODIFIED,
            DIFF_DELETED_ABOVE,
        ];
        assert_eq!(codes, [0, 1, 2, 3]);
    }
}
