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
                    for (branch, branch_type) in branches.flatten() {
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
                revwalk
                    .set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)
                    .ok();
                // Push all local branch tips
                if let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) {
                    for (branch, _) in branches.flatten() {
                        if let Ok(reference) = branch.into_reference().resolve() {
                            if let Some(oid) = reference.target() {
                                revwalk.push(oid).ok();
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
                                if let Ok(reference) = repo.find_reference(&if bi.is_remote {
                                    format!("refs/remotes/{}", bi.name)
                                } else {
                                    format!("refs/heads/{}", bi.name)
                                }) {
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
        let (object, reference) = repo
            .revparse_ext(branch_name)
            .map_err(|e| e.message().to_string())?;
        repo.checkout_tree(&object, None)
            .map_err(|e| e.message().to_string())?;
        match reference.as_ref().and_then(|r| r.name()) {
            // A reference (branch, tag, remote): libgit2 attaches HEAD to a
            // local branch and detaches for anything else.
            Some(name) => repo.set_head(name),
            // Not a reference (commit sha, `HEAD~1`, ...): detach at it.
            None => {
                let commit = object
                    .peel_to_commit()
                    .map_err(|e| e.message().to_string())?;
                repo.set_head_detached(commit.id())
            }
        }
        .map_err(|e| e.message().to_string())?;
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
                .set_target(target_oid, &format!("merge {}: fast-forward", branch_name))
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
        // Local branch first, then remote-tracking branch (`origin/x`), then
        // any revision (`HEAD`, a commit hash from the graph, ...).
        let (commit, is_remote) = match repo.find_branch(from_branch, git2::BranchType::Local) {
            Ok(b) => (b.get().peel_to_commit(), false),
            Err(local_err) => match repo.find_branch(from_branch, git2::BranchType::Remote) {
                Ok(b) => (b.get().peel_to_commit(), true),
                Err(_) => match repo.revparse_single(from_branch) {
                    Ok(obj) => (obj.peel_to_commit(), false),
                    Err(_) => return Err(local_err.message().to_string()),
                },
            },
        };
        let commit = commit.map_err(|e| e.message().to_string())?;
        let mut new_branch = repo
            .branch(name, &commit, false)
            .map_err(|e| e.message().to_string())?;
        if is_remote {
            // Like `git branch <name> origin/x`: track the remote branch.
            // Best effort; the branch itself was created successfully.
            let _ = new_branch.set_upstream(Some(from_branch));
        }
        self.refresh();
        Ok(())
    }

    /// Checks a prospective local branch name, so the UI can explain why it
    /// can't be used before anything is submitted.
    pub fn validate_branch_name(&self, name: &str) -> Result<(), String> {
        if name.is_empty() {
            return Err("Branch name is empty".to_string());
        }
        if !git2::Branch::name_is_valid(name).unwrap_or(false) {
            return Err(format!("'{name}' is not a valid branch name"));
        }
        if self.branches.iter().any(|b| !b.is_remote && b.name == name) {
            return Err(format!("Branch '{name}' already exists"));
        }
        Ok(())
    }

    /// Whether every commit of local branch `name` is reachable from HEAD,
    /// i.e. deleting it loses nothing (like `git branch -d`'s check).
    pub fn is_branch_merged(&self, name: &str) -> Result<bool, String> {
        let repo_path = self.repo_path.as_ref().ok_or("No repository")?;
        let repo = git2::Repository::discover(repo_path).map_err(|e| e.message().to_string())?;
        let tip = repo
            .find_branch(name, git2::BranchType::Local)
            .map_err(|e| e.message().to_string())?
            .get()
            .target()
            .ok_or("Branch has no target")?;
        let head = repo
            .head()
            .ok()
            .and_then(|h| h.target())
            .ok_or("No HEAD commit")?;
        Ok(head == tip
            || repo
                .graph_descendant_of(head, tip)
                .map_err(|e| e.message().to_string())?)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::TestRepo;

    fn local_names(s: &GitStatus) -> Vec<String> {
        let mut v: Vec<String> = s
            .branches
            .iter()
            .filter(|b| !b.is_remote)
            .map(|b| b.name.clone())
            .collect();
        v.sort();
        v
    }

    fn current(s: &GitStatus) -> Vec<&str> {
        s.branches
            .iter()
            .filter(|b| b.is_current)
            .map(|b| b.name.as_str())
            .collect()
    }

    /// main: base -> m1 ; feature: base -> f1 (adds feature.txt)
    fn diverged() -> TestRepo {
        let r = TestRepo::new();
        r.write("shared.txt", "base\n");
        r.commit_all("base");
        r.branch("feature");
        r.write("main.txt", "m");
        r.commit_all("m1");
        r.switch("feature");
        r.write("feature.txt", "f");
        r.commit_all("f1");
        r.switch("main");
        r
    }

    // ── listing ────────────────────────────────────────────────────────────

    #[test]
    fn load_branches_lists_local_and_remote_with_current_flag() {
        let r = TestRepo::new();
        r.write("a", "1");
        let base = r.commit_all("c1");
        r.branch("dev");
        r.repo
            .reference("refs/remotes/origin/main", base, true, "t")
            .unwrap();
        let s = r.status();
        assert_eq!(local_names(&s), ["dev", "main"]);
        assert_eq!(current(&s), ["main"]);
        let remote: Vec<_> = s.branches.iter().filter(|b| b.is_remote).collect();
        assert_eq!(remote.len(), 1);
        assert_eq!(remote[0].name, "origin/main");
        assert!(!remote[0].is_current);
    }

    #[test]
    fn load_branches_without_repo_path_or_commits_is_empty() {
        let mut s = GitStatus::new();
        s.branches.push(BranchInfo {
            name: "stale".into(),
            is_remote: false,
            is_current: false,
        });
        s.graph_entries.push(BranchGraphEntry {
            short_hash: "x".into(),
            message: "stale".into(),
            branches: vec![],
            is_head: false,
        });
        s.load_branches();
        assert!(s.branches.is_empty(), "stale branches cleared");
        assert!(s.graph_entries.is_empty(), "stale graph cleared");

        let r = TestRepo::new();
        let s = r.status();
        assert!(s.branches.is_empty());
        assert!(s.graph_entries.is_empty());
    }

    #[test]
    fn graph_lists_commits_newest_first_with_branch_labels_and_head() {
        let r = diverged();
        let base_remote = r
            .repo
            .revparse_single("main~1")
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .id();
        r.repo
            .reference("refs/remotes/origin/main", base_remote, true, "t")
            .unwrap();
        let s = r.status();
        let msgs: Vec<&str> = s.graph_entries.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(msgs.len(), 3);
        assert_eq!(*msgs.last().unwrap(), "base");
        assert!(msgs.contains(&"m1") && msgs.contains(&"f1"));

        let by_msg = |m: &str| s.graph_entries.iter().find(|e| e.message == m).unwrap();
        let m1 = by_msg("m1");
        assert!(m1.is_head);
        assert_eq!(m1.branches, ["main"]);
        assert_eq!(m1.short_hash, r.head_oid().to_string()[..7]);
        let f1 = by_msg("f1");
        assert!(!f1.is_head);
        assert_eq!(f1.branches, ["feature"]);
        let base = by_msg("base");
        assert_eq!(base.branches, ["origin/main"]);
        assert_eq!(base.short_hash.len(), 7);
    }

    #[test]
    fn detached_head_has_no_current_branch_but_marks_head_commit() {
        let r = diverged();
        let base = r.repo.revparse_single("main~1").unwrap().id();
        r.repo.set_head_detached(base).unwrap();
        let s = r.status();
        assert_eq!(s.branch, "HEAD");
        assert!(current(&s).is_empty());
        let heads: Vec<&str> = s
            .graph_entries
            .iter()
            .filter(|e| e.is_head)
            .map(|e| e.message.as_str())
            .collect();
        assert_eq!(heads, ["base"]);
    }

    #[test]
    fn graph_only_walks_local_branch_tips() {
        let r = diverged();
        let feature_tip = r.repo.revparse_single("feature").unwrap().id();
        // Keep f1 reachable only through a remote-tracking ref.
        r.repo
            .reference("refs/remotes/origin/feature", feature_tip, true, "t")
            .unwrap();
        r.repo
            .find_branch("feature", git2::BranchType::Local)
            .unwrap()
            .delete()
            .unwrap();
        let s = r.status();
        let msgs: Vec<&str> = s.graph_entries.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(msgs, ["m1", "base"]);
        assert!(s
            .branches
            .iter()
            .any(|b| b.is_remote && b.name == "origin/feature"));
    }

    #[test]
    fn graph_is_capped_at_twenty_commits() {
        let r = TestRepo::new();
        for i in 0..25 {
            r.write("a", &i.to_string());
            r.commit_all(&format!("c{i}"));
        }
        let s = r.status();
        assert_eq!(s.graph_entries.len(), 20);
        assert_eq!(s.graph_entries[0].message, "c24");
        assert_eq!(s.graph_entries[19].message, "c5");
        assert_eq!(s.graph_entries.iter().filter(|e| e.is_head).count(), 1);
    }

    // ── checkout ───────────────────────────────────────────────────────────

    #[test]
    fn checkout_switches_head_and_worktree() {
        let r = diverged();
        let mut s = r.status();
        s.checkout_branch("feature").unwrap();
        assert_eq!(r.head_name(), "feature");
        assert_eq!(s.branch, "feature");
        assert_eq!(current(&s), ["feature"]);
        assert!(r.path().join("feature.txt").exists());
        assert!(!r.path().join("main.txt").exists());
        assert!(s.files.is_empty());
    }

    #[test]
    fn checkout_unknown_branch_or_without_repo_fails() {
        let r = diverged();
        let mut s = r.status();
        assert!(s.checkout_branch("nope").is_err());
        assert_eq!(r.head_name(), "main");

        let mut s = GitStatus::new();
        assert_eq!(s.checkout_branch("x").unwrap_err(), "No repository");
        assert_eq!(s.merge_branch("x").unwrap_err(), "No repository");
        assert_eq!(s.delete_branch("x").unwrap_err(), "No repository");
        assert_eq!(s.create_branch("x", "y").unwrap_err(), "No repository");
        assert_eq!(s.rename_branch("x", "y").unwrap_err(), "No repository");
    }

    #[test]
    fn checkout_refuses_to_clobber_local_changes() {
        let r = TestRepo::new();
        r.write("f.txt", "base");
        r.commit_all("base");
        r.branch("other");
        r.switch("other");
        r.write("f.txt", "other");
        r.commit_all("other");
        r.switch("main");
        r.write("f.txt", "uncommitted work");
        let mut s = r.status();
        assert!(s.checkout_branch("other").is_err());
        assert_eq!(r.read("f.txt"), "uncommitted work");
        assert_eq!(r.head_name(), "main");
    }

    /// `checkout_branch` accepts any revspec (it uses `revparse_ext`); for one
    /// that is not a reference it sets HEAD to `refs/heads/<spec>`, i.e. an
    /// unborn branch. Not reachable from the current UI (which only passes
    /// local branch names) but the API is wrong.
    #[test]
    fn checkout_commit_hash_detaches_head() {
        let r = diverged();
        let base = r.repo.revparse_single("main~1").unwrap().id();
        let mut s = r.status();
        s.checkout_branch(&base.to_string()).unwrap();
        assert!(r.repo.head_detached().unwrap());
        assert_eq!(r.head_oid(), base);
    }

    #[test]
    fn checkout_tag_follows_reference() {
        let r = diverged();
        let base = r.repo.revparse_single("main~1").unwrap();
        r.repo.tag_lightweight("v1", &base, false).unwrap();
        let mut s = r.status();
        s.checkout_branch("v1").unwrap();
        // A tag reference is not a branch: libgit2 detaches HEAD at it.
        assert_eq!(r.repo.head().unwrap().target(), Some(base.id()));
        assert!(!r.path().join("main.txt").exists());
    }

    // ── merge ──────────────────────────────────────────────────────────────

    #[test]
    fn merge_fast_forward_moves_branch_and_worktree() {
        let r = TestRepo::new();
        r.write("a", "1");
        r.commit_all("base");
        r.branch("ahead");
        r.switch("ahead");
        r.write("b", "2");
        let tip = r.commit_all("ahead work");
        r.switch("main");
        let mut s = r.status();
        s.merge_branch("ahead").unwrap();
        assert_eq!(r.head_name(), "main");
        assert_eq!(r.head_oid(), tip);
        assert_eq!(r.read("b"), "2");
        assert_eq!(r.repo.state(), git2::RepositoryState::Clean);
    }

    #[test]
    fn merge_up_to_date_is_an_error() {
        let r = diverged();
        r.branch("same");
        let mut s = r.status();
        assert_eq!(
            s.merge_branch("same").unwrap_err(),
            "Nothing to merge (already up to date)"
        );
        assert!(s.merge_branch("missing").is_err());
    }

    #[test]
    fn merge_diverged_without_conflicts_stages_merge_result() {
        let r = diverged();
        let before = r.head_oid();
        let mut s = r.status();
        s.merge_branch("feature").unwrap();
        // A normal merge leaves the result staged for the user to commit.
        assert_eq!(r.head_oid(), before);
        assert_eq!(r.repo.state(), git2::RepositoryState::Merge);
        assert_eq!(r.read("feature.txt"), "f");
        assert!(s.files.iter().any(
            |f| f.path == "feature.txt" && f.index_status == crate::git::FileChangeKind::Added
        ));
    }

    #[test]
    fn commit_after_merge_records_both_parents() {
        let r = diverged();
        let feature_tip = r
            .repo
            .find_reference("refs/heads/feature")
            .unwrap()
            .target()
            .unwrap();
        let mut s = r.status();
        s.merge_branch("feature").unwrap();
        s.commit("Merge feature").unwrap();
        let c = r.repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(c.parent_count(), 2);
        assert_eq!(c.parent_id(1).unwrap(), feature_tip);
        assert_eq!(r.repo.state(), git2::RepositoryState::Clean);
    }

    #[test]
    fn merge_with_conflicts_writes_markers_parseable_by_merge_view() {
        let r = TestRepo::new();
        r.write("f.txt", "top\nbase\nbottom\n");
        r.commit_all("base");
        r.branch("other");
        r.write("f.txt", "top\nours\nbottom\n");
        r.commit_all("ours");
        r.switch("other");
        r.write("f.txt", "top\ntheirs\nbottom\n");
        r.commit_all("theirs");
        r.switch("main");

        let mut s = r.status();
        s.merge_branch("other").unwrap();
        // Fresh handle: `r.repo` caches its in-memory index.
        let fresh = git2::Repository::open(r.path()).unwrap();
        assert!(fresh.index().unwrap().has_conflicts());
        assert_eq!(fresh.state(), git2::RepositoryState::Merge);
        let parsed = crate::git::merge::parse_conflict_file(&r.read("f.txt")).unwrap();
        assert_eq!(parsed.hunks.len(), 1);
        assert_eq!(parsed.hunks[0].ours, ["ours"]);
        assert_eq!(parsed.hunks[0].theirs, ["theirs"]);
        assert_eq!(parsed.ours_content, "top\nours\nbottom");
        assert_eq!(parsed.theirs_content, "top\ntheirs\nbottom");
    }

    // ── create / delete / rename ───────────────────────────────────────────

    #[test]
    fn create_branch_from_other_branch_points_at_its_tip() {
        let r = diverged();
        let feature_tip = r.repo.revparse_single("feature").unwrap().id();
        let mut s = r.status();
        s.create_branch("feature-2", "feature").unwrap();
        assert_eq!(local_names(&s), ["feature", "feature-2", "main"]);
        assert_eq!(
            r.repo.revparse_single("feature-2").unwrap().id(),
            feature_tip
        );
        assert_eq!(r.head_name(), "main", "creating does not switch");
    }

    #[test]
    fn create_branch_errors() {
        let r = diverged();
        let mut s = r.status();
        assert!(s.create_branch("x", "no-such-branch").is_err());
        assert!(
            s.create_branch("feature", "main").is_err(),
            "already exists"
        );
        assert!(s.create_branch("bad name..", "main").is_err());
    }

    /// The git panel offers "Create new branch from here" on *remote*
    /// branches, passing names like `origin/feature`.
    #[test]
    fn create_branch_from_remote_branch() {
        let r = diverged();
        let tip = r.repo.revparse_single("feature").unwrap().id();
        r.repo
            .reference("refs/remotes/origin/feature", tip, true, "t")
            .unwrap();
        let mut s = r.status();
        s.create_branch("feature-local", "origin/feature").unwrap();
        assert_eq!(r.repo.revparse_single("feature-local").unwrap().id(), tip);
    }

    #[test]
    fn create_branch_from_remote_sets_upstream_when_remote_exists() {
        let r = diverged();
        let (_origin_dir, url) = crate::git::tests::bare_origin();
        r.repo.remote("origin", &url).unwrap();
        let tip = r.repo.revparse_single("feature").unwrap().id();
        r.repo
            .reference("refs/remotes/origin/feature", tip, true, "t")
            .unwrap();
        let mut s = r.status();
        s.create_branch("feature-local", "origin/feature").unwrap();
        let b = r
            .repo
            .find_branch("feature-local", git2::BranchType::Local)
            .unwrap();
        assert_eq!(
            b.upstream().unwrap().name().unwrap(),
            Some("origin/feature")
        );
        // Local branches still win and get no upstream.
        s.create_branch("main-2", "main").unwrap();
        let b = r
            .repo
            .find_branch("main-2", git2::BranchType::Local)
            .unwrap();
        assert!(b.upstream().is_err());
    }

    #[test]
    fn checkout_relative_revision_detaches_head() {
        let r = diverged();
        let base = r.repo.revparse_single("main~1").unwrap().id();
        let mut s = r.status();
        s.checkout_branch("main~1").unwrap();
        assert!(r.repo.head_detached().unwrap());
        assert_eq!(r.head_oid(), base);
        assert!(r.repo.find_reference("refs/heads/main~1").is_err());
        // And back onto a branch re-attaches HEAD.
        s.checkout_branch("main").unwrap();
        assert!(!r.repo.head_detached().unwrap());
        assert_eq!(r.head_name(), "main");
    }

    #[test]
    fn commit_after_conflicted_merge_resolution_records_both_parents() {
        let r = TestRepo::new();
        r.write("f.txt", "base\n");
        r.commit_all("base");
        r.branch("other");
        r.write("f.txt", "ours\n");
        r.commit_all("ours");
        r.switch("other");
        r.write("f.txt", "theirs\n");
        let theirs = r.commit_all("theirs");
        r.switch("main");
        let mut s = r.status();
        s.merge_branch("other").unwrap();
        r.write("f.txt", "resolved\n");
        s.refresh();
        s.stage_file("f.txt");
        assert!(s.last_error.is_none(), "{:?}", s.last_error);
        s.commit("Merge other").unwrap();
        let fresh = git2::Repository::open(r.path()).unwrap();
        let c = fresh.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(c.parent_count(), 2);
        assert_eq!(c.parent_id(1).unwrap(), theirs);
        assert_eq!(fresh.state(), git2::RepositoryState::Clean);
        // A following ordinary commit is single-parent again.
        r.write("g.txt", "g");
        s.refresh();
        s.stage_file("g.txt");
        s.commit("after").unwrap();
        let c = fresh.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(c.parent_count(), 1);
    }

    #[test]
    fn delete_branch_removes_it() {
        let r = diverged();
        let mut s = r.status();
        s.delete_branch("feature").unwrap();
        assert_eq!(local_names(&s), ["main"]);
        assert!(s.delete_branch("feature").is_err(), "already gone");
    }

    #[test]
    fn delete_current_branch_is_refused() {
        let r = diverged();
        let mut s = r.status();
        assert!(s.delete_branch("main").is_err());
        assert_eq!(local_names(&s), ["feature", "main"]);
    }

    #[test]
    fn rename_branch_keeps_commit_and_current_flag() {
        let r = diverged();
        let tip = r.repo.revparse_single("feature").unwrap().id();
        let mut s = r.status();
        s.rename_branch("feature", "feat/renamed").unwrap();
        assert_eq!(local_names(&s), ["feat/renamed", "main"]);
        assert_eq!(r.repo.revparse_single("feat/renamed").unwrap().id(), tip);

        s.rename_branch("main", "trunk").unwrap();
        assert_eq!(s.branch, "trunk");
        assert_eq!(current(&s), ["trunk"]);
    }

    #[test]
    fn create_branch_from_head_or_commit_hash() {
        let r = diverged();
        let base = r.repo.revparse_single("main~1").unwrap().id();
        let mut s = r.status();
        s.create_branch("from-head", "HEAD").unwrap();
        assert_eq!(
            r.repo.revparse_single("from-head").unwrap().id(),
            r.head_oid()
        );
        let short = base.to_string()[..7].to_string();
        s.create_branch("from-hash", &short).unwrap();
        assert_eq!(r.repo.revparse_single("from-hash").unwrap().id(), base);
        let b = r
            .repo
            .find_branch("from-hash", git2::BranchType::Local)
            .unwrap();
        assert!(b.upstream().is_err(), "no upstream for a plain commit");
    }

    #[test]
    fn create_branch_from_detached_head() {
        let r = diverged();
        let base = r.repo.revparse_single("main~1").unwrap().id();
        r.repo.set_head_detached(base).unwrap();
        let mut s = r.status();
        s.create_branch("rescue", "HEAD").unwrap();
        assert_eq!(r.repo.revparse_single("rescue").unwrap().id(), base);
    }

    #[test]
    fn validate_branch_name_rejects_empty_invalid_and_existing() {
        let r = diverged();
        let s = r.status();
        assert!(s.validate_branch_name("feat/ok-1").is_ok());
        assert!(s.validate_branch_name("").is_err());
        assert!(s.validate_branch_name("bad name").is_err());
        assert!(s.validate_branch_name("a..b").is_err());
        assert!(s.validate_branch_name("ends.lock").is_err());
        assert_eq!(
            s.validate_branch_name("feature").unwrap_err(),
            "Branch 'feature' already exists"
        );
    }

    #[test]
    fn is_branch_merged_detects_unmerged_commits() {
        let r = diverged();
        r.branch("same");
        let s = r.status();
        assert!(!s.is_branch_merged("feature").unwrap(), "f1 not on main");
        assert!(s.is_branch_merged("same").unwrap());
        assert!(s.is_branch_merged("main").unwrap());
        assert!(s.is_branch_merged("missing").is_err());

        let mut s = r.status();
        s.merge_branch("feature").unwrap();
        s.commit("Merge feature").unwrap();
        assert!(s.is_branch_merged("feature").unwrap());
    }

    #[test]
    fn rename_branch_errors() {
        let r = diverged();
        let mut s = r.status();
        assert!(s.rename_branch("nope", "x").is_err());
        assert!(s.rename_branch("feature", "main").is_err(), "target exists");
    }

    #[test]
    fn operations_on_non_repo_dir_fail() {
        let dir = tempfile::tempdir().unwrap();
        if git2::Repository::discover(dir.path()).is_ok() {
            return;
        }
        let mut s = GitStatus::new();
        s.repo_path = Some(dir.path().to_path_buf());
        assert!(s.checkout_branch("x").is_err());
        assert!(s.merge_branch("x").is_err());
        assert!(s.delete_branch("x").is_err());
        assert!(s.create_branch("x", "y").is_err());
        assert!(s.rename_branch("x", "y").is_err());
    }
}
