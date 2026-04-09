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
