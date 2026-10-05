use crate::filetree::FileTree;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use std::path::PathBuf;

#[allow(dead_code)] // kept for the signature of show()
const _FILETREE_USED: () = ();

#[derive(Debug, Clone, PartialEq)]
pub enum PaletteCommand {
    ToggleTerminal,
    ToggleClaude,
    ToggleSidebar,
    GoToLine,
    SaveFile,
    NewFile,
    OpenFolder,
    OpenSettings,
    Find,
    FindReplace,
    RestartLsp,
    CheckForUpdates,
    ShowProblems,
}

impl PaletteCommand {
    fn all() -> &'static [PaletteCommand] {
        &[
            PaletteCommand::ToggleTerminal,
            PaletteCommand::ToggleClaude,
            PaletteCommand::ToggleSidebar,
            PaletteCommand::GoToLine,
            PaletteCommand::SaveFile,
            PaletteCommand::NewFile,
            PaletteCommand::OpenFolder,
            PaletteCommand::OpenSettings,
            PaletteCommand::Find,
            PaletteCommand::FindReplace,
            PaletteCommand::RestartLsp,
            PaletteCommand::CheckForUpdates,
            PaletteCommand::ShowProblems,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::ToggleTerminal => "Toggle Terminal",
            Self::ToggleClaude => "Toggle Claude panel",
            Self::ToggleSidebar => "Toggle Sidebar",
            Self::GoToLine => "Go to Line…",
            Self::SaveFile => "Save File",
            Self::NewFile => "New File",
            Self::OpenFolder => "Open Folder…",
            Self::OpenSettings => "Open Settings",
            Self::Find => "Find in File",
            Self::FindReplace => "Find & Replace",
            Self::RestartLsp => "Restart LSP Server",
            Self::CheckForUpdates => "Check for Updates",
            Self::ShowProblems => "Problems: Show",
        }
    }

    pub fn shortcut(&self) -> &'static str {
        match self {
            Self::ToggleTerminal => "Ctrl+`",
            Self::ToggleClaude => "Ctrl+Shift+I",
            Self::ToggleSidebar => "Ctrl+B",
            Self::GoToLine => "Ctrl+G",
            Self::SaveFile => "Ctrl+S",
            Self::NewFile => "Ctrl+N",
            Self::OpenFolder => "Ctrl+O",
            Self::OpenSettings => "Ctrl+,",
            Self::Find => "Ctrl+F",
            Self::FindReplace => "Ctrl+H",
            Self::RestartLsp => "",
            Self::CheckForUpdates => "",
            Self::ShowProblems => "Ctrl+Shift+M",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::ToggleTerminal => "open close integrated shell console",
            Self::ToggleClaude => "ai assistant chat panel",
            Self::ToggleSidebar => "explorer file tree side panel",
            Self::GoToLine => "jump navigate to line number",
            Self::SaveFile => "write persist current document",
            Self::NewFile => "create blank document",
            Self::OpenFolder => "open workspace project directory",
            Self::OpenSettings => "preferences configuration options theme",
            Self::Find => "search text in current file",
            Self::FindReplace => "search and substitute replace text",
            Self::RestartLsp => "restart language server diagnostics",
            Self::CheckForUpdates => "upgrade new version release install",
            Self::ShowProblems => "errors warnings lint workspace problems panel",
        }
    }
}

#[derive(Debug, Clone)]
enum PaletteEntry {
    File(PathBuf),
    Command(PaletteCommand),
}

pub struct CommandPalette {
    pub open: bool,
    pub query: String,
    entries: Vec<PaletteEntry>,
    matcher: SkimMatcherV2,
    /// Index of the highlighted result row.
    selected_idx: usize,
    /// All workspace files, cached when the palette opens.
    cached_files: Vec<PathBuf>,
}

impl CommandPalette {
    pub fn new() -> Self {
        Self {
            open: false,
            query: String::new(),
            entries: vec![],
            matcher: SkimMatcherV2::default(),
            selected_idx: 0,
            cached_files: vec![],
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.query.clear();
            self.entries.clear();
            self.selected_idx = 0;
            self.cached_files.clear();
        }
    }

    /// Open the palette in commands mode (prefixed with '>').
    pub fn toggle_commands(&mut self) {
        self.open = !self.open;
        if self.open {
            self.query = ">".to_string();
            self.entries.clear();
            self.selected_idx = 0;
            self.cached_files.clear();
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Returns (opened_file, command).
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        _file_tree: &mut FileTree,
        workspace: &mut Option<PathBuf>,
        palette: crate::ui::theme::Palette,
    ) -> (Option<PathBuf>, Option<PaletteCommand>) {
        // Load file cache once when palette opens (cached_files is cleared in toggle()).
        if self.cached_files.is_empty() {
            if let Some(ws) = workspace.as_ref() {
                self.cached_files = collect_workspace_files(ws);
            }
        }

        let mut close = false;
        let mut opened_file: Option<PathBuf> = None;
        let mut triggered_cmd: Option<PaletteCommand> = None;

        // Keyboard navigation outside the window (so it fires even when text edit has focus).
        let (nav_down, nav_up, nav_confirm) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::Tab),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::Enter),
            )
        });

        let commands_only = self.query.starts_with('>');
        let effective_query = if commands_only {
            self.query.trim_start_matches('>').trim().to_string()
        } else {
            self.query.clone()
        };

        // Rebuild entry list whenever query changes (cheap enough each frame).
        self.entries.clear();
        if commands_only {
            for cmd in PaletteCommand::all() {
                let haystack = format!("{} {}", cmd.label(), cmd.description());
                if effective_query.is_empty()
                    || self
                        .matcher
                        .fuzzy_match(&haystack, &effective_query)
                        .is_some()
                {
                    self.entries.push(PaletteEntry::Command(cmd.clone()));
                }
            }
        } else {
            if effective_query.is_empty() {
                for p in self.cached_files.iter().take(15) {
                    self.entries.push(PaletteEntry::File(p.clone()));
                }
            } else {
                let q = &effective_query;
                let mut scored: Vec<(i64, PathBuf)> = self
                    .cached_files
                    .iter()
                    .filter_map(|p| {
                        // Match against relative path so partial paths work (e.g. "src/main")
                        let display = p.to_string_lossy().to_string();
                        let score = self.matcher.fuzzy_match(&display, q)?;
                        Some((score, p.clone()))
                    })
                    .collect();
                scored.sort_by_key(|b| std::cmp::Reverse(b.0));
                for (_, p) in scored.into_iter().take(20) {
                    self.entries.push(PaletteEntry::File(p));
                }
            }
            // Commands at the bottom
            for cmd in PaletteCommand::all() {
                let haystack = format!("{} {}", cmd.label(), cmd.description());
                if effective_query.is_empty()
                    || self
                        .matcher
                        .fuzzy_match(&haystack, &effective_query)
                        .is_some()
                {
                    self.entries.push(PaletteEntry::Command(cmd.clone()));
                }
            }
        }

        // Clamp selection index.
        let entry_count = self.entries.len();
        if entry_count == 0 {
            self.selected_idx = 0;
        } else {
            if nav_down {
                self.selected_idx = (self.selected_idx + 1) % entry_count;
            }
            if nav_up {
                self.selected_idx = self.selected_idx.checked_sub(1).unwrap_or(entry_count - 1);
            }
            self.selected_idx = self.selected_idx.min(entry_count - 1);
        }

        // Enter confirms the currently selected entry.
        if nav_confirm && entry_count > 0 {
            if let Some(entry) = self.entries.get(self.selected_idx) {
                match entry {
                    PaletteEntry::File(p) => {
                        opened_file = Some(p.clone());
                        close = true;
                    }
                    PaletteEntry::Command(c) => {
                        triggered_cmd = Some(c.clone());
                        close = true;
                    }
                }
            }
        }

        egui::Window::new("Command Palette")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .fixed_pos(egui::pos2(
                ctx.screen_rect().center().x - 280.0,
                ctx.screen_rect().top() + 60.0,
            ))
            .fixed_size(egui::vec2(560.0, 420.0))
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    let hint = if self.query.starts_with('>') {
                        "Run command (> to search commands, clear for files)…"
                    } else {
                        "Search files… (type > for commands)"
                    };
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .desired_width(ui.available_width())
                            .hint_text(hint)
                            .font(egui::TextStyle::Monospace)
                            .lock_focus(true), // prevents Tab from cycling egui focus
                    );
                    response.request_focus();

                    ui.separator();

                    let sel = self.selected_idx;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (i, entry) in self.entries.clone().iter().enumerate() {
                            let is_selected = i == sel;
                            match entry {
                                PaletteEntry::File(path) => {
                                    let name = path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    let dir = path
                                        .parent()
                                        .map(|p| p.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    let resp = ui.selectable_label(
                                        is_selected,
                                        format!("{}\n  {}", name, dir),
                                    );
                                    if is_selected {
                                        resp.scroll_to_me(None);
                                    }
                                    if resp.clicked() {
                                        opened_file = Some(path.clone());
                                        close = true;
                                    }
                                }
                                PaletteEntry::Command(cmd) => {
                                    let label = format!(
                                        "⚡  {}{}",
                                        cmd.label(),
                                        if cmd.shortcut().is_empty() {
                                            String::new()
                                        } else {
                                            format!("    {}", cmd.shortcut())
                                        }
                                    );
                                    let resp = ui.selectable_label(
                                        is_selected,
                                        egui::RichText::new(label).color(palette.accent),
                                    );
                                    if is_selected {
                                        resp.scroll_to_me(None);
                                    }
                                    if resp.clicked() {
                                        triggered_cmd = Some(cmd.clone());
                                        close = true;
                                    }
                                }
                            }
                        }
                    });

                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close = true;
                    }
                });
            });

        if close {
            self.open = false;
            self.selected_idx = 0;
        }
        (opened_file, triggered_cmd)
    }
}

/// Walk the workspace using `git ls-files` (respects .gitignore).
/// Falls back to a simple recursive walk if git is not available.
fn collect_workspace_files(workspace: &std::path::Path) -> Vec<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .current_dir(workspace)
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            let files: Vec<PathBuf> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|l| !l.is_empty())
                .map(|line| workspace.join(line))
                .filter(|p| p.is_file())
                .collect();
            if !files.is_empty() {
                return files;
            }
        }
    }

    // Fallback: recursive walk, skip hidden dirs and common build dirs.
    let mut out = Vec::new();
    walk_dir_fallback(workspace, &mut out);
    out
}

fn walk_dir_fallback(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }
        if matches!(
            name_str.as_ref(),
            "target" | "node_modules" | "dist" | "build"
        ) {
            continue;
        }
        if path.is_dir() {
            walk_dir_fallback(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

impl Default for CommandPalette {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect};

    // ---- command metadata ----

    #[test]
    fn every_command_has_metadata() {
        let all = PaletteCommand::all();
        assert_eq!(all.len(), 13);
        let mut labels = std::collections::HashSet::new();
        for cmd in all {
            assert!(!cmd.label().trim().is_empty(), "{cmd:?} label");
            assert!(!cmd.description().trim().is_empty(), "{cmd:?} description");
            assert!(
                labels.insert(cmd.label()),
                "duplicate label {}",
                cmd.label()
            );
            let s = cmd.shortcut();
            assert!(
                s.is_empty() || s.starts_with("Ctrl+"),
                "{cmd:?} shortcut {s:?}"
            );
        }
    }

    #[test]
    fn all_lists_every_variant_once() {
        // Exhaustive match: adding a variant without listing it here (and in all()) fails.
        fn index(c: &PaletteCommand) -> usize {
            match c {
                PaletteCommand::ToggleTerminal => 0,
                PaletteCommand::ToggleClaude => 1,
                PaletteCommand::ToggleSidebar => 2,
                PaletteCommand::GoToLine => 3,
                PaletteCommand::SaveFile => 4,
                PaletteCommand::NewFile => 5,
                PaletteCommand::OpenFolder => 6,
                PaletteCommand::OpenSettings => 7,
                PaletteCommand::Find => 8,
                PaletteCommand::FindReplace => 9,
                PaletteCommand::RestartLsp => 10,
                PaletteCommand::CheckForUpdates => 11,
                PaletteCommand::ShowProblems => 12,
            }
        }
        let mut seen = [false; 13];
        for c in PaletteCommand::all() {
            assert!(!seen[index(c)], "{c:?} listed twice");
            seen[index(c)] = true;
        }
        assert!(seen.iter().all(|s| *s));
    }

    #[test]
    fn shortcuts_match_known_bindings() {
        assert_eq!(PaletteCommand::SaveFile.shortcut(), "Ctrl+S");
        assert_eq!(PaletteCommand::ToggleClaude.shortcut(), "Ctrl+Shift+I");
        assert_eq!(PaletteCommand::RestartLsp.shortcut(), "");
        assert_eq!(PaletteCommand::CheckForUpdates.label(), "Check for Updates");
    }

    // ---- open/close state ----

    #[test]
    fn toggle_opens_in_file_mode_and_resets() {
        let mut p = CommandPalette::default();
        assert!(!p.is_open());
        p.query = "stale".into();
        p.selected_idx = 3;
        p.cached_files = vec![PathBuf::from("x")];
        p.toggle();
        assert!(p.is_open());
        assert!(p.query.is_empty());
        assert_eq!(p.selected_idx, 0);
        assert!(p.cached_files.is_empty(), "file cache refreshed on open");
        p.query = "keep".into();
        p.toggle();
        assert!(!p.is_open());
        assert_eq!(p.query, "keep", "closing does not reset");
    }

    #[test]
    fn toggle_commands_opens_with_prefix() {
        let mut p = CommandPalette::new();
        p.selected_idx = 2;
        p.toggle_commands();
        assert!(p.is_open());
        assert_eq!(p.query, ">");
        assert_eq!(p.selected_idx, 0);
        p.toggle_commands();
        assert!(!p.is_open());
    }

    // ---- headless show() ----

    struct Harness {
        ctx: egui::Context,
        p: CommandPalette,
        tree: FileTree,
    }

    impl Harness {
        fn new(files: &[&str]) -> Self {
            let mut p = CommandPalette::new();
            p.toggle();
            // Pre-filled cache: with no workspace show() never spawns `git`.
            p.cached_files = files.iter().map(PathBuf::from).collect();
            Self {
                ctx: egui::Context::default(),
                p,
                tree: FileTree::new(),
            }
        }

        fn frame(&mut self, events: Vec<Event>) -> (Option<PathBuf>, Option<PaletteCommand>) {
            let palette =
                crate::ui::theme::Palette::from_theme(&crate::config::Config::default().theme);
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1024.0, 768.0))),
                events,
                ..Default::default()
            };
            let mut res = (None, None);
            let mut ws = None;
            let Self { ctx, p, tree } = self;
            let _ = ctx.run(input, |ctx| {
                res = p.show(ctx, tree, &mut ws, palette);
            });
            res
        }

        fn key(&mut self, key: Key) -> (Option<PathBuf>, Option<PaletteCommand>) {
            self.frame(vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }])
        }

        fn query(&mut self, q: &str) {
            self.p.query = q.into();
            // Two frames: the window is laid out invisibly on its first one.
            for _ in 0..2 {
                let r = self.frame(vec![]);
                assert_eq!(r, (None, None), "typing never triggers");
            }
        }

        /// Result rows (selectable labels) of the last frame, top to bottom. Excludes
        /// the window background, which also senses clicks but spans the whole window.
        fn rows(&self) -> Vec<Rect> {
            let mut rows: Vec<Rect> = self.ctx.viewport(|vp| {
                let w = &vp.prev_pass.widgets;
                w.layer_ids()
                    .flat_map(|l| w.get_layer(l))
                    .filter(|r| {
                        r.sense.is_focusable()
                            && r.sense.senses_click()
                            && !r.sense.senses_drag()
                            && r.rect.width() < 500.0
                    })
                    .map(|r| r.rect)
                    .collect()
            });
            rows.sort_by(|a, b| a.top().partial_cmp(&b.top()).unwrap());
            rows
        }

        fn files(&self) -> Vec<String> {
            self.p
                .entries
                .iter()
                .filter_map(|e| match e {
                    PaletteEntry::File(p) => Some(p.to_string_lossy().replace('\\', "/")),
                    _ => None,
                })
                .collect()
        }

        fn commands(&self) -> Vec<PaletteCommand> {
            self.p
                .entries
                .iter()
                .filter_map(|e| match e {
                    PaletteEntry::Command(c) => Some(c.clone()),
                    _ => None,
                })
                .collect()
        }
    }

    #[test]
    fn empty_query_lists_files_then_all_commands() {
        let mut h = Harness::new(&["a.rs", "b.rs"]);
        h.query("");
        assert_eq!(h.files(), ["a.rs", "b.rs"]);
        assert_eq!(h.commands(), PaletteCommand::all());
        assert!(matches!(h.p.entries[0], PaletteEntry::File(_)));
        assert!(matches!(h.p.entries[2], PaletteEntry::Command(_)));
    }

    #[test]
    fn empty_query_caps_files_at_15() {
        let names: Vec<String> = (0..30).map(|i| format!("f{i:02}.rs")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut h = Harness::new(&refs);
        h.query("");
        assert_eq!(h.files().len(), 15);
        assert_eq!(h.files()[0], "f00.rs");
    }

    #[test]
    fn fuzzy_query_ranks_and_filters_files() {
        let mut h = Harness::new(&[
            "docs/readme.md",
            "src/main.rs",
            "src/ui/mainmenu.rs",
            "x.txt",
        ]);
        h.query("src/main");
        let files = h.files();
        assert_eq!(files.first().map(String::as_str), Some("src/main.rs"));
        assert!(!files.contains(&"x.txt".to_string()));
        assert!(!files.contains(&"docs/readme.md".to_string()));
    }

    #[test]
    fn fuzzy_query_caps_files_at_20() {
        let names: Vec<String> = (0..40).map(|i| format!("mod{i:02}.rs")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut h = Harness::new(&refs);
        h.query("mod");
        assert_eq!(h.files().len(), 20);
    }

    #[test]
    fn file_mode_query_also_matches_commands_by_description() {
        let mut h = Harness::new(&["a.rs"]);
        h.query("upgrade");
        assert!(h.files().is_empty());
        assert_eq!(h.commands(), [PaletteCommand::CheckForUpdates]);
    }

    #[test]
    fn command_mode_hides_files() {
        let mut h = Harness::new(&["settings.rs", "a.rs"]);
        h.query(">");
        assert!(h.files().is_empty());
        assert_eq!(h.commands(), PaletteCommand::all());

        h.query(">  settings ");
        assert!(h.files().is_empty());
        assert_eq!(h.commands()[0], PaletteCommand::OpenSettings);
    }

    #[test]
    fn command_mode_matches_label_and_description() {
        let mut h = Harness::new(&[]);
        h.query(">restart");
        assert_eq!(h.commands(), [PaletteCommand::RestartLsp]);
        h.query(">diagnostics");
        assert_eq!(h.commands(), [PaletteCommand::RestartLsp]);
        h.query(">qqqzzz");
        assert!(h.p.entries.is_empty());
    }

    #[test]
    fn enter_runs_first_command() {
        let mut h = Harness::new(&[]);
        h.query(">restart");
        let (file, cmd) = h.key(Key::Enter);
        assert_eq!(file, None);
        assert_eq!(cmd, Some(PaletteCommand::RestartLsp));
        assert!(!h.p.is_open());
    }

    #[test]
    fn enter_opens_selected_file() {
        let mut h = Harness::new(&["one.rs", "two.rs"]);
        h.query("");
        h.key(Key::ArrowDown);
        assert_eq!(h.p.selected_idx, 1);
        let (file, cmd) = h.key(Key::Enter);
        assert_eq!(file, Some(PathBuf::from("two.rs")));
        assert_eq!(cmd, None);
        assert!(!h.p.is_open());
        assert_eq!(h.p.selected_idx, 0, "selection reset on close");
    }

    #[test]
    fn enter_with_no_results_does_nothing() {
        let mut h = Harness::new(&["a.rs"]);
        h.query(">qqqzzz");
        assert_eq!(h.key(Key::Enter), (None, None));
        assert!(h.p.is_open());
        assert_eq!(h.p.selected_idx, 0);
    }

    #[test]
    fn arrow_and_tab_navigation_wraps() {
        let mut h = Harness::new(&[]);
        h.query(">"); // every command
        let n = PaletteCommand::all().len();
        h.key(Key::ArrowUp);
        assert_eq!(h.p.selected_idx, n - 1, "up from top wraps to bottom");
        h.key(Key::ArrowDown);
        assert_eq!(h.p.selected_idx, 0, "down from bottom wraps to top");
        h.key(Key::Tab);
        assert_eq!(h.p.selected_idx, 1, "Tab moves down");
        h.key(Key::ArrowUp);
        assert_eq!(h.p.selected_idx, 0);
        h.key(Key::ArrowDown);
        h.key(Key::ArrowDown);
        let (_, cmd) = h.key(Key::Enter);
        assert_eq!(cmd, Some(PaletteCommand::all()[2].clone()));
    }

    #[test]
    fn selection_clamped_when_results_shrink() {
        let mut h = Harness::new(&[]);
        h.query(">");
        h.key(Key::ArrowUp);
        assert_eq!(h.p.selected_idx, PaletteCommand::all().len() - 1);
        h.query(">restart");
        assert_eq!(h.p.selected_idx, 0);
        h.query(">qqqzzz");
        assert_eq!(h.p.selected_idx, 0);
    }

    #[test]
    fn escape_closes_without_result() {
        let mut h = Harness::new(&["a.rs"]);
        h.query("");
        h.key(Key::ArrowDown);
        assert_eq!(h.key(Key::Escape), (None, None));
        assert!(!h.p.is_open());
        assert_eq!(h.p.selected_idx, 0);
    }

    #[test]
    fn clicking_an_entry_triggers_it() {
        let mut h = Harness::new(&[]);
        h.query(">restart");
        let rows = h.rows();
        assert_eq!(rows.len(), 1, "one matching command row {rows:?}");
        let pos = rows[0].center();
        let press = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        h.p.query = ">restart".into();
        assert_eq!(
            h.frame(vec![Event::PointerMoved(pos), press(true)]),
            (None, None)
        );
        let (_, cmd) = h.frame(vec![press(false)]);
        assert_eq!(cmd, Some(PaletteCommand::RestartLsp));
        assert!(!h.p.is_open());
    }

    #[test]
    fn clicking_a_file_row_opens_it() {
        let mut h = Harness::new(&["dir/only.rs"]);
        h.query(">qqqzzz"); // nothing
        h.query("only");
        let rows = h.rows();
        assert!(!rows.is_empty());
        // Files come first, so the top row is the file.
        let pos = rows[0].center();
        let press = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        h.frame(vec![Event::PointerMoved(pos), press(true)]);
        let (file, _) = h.frame(vec![press(false)]);
        assert_eq!(file, Some(PathBuf::from("dir/only.rs")));
    }

    // ---- fallback directory walk ----

    #[test]
    fn walk_skips_hidden_and_build_dirs() {
        let root = std::env::temp_dir().join(format!("cu-palette-walk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in [
            "src/nested",
            ".git",
            "target",
            "node_modules",
            "dist",
            "build",
        ] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        for f in [
            "README.md",
            "src/lib.rs",
            "src/nested/deep.rs",
            ".hidden",
            ".git/config",
            "target/out",
            "node_modules/pkg.js",
            "dist/a.js",
            "build/b.o",
        ] {
            std::fs::write(root.join(f), b"").unwrap();
        }
        let mut out = Vec::new();
        walk_dir_fallback(&root, &mut out);
        let mut rel: Vec<String> = out
            .iter()
            .map(|p| {
                p.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        rel.sort();
        assert_eq!(rel, ["README.md", "src/lib.rs", "src/nested/deep.rs"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn walk_missing_dir_is_empty() {
        let mut out = Vec::new();
        walk_dir_fallback(std::path::Path::new("definitely/not/here/xyz"), &mut out);
        assert!(out.is_empty());
    }
}
