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
            return Err(
                "Cannot fast-forward: diverged history. Please merge manually.".to_string(),
            );
        }
        self.refresh();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::{bare_origin, TestRepo};

    /// A repo with one commit pushed to a fresh bare `origin`.
    fn with_origin() -> (TestRepo, tempfile::TempDir, String) {
        let r = TestRepo::new();
        r.write("a.txt", "1");
        r.commit_all("init");
        let (bare, url) = bare_origin();
        r.repo.remote("origin", &url).unwrap();
        let mut s = r.status();
        s.push().unwrap();
        (r, bare, url)
    }

    /// Clone `url` into a new temp dir and give it a local identity.
    fn clone(url: &str) -> (tempfile::TempDir, git2::Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::clone(url, dir.path()).unwrap();
        (dir, repo)
    }

    fn commit_in(repo: &git2::Repository, rel: &str, content: &str, msg: &str) -> git2::Oid {
        let wd = repo.workdir().unwrap();
        std::fs::write(wd.join(rel), content).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new(rel)).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let s = crate::git::tests::sig("Other");
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &s, &s, msg, &tree, &[&parent])
            .unwrap()
    }

    fn push_from(repo: &git2::Repository) {
        repo.find_remote("origin")
            .unwrap()
            .push(&["refs/heads/main:refs/heads/main"], None)
            .unwrap();
    }

    // ── commit ─────────────────────────────────────────────────────────────

    #[test]
    fn first_commit_has_no_parent_and_uses_repo_identity() {
        let r = TestRepo::new();
        r.write("a.txt", "hello");
        let mut s = r.status();
        s.stage_file("a.txt");
        s.commit("initial commit").unwrap();

        let c = r.repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(c.message(), Some("initial commit"));
        assert_eq!(c.parent_count(), 0);
        assert_eq!(c.author().name(), Some("Test"));
        assert_eq!(c.author().email(), Some("test@example.com"));
        assert!(c.tree().unwrap().get_name("a.txt").is_some());
        // Status was refreshed: nothing left to commit, branch now known.
        assert!(s.files.is_empty());
        assert_eq!(s.branch, "main");
    }

    #[test]
    fn commit_only_includes_staged_changes_and_chains_parent() {
        let r = TestRepo::new();
        r.write("a.txt", "1");
        r.write("b.txt", "1");
        let first = r.commit_all("init");
        r.write("a.txt", "2");
        r.write("b.txt", "2");
        let mut s = r.status();
        s.stage_file("a.txt");
        s.commit("change a").unwrap();

        let c = r.repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(c.parent_id(0).unwrap(), first);
        let tree = c.tree().unwrap();
        let blob = |n: &str| {
            r.repo
                .find_blob(tree.get_name(n).unwrap().id())
                .unwrap()
                .content()
                .to_vec()
        };
        assert_eq!(blob("a.txt"), b"2");
        assert_eq!(blob("b.txt"), b"1", "unstaged change not committed");
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].path, "b.txt");
    }

    #[test]
    fn commit_errors() {
        let mut s = GitStatus::new();
        assert_eq!(s.commit("x").unwrap_err(), "No repo path");

        let dir = tempfile::tempdir().unwrap();
        if git2::Repository::discover(dir.path()).is_err() {
            s.repo_path = Some(dir.path().to_path_buf());
            assert!(s.commit("x").unwrap_err().starts_with("Repo error"));
        }
    }

    #[test]
    fn commit_without_valid_identity_fails_cleanly() {
        let r = TestRepo::new();
        r.write("a.txt", "1");
        // An empty repo-local name overrides any global identity.
        r.repo.config().unwrap().set_str("user.name", "").unwrap();
        let mut s = r.status();
        s.stage_file("a.txt");
        let err = s.commit("x").unwrap_err();
        assert!(err.starts_with("Signature error"), "{err}");
        assert!(r.repo.head().is_err(), "nothing committed");
    }

    #[test]
    fn commit_with_unresolved_conflicts_fails() {
        let r = TestRepo::new();
        r.write("f.txt", "base\n");
        r.commit_all("base");
        r.branch("other");
        r.write("f.txt", "main side\n");
        r.commit_all("main");
        r.switch("other");
        r.write("f.txt", "other side\n");
        r.commit_all("other");
        r.switch("main");
        let mut s = r.status();
        s.merge_branch("other").unwrap();
        let err = s.commit("merge").unwrap_err();
        assert!(err.starts_with("Write tree error"), "{err}");
    }

    // ── push ───────────────────────────────────────────────────────────────

    #[test]
    fn push_updates_origin_branch() {
        let (r, bare, _) = with_origin();
        let bare_repo = git2::Repository::open_bare(bare.path()).unwrap();
        let remote_main = |b: &git2::Repository| {
            b.find_reference("refs/heads/main")
                .unwrap()
                .target()
                .unwrap()
        };
        assert_eq!(remote_main(&bare_repo), r.head_oid());

        r.write("a.txt", "2");
        let tip = r.commit_all("second");
        let mut s = r.status();
        s.push().unwrap();
        assert_eq!(remote_main(&bare_repo), tip);
    }

    #[test]
    fn push_errors() {
        let mut s = GitStatus::new();
        assert_eq!(s.push().unwrap_err(), "No repo path");

        let r = TestRepo::new();
        let mut s = r.status();
        assert!(
            s.push().unwrap_err().starts_with("HEAD error"),
            "unborn HEAD"
        );

        r.write("a", "1");
        r.commit_all("c");
        assert!(
            s.push().unwrap_err().starts_with("Remote error"),
            "no origin"
        );

        let missing = r.path().join("no-such-remote-dir");
        r.repo.remote("origin", &missing.to_string_lossy()).unwrap();
        assert!(s.push().unwrap_err().starts_with("Push error"));
    }

    #[test]
    fn push_and_pull_report_missing_repo() {
        let dir = tempfile::tempdir().unwrap();
        if git2::Repository::discover(dir.path()).is_ok() {
            return;
        }
        let mut s = GitStatus::new();
        s.repo_path = Some(dir.path().to_path_buf());
        assert!(s.push().unwrap_err().starts_with("Repo error"));
        assert!(s.pull().unwrap_err().starts_with("Repo error"));
    }

    // ── pull ───────────────────────────────────────────────────────────────

    #[test]
    fn pull_up_to_date_is_ok_and_changes_nothing() {
        let (r, _bare, _) = with_origin();
        let before = r.head_oid();
        let mut s = r.status();
        s.pull().unwrap();
        assert_eq!(r.head_oid(), before);
        assert_eq!((s.ahead, s.behind), (0, 0));
    }

    #[test]
    fn pull_fast_forwards_to_remote_commits() {
        let (r, _bare, url) = with_origin();
        let (_other_dir, other) = clone(&url);
        let remote_tip = commit_in(&other, "b.txt", "from other", "remote work");
        push_from(&other);

        let mut s = r.status();
        s.pull().unwrap();
        assert_eq!(r.head_oid(), remote_tip);
        assert_eq!(r.head_name(), "main");
        assert_eq!(r.read("b.txt"), "from other", "worktree checked out");
        assert!(s.files.is_empty());
        assert_eq!((s.ahead, s.behind), (0, 0));
    }

    #[test]
    fn pull_refuses_diverged_history() {
        let (r, _bare, url) = with_origin();
        let (_other_dir, other) = clone(&url);
        commit_in(&other, "b.txt", "remote", "remote work");
        push_from(&other);
        r.write("a.txt", "local");
        let local = r.commit_all("local work");

        let mut s = r.status();
        let err = s.pull().unwrap_err();
        assert!(err.contains("diverged"), "{err}");
        assert_eq!(r.head_oid(), local, "local branch untouched");
        // The fetch did update the tracking ref, so we now know we're behind.
        s.refresh();
        assert_eq!((s.ahead, s.behind), (1, 1));
    }

    #[test]
    fn pull_errors() {
        let mut s = GitStatus::new();
        assert_eq!(s.pull().unwrap_err(), "No repo path");

        let r = TestRepo::new();
        let mut s = r.status();
        assert!(s.pull().unwrap_err().starts_with("HEAD error"));

        r.write("a", "1");
        r.commit_all("c");
        assert!(s.pull().unwrap_err().starts_with("Remote error"));

        let missing = r.path().join("no-such-remote-dir");
        r.repo.remote("origin", &missing.to_string_lossy()).unwrap();
        assert!(s.pull().unwrap_err().starts_with("Fetch error"));
    }

    #[test]
    fn pull_branch_missing_on_remote_fails() {
        let (r, _bare, _) = with_origin();
        r.branch("local-only");
        r.switch("local-only");
        let mut s = r.status();
        let err = s.pull().unwrap_err();
        assert!(
            err.starts_with("Fetch error") || err.starts_with("Remote ref error"),
            "{err}"
        );
    }
}
