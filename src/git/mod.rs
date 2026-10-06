pub mod blame;
pub mod branches;
pub mod commit;
pub mod merge;
pub mod remote;
pub mod staging;
pub mod stash;
pub mod status;

pub use blame::{blame_file, BlameEntry};
pub use branches::{BranchGraphEntry, BranchInfo};
pub use stash::StashEntry;

use std::path::{Path, PathBuf};

/// `path` relative to the repository `workdir`.
///
/// Falls back to comparing canonical paths when a plain prefix match fails,
/// so symlinked locations (e.g. macOS `/var` -> `/private/var`) still resolve.
pub fn relative_to_workdir(path: &Path, workdir: &Path) -> Option<PathBuf> {
    if let Ok(rel) = path.strip_prefix(workdir) {
        return Some(rel.to_path_buf());
    }
    let path = path.canonicalize().ok()?;
    let workdir = workdir.canonicalize().ok()?;
    path.strip_prefix(workdir).ok().map(Path::to_path_buf)
}

#[derive(Debug, Clone, Default)]
pub struct FileStatus {
    pub path: String,
    pub index_status: FileChangeKind,
    pub wt_status: FileChangeKind,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum FileChangeKind {
    #[default]
    None,
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
}

pub struct GitStatus {
    pub branch: String,
    pub files: Vec<FileStatus>,
    pub branches: Vec<BranchInfo>,
    pub graph_entries: Vec<BranchGraphEntry>,
    pub repo_path: Option<PathBuf>,
    pub ahead: usize,
    pub behind: usize,
    pub last_error: Option<String>,
    /// Stash stack, most recent first.
    pub stashes: Vec<StashEntry>,
}

impl GitStatus {
    pub fn new() -> Self {
        Self {
            branch: String::from("—"),
            files: vec![],
            branches: vec![],
            graph_entries: vec![],
            repo_path: None,
            ahead: 0,
            behind: 0,
            last_error: None,
            stashes: vec![],
        }
    }

    /// Open the git repository. Centralizes the discover+error pattern.
    pub(crate) fn open_repo(&self) -> Result<git2::Repository, String> {
        let repo_path = self
            .repo_path
            .as_ref()
            .ok_or_else(|| "No repository path".to_string())?;
        git2::Repository::discover(repo_path).map_err(|e| format!("Repo error: {e}"))
    }

    pub fn load(&mut self, path: PathBuf) {
        self.repo_path = Some(path.clone());
        self.last_error = None;
        self.stashes.clear();
        if let Ok(mut repo) = git2::Repository::discover(&path) {
            match repo.head() {
                Ok(head) => {
                    if let Some(name) = head.shorthand() {
                        self.branch = name.to_string();
                    }
                }
                Err(_) => {
                    // Unborn branch (no commits yet): HEAD is a symbolic ref
                    // to a branch that does not exist yet.
                    if let Some(name) = repo.find_reference("HEAD").ok().and_then(|h| {
                        h.symbolic_target()
                            .map(|t| t.strip_prefix("refs/heads/").unwrap_or(t).to_string())
                    }) {
                        self.branch = name;
                    }
                }
            }
            self.compute_ahead_behind(&repo);
            let mut opts = git2::StatusOptions::new();
            opts.include_untracked(true);
            if let Ok(statuses) = repo.statuses(Some(&mut opts)) {
                self.files = statuses
                    .iter()
                    .filter_map(|s| {
                        let path = s.path()?.to_string();
                        let st = s.status();
                        if st.contains(git2::Status::IGNORED) {
                            return None;
                        }

                        let index_status = if st.contains(git2::Status::INDEX_MODIFIED) {
                            FileChangeKind::Modified
                        } else if st.contains(git2::Status::INDEX_NEW) {
                            FileChangeKind::Added
                        } else if st.contains(git2::Status::INDEX_DELETED) {
                            FileChangeKind::Deleted
                        } else if st.contains(git2::Status::INDEX_RENAMED) {
                            FileChangeKind::Renamed
                        } else {
                            FileChangeKind::None
                        };

                        // Unmerged paths only carry CONFLICTED: list them as
                        // worktree changes so they can be resolved and staged.
                        let wt_status = if st.contains(git2::Status::WT_MODIFIED)
                            || st.contains(git2::Status::CONFLICTED)
                        {
                            FileChangeKind::Modified
                        } else if st.contains(git2::Status::WT_NEW) {
                            FileChangeKind::Untracked
                        } else if st.contains(git2::Status::WT_DELETED) {
                            FileChangeKind::Deleted
                        } else if st.contains(git2::Status::WT_RENAMED) {
                            FileChangeKind::Renamed
                        } else {
                            FileChangeKind::None
                        };

                        if index_status == FileChangeKind::None && wt_status == FileChangeKind::None
                        {
                            return None;
                        }

                        Some(FileStatus {
                            path,
                            index_status,
                            wt_status,
                        })
                    })
                    .collect();
            }
            self.stashes = stash::list_stashes(&mut repo).unwrap_or_default();
        }
        self.load_branches();
    }

    pub fn has_staged_files(&self) -> bool {
        self.files
            .iter()
            .any(|f| f.index_status != FileChangeKind::None)
    }

    pub fn refresh(&mut self) {
        if let Some(path) = self.repo_path.clone() {
            self.load(path);
        }
    }
}

/// Shared fixtures for the git tests (used by the sibling modules' tests too).
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::Path;

    pub(crate) fn sig(name: &str) -> git2::Signature<'static> {
        git2::Signature::now(name, "test@example.com").unwrap()
    }

    /// A throwaway repository in a temp dir, with a deterministic initial
    /// branch (`main`) and a repo-local identity so `repo.signature()` works
    /// without relying on the user's global git config.
    pub(crate) struct TestRepo {
        pub dir: tempfile::TempDir,
        pub repo: git2::Repository,
    }

    impl TestRepo {
        pub fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let mut opts = git2::RepositoryInitOptions::new();
            opts.initial_head("main");
            let repo = git2::Repository::init_opts(dir.path(), &opts).unwrap();
            {
                let mut cfg = repo.config().unwrap();
                cfg.set_str("user.name", "Test").unwrap();
                cfg.set_str("user.email", "test@example.com").unwrap();
            }
            Self { dir, repo }
        }

        pub fn path(&self) -> &Path {
            self.dir.path()
        }

        pub fn write(&self, rel: &str, content: &str) {
            let p = self.path().join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(p, content).unwrap();
        }

        pub fn read(&self, rel: &str) -> String {
            std::fs::read_to_string(self.path().join(rel)).unwrap()
        }

        pub fn remove(&self, rel: &str) {
            std::fs::remove_file(self.path().join(rel)).unwrap();
        }

        /// Stage everything (including deletions) and commit as `author`.
        pub fn commit_as(&self, msg: &str, author: &str) -> git2::Oid {
            let mut index = self.repo.index().unwrap();
            index
                .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
                .unwrap();
            index.update_all(["*"], None).unwrap();
            index.write().unwrap();
            let tree = self.repo.find_tree(index.write_tree().unwrap()).unwrap();
            let s = sig(author);
            let parent = self.repo.head().ok().and_then(|h| h.peel_to_commit().ok());
            let parents: Vec<&git2::Commit> = parent.iter().collect();
            self.repo
                .commit(Some("HEAD"), &s, &s, msg, &tree, &parents)
                .unwrap()
        }

        pub fn commit_all(&self, msg: &str) -> git2::Oid {
            self.commit_as(msg, "Test")
        }

        pub fn head_oid(&self) -> git2::Oid {
            self.repo.head().unwrap().target().unwrap()
        }

        pub fn head_name(&self) -> String {
            self.repo.head().unwrap().shorthand().unwrap().to_string()
        }

        /// Create branch `name` at HEAD.
        pub fn branch(&self, name: &str) {
            let c = self.repo.head().unwrap().peel_to_commit().unwrap();
            self.repo.branch(name, &c, false).unwrap();
        }

        /// Switch HEAD + working tree to local branch `name`.
        pub fn switch(&self, name: &str) {
            self.repo.set_head(&format!("refs/heads/{name}")).unwrap();
            self.repo
                .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
                .unwrap();
        }

        pub fn status(&self) -> GitStatus {
            let mut s = GitStatus::new();
            s.load(self.path().to_path_buf());
            s
        }
    }

    /// A bare repository usable as a local `origin`.
    pub(crate) fn bare_origin() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = git2::RepositoryInitOptions::new();
        opts.bare(true).initial_head("main");
        git2::Repository::init_opts(dir.path(), &opts).unwrap();
        let url = dir.path().to_string_lossy().to_string();
        (dir, url)
    }

    fn file<'a>(s: &'a GitStatus, path: &str) -> Option<&'a FileStatus> {
        s.files.iter().find(|f| f.path == path)
    }

    #[test]
    fn new_status_is_empty() {
        let s = GitStatus::new();
        assert_eq!(s.branch, "—");
        assert!(s.files.is_empty() && s.branches.is_empty() && s.graph_entries.is_empty());
        assert!(s.repo_path.is_none());
        assert_eq!((s.ahead, s.behind), (0, 0));
        assert!(s.last_error.is_none());
        assert!(!s.has_staged_files());
    }

    #[test]
    fn open_repo_reports_missing_path_and_non_repo() {
        let mut s = GitStatus::new();
        assert_eq!(s.open_repo().err().unwrap(), "No repository path");
        let dir = tempfile::tempdir().unwrap();
        s.repo_path = Some(dir.path().to_path_buf());
        // A temp dir is (normally) not inside any repository.
        if git2::Repository::discover(dir.path()).is_err() {
            assert!(s.open_repo().err().unwrap().starts_with("Repo error:"));
        }
        let r = TestRepo::new();
        s.repo_path = Some(r.path().to_path_buf());
        assert!(s.open_repo().is_ok());
    }

    #[test]
    fn load_on_non_repo_keeps_placeholder_branch_and_no_files() {
        let dir = tempfile::tempdir().unwrap();
        if git2::Repository::discover(dir.path()).is_ok() {
            return; // temp dir nested inside a repo: not meaningful here
        }
        std::fs::write(dir.path().join("x.txt"), "x").unwrap();
        let mut s = GitStatus::new();
        s.last_error = Some("stale".into());
        s.load(dir.path().to_path_buf());
        assert_eq!(s.repo_path.as_deref(), Some(dir.path()));
        assert!(s.last_error.is_none(), "load clears previous errors");
        assert_eq!(s.branch, "—");
        assert!(s.files.is_empty() && s.branches.is_empty());
    }

    #[test]
    fn load_before_first_commit_lists_files_and_branch_after_it() {
        let r = TestRepo::new();
        r.write("new.txt", "hi");
        let s = r.status();
        // Unborn HEAD: the branch name comes from the symbolic HEAD target.
        assert_eq!(s.branch, "main");
        let f = file(&s, "new.txt").unwrap();
        assert_eq!(f.wt_status, FileChangeKind::Untracked);
        assert_eq!(f.index_status, FileChangeKind::None);

        r.commit_all("init");
        assert_eq!(r.status().branch, "main");
    }

    #[test]
    fn load_unborn_branch_uses_symbolic_head_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = git2::RepositoryInitOptions::new();
        opts.initial_head("trunk");
        git2::Repository::init_opts(dir.path(), &opts).unwrap();
        let mut s = GitStatus::new();
        s.load(dir.path().to_path_buf());
        assert_eq!(s.branch, "trunk");
    }

    #[test]
    fn load_classifies_index_and_worktree_changes() {
        let r = TestRepo::new();
        r.write("unchanged.txt", "same");
        r.write("wt_mod.txt", "v1");
        r.write("idx_mod.txt", "v1");
        r.write("both_mod.txt", "v1");
        r.write("wt_del.txt", "x");
        r.write("idx_del.txt", "x");
        r.write(".gitignore", "*.log\n");
        r.commit_all("init");

        r.write("wt_mod.txt", "v2");
        r.write("idx_mod.txt", "v2");
        r.write("both_mod.txt", "v2");
        r.write("added.txt", "new");
        r.write("untracked.txt", "u");
        r.write("debug.log", "ignored");
        r.remove("wt_del.txt");
        {
            let mut idx = r.repo.index().unwrap();
            idx.add_path(Path::new("idx_mod.txt")).unwrap();
            idx.add_path(Path::new("both_mod.txt")).unwrap();
            idx.add_path(Path::new("added.txt")).unwrap();
            idx.remove_path(Path::new("idx_del.txt")).unwrap();
            idx.write().unwrap();
        }
        r.write("both_mod.txt", "v3");

        let s = r.status();
        use FileChangeKind::*;
        let kinds = |p: &str| {
            let f = file(&s, p).unwrap_or_else(|| panic!("{p} missing from {:?}", s.files));
            (f.index_status.clone(), f.wt_status.clone())
        };
        assert_eq!(kinds("wt_mod.txt"), (None, Modified));
        assert_eq!(kinds("idx_mod.txt"), (Modified, None));
        assert_eq!(kinds("both_mod.txt"), (Modified, Modified));
        assert_eq!(kinds("added.txt"), (Added, None));
        assert_eq!(kinds("wt_del.txt"), (None, Deleted));
        // Removed from the index but still on disk: staged delete + untracked.
        assert_eq!(kinds("idx_del.txt"), (Deleted, Untracked));
        assert_eq!(kinds("untracked.txt"), (None, Untracked));
        assert!(file(&s, "unchanged.txt").is_none());
        assert!(file(&s, "debug.log").is_none(), "ignored files are hidden");
        assert!(s.has_staged_files());
    }

    #[test]
    fn untracked_directory_is_reported_as_single_entry() {
        let r = TestRepo::new();
        r.write("a.txt", "1");
        r.commit_all("init");
        r.write("newdir/one.txt", "1");
        r.write("newdir/two.txt", "2");
        let s = r.status();
        let paths: Vec<&str> = s.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["newdir/"]);
        assert_eq!(s.files[0].wt_status, FileChangeKind::Untracked);
    }

    #[test]
    fn has_staged_files_false_with_only_worktree_changes() {
        let r = TestRepo::new();
        r.write("a.txt", "1");
        r.commit_all("init");
        r.write("a.txt", "2");
        r.write("b.txt", "new");
        let s = r.status();
        assert_eq!(s.files.len(), 2);
        assert!(!s.has_staged_files());
    }

    #[test]
    fn refresh_reloads_from_disk_and_is_noop_without_path() {
        let mut empty = GitStatus::new();
        empty.refresh();
        assert!(empty.repo_path.is_none());

        let r = TestRepo::new();
        r.write("a.txt", "1");
        r.commit_all("init");
        let mut s = r.status();
        assert!(s.files.is_empty());
        r.write("a.txt", "2");
        s.refresh();
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].wt_status, FileChangeKind::Modified);
    }

    #[test]
    fn load_from_subdirectory_discovers_repository() {
        let r = TestRepo::new();
        r.write("sub/deeper/f.txt", "x");
        r.commit_all("init");
        r.write("sub/deeper/f.txt", "y");
        let mut s = GitStatus::new();
        s.load(r.path().join("sub").join("deeper"));
        assert_eq!(s.branch, "main");
        assert_eq!(s.files[0].path, "sub/deeper/f.txt");
    }

    #[test]
    fn default_file_status_is_unchanged() {
        let f = FileStatus::default();
        assert_eq!(f.path, "");
        assert_eq!(f.index_status, FileChangeKind::None);
        assert_eq!(f.wt_status, FileChangeKind::None);
    }
}
