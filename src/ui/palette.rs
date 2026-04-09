use crate::filetree::FileTree;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use std::path::PathBuf;

#[allow(dead_code)] // kept for the signature of show()
const _FILETREE_USED: () = ();

#[derive(Debug, Clone, PartialEq)]
pub enum PaletteCommand {
    ToggleTerminal,
    ToggleSidebar,
    GoToLine,
    SaveFile,
    NewFile,
    OpenFolder,
    OpenSettings,
    Find,
    FindReplace,
    RestartLsp,
}

impl PaletteCommand {
    fn all() -> &'static [PaletteCommand] {
        &[
            PaletteCommand::ToggleTerminal,
            PaletteCommand::ToggleSidebar,
            PaletteCommand::GoToLine,
            PaletteCommand::SaveFile,
            PaletteCommand::NewFile,
            PaletteCommand::OpenFolder,
            PaletteCommand::OpenSettings,
            PaletteCommand::Find,
            PaletteCommand::FindReplace,
            PaletteCommand::RestartLsp,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::ToggleTerminal => "Toggle Terminal",
            Self::ToggleSidebar => "Toggle Sidebar",
            Self::GoToLine => "Go to Line…",
            Self::SaveFile => "Save File",
            Self::NewFile => "New File",
            Self::OpenFolder => "Open Folder…",
            Self::OpenSettings => "Open Settings",
            Self::Find => "Find in File",
            Self::FindReplace => "Find & Replace",
            Self::RestartLsp => "Restart LSP Server",
        }
    }

    pub fn shortcut(&self) -> &'static str {
        match self {
            Self::ToggleTerminal => "Ctrl+`",
            Self::ToggleSidebar => "Ctrl+B",
            Self::GoToLine => "Ctrl+G",
            Self::SaveFile => "Ctrl+S",
            Self::NewFile => "Ctrl+N",
            Self::OpenFolder => "Ctrl+O",
            Self::OpenSettings => "Ctrl+,",
            Self::Find => "Ctrl+F",
            Self::FindReplace => "Ctrl+H",
            Self::RestartLsp => "",
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
                if effective_query.is_empty()
                    || self
                        .matcher
                        .fuzzy_match(cmd.label(), &effective_query)
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
                scored.sort_by(|a, b| b.0.cmp(&a.0));
                for (_, p) in scored.into_iter().take(20) {
                    self.entries.push(PaletteEntry::File(p));
                }
            }
            // Commands at the bottom
            for cmd in PaletteCommand::all() {
                if effective_query.is_empty()
                    || self
                        .matcher
                        .fuzzy_match(cmd.label(), &effective_query)
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
                                        egui::RichText::new(label)
                                            .color(egui::Color32::from_rgb(180, 200, 255)),
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
