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
