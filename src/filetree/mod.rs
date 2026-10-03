pub mod entry;
pub mod icons;

pub use entry::FileEntry;
pub use icons::file_icon;

use egui_phosphor::regular as ph;
use std::path::PathBuf;

/// Context passed through recursive file tree rendering.
struct ShowContext<'a> {
    selected: &'a mut Option<PathBuf>,
    opened: &'a mut Option<PathBuf>,
    action: &'a mut Option<FileTreeAction>,
    rename_state: &'a mut Option<(PathBuf, String)>,
    repo: Option<&'a git2::Repository>,
    show_gitignored: bool,
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
        }
    }

    pub fn load(&mut self, path: PathBuf) {
        self.repo = git2::Repository::discover(&path).ok();
        let mut root = FileEntry::new(path, 0);
        root.load_children(self.repo.as_ref(), self.show_gitignored);
        self.root = Some(root);
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
        let mut rename_state = self.rename_state.take();
        let mut ctx = ShowContext {
            selected: &mut self.selected,
            opened: &mut opened,
            action: &mut action,
            rename_state: &mut rename_state,
            repo: self.repo.as_ref(),
            show_gitignored: self.show_gitignored,
        };
        if let Some(root) = &mut self.root {
            Self::show_entry_recursive(ui, root, &mut ctx);
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
                let color = egui::Color32::from_rgb(220, 180, 100);
                let label =
                    egui::RichText::new(format!("{} {} {}", arrow, folder_icon, entry.name))
                        .color(color);
                let resp = ui.selectable_label(false, label);
                if resp.clicked() {
                    entry.is_expanded = !entry.is_expanded;
                    if entry.is_expanded && entry.children.is_empty() {
                        entry.load_children(ctx.repo, ctx.show_gitignored);
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
                let icon_label = egui::RichText::new(icon).color(color);
                ui.label(icon_label);
                let resp = ui.selectable_label(
                    is_selected,
                    egui::RichText::new(&entry.name).color(egui::Color32::from_rgb(212, 212, 212)),
                );
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
        assert_eq!(names, ["src"]);

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
        assert_eq!(names, ["src", "readme.md"]);
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
