use super::diff::compute_line_diff;
use super::Editor;

impl Editor {
    /// Recompute git diff indicators for the current file.
    pub fn refresh_line_diff(&mut self) {
        let path = match &self.current_path {
            Some(p) => p.clone(),
            None => {
                self.line_diff.clear();
                return;
            }
        };
        if self.line_diff_path.as_ref() == Some(&path) {
            return; // already current
        }
        self.line_diff_path = Some(path.clone());
        self.line_diff = compute_line_diff(&path, self.buffer.num_lines());
    }

    /// Force a refresh of line diff (called after save).
    pub fn invalidate_line_diff(&mut self) {
        self.line_diff_path = None;
    }
}

#[cfg(test)]
mod tests {
    use super::super::diff::{DIFF_ADDED, DIFF_UNCHANGED};
    use super::*;

    #[test]
    fn no_path_clears_diff() {
        let mut ed = Editor::new();
        ed.line_diff = vec![DIFF_ADDED, DIFF_ADDED];
        ed.refresh_line_diff();
        assert!(ed.line_diff.is_empty());
        assert!(ed.line_diff_path.is_none());
    }

    #[test]
    fn refresh_computes_once_per_path_until_invalidated() {
        let dir = tempfile::tempdir().unwrap();
        // A fresh repository with no commits: every line counts as added.
        git2::Repository::init(dir.path()).unwrap();
        let path = dir.path().join("new.txt");
        std::fs::write(&path, "a\nb\nc").unwrap();

        let mut ed = Editor::new();
        ed.set_content("a\nb\nc".to_string(), Some(path.clone()));
        ed.refresh_line_diff();
        assert_eq!(ed.line_diff, vec![DIFF_ADDED; 3]);
        assert_eq!(ed.line_diff_path.as_ref(), Some(&path));

        // Cached: a second refresh for the same path doesn't recompute.
        ed.line_diff = vec![DIFF_UNCHANGED];
        ed.refresh_line_diff();
        assert_eq!(ed.line_diff, vec![DIFF_UNCHANGED]);

        // Invalidation forces a recompute.
        ed.invalidate_line_diff();
        assert!(ed.line_diff_path.is_none());
        ed.refresh_line_diff();
        assert_eq!(ed.line_diff, vec![DIFF_ADDED; 3]);
    }

    #[test]
    fn refresh_outside_a_repository_marks_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut ed = Editor::new();
        ed.set_content(
            "x\ny".to_string(),
            Some(dir.path().join("missing").join("file.rs")),
        );
        ed.refresh_line_diff();
        assert_eq!(ed.line_diff, vec![DIFF_UNCHANGED; 2]);
    }
}
