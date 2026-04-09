use super::{FileChangeKind, GitStatus};

impl GitStatus {
    pub fn stage_file(&mut self, file_path: &str) {
        let repo_path = match &self.repo_path {
            Some(p) => p.clone(),
            None => return,
        };
        let repo = match git2::Repository::discover(&repo_path) {
            Ok(r) => r,
            Err(e) => {
                self.last_error = Some(format!("Failed to open repo: {e}"));
                return;
            }
        };
        let mut index = match repo.index() {
            Ok(i) => i,
            Err(e) => {
                self.last_error = Some(format!("Failed to get index: {e}"));
                return;
            }
        };
        let path = std::path::Path::new(file_path);
        // Check if this is a deleted file (wt_deleted means remove from index)
        let is_deleted = self
            .files
            .iter()
            .find(|f| f.path == file_path)
            .map(|f| f.wt_status == FileChangeKind::Deleted)
            .unwrap_or(false);

        let result = if is_deleted {
            index.remove_path(path)
        } else {
            index.add_path(path)
        };

        if let Err(e) = result {
            self.last_error = Some(format!("Failed to stage {file_path}: {e}"));
            return;
        }
        if let Err(e) = index.write() {
            self.last_error = Some(format!("Failed to write index: {e}"));
            return;
        }
        self.refresh();
    }

    pub fn unstage_file(&mut self, file_path: &str) {
        let repo_path = match &self.repo_path {
            Some(p) => p.clone(),
            None => return,
        };
        let repo = match git2::Repository::discover(&repo_path) {
            Ok(r) => r,
            Err(e) => {
                self.last_error = Some(format!("Failed to open repo: {e}"));
                return;
            }
        };
        // Reset the file in index to HEAD state
        let result: Result<(), git2::Error> = (|| {
            match repo.head() {
                Ok(head) => {
                    match head.peel_to_commit() {
                        Ok(commit) => {
                            let obj = commit.into_object();
                            repo.reset_default(Some(&obj), [file_path].iter())
                        }
                        Err(_) => {
                            // No commits yet: just remove from index
                            let mut index = repo.index()?;
                            index.remove_path(std::path::Path::new(file_path))?;
                            index.write()
                        }
                    }
                }
                Err(_) => {
                    // No HEAD: remove from index
                    let mut index = repo.index()?;
                    index.remove_path(std::path::Path::new(file_path))?;
                    index.write()
                }
            }
        })();
        if let Err(e) = result {
            self.last_error = Some(format!("Failed to unstage {file_path}: {e}"));
            return;
        }
        self.refresh();
    }

    pub fn stage_all(&mut self) {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => {
                self.last_error = Some(e);
                return;
            }
        };
        let mut index = match repo.index() {
            Ok(i) => i,
            Err(e) => {
                self.last_error = Some(format!("Index error: {e}"));
                return;
            }
        };
        let paths: Vec<(String, bool)> = self
            .files
            .iter()
            .filter(|f| f.wt_status != FileChangeKind::None)
            .map(|f| (f.path.clone(), f.wt_status == FileChangeKind::Deleted))
            .collect();
        for (path, is_deleted) in &paths {
            let p = std::path::Path::new(path);
            if *is_deleted {
                let _ = index.remove_path(p);
            } else {
                let _ = index.add_path(p);
            }
        }
        if let Err(e) = index.write() {
            self.last_error = Some(format!("Failed to write index: {e}"));
            return;
        }
        self.refresh();
    }

    pub fn unstage_all(&mut self) {
        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => {
                self.last_error = Some(e);
                return;
            }
        };
        let paths: Vec<String> = self
            .files
            .iter()
            .filter(|f| f.index_status != FileChangeKind::None)
            .map(|f| f.path.clone())
            .collect();
        match repo.head() {
            Ok(head) => {
                if let Ok(commit) = head.peel_to_commit() {
                    let obj = commit.into_object();
                    let _ = repo.reset_default(Some(&obj), paths.iter().map(|s| s.as_str()));
                }
            }
            Err(_) => {
                // No HEAD: remove all from index
                if let Ok(mut index) = repo.index() {
                    for path in &paths {
                        let _ = index.remove_path(std::path::Path::new(path));
                    }
                    let _ = index.write();
                }
            }
        }
        self.refresh();
    }
}
