//! Pull request tabs: opening them, applying what they ask for, and
//! dropping the views of closed tabs.

use super::CodingUnicorns;
use crate::github::{PullSummary, RepoRef};
use crate::tabs::Tab;
use crate::ui::pr_view::{PrAction, PrView};
use std::collections::HashMap;

/// Text of the toast shown after a successful checkout of `branch`.
/// `existed`: the branch was already there; `on_head`: it now points at the
/// pull request's head commit.
fn checkout_toast(existed: bool, on_head: bool, branch: &str) -> String {
    if existed && !on_head {
        format!("Checked out the existing branch {branch}: it may differ from the pull request.")
    } else {
        format!("Checked out {branch}.")
    }
}

/// Keep only the views whose pull request still has a tab.
fn retain_open_views<V>(views: &mut HashMap<u64, V>, tabs: &[Tab]) {
    views.retain(|number, _| tabs.iter().any(|t| t.pr == Some(*number)));
}

impl CodingUnicorns {
    /// Open (or focus) the tab of pull request `pull` of `repo`.
    pub fn open_pull_request(&mut self, repo: RepoRef, pull: PullSummary) {
        let Some(workdir) = self.git_status.workdir() else {
            return;
        };
        let token = self.config.github_token.clone();
        self.pr_views
            .entry(pull.number)
            .or_insert_with(|| PrView::open(workdir, repo, &pull, token));
        self.active_pane = 0;
        self.tab_manager.open_pr(pull.number, &pull.title);
    }

    /// Apply what the tab of pull request `number` asked for.
    pub fn apply_pr_action(&mut self, number: u64, action: PrAction) {
        match action {
            PrAction::OpenFile { path, line } => {
                let Some(root) = self.git_status.workdir() else {
                    return;
                };
                self.active_pane = 0;
                self.open_file_at_line(root.join(path), line);
            }
            PrAction::Checkout {
                branch,
                head_sha,
                merge_ref,
            } => {
                match self
                    .git_status
                    .checkout_pull(&branch, &head_sha, &merge_ref)
                {
                    Ok(existed) => {
                        let on_head = self
                            .git_status
                            .open_repo()
                            .ok()
                            .and_then(|r| r.head().ok()?.target())
                            .is_some_and(|oid| oid.to_string() == head_sha);
                        if let Some(view) = self.pr_views.get_mut(&number) {
                            view.set_checked_out(on_head);
                        }
                        self.toast(checkout_toast(existed, on_head, &branch));
                    }
                    Err(e) => self.toast(format!("Checkout failed: {e}")),
                }
            }
        }
    }

    /// Forget the views of closed pull request tabs.
    pub fn drop_closed_pr_views(&mut self) {
        retain_open_views(&mut self.pr_views, &self.tab_manager.tabs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs::TabManager;

    #[test]
    fn toast_warns_only_for_an_existing_branch_not_on_the_head() {
        assert!(checkout_toast(true, false, "feat").contains("may differ"));
        assert_eq!(checkout_toast(true, true, "feat"), "Checked out feat.");
        assert_eq!(checkout_toast(false, false, "feat"), "Checked out feat.");
        assert_eq!(checkout_toast(false, true, "feat"), "Checked out feat.");
    }

    #[test]
    fn closed_pr_tab_loses_its_view_others_are_kept() {
        let mut tm = TabManager::new();
        let id1 = tm.open_pr(1, "one");
        tm.open_pr(2, "two");
        let mut views: HashMap<u64, ()> = HashMap::from([(1, ()), (2, ()), (3, ())]);
        tm.close(id1);
        retain_open_views(&mut views, &tm.tabs);
        let mut left: Vec<u64> = views.keys().copied().collect();
        left.sort();
        assert_eq!(left, vec![2]);
    }
}
