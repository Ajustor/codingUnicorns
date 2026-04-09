use super::GitStatus;

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

impl GitStatus {
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
}
