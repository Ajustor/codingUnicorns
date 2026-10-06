use super::GitStatus;

#[derive(Debug, Clone, PartialEq)]
pub struct StashEntry {
    /// Position in the stash stack (`stash@{index}`), 0 = most recent.
    pub index: usize,
    pub message: String,
    pub oid: git2::Oid,
}

/// Every stash entry of `repo`, most recent first.
pub(crate) fn list_stashes(repo: &mut git2::Repository) -> Result<Vec<StashEntry>, String> {
    let mut entries = Vec::new();
    repo.stash_foreach(|index, message, oid| {
        entries.push(StashEntry {
            index,
            message: message.to_string(),
            oid: *oid,
        });
        true
    })
    .map_err(|e| format!("Stash list error: {e}"))?;
    Ok(entries)
}

fn apply_error(context: &str, e: git2::Error) -> String {
    if e.code() == git2::ErrorCode::Conflict {
        format!("{context} conflicts with local changes: commit or stash them first ({e})")
    } else {
        format!("{context} error: {e}")
    }
}

impl GitStatus {
    /// Stash staged, unstaged and untracked changes (`git stash -u`).
    pub fn stash_save(&mut self, message: &str) -> Result<(), String> {
        let mut repo = self.open_repo()?;
        let sig = repo
            .signature()
            .map_err(|e| format!("Signature error: {e}"))?;
        let message = message.trim();
        let message = (!message.is_empty()).then_some(message);
        repo.stash_save2(&sig, message, Some(git2::StashFlags::INCLUDE_UNTRACKED))
            .map_err(|e| {
                if e.code() == git2::ErrorCode::NotFound {
                    "No local changes to stash".to_string()
                } else {
                    format!("Stash error: {e}")
                }
            })?;
        self.refresh();
        Ok(())
    }

    pub fn stash_list(&self) -> Result<Vec<StashEntry>, String> {
        list_stashes(&mut self.open_repo()?)
    }

    /// Re-apply `stash@{index}` and keep it on the stack.
    pub fn stash_apply(&mut self, index: usize) -> Result<(), String> {
        let mut repo = self.open_repo()?;
        repo.stash_apply(index, None)
            .map_err(|e| apply_error("Stash apply", e))?;
        self.refresh();
        Ok(())
    }

    /// Re-apply `stash@{index}` and remove it (kept if applying fails).
    pub fn stash_pop(&mut self, index: usize) -> Result<(), String> {
        let mut repo = self.open_repo()?;
        repo.stash_pop(index, None)
            .map_err(|e| apply_error("Stash pop", e))?;
        self.refresh();
        Ok(())
    }

    pub fn stash_drop(&mut self, index: usize) -> Result<(), String> {
        let mut repo = self.open_repo()?;
        repo.stash_drop(index)
            .map_err(|e| format!("Stash drop error: {e}"))?;
        self.refresh();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::TestRepo;

    fn repo_with_commit() -> TestRepo {
        let r = TestRepo::new();
        r.write("a.txt", "1");
        r.commit_all("init");
        r
    }

    #[test]
    fn save_stashes_tracked_and_untracked_changes() {
        let r = repo_with_commit();
        r.write("a.txt", "2");
        r.write("new.txt", "untracked");
        let mut s = r.status();
        assert_eq!(s.files.len(), 2);
        s.stash_save("wip: feature").unwrap();

        assert!(s.files.is_empty(), "{:?}", s.files);
        assert_eq!(r.read("a.txt"), "1");
        assert!(!r.path().join("new.txt").exists());
        assert_eq!(s.stashes.len(), 1, "load lists stashes");
        assert!(s.stashes[0].message.contains("wip: feature"));
        assert_eq!(s.stash_list().unwrap(), s.stashes);
    }

    #[test]
    fn save_without_message_uses_default_and_nothing_to_stash_errors() {
        let r = repo_with_commit();
        let mut s = r.status();
        assert_eq!(s.stash_save("").unwrap_err(), "No local changes to stash");
        r.write("a.txt", "2");
        s.stash_save("   ").unwrap();
        assert!(
            s.stashes[0].message.starts_with("WIP on main"),
            "{:?}",
            s.stashes
        );
    }

    #[test]
    fn apply_keeps_entry_pop_removes_it() {
        let r = repo_with_commit();
        r.write("a.txt", "2");
        r.write("new.txt", "u");
        let mut s = r.status();
        s.stash_save("one").unwrap();

        s.stash_apply(0).unwrap();
        assert_eq!(r.read("a.txt"), "2");
        assert_eq!(r.read("new.txt"), "u");
        assert_eq!(s.stashes.len(), 1);

        // Reset the worktree, then pop.
        r.repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        r.remove("new.txt");
        s.stash_pop(0).unwrap();
        assert_eq!(r.read("a.txt"), "2");
        assert!(s.stashes.is_empty());
    }

    #[test]
    fn list_orders_most_recent_first_and_drop_targets_index() {
        let r = repo_with_commit();
        let mut s = r.status();
        r.write("a.txt", "first");
        s.stash_save("first").unwrap();
        r.write("a.txt", "second");
        s.stash_save("second").unwrap();
        let msgs: Vec<&str> = s.stashes.iter().map(|e| e.message.as_str()).collect();
        assert!(
            msgs[0].contains("second") && msgs[1].contains("first"),
            "{msgs:?}"
        );
        assert_eq!(s.stashes[1].index, 1);

        s.stash_drop(1).unwrap();
        assert_eq!(s.stashes.len(), 1);
        assert!(s.stashes[0].message.contains("second"));
        assert!(s.stash_drop(5).unwrap_err().starts_with("Stash drop error"));
    }

    #[test]
    fn apply_over_conflicting_local_change_fails_and_keeps_entry() {
        let r = repo_with_commit();
        r.write("a.txt", "stashed");
        let mut s = r.status();
        s.stash_save("x").unwrap();
        r.write("a.txt", "local edit");
        let err = s.stash_pop(0).unwrap_err();
        assert!(err.starts_with("Stash pop"), "{err}");
        assert_eq!(r.read("a.txt"), "local edit", "local change preserved");
        assert_eq!(s.stash_list().unwrap().len(), 1, "entry kept");
    }

    #[test]
    fn stash_errors_without_repo() {
        let mut s = GitStatus::new();
        assert!(s.stash_save("x").is_err());
        assert!(s.stash_list().is_err());
        assert!(s.stash_apply(0).is_err());
        assert!(s.stash_pop(0).is_err());
        assert!(s.stash_drop(0).is_err());
    }
}
