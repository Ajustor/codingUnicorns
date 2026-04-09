pub mod blame;
pub mod merge;
pub use blame::{blame_file, BlameEntry};

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct BranchInfo {
    pub name: String,
    pub is_remote: bool,
    pub is_current: bool,
}

#[derive(Debug, Clone)]
pub struct BranchGraphEntry {
    pub short_hash: String,
    pub message: String,
    pub branches: Vec<String>,
    pub is_head: bool,
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

                        if index_status == FileChangeKind::None
                            && wt_status == FileChangeKind::None
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

    pub fn compute_ahead_behind(&mut self, repo: &git2::Repository) {
        self.ahead = 0;
        self.behind = 0;
        let head = match repo.head() {
            Ok(h) => h,
            Err(_) => return,
        };
        let local_oid = match head.target() {
            Some(oid) => oid,
            None => return,
        };
        let branch_name = match head.shorthand() {
            Some(n) => n.to_string(),
            None => return,
        };
        let remote_ref = format!("refs/remotes/origin/{}", branch_name);
        let remote_oid = match repo.find_reference(&remote_ref) {
            Ok(r) => match r.target() {
                Some(oid) => oid,
                None => return,
            },
            Err(_) => return,
        };
        if let Ok((ahead, behind)) = repo.graph_ahead_behind(local_oid, remote_oid) {
            self.ahead = ahead;
            self.behind = behind;
        }
    }

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
        let paths: Vec<String> = self
            .files
            .iter()
            .filter(|f| f.wt_status != FileChangeKind::None)
            .map(|f| f.path.clone())
            .collect();
        for path in paths {
            self.stage_file(&path);
        }
    }

    pub fn unstage_all(&mut self) {
        let paths: Vec<String> = self
            .files
            .iter()
            .filter(|f| f.index_status != FileChangeKind::None)
            .map(|f| f.path.clone())
            .collect();
        for path in paths {
            self.unstage_file(&path);
        }
    }

    pub fn commit(&mut self, message: &str) -> Result<(), String> {
        let repo_path = self
            .repo_path
            .clone()
            .ok_or_else(|| "No repo path".to_string())?;
        let repo =
            git2::Repository::discover(&repo_path).map_err(|e| format!("Repo error: {e}"))?;
        let mut index = repo.index().map_err(|e| format!("Index error: {e}"))?;
        let tree_oid = index
            .write_tree()
            .map_err(|e| format!("Write tree error: {e}"))?;
        let tree = repo
            .find_tree(tree_oid)
            .map_err(|e| format!("Find tree error: {e}"))?;
        let sig = repo
            .signature()
            .map_err(|e| format!("Signature error: {e}"))?;
        let parent_commits: Vec<git2::Commit> = match repo.head() {
            Ok(head) => {
                let oid = head
                    .target()
                    .ok_or_else(|| "HEAD has no target".to_string())?;
                let commit = repo
                    .find_commit(oid)
                    .map_err(|e| format!("Find commit error: {e}"))?;
                vec![commit]
            }
            Err(_) => vec![],
        };
        let parent_refs: Vec<&git2::Commit> = parent_commits.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
            .map_err(|e| format!("Commit error: {e}"))?;
        self.refresh();
        Ok(())
    }

    pub fn push(&mut self) -> Result<(), String> {
        let repo_path = self
            .repo_path
            .clone()
            .ok_or_else(|| "No repo path".to_string())?;
        let repo =
            git2::Repository::discover(&repo_path).map_err(|e| format!("Repo error: {e}"))?;
        let head = repo.head().map_err(|e| format!("HEAD error: {e}"))?;
        let branch_name = head
            .shorthand()
            .ok_or_else(|| "No branch name".to_string())?
            .to_string();
        let mut remote = repo
            .find_remote("origin")
            .map_err(|e| format!("Remote error: {e}"))?;
        let refspec = format!("refs/heads/{}:refs/heads/{}", branch_name, branch_name);
        remote
            .push(&[&refspec], None)
            .map_err(|e| format!("Push error: {e}"))?;
        self.refresh();
        Ok(())
    }

    pub fn pull(&mut self) -> Result<(), String> {
        let repo_path = self
            .repo_path
            .clone()
            .ok_or_else(|| "No repo path".to_string())?;
        let repo =
            git2::Repository::discover(&repo_path).map_err(|e| format!("Repo error: {e}"))?;
        let head = repo.head().map_err(|e| format!("HEAD error: {e}"))?;
        let branch_name = head
            .shorthand()
            .ok_or_else(|| "No branch name".to_string())?
            .to_string();
        let mut remote = repo
            .find_remote("origin")
            .map_err(|e| format!("Remote error: {e}"))?;
        remote
            .fetch(&[&branch_name], None, None)
            .map_err(|e| format!("Fetch error: {e}"))?;
        let remote_ref = format!("refs/remotes/origin/{}", branch_name);
        let remote_oid = repo
            .find_reference(&remote_ref)
            .map_err(|e| format!("Remote ref error: {e}"))?
            .target()
            .ok_or_else(|| "Remote ref has no target".to_string())?;
        let annotated = repo
            .find_annotated_commit(remote_oid)
            .map_err(|e| format!("Annotated commit error: {e}"))?;
        let (analysis, _) = repo
            .merge_analysis(&[&annotated])
            .map_err(|e| format!("Merge analysis error: {e}"))?;
        if analysis.is_fast_forward() {
            let mut reference = repo
                .find_reference(&format!("refs/heads/{}", branch_name))
                .map_err(|e| format!("Branch ref error: {e}"))?;
            reference
                .set_target(remote_oid, "fast-forward pull")
                .map_err(|e| format!("Fast-forward error: {e}"))?;
            repo.set_head(&format!("refs/heads/{}", branch_name))
                .map_err(|e| format!("Set HEAD error: {e}"))?;
            repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
                .map_err(|e| format!("Checkout error: {e}"))?;
        } else if analysis.is_up_to_date() {
            // nothing to do
        } else {
            return Err("Cannot fast-forward: diverged history. Please merge manually.".to_string());
        }
        self.refresh();
        Ok(())
    }

    pub fn load_branches(&mut self) {
        self.branches.clear();
        if let Some(ref repo_path) = self.repo_path {
            if let Ok(repo) = git2::Repository::discover(repo_path) {
                if let Ok(branches) = repo.branches(None) {
                    for branch_result in branches {
                        if let Ok((branch, branch_type)) = branch_result {
                            if let Some(name) = branch.name().ok().flatten() {
                                let is_remote = branch_type == git2::BranchType::Remote;
                                let is_current = branch.is_head();
                                self.branches.push(BranchInfo {
                                    name: name.to_string(),
                                    is_remote,
                                    is_current,
                                });
                            }
                        }
                    }
                }
            }
        }
        self.load_graph();
    }

    pub fn load_graph(&mut self) {
        self.graph_entries.clear();
        if let Some(ref repo_path) = self.repo_path {
            if let Ok(repo) = git2::Repository::discover(repo_path) {
                let mut revwalk = match repo.revwalk() {
                    Ok(rw) => rw,
                    Err(_) => return,
                };
                revwalk.set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL).ok();
                // Push all local branch tips
                if let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) {
                    for branch_result in branches {
                        if let Ok((branch, _)) = branch_result {
                            if let Ok(reference) = branch.into_reference().resolve() {
                                if let Some(oid) = reference.target() {
                                    revwalk.push(oid).ok();
                                }
                            }
                        }
                    }
                }
                // Collect up to 20 commits
                let mut count = 0;
                for oid_result in &mut revwalk {
                    if count >= 20 {
                        break;
                    }
                    if let Ok(oid) = oid_result {
                        if let Ok(commit) = repo.find_commit(oid) {
                            let short_hash = format!("{:.7}", oid);
                            let message = commit.summary().unwrap_or("").to_string();
                            // Find branches pointing to this commit
                            let mut branch_names: Vec<String> = vec![];
                            for bi in &self.branches {
                                if let Ok(reference) = repo.find_reference(
                                    &if bi.is_remote {
                                        format!("refs/remotes/{}", bi.name)
                                    } else {
                                        format!("refs/heads/{}", bi.name)
                                    },
                                ) {
                                    if reference.target() == Some(oid) {
                                        branch_names.push(bi.name.clone());
                                    }
                                }
                            }
                            let is_head = repo
                                .head()
                                .ok()
                                .and_then(|h| h.target())
                                .map(|h| h == oid)
                                .unwrap_or(false);
                            self.graph_entries.push(BranchGraphEntry {
                                short_hash,
                                message,
                                branches: branch_names,
                                is_head,
                            });
                            count += 1;
                        }
                    }
                }
            }
        }
    }

    pub fn merge_branch(&mut self, branch_name: &str) -> Result<(), String> {
        let repo_path = self.repo_path.as_ref().ok_or("No repository")?;
        let repo = git2::Repository::discover(repo_path).map_err(|e| e.message().to_string())?;
        let reference = repo
            .find_reference(&format!("refs/heads/{}", branch_name))
            .map_err(|e| e.message().to_string())?;
        let annotated = repo
            .reference_to_annotated_commit(&reference)
            .map_err(|e| e.message().to_string())?;
        let (analysis, _) = repo
            .merge_analysis(&[&annotated])
            .map_err(|e| e.message().to_string())?;
        if analysis.is_fast_forward() {
            let target_oid = reference.target().ok_or("No target")?;
            let mut head_ref = repo.head().map_err(|e| e.message().to_string())?;
            head_ref
                .set_target(
                    target_oid,
                    &format!("merge {}: fast-forward", branch_name),
                )
                .map_err(|e| e.message().to_string())?;
            repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
                .map_err(|e| e.message().to_string())?;
        } else if analysis.is_normal() {
            repo.merge(&[&annotated], None, None)
                .map_err(|e| e.message().to_string())?;
        } else {
            return Err("Nothing to merge (already up to date)".to_string());
        }
        self.refresh();
        Ok(())
    }

    pub fn delete_branch(&mut self, branch_name: &str) -> Result<(), String> {
        let repo_path = self.repo_path.as_ref().ok_or("No repository")?;
        let repo = git2::Repository::discover(repo_path).map_err(|e| e.message().to_string())?;
        let mut branch = repo
            .find_branch(branch_name, git2::BranchType::Local)
            .map_err(|e| e.message().to_string())?;
        branch.delete().map_err(|e| e.message().to_string())?;
        self.refresh();
        Ok(())
    }

    pub fn create_branch(&mut self, name: &str, from_branch: &str) -> Result<(), String> {
        let repo_path = self.repo_path.as_ref().ok_or("No repository")?;
        let repo = git2::Repository::discover(repo_path).map_err(|e| e.message().to_string())?;
        let reference = repo
            .find_reference(&format!("refs/heads/{}", from_branch))
            .map_err(|e| e.message().to_string())?;
        let commit = reference
            .peel_to_commit()
            .map_err(|e| e.message().to_string())?;
        repo.branch(name, &commit, false)
            .map_err(|e| e.message().to_string())?;
        self.refresh();
        Ok(())
    }

    pub fn rename_branch(&mut self, old_name: &str, new_name: &str) -> Result<(), String> {
        let repo_path = self.repo_path.as_ref().ok_or("No repository")?;
        let repo = git2::Repository::discover(repo_path).map_err(|e| e.message().to_string())?;
        let mut branch = repo
            .find_branch(old_name, git2::BranchType::Local)
            .map_err(|e| e.message().to_string())?;
        branch
            .rename(new_name, false)
            .map_err(|e| e.message().to_string())?;
        self.refresh();
        Ok(())
    }

    pub fn checkout_branch(&mut self, branch_name: &str) -> Result<(), String> {
        let repo_path = self.repo_path.as_ref().ok_or("No repository")?;
        let repo = git2::Repository::discover(repo_path).map_err(|e| e.message().to_string())?;
        let (object, reference) = repo.revparse_ext(branch_name).map_err(|e| e.message().to_string())?;
        repo.checkout_tree(&object, None).map_err(|e| e.message().to_string())?;
        if let Some(reference) = reference {
            repo.set_head(reference.name().unwrap_or(&format!("refs/heads/{}", branch_name)))
                .map_err(|e| e.message().to_string())?;
        } else {
            repo.set_head(&format!("refs/heads/{}", branch_name))
                .map_err(|e| e.message().to_string())?;
        }
        self.refresh();
        Ok(())
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
