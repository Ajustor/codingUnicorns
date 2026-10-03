use super::GitStatus;

impl GitStatus {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::TestRepo;

    fn set_origin(r: &TestRepo, branch: &str, oid: git2::Oid) {
        r.repo
            .reference(&format!("refs/remotes/origin/{branch}"), oid, true, "test")
            .unwrap();
    }

    fn counts(r: &TestRepo) -> (usize, usize) {
        let mut s = GitStatus::new();
        s.ahead = 99;
        s.behind = 99;
        s.compute_ahead_behind(&r.repo);
        (s.ahead, s.behind)
    }

    #[test]
    fn zero_without_commits_or_without_remote_branch() {
        let r = TestRepo::new();
        assert_eq!(counts(&r), (0, 0), "unborn HEAD resets counters");
        r.write("a", "1");
        r.commit_all("c1");
        assert_eq!(counts(&r), (0, 0), "no origin/main ref");
    }

    #[test]
    fn counts_local_commits_ahead_of_origin() {
        let r = TestRepo::new();
        r.write("a", "1");
        let base = r.commit_all("c1");
        set_origin(&r, "main", base);
        assert_eq!(counts(&r), (0, 0));
        r.write("a", "2");
        r.commit_all("c2");
        r.write("a", "3");
        r.commit_all("c3");
        assert_eq!(counts(&r), (2, 0));
        // `load` computes it too.
        let s = r.status();
        assert_eq!((s.ahead, s.behind), (2, 0));
    }

    #[test]
    fn counts_both_directions_when_diverged() {
        let r = TestRepo::new();
        r.write("a", "1");
        r.commit_all("base");
        r.branch("other");
        r.write("a", "local");
        r.commit_all("local");
        // Build 3 commits on "other" and pretend they are origin/main.
        r.switch("other");
        for i in 0..3 {
            r.write("b", &i.to_string());
            r.commit_all("remote");
        }
        let remote_tip = r.head_oid();
        r.switch("main");
        set_origin(&r, "main", remote_tip);
        assert_eq!(counts(&r), (1, 3));
    }

    #[test]
    fn uses_remote_branch_matching_current_branch_name() {
        let r = TestRepo::new();
        r.write("a", "1");
        let base = r.commit_all("c1");
        r.write("a", "2");
        r.commit_all("c2");
        // Only a *different* branch is tracked remotely.
        set_origin(&r, "feature", base);
        assert_eq!(counts(&r), (0, 0));
    }

    #[test]
    fn symbolic_remote_ref_is_ignored() {
        let r = TestRepo::new();
        r.write("a", "1");
        let base = r.commit_all("c1");
        set_origin(&r, "real", base);
        r.repo
            .reference_symbolic(
                "refs/remotes/origin/main",
                "refs/remotes/origin/real",
                true,
                "t",
            )
            .unwrap();
        assert_eq!(counts(&r), (0, 0));
    }

    #[test]
    fn detached_head_has_no_tracking_info() {
        let r = TestRepo::new();
        r.write("a", "1");
        let base = r.commit_all("c1");
        set_origin(&r, "main", base);
        r.write("a", "2");
        let tip = r.commit_all("c2");
        r.repo.set_head_detached(tip).unwrap();
        assert_eq!(counts(&r), (0, 0));
    }
}
