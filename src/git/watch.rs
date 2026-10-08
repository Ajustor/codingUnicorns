//! Cheap detection of repository changes made outside the git panel (a
//! `git checkout`, `commit`, `pull`… in a terminal), so the branch, its
//! ahead/behind counts and the changed files stay current on their own.
//!
//! Recomputing the status walks the whole working tree, so it only happens when
//! something changed: the size + mtime of a handful of files in the git dir
//! that every such command rewrites are compared, at most every
//! [`CHECK_INTERVAL`] or right away when the file watcher saw `.git` change.
//! Edits to working-tree files mark the status stale; it is then refreshed at
//! most every [`CHECK_INTERVAL`] too.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Minimum delay between two checks (and between two refreshes for edits).
pub const CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// Files of the (worktree) git dir: current branch, index, branch history.
const GIT_DIR_FILES: &[&str] = &["HEAD", "index", "logs/HEAD", "ORIG_HEAD", "MERGE_HEAD"];
/// Files of the common git dir: fetch results, packed refs, stash.
const COMMON_DIR_FILES: &[&str] = &["FETCH_HEAD", "packed-refs", "logs/refs/stash"];

type Stamp = Option<(SystemTime, u64)>;

fn stamp(path: &Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

pub struct RepoWatch {
    files: Vec<PathBuf>,
    stamps: Vec<Stamp>,
    last_check: Instant,
    /// Working-tree files changed since the last refresh.
    worktree_dirty: bool,
    /// The git dir changed (file watcher): check without waiting.
    check_now: bool,
}

impl RepoWatch {
    pub fn new(repo: &git2::Repository) -> Self {
        let files: Vec<PathBuf> = GIT_DIR_FILES
            .iter()
            .map(|f| repo.path().join(f))
            .chain(COMMON_DIR_FILES.iter().map(|f| repo.commondir().join(f)))
            .collect();
        let stamps = files.iter().map(|f| stamp(f)).collect();
        Self {
            files,
            stamps,
            last_check: Instant::now(),
            worktree_dirty: false,
            check_now: false,
        }
    }

    pub fn note_worktree_change(&mut self) {
        self.worktree_dirty = true;
    }

    pub fn note_git_dir_change(&mut self) {
        self.check_now = true;
    }

    /// Whether the status should be recomputed now. Cheap: a few `stat` calls
    /// at most every [`CHECK_INTERVAL`], nothing in between.
    pub fn poll(&mut self, now: Instant) -> bool {
        let due = now.duration_since(self.last_check) >= CHECK_INTERVAL;
        if !due && !self.check_now {
            return false;
        }
        self.check_now = false;
        self.last_check = now;
        let stamps: Vec<Stamp> = self.files.iter().map(|f| stamp(f)).collect();
        let git_changed = stamps != self.stamps;
        self.stamps = stamps;
        let changed = git_changed || self.worktree_dirty;
        self.worktree_dirty = false;
        changed
    }

    /// Keep the changes `old` noticed but hasn't handled yet.
    pub fn carry_pending(&mut self, old: &RepoWatch) {
        self.worktree_dirty |= old.worktree_dirty;
        self.check_now |= old.check_now;
    }

    /// When the next [`poll`](Self::poll) can do something, if work is pending.
    pub fn pending_in(&self, now: Instant) -> Option<Duration> {
        (self.worktree_dirty || self.check_now)
            .then(|| CHECK_INTERVAL.saturating_sub(now.duration_since(self.last_check)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::TestRepo;

    fn later(w: &RepoWatch) -> Instant {
        w.last_check + CHECK_INTERVAL
    }

    #[test]
    fn nothing_changed_means_no_refresh() {
        let r = TestRepo::new();
        r.write("a", "1");
        r.commit_all("c1");
        let mut w = RepoWatch::new(&r.repo);
        assert!(!w.poll(later(&w)));
        assert_eq!(w.pending_in(Instant::now()), None);
    }

    #[test]
    fn branch_switch_and_commit_are_detected() {
        let r = TestRepo::new();
        r.write("a", "1");
        r.commit_all("c1");
        r.branch("dev");
        let mut w = RepoWatch::new(&r.repo);
        r.switch("dev");
        assert!(w.poll(later(&w)), "checkout rewrites HEAD");
        assert!(!w.poll(later(&w)), "and is reported once");
        r.write("b", "2");
        r.commit_all("c2");
        assert!(w.poll(later(&w)), "commit updates the index and logs/HEAD");
    }

    #[test]
    fn checks_are_throttled_unless_the_git_dir_changed() {
        let r = TestRepo::new();
        r.write("a", "1");
        r.commit_all("c1");
        r.branch("dev");
        let mut w = RepoWatch::new(&r.repo);
        r.switch("dev");
        let soon = w.last_check + CHECK_INTERVAL / 4;
        assert!(!w.poll(soon), "no stat before the interval");
        w.note_git_dir_change();
        assert!(w.poll(soon), "the watcher saw .git change: check now");
    }

    #[test]
    fn worktree_edits_refresh_at_most_every_interval() {
        let r = TestRepo::new();
        r.write("a", "1");
        r.commit_all("c1");
        let mut w = RepoWatch::new(&r.repo);
        w.note_worktree_change();
        let start = w.last_check;
        assert_eq!(w.pending_in(start), Some(CHECK_INTERVAL));
        assert!(!w.poll(start + CHECK_INTERVAL / 2));
        assert!(w.poll(start + CHECK_INTERVAL));
        assert!(!w.poll(start + CHECK_INTERVAL * 2), "handled");
    }

    /// End to end through `GitStatus`: an outside checkout updates the branch
    /// on the next check, and a shown error survives the background refresh.
    #[test]
    fn git_status_follows_an_outside_checkout() {
        let r = TestRepo::new();
        r.write("a", "1");
        r.commit_all("c1");
        r.branch("dev");
        let mut s = r.status();
        assert_eq!(s.branch, "main");
        s.last_error = Some("push rejected".into());
        r.switch("dev");
        // The first call starts the background refresh, a later one applies it.
        let later = Instant::now() + CHECK_INTERVAL;
        assert!(!s.refresh_if_changed(later), "computed off the UI thread");
        assert_eq!(s.branch, "main", "not applied yet");
        assert!(s.refresh_pending_in(later).is_some(), "UI wakes up for it");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !s.refresh_if_changed(later) {
            assert!(
                Instant::now() < deadline,
                "background refresh never finished"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(s.branch, "dev");
        assert_eq!(s.last_error.as_deref(), Some("push rejected"));
        assert!(!s.refresh_if_changed(Instant::now() + CHECK_INTERVAL * 2));
    }

    /// A load made meanwhile (a git panel action) wins over an older
    /// background refresh, whose result is dropped.
    #[test]
    fn a_direct_load_supersedes_a_background_refresh() {
        let r = TestRepo::new();
        r.write("a", "1");
        r.commit_all("c1");
        r.branch("dev");
        let mut s = r.status();
        r.switch("dev");
        assert!(!s.refresh_if_changed(Instant::now() + CHECK_INTERVAL));
        s.refresh();
        assert!(s.refreshing.is_none());
        assert_eq!(s.branch, "dev");
    }
}
