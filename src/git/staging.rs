use super::{FileChangeKind, GitStatus};

impl GitStatus {
    pub fn stage_file(&mut self, file_path: &str) {
        let repo_path = match &self.repo_path {
            Some(p) => p.clone(),
            None => return,
        };
        let repo = match git2::Repository::discover(&repo_path) {
            Ok(r) => r,
            Err(e) => {
                self.last_error = Some(format!("Failed to open repo: {e}"));
                return;
            }
        };
        let mut index = match repo.index() {
            Ok(i) => i,
            Err(e) => {
                self.last_error = Some(format!("Failed to get index: {e}"));
                return;
            }
        };
        let path = std::path::Path::new(file_path);
        // Check if this is a deleted file (wt_deleted means remove from index)
        let is_deleted = self
            .files
            .iter()
            .find(|f| f.path == file_path)
            .map(|f| f.wt_status == FileChangeKind::Deleted)
            .unwrap_or(false);

        let result = if is_deleted {
            index.remove_path(path)
        } else {
            index.add_path(path)
        };

        if let Err(e) = result {
            self.last_error = Some(format!("Failed to stage {file_path}: {e}"));
            return;
        }
        if let Err(e) = index.write() {
            self.last_error = Some(format!("Failed to write index: {e}"));
            return;
        }
        self.refresh();
    }

    pub fn unstage_file(&mut self, file_path: &str) {
        let repo_path = match &self.repo_path {
            Some(p) => p.clone(),
            None => return,
        };
        let repo = match git2::Repository::discover(&repo_path) {
            Ok(r) => r,
            Err(e) => {
                self.last_error = Some(format!("Failed to open repo: {e}"));
                return;
            }
        };
        // Reset the file in index to HEAD state
        let result: Result<(), git2::Error> = (|| {
            match repo.head() {
                Ok(head) => {
                    match head.peel_to_commit() {
                        Ok(commit) => {
                            let obj = commit.into_object();
                            repo.reset_default(Some(&obj), [file_path].iter())
                        }
                        Err(_) => {
                            // No commits yet: just remove from index
                            let mut index = repo.index()?;
                            index.remove_path(std::path::Path::new(file_path))?;
                            index.write()
                        }
                    }
                }
                Err(_) => {
                    // No HEAD: remove from index
                    let mut index = repo.index()?;
                    index.remove_path(std::path::Path::new(file_path))?;
                    index.write()
                }
            }
        })();
        if let Err(e) = result {
            self.last_error = Some(format!("Failed to unstage {file_path}: {e}"));
            return;
        }
        self.refresh();
    }

    pub fn stage_all(&mut self) {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => {
                self.last_error = Some(e);
                return;
            }
        };
        let mut index = match repo.index() {
            Ok(i) => i,
            Err(e) => {
                self.last_error = Some(format!("Index error: {e}"));
                return;
            }
        };
        let paths: Vec<(String, bool)> = self
            .files
            .iter()
            .filter(|f| f.wt_status != FileChangeKind::None)
            .map(|f| (f.path.clone(), f.wt_status == FileChangeKind::Deleted))
            .collect();
        for (path, is_deleted) in &paths {
            let p = std::path::Path::new(path);
            if *is_deleted {
                let _ = index.remove_path(p);
            } else {
                let _ = index.add_path(p);
            }
        }
        if let Err(e) = index.write() {
            self.last_error = Some(format!("Failed to write index: {e}"));
            return;
        }
        self.refresh();
    }

    pub fn unstage_all(&mut self) {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => {
                self.last_error = Some(e);
                return;
            }
        };
        let paths: Vec<String> = self
            .files
            .iter()
            .filter(|f| f.index_status != FileChangeKind::None)
            .map(|f| f.path.clone())
            .collect();
        match repo.head() {
            Ok(head) => {
                if let Ok(commit) = head.peel_to_commit() {
                    let obj = commit.into_object();
                    let _ = repo.reset_default(Some(&obj), paths.iter().map(|s| s.as_str()));
                }
            }
            Err(_) => {
                // No HEAD: remove all from index
                if let Ok(mut index) = repo.index() {
                    for path in &paths {
                        let _ = index.remove_path(std::path::Path::new(path));
                    }
                    let _ = index.write();
                }
            }
        }
        self.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::TestRepo;
    use crate::git::FileStatus;

    fn kinds(s: &GitStatus, path: &str) -> Option<(FileChangeKind, FileChangeKind)> {
        s.files
            .iter()
            .find(|f| f.path == path)
            .map(|f| (f.index_status.clone(), f.wt_status.clone()))
    }

    /// A repo with one commit containing `tracked.txt` and `gone.txt`, then:
    /// tracked.txt modified, gone.txt deleted, new.txt untracked.
    fn dirty_repo() -> TestRepo {
        let r = TestRepo::new();
        r.write("tracked.txt", "v1");
        r.write("gone.txt", "bye");
        r.commit_all("init");
        r.write("tracked.txt", "v2");
        r.remove("gone.txt");
        r.write("new.txt", "hello");
        r
    }

    /// Hold the index lock so libgit2 cannot write the index.
    fn lock_index(r: &TestRepo) {
        std::fs::write(r.repo.path().join("index.lock"), "").unwrap();
    }

    #[test]
    fn stage_and_unstage_untracked_file() {
        let r = dirty_repo();
        let mut s = r.status();
        assert_eq!(
            kinds(&s, "new.txt"),
            Some((FileChangeKind::None, FileChangeKind::Untracked))
        );
        s.stage_file("new.txt");
        assert!(s.last_error.is_none(), "{:?}", s.last_error);
        assert_eq!(
            kinds(&s, "new.txt"),
            Some((FileChangeKind::Added, FileChangeKind::None))
        );
        s.unstage_file("new.txt");
        assert!(s.last_error.is_none(), "{:?}", s.last_error);
        assert_eq!(
            kinds(&s, "new.txt"),
            Some((FileChangeKind::None, FileChangeKind::Untracked))
        );
        assert_eq!(r.read("new.txt"), "hello", "unstage never touches disk");
    }

    #[test]
    fn stage_and_unstage_modified_file() {
        let r = dirty_repo();
        let mut s = r.status();
        s.stage_file("tracked.txt");
        assert_eq!(
            kinds(&s, "tracked.txt"),
            Some((FileChangeKind::Modified, FileChangeKind::None))
        );
        s.unstage_file("tracked.txt");
        assert_eq!(
            kinds(&s, "tracked.txt"),
            Some((FileChangeKind::None, FileChangeKind::Modified))
        );
        assert_eq!(r.read("tracked.txt"), "v2");
    }

    #[test]
    fn staging_a_deleted_file_removes_it_from_index() {
        let r = dirty_repo();
        let mut s = r.status();
        s.stage_file("gone.txt");
        assert!(s.last_error.is_none(), "{:?}", s.last_error);
        assert_eq!(
            kinds(&s, "gone.txt"),
            Some((FileChangeKind::Deleted, FileChangeKind::None))
        );
        s.unstage_file("gone.txt");
        assert_eq!(
            kinds(&s, "gone.txt"),
            Some((FileChangeKind::None, FileChangeKind::Deleted))
        );
    }

    #[test]
    fn stage_missing_file_sets_error() {
        let r = dirty_repo();
        let mut s = r.status();
        s.stage_file("does-not-exist.txt");
        let err = s.last_error.clone().unwrap();
        assert!(
            err.starts_with("Failed to stage does-not-exist.txt"),
            "{err}"
        );
    }

    #[test]
    fn operations_without_repo_path() {
        let mut s = GitStatus::new();
        s.stage_file("a");
        s.unstage_file("a");
        assert!(s.last_error.is_none(), "single-file ops silently no-op");
        s.stage_all();
        assert_eq!(s.last_error.as_deref(), Some("No repository path"));
        s.last_error = None;
        s.unstage_all();
        assert_eq!(s.last_error.as_deref(), Some("No repository path"));
    }

    #[test]
    fn operations_on_non_repo_dir_report_open_errors() {
        let dir = tempfile::tempdir().unwrap();
        if git2::Repository::discover(dir.path()).is_ok() {
            return;
        }
        let mut s = GitStatus::new();
        s.repo_path = Some(dir.path().to_path_buf());
        s.stage_file("a");
        assert!(s
            .last_error
            .take()
            .unwrap()
            .starts_with("Failed to open repo"));
        s.unstage_file("a");
        assert!(s
            .last_error
            .take()
            .unwrap()
            .starts_with("Failed to open repo"));
        s.stage_all();
        assert!(s.last_error.take().unwrap().starts_with("Repo error"));
        s.unstage_all();
        assert!(s.last_error.take().unwrap().starts_with("Repo error"));
    }

    #[test]
    fn bare_repository_has_no_index_to_stage_into() {
        let (dir, _) = crate::git::tests::bare_origin();
        let mut s = GitStatus::new();
        s.repo_path = Some(dir.path().to_path_buf());
        s.files = vec![FileStatus {
            path: "a".into(),
            index_status: FileChangeKind::None,
            wt_status: FileChangeKind::Modified,
        }];
        s.stage_file("a");
        let err = s.last_error.take().unwrap();
        assert!(err.starts_with("Failed to stage a"), "{err}");
    }

    #[test]
    #[ignore = "BUG: new files inside an untracked directory cannot be staged; status reports `dir/` and add_path(\"dir/\") fails (stage_all swallows it)"]
    fn stage_all_stages_files_in_new_directories() {
        let r = dirty_repo();
        r.write("newdir/inner.txt", "x");
        let mut s = r.status();
        s.stage_all();
        assert!(s.last_error.is_none(), "{:?}", s.last_error);
        assert_eq!(
            kinds(&s, "newdir/inner.txt"),
            Some((FileChangeKind::Added, FileChangeKind::None))
        );
    }

    #[test]
    #[ignore = "BUG: staging an untracked directory entry (`dir/`) from the git panel fails"]
    fn stage_file_on_untracked_directory_entry() {
        let r = dirty_repo();
        r.write("newdir/inner.txt", "x");
        let mut s = r.status();
        assert!(s.files.iter().any(|f| f.path == "newdir/"));
        s.stage_file("newdir/");
        assert!(s.last_error.is_none(), "{:?}", s.last_error);
        assert!(s.has_staged_files());
    }

    #[test]
    fn locked_index_reports_write_failures() {
        let r = dirty_repo();
        let mut s = r.status();
        lock_index(&r);
        s.stage_file("new.txt");
        assert!(
            s.last_error
                .take()
                .unwrap()
                .starts_with("Failed to write index"),
            "stage_file"
        );
        s.stage_all();
        assert!(
            s.last_error
                .take()
                .unwrap()
                .starts_with("Failed to write index"),
            "stage_all"
        );
        s.unstage_file("tracked.txt");
        assert!(
            s.last_error
                .take()
                .unwrap()
                .starts_with("Failed to unstage tracked.txt"),
            "unstage_file"
        );
    }

    #[test]
    fn stage_all_then_unstage_all() {
        let r = dirty_repo();
        let mut s = r.status();
        s.stage_all();
        assert!(s.last_error.is_none(), "{:?}", s.last_error);
        assert_eq!(
            kinds(&s, "tracked.txt"),
            Some((FileChangeKind::Modified, FileChangeKind::None))
        );
        assert_eq!(
            kinds(&s, "gone.txt"),
            Some((FileChangeKind::Deleted, FileChangeKind::None))
        );
        assert_eq!(
            kinds(&s, "new.txt"),
            Some((FileChangeKind::Added, FileChangeKind::None))
        );

        s.unstage_all();
        assert!(!s.has_staged_files());
        assert_eq!(
            kinds(&s, "tracked.txt"),
            Some((FileChangeKind::None, FileChangeKind::Modified))
        );
        assert_eq!(
            kinds(&s, "gone.txt"),
            Some((FileChangeKind::None, FileChangeKind::Deleted))
        );
        assert_eq!(
            kinds(&s, "new.txt"),
            Some((FileChangeKind::None, FileChangeKind::Untracked))
        );
    }

    #[test]
    fn stage_all_only_touches_worktree_changes() {
        let r = dirty_repo();
        let mut s = r.status();
        s.stage_file("tracked.txt");
        // Now modify again so it is both staged and changed in the worktree.
        r.write("tracked.txt", "v3");
        s.refresh();
        s.stage_all();
        let blob = {
            // Fresh handle: `r.repo` caches its in-memory index.
            let repo = git2::Repository::open(r.path()).unwrap();
            let idx = repo.index().unwrap();
            let e = idx
                .get_path(std::path::Path::new("tracked.txt"), 0)
                .unwrap();
            let content = repo.find_blob(e.id).unwrap().content().to_vec();
            content
        };
        assert_eq!(blob, b"v3", "latest worktree content is staged");
    }

    #[test]
    fn unstage_before_first_commit_removes_from_index() {
        let r = TestRepo::new();
        r.write("a.txt", "a");
        r.write("b.txt", "b");
        let mut s = r.status();
        s.stage_file("a.txt");
        s.stage_file("b.txt");
        assert!(s.has_staged_files());

        s.unstage_file("a.txt");
        assert!(s.last_error.is_none(), "{:?}", s.last_error);
        assert_eq!(
            kinds(&s, "a.txt"),
            Some((FileChangeKind::None, FileChangeKind::Untracked))
        );
        assert_eq!(
            kinds(&s, "b.txt"),
            Some((FileChangeKind::Added, FileChangeKind::None))
        );

        s.unstage_all();
        assert!(!s.has_staged_files());
        assert_eq!(
            kinds(&s, "b.txt"),
            Some((FileChangeKind::None, FileChangeKind::Untracked))
        );
    }

    #[test]
    fn unstage_before_first_commit_with_locked_index_reports_error() {
        let r = TestRepo::new();
        r.write("a.txt", "a");
        let mut s = r.status();
        s.stage_file("a.txt");
        lock_index(&r);
        s.unstage_file("a.txt");
        assert!(s.last_error.unwrap().starts_with("Failed to unstage a.txt"));
    }
}
