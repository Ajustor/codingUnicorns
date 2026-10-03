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

    /// Reload this directory's children, preserving the expansion state of the
    /// whole subtree (at every depth). The full set of expanded paths is captured
    /// BEFORE reloading — otherwise `load_children` wipes the subtree and nested
    /// folders would collapse.
    pub fn reload_recursive(&mut self, repo: Option<&git2::Repository>, show_gitignored: bool) {
        if !self.is_dir {
            return;
        }
        let mut expanded = std::collections::HashSet::new();
        self.collect_expanded(&mut expanded);
        self.load_children(repo, show_gitignored);
        self.restore_expanded(&expanded, repo, show_gitignored);
    }

    /// Collect the paths of all expanded directories in this subtree.
    fn collect_expanded(&self, set: &mut std::collections::HashSet<PathBuf>) {
        for child in &self.children {
            if child.is_dir && child.is_expanded {
                set.insert(child.path.clone());
                child.collect_expanded(set);
            }
        }
    }

    /// Re-expand (and load) every directory in this freshly-reloaded subtree whose
    /// path was previously expanded.
    fn restore_expanded(
        &mut self,
        expanded: &std::collections::HashSet<PathBuf>,
        repo: Option<&git2::Repository>,
        show_gitignored: bool,
    ) {
        for child in &mut self.children {
            if child.is_dir && expanded.contains(&child.path) {
                child.is_expanded = true;
                child.load_children(repo, show_gitignored);
                child.restore_expanded(expanded, repo, show_gitignored);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn names(entry: &FileEntry) -> Vec<&str> {
        entry.children.iter().map(|c| c.name.as_str()).collect()
    }

    fn child<'a>(entry: &'a mut FileEntry, name: &str) -> &'a mut FileEntry {
        entry
            .children
            .iter_mut()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no child {name}"))
    }

    /// Build a plain (non-git) directory layout.
    fn layout(root: &Path) {
        fs::create_dir_all(root.join("src/nested/deep")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(root.join(".hidden_dir")).unwrap();
        fs::write(root.join("b.txt"), "b").unwrap();
        fs::write(root.join("A.md"), "a").unwrap();
        fs::write(root.join(".env"), "secret").unwrap();
        fs::write(root.join("build"), "a file named like an ignored dir").unwrap();
        fs::write(root.join("src/main.rs"), "").unwrap();
        fs::write(root.join("src/nested/mod.rs"), "").unwrap();
        fs::write(root.join("src/nested/deep/x.rs"), "").unwrap();
    }

    #[test]
    fn new_reads_name_and_kind_and_expands_only_root() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("f.rs"), "").unwrap();
        let root = FileEntry::new(dir.path().to_path_buf(), 0);
        assert!(root.is_dir);
        assert!(root.is_expanded);
        assert!(root.children.is_empty());
        assert_eq!(root.depth, 0);

        let f = FileEntry::new(dir.path().join("f.rs"), 3);
        assert_eq!(f.name, "f.rs");
        assert!(!f.is_dir);
        assert!(!f.is_expanded);
        assert_eq!(f.depth, 3);

        let sub = FileEntry::new(dir.path().to_path_buf(), 1);
        assert!(!sub.is_expanded, "non-root directories start collapsed");
    }

    #[test]
    fn new_on_path_without_file_name_has_empty_name() {
        let e = FileEntry::new(PathBuf::from(".."), 0);
        assert_eq!(e.name, "");
    }

    #[test]
    fn load_children_sorts_dirs_first_and_skips_hidden_and_ignored() {
        let dir = tempfile::tempdir().unwrap();
        layout(dir.path());
        let mut root = FileEntry::new(dir.path().to_path_buf(), 0);
        root.load_children(None, false);
        // target/ and node_modules/ are ignored as dirs, but a *file* named
        // "build" is kept. Dot-entries are always hidden.
        assert_eq!(names(&root), ["docs", "src", "A.md", "b.txt", "build"]);
        assert!(root.children[0].is_dir && root.children[1].is_dir);
        assert!(root.children[2..].iter().all(|c| !c.is_dir));
        assert!(root.children.iter().all(|c| c.depth == 1));
        assert!(root.children.iter().all(|c| !c.is_expanded));
    }

    #[test]
    fn load_children_with_show_gitignored_keeps_fallback_ignored_dirs() {
        let dir = tempfile::tempdir().unwrap();
        layout(dir.path());
        let mut root = FileEntry::new(dir.path().to_path_buf(), 0);
        root.load_children(None, true);
        assert_eq!(
            names(&root),
            [
                "docs",
                "node_modules",
                "src",
                "target",
                "A.md",
                "b.txt",
                "build"
            ]
        );
    }

    #[test]
    fn load_children_replaces_previous_children() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("one"), "").unwrap();
        let mut root = FileEntry::new(dir.path().to_path_buf(), 0);
        root.load_children(None, false);
        assert_eq!(names(&root), ["one"]);
        fs::remove_file(dir.path().join("one")).unwrap();
        fs::write(dir.path().join("two"), "").unwrap();
        root.load_children(None, false);
        assert_eq!(names(&root), ["two"]);
    }

    #[test]
    fn load_children_on_file_or_vanished_dir_is_harmless() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("f"), "").unwrap();
        let mut f = FileEntry::new(dir.path().join("f"), 1);
        f.load_children(None, false);
        assert!(f.children.is_empty());
        f.reload_recursive(None, false);
        assert!(f.children.is_empty());

        fs::create_dir(dir.path().join("gone")).unwrap();
        let mut gone = FileEntry::new(dir.path().join("gone"), 1);
        fs::remove_dir(dir.path().join("gone")).unwrap();
        gone.load_children(None, false);
        assert!(gone.children.is_empty());
    }

    #[test]
    fn git_repo_uses_gitignore_instead_of_fallback_list() {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        layout(dir.path());
        fs::write(dir.path().join(".gitignore"), "docs/\n*.txt\n").unwrap();
        let mut root = FileEntry::new(dir.path().to_path_buf(), 0);
        root.load_children(Some(&repo), false);
        // docs/ and b.txt are gitignored; target/ and node_modules/ are NOT
        // (the fallback list only applies outside a repository).
        assert_eq!(
            names(&root),
            ["node_modules", "src", "target", "A.md", "build"]
        );

        root.load_children(Some(&repo), true);
        assert_eq!(
            names(&root),
            [
                "docs",
                "node_modules",
                "src",
                "target",
                "A.md",
                "b.txt",
                "build"
            ]
        );
    }

    #[test]
    fn path_outside_repo_workdir_uses_fallback_list() {
        let repo_dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(repo_dir.path()).unwrap();
        let other = tempfile::tempdir().unwrap();
        layout(other.path());
        let mut root = FileEntry::new(other.path().to_path_buf(), 0);
        root.load_children(Some(&repo), false);
        assert_eq!(names(&root), ["docs", "src", "A.md", "b.txt", "build"]);
    }

    #[test]
    fn bare_repo_without_workdir_uses_fallback_list() {
        let bare_dir = tempfile::tempdir().unwrap();
        let bare = git2::Repository::init_bare(bare_dir.path()).unwrap();
        assert!(bare.workdir().is_none());
        let other = tempfile::tempdir().unwrap();
        layout(other.path());
        let mut root = FileEntry::new(other.path().to_path_buf(), 0);
        root.load_children(Some(&bare), false);
        assert_eq!(names(&root), ["docs", "src", "A.md", "b.txt", "build"]);
    }

    #[test]
    fn reload_recursive_preserves_nested_expansion_and_picks_up_changes() {
        let dir = tempfile::tempdir().unwrap();
        layout(dir.path());
        let mut root = FileEntry::new(dir.path().to_path_buf(), 0);
        root.load_children(None, false);
        {
            let src = child(&mut root, "src");
            src.is_expanded = true;
            src.load_children(None, false);
            let nested = child(src, "nested");
            nested.is_expanded = true;
            nested.load_children(None, false);
            // "deep" is loaded but left collapsed.
        }

        // Filesystem changes in an expanded nested folder and at the root.
        fs::write(dir.path().join("src/nested/new.rs"), "").unwrap();
        fs::write(dir.path().join("c.txt"), "").unwrap();
        fs::remove_dir_all(dir.path().join("docs")).unwrap();

        root.reload_recursive(None, false);
        assert_eq!(names(&root), ["src", "A.md", "b.txt", "build", "c.txt"]);
        let src = child(&mut root, "src");
        assert!(src.is_expanded, "expanded dir must stay expanded");
        assert_eq!(names(src), ["nested", "main.rs"]);
        let nested = child(src, "nested");
        assert!(nested.is_expanded, "nested expansion must survive reload");
        assert_eq!(nested.depth, 2);
        assert_eq!(names(nested), ["deep", "mod.rs", "new.rs"]);
        let deep = child(nested, "deep");
        assert!(!deep.is_expanded);
        assert!(deep.children.is_empty(), "collapsed dirs are not loaded");
    }
}
