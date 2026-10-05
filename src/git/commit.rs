use super::remote::{fetch_remote, push_remote};
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
        let mut parent_commits: Vec<git2::Commit> = match repo.head() {
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
        // Concluding a merge: MERGE_HEAD lists the other parent(s).
        let merging = repo.state() == git2::RepositoryState::Merge;
        if merging {
            let mut merge_heads = Vec::new();
            // `mergehead_foreach` needs `&mut Repository`, but `repo` is
            // borrowed by the parent commits, so use a second handle.
            let mut merge_repo =
                git2::Repository::open(repo.path()).map_err(|e| format!("Repo error: {e}"))?;
            merge_repo
                .mergehead_foreach(|oid| {
                    merge_heads.push(*oid);
                    true
                })
                .map_err(|e| format!("MERGE_HEAD error: {e}"))?;
            for oid in merge_heads {
                parent_commits.push(
                    repo.find_commit(oid)
                        .map_err(|e| format!("Find commit error: {e}"))?,
                );
            }
        }
        let parent_refs: Vec<&git2::Commit> = parent_commits.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs)
            .map_err(|e| format!("Commit error: {e}"))?;
        if merging {
            repo.cleanup_state()
                .map_err(|e| format!("Cleanup state error: {e}"))?;
        }
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
        push_remote(&mut remote, &[&refspec])?;
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
        fetch_remote(&mut remote, &[&branch_name])?;
        if repo.state() != git2::RepositoryState::Clean {
            self.refresh();
            return Err(
                "Cannot pull: a merge is already in progress. Resolve and commit it first."
                    .to_string(),
            );
        }
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
        let result = if analysis.is_up_to_date() {
            Ok(())
        } else if analysis.is_fast_forward() {
            fast_forward(&repo, &branch_name, remote_oid)
        } else if analysis.is_normal() {
            merge_upstream(&repo, &branch_name, &annotated)
        } else {
            Err("Cannot pull: unsupported merge analysis result".to_string())
        };
        // Refresh even on failure: a conflicted merge must show its files.
        self.refresh();
        result
    }
}

/// Map a checkout/merge error caused by uncommitted local changes.
fn local_changes_error(context: &str, e: git2::Error) -> String {
    if e.code() == git2::ErrorCode::Conflict {
        format!("Pull would overwrite local changes: commit or stash them first ({e})")
    } else {
        format!("{context} error: {e}")
    }
}

/// Move `branch` to `target`, updating the working tree without discarding
/// uncommitted changes (the checkout fails instead).
fn fast_forward(
    repo: &git2::Repository,
    branch_name: &str,
    target: git2::Oid,
) -> Result<(), String> {
    let commit = repo
        .find_commit(target)
        .map_err(|e| format!("Find commit error: {e}"))?;
    repo.checkout_tree(
        commit.as_object(),
        Some(git2::build::CheckoutBuilder::new().safe()),
    )
    .map_err(|e| local_changes_error("Checkout", e))?;
    let refname = format!("refs/heads/{}", branch_name);
    repo.find_reference(&refname)
        .map_err(|e| format!("Branch ref error: {e}"))?
        .set_target(target, "pull: fast-forward")
        .map_err(|e| format!("Fast-forward error: {e}"))?;
    repo.set_head(&refname)
        .map_err(|e| format!("Set HEAD error: {e}"))?;
    Ok(())
}

/// Merge the fetched upstream into the current branch. Without conflicts the
/// merge commit is created right away; with conflicts the repository is left
/// in merging state (markers in the files) for the merge view, and the
/// regular commit flow concludes it.
fn merge_upstream(
    repo: &git2::Repository,
    branch_name: &str,
    upstream: &git2::AnnotatedCommit,
) -> Result<(), String> {
    repo.merge(&[upstream], None, None)
        .map_err(|e| local_changes_error("Merge", e))?;
    let mut index = repo.index().map_err(|e| format!("Index error: {e}"))?;
    if index.has_conflicts() {
        return Err("Merge conflicts: resolve them then commit".to_string());
    }
    let tree_oid = index
        .write_tree()
        .map_err(|e| format!("Write tree error: {e}"))?;
    let tree = repo
        .find_tree(tree_oid)
        .map_err(|e| format!("Find tree error: {e}"))?;
    let sig = repo
        .signature()
        .map_err(|e| format!("Signature error: {e}"))?;
    let head = repo
        .head()
        .and_then(|h| h.peel_to_commit())
        .map_err(|e| format!("HEAD error: {e}"))?;
    let theirs = repo
        .find_commit(upstream.id())
        .map_err(|e| format!("Find commit error: {e}"))?;
    let message = format!("Merge remote-tracking branch 'origin/{branch_name}'");
    repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[&head, &theirs])
        .map_err(|e| format!("Commit error: {e}"))?;
    repo.cleanup_state()
        .map_err(|e| format!("Cleanup state error: {e}"))?;
    Ok(())
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
    fn pull_merges_diverged_history_with_merge_commit() {
        let (r, _bare, url) = with_origin();
        let (_other_dir, other) = clone(&url);
        let remote_tip = commit_in(&other, "b.txt", "remote", "remote work");
        push_from(&other);
        r.write("a.txt", "local");
        let local = r.commit_all("local work");

        let mut s = r.status();
        s.pull().unwrap();
        let fresh = git2::Repository::open(r.path()).unwrap();
        let c = fresh.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(
            c.message(),
            Some("Merge remote-tracking branch 'origin/main'")
        );
        assert_eq!(c.parent_id(0).unwrap(), local);
        assert_eq!(c.parent_id(1).unwrap(), remote_tip);
        assert_eq!(c.author().name(), Some("Test"), "repo identity");
        assert_eq!(fresh.state(), git2::RepositoryState::Clean);
        assert_eq!(r.read("a.txt"), "local");
        assert_eq!(r.read("b.txt"), "remote");
        assert!(s.files.is_empty(), "{:?}", s.files);
        // Merged but not pushed yet: two local commits ahead.
        assert_eq!((s.ahead, s.behind), (2, 0));
        s.push().unwrap();
        assert_eq!((s.ahead, s.behind), (0, 0));
    }

    #[test]
    fn pull_with_conflicts_leaves_merge_for_the_merge_view() {
        let (r, _bare, url) = with_origin();
        let (_other_dir, other) = clone(&url);
        let remote_tip = commit_in(&other, "a.txt", "theirs\n", "remote work");
        push_from(&other);
        r.write("a.txt", "ours\n");
        let local = r.commit_all("local work");

        let mut s = r.status();
        let err = s.pull().unwrap_err();
        assert_eq!(err, "Merge conflicts: resolve them then commit");
        assert_eq!(r.head_oid(), local, "no commit yet");
        let fresh = git2::Repository::open(r.path()).unwrap();
        assert_eq!(fresh.state(), git2::RepositoryState::Merge);
        assert!(fresh.index().unwrap().has_conflicts());
        let parsed = crate::git::merge::parse_conflict_file(&r.read("a.txt")).unwrap();
        assert_eq!(parsed.hunks[0].ours, ["ours"]);
        assert_eq!(parsed.hunks[0].theirs, ["theirs"]);
        assert!(
            s.files.iter().any(|f| f.path == "a.txt"),
            "conflicted file listed: {:?}",
            s.files
        );

        // A second pull refuses while the merge is in progress.
        assert!(s
            .pull()
            .unwrap_err()
            .contains("merge is already in progress"));

        // Resolve, stage and commit through the normal flow.
        r.write("a.txt", "resolved\n");
        s.refresh();
        s.stage_file("a.txt");
        s.commit("Merge remote-tracking branch 'origin/main'")
            .unwrap();
        let fresh = git2::Repository::open(r.path()).unwrap();
        let c = fresh.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(c.parent_count(), 2);
        assert_eq!(c.parent_id(1).unwrap(), remote_tip);
        assert_eq!(fresh.state(), git2::RepositoryState::Clean);
    }

    #[test]
    fn pull_fast_forward_keeps_unrelated_local_changes_and_refuses_overwrite() {
        let (r, _bare, url) = with_origin();
        let (_other_dir, other) = clone(&url);
        let remote_tip = commit_in(&other, "b.txt", "remote", "remote work");
        push_from(&other);

        // Uncommitted change to a file the pull touches: refused, nothing lost.
        r.write("b.txt", "local draft");
        let before = r.head_oid();
        let mut s = r.status();
        let err = s.pull().unwrap_err();
        assert!(err.contains("local changes"), "{err}");
        assert_eq!(r.head_oid(), before);
        assert_eq!(r.read("b.txt"), "local draft");

        // Unrelated uncommitted change: fast-forward proceeds and keeps it.
        r.remove("b.txt");
        r.write("a.txt", "edited");
        s.pull().unwrap();
        assert_eq!(r.head_oid(), remote_tip);
        assert_eq!(r.read("a.txt"), "edited");
        assert_eq!(r.read("b.txt"), "remote");
    }

    #[test]
    fn push_non_fast_forward_is_reported_as_rejected() {
        let (r, _bare, url) = with_origin();
        let (_other_dir, other) = clone(&url);
        commit_in(&other, "b.txt", "remote", "remote work");
        push_from(&other);
        r.write("a.txt", "local");
        r.commit_all("local work");
        let mut s = r.status();
        let err = s.push().unwrap_err();
        assert!(err.contains("non-fast-forward"), "{err}");
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
