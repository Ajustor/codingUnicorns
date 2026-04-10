pub mod blame;
pub mod branches;
pub mod commit;
pub mod merge;
pub mod staging;
pub mod status;

pub use blame::{blame_file, BlameEntry};
pub use branches::{BranchGraphEntry, BranchInfo};

use std::path::PathBuf;

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
        if let Ok(repo) = git2::Repository::discover(&path) {
            if let Ok(head) = repo.head() {
                if let Some(name) = head.shorthand() {
                    self.branch = name.to_string();
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

                        let wt_status = if st.contains(git2::Status::WT_MODIFIED) {
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
