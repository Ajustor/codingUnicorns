use super::GitStatus;

impl GitStatus {
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
}
