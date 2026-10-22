pub mod entry;
pub mod icons;

pub use entry::FileEntry;
pub use icons::file_icon;

use egui_phosphor::regular as ph;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Context passed through recursive file tree rendering.
struct ShowContext<'a> {
    selected: &'a mut Option<PathBuf>,
    opened: &'a mut Option<PathBuf>,
    action: &'a mut Option<FileTreeAction>,
    rename_state: &'a mut Option<(PathBuf, String)>,
    repo: Option<&'a git2::Repository>,
    show_gitignored: bool,
    marks: &'a GitMarks,
}

pub struct FileTree {
    pub root: Option<FileEntry>,
    pub selected: Option<PathBuf>,
    /// Action requested via context menu, consumed by the caller each frame.
    pub context_action: Option<FileTreeAction>,
    /// Path being renamed (in-progress text).  Exposed so callers can pre-set it.
    pub rename_state: Option<(PathBuf, String)>,
    /// Git repository for .gitignore filtering in the file tree.
    repo: Option<git2::Repository>,
    /// When true, show files that are gitignored.
    pub show_gitignored: bool,
    /// Git changes, colouring the changed files and the folders holding them.
    marks: GitMarks,
    /// The tree was clicked last: Ctrl+C / Ctrl+V act on its files, not on
    /// the editor (which keeps the keyboard focus).
    active: bool,
    /// File copied with Ctrl+C, duplicated by Ctrl+V.
    clipboard: Option<PathBuf>,
}

/// What git says about an explorer entry, shown by its colour. Ordered by
/// importance: a folder takes the most important mark below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GitMark {
    /// New: untracked, or added to the index.
    Added,
    Modified,
    Conflicted,
}

impl GitMark {
    fn color(self) -> egui::Color32 {
        match self {
            Self::Added => egui::Color32::from_rgb(115, 201, 145),
            Self::Modified => egui::Color32::from_rgb(220, 180, 100),
            Self::Conflicted => egui::Color32::from_rgb(228, 103, 107),
        }
    }

    fn of(f: &crate::git::FileStatus) -> Self {
        use crate::git::FileChangeKind as K;
        if f.conflicted {
            Self::Conflicted
        } else if f.wt_status == K::Untracked
            || (f.index_status == K::Added && f.wt_status == K::None)
        {
            Self::Added
        } else {
            Self::Modified
        }
    }
}

/// Git marks of the tree's paths, rebuilt when the status changes.
#[derive(Default)]
struct GitMarks {
    /// The status they were computed from.
    source: Vec<crate::git::FileStatus>,
    /// Changed files and every folder above them, up to the tree's root.
    by_path: HashMap<PathBuf, GitMark>,
    /// Untracked folders (git lists them, not their files): all new inside.
    new_dirs: Vec<PathBuf>,
}

impl GitMarks {
    fn get(&self, path: &Path) -> Option<GitMark> {
        self.by_path.get(path).copied().or_else(|| {
            self.new_dirs
                .iter()
                .any(|d| path.starts_with(d))
                .then_some(GitMark::Added)
        })
    }
}

#[derive(Debug, Clone)]
pub enum FileTreeAction {
    OpenFile(PathBuf),
    NewFile(PathBuf),        // parent dir
    NewFolder(PathBuf),      // parent dir
    Rename(PathBuf, String), // old path, new name
    Delete(PathBuf),
    RevealInExplorer(PathBuf),
    CopyPath(PathBuf),
    /// Copy a file into a folder, under a free "-copie" name.
    Duplicate(PathBuf, PathBuf), // file, destination dir
}

impl FileTree {
    pub fn new() -> Self {
        Self {
            root: None,
            selected: None,
            context_action: None,
            rename_state: None,
            repo: None,
            show_gitignored: false,
            marks: GitMarks::default(),
            active: false,
            clipboard: None,
        }
    }

    /// Colour the entries git reports as changed, and the folders holding
    /// them; untouched folders stay neutral. Cheap when `files` is unchanged.
    pub fn sync_git_marks(&mut self, files: &[crate::git::FileStatus]) {
        if self.marks.source == files {
            return;
        }
        let mut marks = GitMarks {
            source: files.to_vec(),
            ..GitMarks::default()
        };
        let base = self.root.as_ref().map(|r| r.path.clone());
        let workdir = self.repo.as_ref().and_then(|r| r.workdir());
        if let (Some(base), Some(workdir)) = (base, workdir) {
            // Status paths are relative to the repository; the tree may show
            // a folder inside it. Built on the tree's own root path so the
            // keys match its entries exactly.
            if let Some(prefix) = crate::git::relative_to_workdir(&base, workdir) {
                for f in files {
                    let rel = Path::new(f.path.trim_end_matches('/'));
                    let Ok(rel) = rel.strip_prefix(&prefix) else {
                        continue;
                    };
                    let path = base.join(rel);
                    if f.path.ends_with('/') {
                        marks.new_dirs.push(path.clone());
                    }
                    let mark = GitMark::of(f);
                    let mut cur = Some(path.as_path());
                    while let Some(p) = cur.filter(|p| *p != base) {
                        let m = marks.by_path.entry(p.to_path_buf()).or_insert(mark);
                        *m = (*m).max(mark);
                        cur = p.parent();
                    }
                }
            }
        }
        self.marks = marks;
    }

    pub fn load(&mut self, path: PathBuf) {
        self.repo = git2::Repository::discover(&path).ok();
        let mut root = FileEntry::new(path, 0);
        root.load_children(self.repo.as_ref(), self.show_gitignored);
        self.root = Some(root);
        // Recomputed against the new root on the next sync.
        self.marks = GitMarks::default();
    }

    /// Reload children of the root entry and all expanded subdirectories.
    pub fn reload_children(&mut self) {
        if let Some(root) = &mut self.root {
            root.reload_recursive(self.repo.as_ref(), self.show_gitignored);
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        let mut opened = None;
        let mut action: Option<FileTreeAction> = None;
        if self.active && self.rename_state.is_none() {
            action = self.handle_clipboard_keys(ui);
        }
        let mut rename_state = self.rename_state.take();
        let mut ctx = ShowContext {
            selected: &mut self.selected,
            opened: &mut opened,
            action: &mut action,
            rename_state: &mut rename_state,
            repo: self.repo.as_ref(),
            show_gitignored: self.show_gitignored,
            marks: &self.marks,
        };
        if let Some(root) = &mut self.root {
            Self::show_entry_recursive(ui, root, &mut ctx);
        }
        // A press inside the tree makes it the target of Ctrl+C / Ctrl+V,
        // one anywhere else hands them back.
        if ui.input(|i| i.pointer.any_pressed()) {
            let pos = ui.input(|i| i.pointer.interact_pos());
            self.active = pos.is_some_and(|p| ui.min_rect().contains(p));
        }
        self.rename_state = rename_state;
        // Store non-open actions for the caller to pick up
        if let Some(a) = action {
            match &a {
                FileTreeAction::OpenFile(p) => {
                    opened = Some(p.clone());
                }
                _ => {
                    self.context_action = Some(a);
                }
            }
        }
        opened
    }

    /// The rename shortcut (F2) pressed while the tree was clicked last:
    /// rename the selected entry inline, starting from its current name.
    /// Returns false when the shortcut is the editor's (Rename Symbol).
    pub fn rename_selected(&mut self) -> bool {
        if !self.active {
            return false;
        }
        if self.rename_state.is_some() {
            return true;
        }
        let Some(path) = self.selected.clone() else {
            return false;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.rename_state = Some((path, name));
        true
    }

    /// Take Ctrl+C / Ctrl+V / Delete out of this frame's input, before the
    /// editor sees them: copy the selected file, ask to duplicate the copied
    /// one, or ask to delete the selected one. Any other key means the user is
    /// typing elsewhere: the tree lets go.
    fn handle_clipboard_keys(&mut self, ui: &mut egui::Ui) -> Option<FileTreeAction> {
        let (mut copy, mut paste, mut delete) = (false, false, false);
        ui.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Copy => {
                    copy = true;
                    false
                }
                egui::Event::Paste(_) => {
                    paste = true;
                    false
                }
                // egui-winit sends Copy / Paste with the key press: only a
                // press acts, the release is just swallowed (it used to
                // paste a second time).
                egui::Event::Key {
                    key: key @ (egui::Key::C | egui::Key::V),
                    pressed,
                    modifiers,
                    ..
                } if modifiers.command_only() => {
                    copy |= *pressed && *key == egui::Key::C;
                    paste |= *pressed && *key == egui::Key::V;
                    false
                }
                egui::Event::Key {
                    key: egui::Key::Delete,
                    pressed,
                    modifiers,
                    ..
                } if modifiers.is_none() => {
                    delete |= *pressed;
                    false
                }
                egui::Event::Key { pressed: true, .. } | egui::Event::Text(_) => {
                    self.active = false;
                    true
                }
                _ => true,
            })
        });
        if delete {
            return self.selected.clone().map(FileTreeAction::Delete);
        }
        if copy && self.selected.is_some() {
            self.clipboard = self.selected.clone();
        }
        let src = self.clipboard.clone().filter(|_| paste)?;
        let dir = self
            .selected
            .as_deref()
            .unwrap_or(&src)
            .parent()?
            .to_path_buf();
        Some(FileTreeAction::Duplicate(src, dir))
    }

    fn show_entry_recursive(ui: &mut egui::Ui, entry: &mut FileEntry, ctx: &mut ShowContext<'_>) {
        let indent = entry.depth as f32 * 14.0;

        // Inline rename mode
        let is_renaming = ctx
            .rename_state
            .as_ref()
            .map(|(p, _)| p == &entry.path)
            .unwrap_or(false);

        let row_resp = ui.horizontal(|ui| {
            ui.add_space(indent);

            if is_renaming {
                if let Some((_, ref mut new_name)) = ctx.rename_state {
                    let resp = ui.add(egui::TextEdit::singleline(new_name).desired_width(150.0));
                    resp.request_focus();
                    if resp.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        let new_n = new_name.clone();
                        *ctx.action = Some(FileTreeAction::Rename(entry.path.clone(), new_n));
                        *ctx.rename_state = None;
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        *ctx.rename_state = None;
                    }
                }
                return;
            }

            if entry.is_dir {
                let arrow = if entry.is_expanded {
                    ph::CARET_DOWN
                } else {
                    ph::CARET_RIGHT
                };
                let folder_icon = if entry.is_expanded {
                    ph::FOLDER_OPEN
                } else {
                    ph::FOLDER
                };
                let mark = ctx.marks.get(&entry.path);
                let color = dim_if(
                    entry.ignored,
                    mark.map_or(egui::Color32::from_rgb(212, 212, 212), GitMark::color),
                );
                let mut label =
                    egui::RichText::new(format!("{} {} {}", arrow, folder_icon, entry.name))
                        .color(color);
                if entry.ignored {
                    label = label.italics();
                }
                let resp = ignored_hint(ui.selectable_label(false, label), entry.ignored);
                if resp.clicked() {
                    entry.is_expanded = !entry.is_expanded;
                    if entry.is_expanded {
                        // Always re-read: children kept from an earlier expansion
                        // may be stale (the watcher only reloads expanded folders).
                        entry.reload_recursive(ctx.repo, ctx.show_gitignored);
                    }
                }
                resp.context_menu(|ui| {
                    if ui.button("New File").clicked() {
                        *ctx.action = Some(FileTreeAction::NewFile(entry.path.clone()));
                        ui.close_menu();
                    }
                    if ui.button("New Folder").clicked() {
                        *ctx.action = Some(FileTreeAction::NewFolder(entry.path.clone()));
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Rename").clicked() {
                        *ctx.rename_state = Some((entry.path.clone(), entry.name.clone()));
                        ui.close_menu();
                    }
                    if ui.button("Delete Folder").clicked() {
                        *ctx.action = Some(FileTreeAction::Delete(entry.path.clone()));
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Copy Path").clicked() {
                        *ctx.action = Some(FileTreeAction::CopyPath(entry.path.clone()));
                        ui.close_menu();
                    }
                    if ui.button("Reveal in File Manager").clicked() {
                        *ctx.action = Some(FileTreeAction::RevealInExplorer(entry.path.clone()));
                        ui.close_menu();
                    }
                });
            } else {
                let (icon, color) = file_icon(&entry.name);
                let is_selected = ctx
                    .selected
                    .as_ref()
                    .map(|s| s == &entry.path)
                    .unwrap_or(false);
                let icon_label = egui::RichText::new(icon).color(dim_if(entry.ignored, color));
                ui.label(icon_label);
                let mark = ctx.marks.get(&entry.path);
                let mut name = egui::RichText::new(&entry.name).color(dim_if(
                    entry.ignored,
                    mark.map_or(egui::Color32::from_rgb(212, 212, 212), GitMark::color),
                ));
                if entry.ignored {
                    name = name.italics();
                }
                let resp = ignored_hint(ui.selectable_label(is_selected, name), entry.ignored);
                if resp.clicked() {
                    *ctx.selected = Some(entry.path.clone());
                    *ctx.opened = Some(entry.path.clone());
                }
                resp.context_menu(|ui| {
                    if ui.button("Open").clicked() {
                        *ctx.action = Some(FileTreeAction::OpenFile(entry.path.clone()));
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Rename").clicked() {
                        *ctx.rename_state = Some((entry.path.clone(), entry.name.clone()));
                        ui.close_menu();
                    }
                    if ui.button("Delete File").clicked() {
                        *ctx.action = Some(FileTreeAction::Delete(entry.path.clone()));
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Copy Path").clicked() {
                        *ctx.action = Some(FileTreeAction::CopyPath(entry.path.clone()));
                        ui.close_menu();
                    }
                    if ui.button("Reveal in File Manager").clicked() {
                        *ctx.action = Some(FileTreeAction::RevealInExplorer(entry.path.clone()));
                        ui.close_menu();
                    }
                });
            }
        });
        let _ = row_resp;

        if entry.is_dir && entry.is_expanded {
            for child in &mut entry.children {
                Self::show_entry_recursive(ui, child, ctx);
            }
        }
    }
}

/// Where a copy of `file` goes in `dir`: "name-copie.ext", then
/// "name-copie-2.ext"… whichever is free first.
pub fn copy_destination(file: &Path, dir: &Path) -> PathBuf {
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = file
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (1..)
        .map(|n| {
            let suffix = if n == 1 {
                String::new()
            } else {
                format!("-{n}")
            };
            dir.join(format!("{stem}-copie{suffix}{ext}"))
        })
        .find(|p| !p.exists())
        .expect("an unbounded range always finds a free name")
}

/// Colour of an entry, faded when `.gitignore` excludes it.
fn dim_if(ignored: bool, color: egui::Color32) -> egui::Color32 {
    if ignored {
        color.gamma_multiply(0.45)
    } else {
        color
    }
}

/// Explain the dimmed look on hover.
fn ignored_hint(resp: egui::Response, ignored: bool) -> egui::Response {
    if ignored {
        resp.on_hover_text("Ignored by .gitignore: not part of commits")
    } else {
        resp
    }
}

/// Show `path` in the system file manager: selected in its folder (Windows
/// Explorer, macOS Finder), or its folder opened (Linux).
pub fn reveal(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // Explorer parses `/select,"path"` itself: the argument is passed
        // raw so a path with spaces stays quoted the way it expects.
        std::process::Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map(|_| ())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let folder = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        std::process::Command::new("xdg-open")
            .arg(folder)
            .spawn()
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    // ── Headless egui harness ──────────────────────────────────────────────

    struct Harness {
        ctx: egui::Context,
        texts: Vec<(String, egui::Rect)>,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                ctx: egui::Context::default(),
                texts: vec![],
            }
        }

        fn frame(&mut self, tree: &mut FileTree, events: Vec<egui::Event>) -> Option<PathBuf> {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 1200.0),
                )),
                events,
                ..Default::default()
            };
            let mut result = None;
            let out = self.ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    result = tree.show(ui);
                });
            });
            self.texts.clear();
            for cs in &out.shapes {
                collect_texts(&cs.shape, &mut self.texts);
            }
            result
        }

        /// Position of the rendered text that equals `text`, or that ends with
        /// `" " + text` (directory rows render as "<caret> <folder> <name>").
        fn find(&self, text: &str) -> Option<egui::Pos2> {
            let suffix = format!(" {text}");
            self.texts
                .iter()
                .find(|(t, _)| t == text || t.ends_with(&suffix))
                .map(|(_, r)| r.center())
        }

        fn pos(&self, text: &str) -> egui::Pos2 {
            self.find(text)
                .unwrap_or_else(|| panic!("{text:?} not rendered; have {:?}", self.texts))
        }

        /// Click `button` on `text`; returns `show()`'s result from the frame
        /// on which the click registers.
        fn click(
            &mut self,
            tree: &mut FileTree,
            text: &str,
            button: egui::PointerButton,
        ) -> Option<PathBuf> {
            self.frame(tree, vec![]);
            let pos = self.pos(text);
            let ev = |pressed| egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let a = self.frame(tree, vec![egui::Event::PointerMoved(pos), ev(true)]);
            let b = self.frame(tree, vec![ev(false)]);
            a.or(b)
        }

        fn key(&mut self, tree: &mut FileTree, key: egui::Key) -> Option<PathBuf> {
            let ev = |pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            let a = self.frame(tree, vec![ev(true)]);
            let b = self.frame(tree, vec![ev(false)]);
            a.or(b)
        }

        /// Open the context menu on `row` and click `item` in it.
        fn menu(&mut self, tree: &mut FileTree, row: &str, item: &str) -> Option<PathBuf> {
            self.click(tree, row, egui::PointerButton::Secondary);
            self.frame(tree, vec![]);
            self.click(tree, item, egui::PointerButton::Primary)
        }
    }

    fn collect_texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(t) => {
                out.push((t.galley.text().to_string(), t.visual_bounding_rect()));
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect_texts(s, out)),
            _ => {}
        }
    }

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src/inner")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "").unwrap();
        fs::write(dir.path().join("src/inner/deep.rs"), "").unwrap();
        fs::write(dir.path().join("readme.md"), "").unwrap();
        dir
    }

    fn loaded(dir: &Path) -> FileTree {
        let mut t = FileTree::new();
        t.load(dir.to_path_buf());
        t
    }

    fn child<'a>(tree: &'a FileTree, name: &str) -> &'a FileEntry {
        tree.root
            .as_ref()
            .unwrap()
            .children
            .iter()
            .find(|c| c.name == name)
            .unwrap()
    }

    // ── Non-UI behaviour ───────────────────────────────────────────────────

    #[test]
    fn new_tree_is_empty_and_show_renders_nothing() {
        let mut t = FileTree::new();
        assert!(t.root.is_none() && t.selected.is_none());
        assert!(t.context_action.is_none() && t.rename_state.is_none());
        assert!(!t.show_gitignored);
        t.reload_children(); // no root: no-op
        let mut h = Harness::new();
        assert!(h.frame(&mut t, vec![]).is_none());
        assert!(h.texts.is_empty());
    }

    fn status(path: &str, wt: crate::git::FileChangeKind) -> crate::git::FileStatus {
        crate::git::FileStatus {
            path: path.into(),
            wt_status: wt,
            ..Default::default()
        }
    }

    #[test]
    fn only_folders_holding_changes_are_marked() {
        use crate::git::FileChangeKind as K;
        let dir = workspace();
        git2::Repository::init(dir.path()).unwrap();
        fs::create_dir_all(dir.path().join("docs")).unwrap();
        let mut t = loaded(dir.path());
        t.sync_git_marks(&[
            status("src/inner/deep.rs", K::Modified),
            status("src/new.rs", K::Untracked),
            status("fresh/", K::Untracked),
        ]);
        let root = dir.path();
        let m = |t: &FileTree, p: &str| t.marks.get(&root.join(p));
        assert_eq!(m(&t, "src/inner/deep.rs"), Some(GitMark::Modified));
        assert_eq!(m(&t, "src/inner"), Some(GitMark::Modified));
        // A folder takes the most important change below it.
        assert_eq!(m(&t, "src"), Some(GitMark::Modified));
        assert_eq!(m(&t, "src/new.rs"), Some(GitMark::Added));
        assert_eq!(m(&t, "src/lib.rs"), None);
        assert_eq!(m(&t, "docs"), None, "untouched folder stays neutral");
        assert_eq!(m(&t, "readme.md"), None);
        assert_eq!(
            m(&t, "fresh/a/b.rs"),
            Some(GitMark::Added),
            "inside a new folder"
        );
        assert_eq!(t.marks.get(root), None, "the root is not coloured");

        // A conflict outranks a modification; a clean status clears all.
        t.sync_git_marks(&[crate::git::FileStatus {
            conflicted: true,
            ..status("src/lib.rs", K::Modified)
        }]);
        assert_eq!(m(&t, "src"), Some(GitMark::Conflicted));
        t.sync_git_marks(&[]);
        assert_eq!(m(&t, "src"), None);
    }

    #[test]
    fn marks_follow_a_tree_opened_inside_the_repository() {
        use crate::git::FileChangeKind as K;
        let dir = workspace();
        git2::Repository::init(dir.path()).unwrap();
        let mut t = loaded(&dir.path().join("src"));
        t.sync_git_marks(&[
            status("src/inner/deep.rs", K::Modified),
            status("readme.md", K::Modified),
        ]);
        let src = dir.path().join("src");
        assert_eq!(t.marks.get(&src.join("inner")), Some(GitMark::Modified));
        assert_eq!(t.marks.by_path.len(), 2, "readme.md is outside the tree");
    }

    #[test]
    fn load_populates_root_children() {
        let dir = workspace();
        let t = loaded(dir.path());
        let root = t.root.as_ref().unwrap();
        assert_eq!(root.path, dir.path());
        assert!(root.is_expanded);
        let names: Vec<_> = root.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["src", "readme.md"]);
    }

    #[test]
    fn load_in_git_repo_honours_gitignore_and_toggle() {
        let dir = workspace();
        git2::Repository::init(dir.path()).unwrap();
        fs::write(dir.path().join(".gitignore"), "readme.md\n").unwrap();
        let mut t = loaded(dir.path());
        let names: Vec<_> = t
            .root
            .as_ref()
            .unwrap()
            .children
            .iter()
            .map(|c| c.name.clone())
            .collect();
        assert_eq!(names, ["src", ".gitignore"]);

        t.show_gitignored = true;
        t.reload_children();
        let names: Vec<_> = t
            .root
            .as_ref()
            .unwrap()
            .children
            .iter()
            .map(|c| c.name.clone())
            .collect();
        assert_eq!(names, ["src", ".gitignore", "readme.md"]);
    }

    #[test]
    fn reload_children_picks_up_new_files() {
        let dir = workspace();
        let mut t = loaded(dir.path());
        fs::write(dir.path().join("zzz.txt"), "").unwrap();
        t.reload_children();
        assert!(t
            .root
            .as_ref()
            .unwrap()
            .children
            .iter()
            .any(|c| c.name == "zzz.txt"));
    }

    // ── Rendering / interaction ────────────────────────────────────────────

    #[test]
    fn show_renders_expanded_root_and_collapsed_children() {
        let dir = workspace();
        let mut t = loaded(dir.path());
        let mut h = Harness::new();
        assert!(h.frame(&mut t, vec![]).is_none());
        let root_name = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(h.find(&root_name).is_some());
        assert!(h.find("src").is_some());
        assert!(h.find("readme.md").is_some());
        assert!(
            h.find("lib.rs").is_none(),
            "collapsed folder content hidden"
        );
    }

    #[test]
    fn clicking_folder_expands_loading_children_then_collapses() {
        let dir = workspace();
        let mut t = loaded(dir.path());
        let mut h = Harness::new();
        assert!(h
            .click(&mut t, "src", egui::PointerButton::Primary)
            .is_none());
        assert!(child(&t, "src").is_expanded);
        assert_eq!(child(&t, "src").children.len(), 2);
        h.frame(&mut t, vec![]);
        assert!(h.find("lib.rs").is_some());
        assert!(h.find("inner").is_some());

        h.click(&mut t, "src", egui::PointerButton::Primary);
        assert!(!child(&t, "src").is_expanded);
        h.frame(&mut t, vec![]);
        assert!(h.find("lib.rs").is_none());
    }

    #[test]
    fn reexpanding_a_folder_shows_changes_made_while_collapsed() {
        let dir = workspace();
        let mut t = loaded(dir.path());
        let mut h = Harness::new();
        h.click(&mut t, "src", egui::PointerButton::Primary);
        h.click(&mut t, "src", egui::PointerButton::Primary);
        assert!(!child(&t, "src").is_expanded);

        fs::create_dir(dir.path().join("src/added")).unwrap();
        fs::remove_file(dir.path().join("src/lib.rs")).unwrap();
        h.click(&mut t, "src", egui::PointerButton::Primary);
        let names: Vec<_> = child(&t, "src")
            .children
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["added", "inner"]);
    }

    #[test]
    fn clicking_file_selects_and_opens_it() {
        let dir = workspace();
        let mut t = loaded(dir.path());
        let mut h = Harness::new();
        let opened = h.click(&mut t, "readme.md", egui::PointerButton::Primary);
        let expected = dir.path().join("readme.md");
        assert_eq!(opened, Some(expected.clone()));
        assert_eq!(t.selected, Some(expected));
        assert!(t.context_action.is_none());
    }

    #[test]
    fn file_context_menu_open_returns_path() {
        let dir = workspace();
        let mut t = loaded(dir.path());
        let mut h = Harness::new();
        let opened = h.menu(&mut t, "readme.md", "Open");
        assert_eq!(opened, Some(dir.path().join("readme.md")));
        assert!(t.context_action.is_none(), "Open is returned, not queued");
    }

    #[test]
    fn file_context_menu_actions_are_queued() {
        let dir = workspace();
        let file = dir.path().join("readme.md");
        for (item, check) in [
            ("Delete File", "Delete"),
            ("Copy Path", "CopyPath"),
            ("Reveal in File Manager", "Reveal"),
        ] {
            let mut t = loaded(dir.path());
            let mut h = Harness::new();
            assert!(h.menu(&mut t, "readme.md", item).is_none());
            match (check, t.context_action.take()) {
                ("Delete", Some(FileTreeAction::Delete(p)))
                | ("CopyPath", Some(FileTreeAction::CopyPath(p)))
                | ("Reveal", Some(FileTreeAction::RevealInExplorer(p))) => assert_eq!(p, file),
                (c, other) => panic!("{item}: expected {c}, got {other:?}"),
            }
        }
    }

    #[test]
    fn ctrl_c_ctrl_v_on_a_clicked_file_asks_to_duplicate_it() {
        let dir = workspace();
        let file = dir.path().join("readme.md");
        let mut t = loaded(dir.path());
        let mut h = Harness::new();
        h.click(&mut t, "readme.md", egui::PointerButton::Primary);
        h.frame(&mut t, vec![egui::Event::Copy]);
        assert!(t.context_action.is_none(), "copying alone does nothing");
        h.frame(&mut t, vec![egui::Event::Paste("clipboard text".into())]);
        match t.context_action.take() {
            Some(FileTreeAction::Duplicate(src, to)) => {
                assert_eq!(src, file);
                assert_eq!(to, dir.path());
            }
            other => panic!("expected Duplicate, got {other:?}"),
        }
    }

    #[test]
    fn one_ctrl_v_pastes_once() {
        let dir = workspace();
        let mut t = loaded(dir.path());
        let mut h = Harness::new();
        h.click(&mut t, "readme.md", egui::PointerButton::Primary);
        h.frame(&mut t, vec![egui::Event::Copy]);
        let v = |pressed| egui::Event::Key {
            key: egui::Key::V,
            physical_key: None,
            pressed,
            repeat: false,
            // What egui-winit sends for Ctrl on Windows / Linux.
            modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
        };
        let mut pastes = 0;
        for events in [
            vec![v(true), egui::Event::Paste("text".into())],
            vec![v(false)],
        ] {
            h.frame(&mut t, events);
            if let Some(FileTreeAction::Duplicate(..)) = t.context_action.take() {
                pastes += 1;
            }
        }
        assert_eq!(pastes, 1, "releasing V must not paste again");
    }

    #[test]
    fn delete_key_asks_to_delete_the_selected_file() {
        let dir = workspace();
        let mut t = loaded(dir.path());
        let mut h = Harness::new();
        h.click(&mut t, "readme.md", egui::PointerButton::Primary);
        h.key(&mut t, egui::Key::Delete);
        match t.context_action.take() {
            Some(FileTreeAction::Delete(p)) => assert_eq!(p, dir.path().join("readme.md")),
            other => panic!("expected Delete, got {other:?}"),
        }
    }

    #[test]
    fn typing_elsewhere_gives_ctrl_v_back_to_the_editor() {
        let dir = workspace();
        let mut t = loaded(dir.path());
        let mut h = Harness::new();
        h.click(&mut t, "readme.md", egui::PointerButton::Primary);
        h.frame(&mut t, vec![egui::Event::Copy]);
        h.frame(&mut t, vec![egui::Event::Text("x".into())]);
        h.frame(&mut t, vec![egui::Event::Paste("text".into())]);
        assert!(t.context_action.is_none());
    }

    #[test]
    fn copy_destination_picks_a_free_copie_name() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("notes.md");
        fs::write(&src, "").unwrap();
        let first = copy_destination(&src, dir.path());
        assert_eq!(first, dir.path().join("notes-copie.md"));
        fs::write(&first, "").unwrap();
        assert_eq!(
            copy_destination(&src, dir.path()),
            dir.path().join("notes-copie-2.md")
        );
        assert_eq!(
            copy_destination(Path::new("Makefile"), dir.path()),
            dir.path().join("Makefile-copie")
        );
    }

    #[test]
    fn folder_context_menu_actions_are_queued() {
        let dir = workspace();
        let folder = dir.path().join("src");
        for (item, check) in [
            ("New File", "NewFile"),
            ("New Folder", "NewFolder"),
            ("Delete Folder", "Delete"),
            ("Copy Path", "CopyPath"),
            ("Reveal in File Manager", "Reveal"),
        ] {
            let mut t = loaded(dir.path());
            let mut h = Harness::new();
            assert!(h.menu(&mut t, "src", item).is_none());
            match (check, t.context_action.take()) {
                ("NewFile", Some(FileTreeAction::NewFile(p)))
                | ("NewFolder", Some(FileTreeAction::NewFolder(p)))
                | ("Delete", Some(FileTreeAction::Delete(p)))
                | ("CopyPath", Some(FileTreeAction::CopyPath(p)))
                | ("Reveal", Some(FileTreeAction::RevealInExplorer(p))) => assert_eq!(p, folder),
                (c, other) => panic!("{item}: expected {c}, got {other:?}"),
            }
        }
    }

    #[test]
    fn rename_menu_items_enter_inline_rename_mode() {
        let dir = workspace();
        let mut t = loaded(dir.path());
        let mut h = Harness::new();
        h.menu(&mut t, "readme.md", "Rename");
        assert_eq!(
            t.rename_state,
            Some((dir.path().join("readme.md"), "readme.md".to_string()))
        );

        let mut t = loaded(dir.path());
        let mut h = Harness::new();
        h.menu(&mut t, "src", "Rename");
        assert_eq!(
            t.rename_state,
            Some((dir.path().join("src"), "src".to_string()))
        );
    }

    #[test]
    fn rename_shortcut_renames_the_selected_file_once_the_tree_is_clicked() {
        let dir = workspace();
        let mut t = loaded(dir.path());
        assert!(!t.rename_selected(), "untouched tree: F2 is the editor's");
        let mut h = Harness::new();
        h.click(&mut t, "readme.md", egui::PointerButton::Primary);
        assert!(t.rename_selected());
        assert_eq!(
            t.rename_state,
            Some((dir.path().join("readme.md"), "readme.md".to_string()))
        );
        h.frame(&mut t, vec![]);
        assert!(
            h.find("readme.md").is_some(),
            "field shows the current name"
        );
    }

    #[test]
    fn rename_shortcut_is_the_editors_once_another_key_is_typed() {
        let dir = workspace();
        let mut t = loaded(dir.path());
        let mut h = Harness::new();
        h.click(&mut t, "readme.md", egui::PointerButton::Primary);
        h.key(&mut t, egui::Key::A);
        assert!(!t.rename_selected());
        assert!(t.rename_state.is_none());
    }

    #[test]
    fn enter_in_rename_mode_emits_rename_action() {
        let dir = workspace();
        let mut t = loaded(dir.path());
        let file = dir.path().join("readme.md");
        t.rename_state = Some((file.clone(), "README.md".to_string()));
        let mut h = Harness::new();
        h.frame(&mut t, vec![]);
        assert!(
            h.find("README.md").is_some(),
            "text edit shows the pending name"
        );
        assert!(t.context_action.is_none());
        h.key(&mut t, egui::Key::Enter);
        match t.context_action.take() {
            Some(FileTreeAction::Rename(p, n)) => {
                assert_eq!(p, file);
                assert_eq!(n, "README.md");
            }
            other => panic!("expected Rename, got {other:?}"),
        }
        assert!(t.rename_state.is_none());
    }

    #[test]
    fn escape_in_rename_mode_cancels_without_action() {
        let dir = workspace();
        let mut t = loaded(dir.path());
        t.rename_state = Some((dir.path().join("readme.md"), "other.md".to_string()));
        let mut h = Harness::new();
        h.frame(&mut t, vec![]);
        h.key(&mut t, egui::Key::Escape);
        assert!(t.rename_state.is_none());
        assert!(
            !matches!(t.context_action, Some(FileTreeAction::Rename(..))),
            "Escape must not rename, got {:?}",
            t.context_action
        );
    }
}
