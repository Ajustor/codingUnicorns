use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct NavigationEntry {
    pub path: PathBuf,
    pub row: usize,
    pub col: usize,
}

pub struct NavigationHistory {
    stack: Vec<NavigationEntry>,
    index: usize,
    max_size: usize,
}

impl NavigationHistory {
    pub fn new() -> Self {
        Self {
            stack: vec![],
            index: 0,
            max_size: 50,
        }
    }

    pub fn push(&mut self, path: PathBuf, row: usize, col: usize) {
        if self.index < self.stack.len() {
            self.stack.truncate(self.index);
        }
        if let Some(last) = self.stack.last() {
            if last.path == path && last.row == row {
                return;
            }
        }
        self.stack.push(NavigationEntry { path, row, col });
        if self.stack.len() > self.max_size {
            self.stack.remove(0);
        }
        self.index = self.stack.len();
    }

    pub fn go_back(&mut self) -> Option<NavigationEntry> {
        if self.index > 0 {
            self.index -= 1;
            Some(self.stack[self.index].clone())
        } else {
            None
        }
    }

    pub fn go_forward(&mut self) -> Option<NavigationEntry> {
        if self.index + 1 < self.stack.len() {
            self.index += 1;
            Some(self.stack[self.index].clone())
        } else {
            None
        }
    }
}
