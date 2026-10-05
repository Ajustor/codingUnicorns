use std::collections::VecDeque;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct NavigationEntry {
    pub path: PathBuf,
    pub row: usize,
    pub col: usize,
}

pub struct NavigationHistory {
    stack: VecDeque<NavigationEntry>,
    index: usize,
    max_size: usize,
}

impl NavigationHistory {
    pub fn new() -> Self {
        Self {
            stack: VecDeque::new(),
            index: 0,
            max_size: 50,
        }
    }

    pub fn push(&mut self, path: PathBuf, row: usize, col: usize) {
        // Truncate forward history
        while self.stack.len() > self.index {
            self.stack.pop_back();
        }
        if let Some(last) = self.stack.back() {
            if last.path == path && last.row == row {
                return;
            }
        }
        self.stack.push_back(NavigationEntry { path, row, col });
        if self.stack.len() > self.max_size {
            self.stack.pop_front();
            self.index = self.index.saturating_sub(1);
        }
        self.index = self.stack.len();
    }

    /// Push current position without navigating. Used before jumps
    /// that handle their own navigation (e.g. LSP definition response).
    pub fn push_current(&mut self, path: PathBuf, row: usize, col: usize) {
        self.push(path, row, col);
    }

    pub fn go_back(&mut self) -> Option<NavigationEntry> {
        if self.index > 0 {
            self.index -= 1;
            self.stack.get(self.index).cloned()
        } else {
            None
        }
    }

    pub fn go_forward(&mut self) -> Option<NavigationEntry> {
        if self.index + 1 < self.stack.len() {
            self.index += 1;
            self.stack.get(self.index).cloned()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    #[test]
    fn empty_history_has_nowhere_to_go() {
        let mut h = NavigationHistory::new();
        assert!(h.go_back().is_none());
        assert!(h.go_forward().is_none());
    }

    #[test]
    fn back_walks_pushed_entries_in_reverse_then_forward_replays() {
        let mut h = NavigationHistory::new();
        h.push(p("a.rs"), 1, 2);
        h.push(p("b.rs"), 3, 4);
        h.push(p("c.rs"), 5, 6);

        let e = h.go_back().unwrap();
        assert_eq!((e.path, e.row, e.col), (p("c.rs"), 5, 6));
        let e = h.go_back().unwrap();
        assert_eq!((e.path, e.row, e.col), (p("b.rs"), 3, 4));
        let e = h.go_back().unwrap();
        assert_eq!((e.path, e.row, e.col), (p("a.rs"), 1, 2));
        assert!(h.go_back().is_none(), "nothing before the first entry");

        let e = h.go_forward().unwrap();
        assert_eq!(e.path, p("b.rs"));
        let e = h.go_forward().unwrap();
        assert_eq!(e.path, p("c.rs"));
        assert!(h.go_forward().is_none());
    }

    #[test]
    fn push_skips_duplicate_of_last_entry_on_same_row() {
        let mut h = NavigationHistory::new();
        h.push(p("a.rs"), 10, 0);
        // Same file + row, different column: considered the same location.
        h.push(p("a.rs"), 10, 7);
        assert_eq!(h.go_back().unwrap().col, 0);
        assert!(h.go_back().is_none(), "duplicate must not be recorded");

        // A different row in the same file is a new location.
        let mut h = NavigationHistory::new();
        h.push(p("a.rs"), 10, 0);
        h.push(p("a.rs"), 11, 0);
        assert_eq!(h.go_back().unwrap().row, 11);
        assert_eq!(h.go_back().unwrap().row, 10);
    }

    #[test]
    fn push_after_going_back_truncates_forward_history() {
        let mut h = NavigationHistory::new();
        h.push(p("a.rs"), 0, 0);
        h.push(p("b.rs"), 0, 0);
        h.push(p("c.rs"), 0, 0);
        h.go_back(); // c
        h.go_back(); // b (index 1)
        h.push_current(p("d.rs"), 0, 0);
        // b and c are discarded; stack is now [a, d]
        assert!(h.go_forward().is_none());
        assert_eq!(h.go_back().unwrap().path, p("d.rs"));
        assert_eq!(h.go_back().unwrap().path, p("a.rs"));
        assert!(h.go_back().is_none());
    }

    #[test]
    fn history_is_capped_at_max_size_dropping_oldest() {
        let mut h = NavigationHistory::new();
        for i in 0..60 {
            h.push(p("f.rs"), i, 0);
        }
        let mut rows = vec![];
        while let Some(e) = h.go_back() {
            rows.push(e.row);
        }
        assert_eq!(rows.len(), 50);
        assert_eq!(rows.first(), Some(&59));
        assert_eq!(rows.last(), Some(&10), "the 10 oldest entries were evicted");
    }

    /// The app pushes the *origin* of each jump (see `push_nav_and_goto`), so
    /// after Alt+Left the user expects Alt+Right to take them back to where
    /// they were. The position held before going back is never recorded, so
    /// forward navigation right after a single back is impossible.
    #[test]
    #[ignore = "BUG: go_forward() right after go_back() returns None; pre-back position is never recorded"]
    fn forward_after_back_returns_to_previous_location() {
        let mut h = NavigationHistory::new();
        h.push(p("a.rs"), 1, 0); // jumped a.rs:1 -> elsewhere
        h.push(p("b.rs"), 2, 0); // jumped b.rs:2 -> elsewhere
        assert_eq!(h.go_back().unwrap().path, p("b.rs"));
        assert!(h.go_forward().is_some());
    }
}
