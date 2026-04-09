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
