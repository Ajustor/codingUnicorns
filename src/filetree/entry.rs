use std::path::{Path, PathBuf};

/// Directories to ignore when not inside a git repository (fallback).
const IGNORED_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "build",
    "__pycache__",
    ".cache",
];

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_expanded: bool,
    pub children: Vec<FileEntry>,
    pub depth: usize,
}

impl FileEntry {
    pub fn new(path: PathBuf, depth: usize) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let is_dir = path.is_dir();
        Self {
            name,
            path,
            is_dir,
            is_expanded: depth == 0,
            children: vec![],
            depth,
        }
    }

    /// Reload this directory's children and recursively reload any expanded subdirectories.
    pub fn reload_recursive(&mut self, repo: Option<&git2::Repository>, show_gitignored: bool) {
        if !self.is_dir {
            return;
        }
        // Remember which subdirectories were expanded.
        let expanded: std::collections::HashSet<PathBuf> = self
            .children
            .iter()
            .filter(|c| c.is_dir && c.is_expanded)
            .map(|c| c.path.clone())
            .collect();

        self.load_children(repo, show_gitignored);

        // Re-expand and recursively reload previously expanded children.
        for child in &mut self.children {
            if child.is_dir && expanded.contains(&child.path) {
                child.is_expanded = true;
                child.load_children(repo, show_gitignored);
                child.reload_recursive(repo, show_gitignored);
            }
        }
    }

    pub fn load_children(&mut self, repo: Option<&git2::Repository>, show_gitignored: bool) {
        if !self.is_dir {
            return;
        }
        self.children.clear();
        if let Ok(entries) = std::fs::read_dir(&self.path) {
            let mut dirs = vec![];
            let mut files = vec![];
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if name.starts_with('.') {
                    continue;
                }
                if !show_gitignored && should_ignore(&path, repo) {
                    continue;
                }
                if path.is_dir() {
                    dirs.push(path);
                } else {
                    files.push(path);
                }
            }
            dirs.sort();
            files.sort();
            for p in dirs.into_iter().chain(files) {
                self.children.push(FileEntry::new(p, self.depth + 1));
            }
        }
    }
}

fn should_ignore(path: &Path, repo: Option<&git2::Repository>) -> bool {
    if let Some(repo) = repo {
        if let Some(workdir) = repo.workdir() {
            if let Ok(relative) = path.strip_prefix(workdir) {
                if let Ok(ignored) = repo.status_should_ignore(relative) {
                    return ignored;
                }
            }
        }
    }
    // Fallback for non-git directories
    if path.is_dir() {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            return IGNORED_DIRS.contains(&name);
        }
    }
    false
}
