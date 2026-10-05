use crate::filetree::FileTree;
use crate::lsp::client::{DocumentSymbol, WorkspaceSymbol};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Quiet time after the last keystroke before `workspace/symbol` is sent.
pub const WORKSPACE_SYMBOL_DEBOUNCE: Duration = Duration::from_millis(250);

/// What the palette is searching, picked by the query's first character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteMode {
    /// No prefix: workspace files (plus matching commands).
    Files,
    /// `>`: commands only.
    Commands,
    /// `#`: LSP `workspace/symbol`.
    WorkspaceSymbols,
    /// `@`: symbols of the current file (LSP `documentSymbol`).
    DocumentSymbols,
}

impl PaletteMode {
    /// Split `query` into its mode and the (trimmed) search text.
    pub fn parse(query: &str) -> (Self, &str) {
        let mode = match query.chars().next() {
            Some('>') => Self::Commands,
            Some('#') => Self::WorkspaceSymbols,
            Some('@') => Self::DocumentSymbols,
            _ => return (Self::Files, query),
        };
        (mode, query[1..].trim())
    }
}

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
    GoToWorkspaceSymbol,
    GoToFileSymbol,
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
            PaletteCommand::GoToWorkspaceSymbol,
            PaletteCommand::GoToFileSymbol,
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
            Self::GoToWorkspaceSymbol => "Go to Symbol in Workspace…",
            Self::GoToFileSymbol => "Go to Symbol in File…",
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
            Self::GoToWorkspaceSymbol => "Ctrl+T",
            Self::GoToFileSymbol => "",
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
            Self::GoToWorkspaceSymbol => "# jump navigate type function class project",
            Self::GoToFileSymbol => "@ outline jump navigate function method current",
        }
    }
}

#[derive(Debug, Clone)]
enum PaletteEntry {
    File(PathBuf),
    Command(PaletteCommand),
    WorkspaceSymbol(WorkspaceSymbol),
    DocumentSymbol(DocumentSymbol),
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
    // ── Symbol modes (`#` / `@`) ────────────────────────────────────────────
    /// Latest `workspace/symbol` results (filled by the app).
    workspace_symbols: Vec<WorkspaceSymbol>,
    /// True while `workspace/symbol` requests are in flight.
    pub workspace_symbols_pending: bool,
    /// Drop the current results when the next response for a new query lands.
    replace_workspace_symbols: bool,
    /// Last `#` query handed to the app, and the last edit (text + time).
    ws_query_sent: Option<String>,
    ws_query_edited: Option<(String, Instant)>,
    /// Symbols of the file at `document_symbols_path` (filled by the app).
    pub document_symbols: Vec<DocumentSymbol>,
    pub document_symbols_path: Option<PathBuf>,
    doc_symbols_requested: bool,
    /// `(path, line, col)` picked from a symbol mode; taken by the app.
    pub picked_location: Option<(PathBuf, usize, usize)>,
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
            workspace_symbols: vec![],
            workspace_symbols_pending: false,
            replace_workspace_symbols: false,
            ws_query_sent: None,
            ws_query_edited: None,
            document_symbols: vec![],
            document_symbols_path: None,
            doc_symbols_requested: false,
            picked_location: None,
        }
    }

    fn reset(&mut self, query: &str) {
        self.query = query.to_string();
        self.entries.clear();
        self.selected_idx = 0;
        self.cached_files.clear();
        self.workspace_symbols.clear();
        self.workspace_symbols_pending = false;
        self.ws_query_sent = None;
        self.ws_query_edited = None;
        self.doc_symbols_requested = false;
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.reset("");
        }
    }

    /// Open the palette in commands mode (prefixed with '>').
    pub fn toggle_commands(&mut self) {
        self.open = !self.open;
        if self.open {
            self.reset(">");
        }
    }

    /// Open (or re-target) the palette with `prefix` typed, e.g. `#` for
    /// workspace symbols (Ctrl+T) or `@` for symbols in the current file.
    pub fn open_with(&mut self, prefix: &str) {
        self.open = true;
        self.reset(prefix);
    }

    pub fn mode(&self) -> PaletteMode {
        PaletteMode::parse(&self.query).0
    }

    /// The `#` query to send as `workspace/symbol`, once it has been stable for
    /// [`WORKSPACE_SYMBOL_DEBOUNCE`] and differs from the last one sent.
    pub fn take_workspace_symbol_query(&mut self, now: Instant) -> Option<String> {
        if !self.open || self.mode() != PaletteMode::WorkspaceSymbols {
            return None;
        }
        let (text, at) = self.ws_query_edited.as_ref()?;
        if text.is_empty()
            || self.ws_query_sent.as_deref() == Some(text.as_str())
            || now.saturating_duration_since(*at) < WORKSPACE_SYMBOL_DEBOUNCE
        {
            return None;
        }
        let q = text.clone();
        self.ws_query_sent = Some(q.clone());
        self.workspace_symbols_pending = true;
        self.replace_workspace_symbols = true;
        Some(q)
    }

    /// True while a `#` query is waiting out its debounce (keep repainting).
    pub fn workspace_symbol_debounce_pending(&self) -> bool {
        self.open
            && self.mode() == PaletteMode::WorkspaceSymbols
            && self.ws_query_edited.as_ref().is_some_and(|(t, _)| {
                !t.is_empty() && self.ws_query_sent.as_deref() != Some(t.as_str())
            })
    }

    /// Feed one server's `workspace/symbol` answer. The first answer after a
    /// new query replaces older results; later ones (other servers) append.
    /// `still_pending` says whether more answers are expected.
    pub fn receive_workspace_symbols(
        &mut self,
        symbols: Vec<WorkspaceSymbol>,
        still_pending: bool,
    ) {
        if self.replace_workspace_symbols {
            self.workspace_symbols.clear();
            self.replace_workspace_symbols = false;
        }
        for s in symbols {
            if !self.workspace_symbols.contains(&s) {
                self.workspace_symbols.push(s);
            }
        }
        self.workspace_symbols_pending = still_pending;
    }

    /// True once per opening of `@` mode: the app should fetch document symbols.
    pub fn take_document_symbols_request(&mut self) -> bool {
        if self.open && self.mode() == PaletteMode::DocumentSymbols && !self.doc_symbols_requested {
            self.doc_symbols_requested = true;
            return true;
        }
        false
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

        let (mode, effective_query) = PaletteMode::parse(&self.query);
        let effective_query = effective_query.to_string();
        let commands_only = mode == PaletteMode::Commands;
        if mode == PaletteMode::WorkspaceSymbols
            && self.ws_query_edited.as_ref().map(|(t, _)| t.as_str())
                != Some(effective_query.as_str())
        {
            self.ws_query_edited = Some((effective_query.clone(), Instant::now()));
        }

        // Rebuild entry list whenever query changes (cheap enough each frame).
        self.entries.clear();
        if mode == PaletteMode::WorkspaceSymbols {
            if !effective_query.is_empty() {
                let mut scored: Vec<(i64, &WorkspaceSymbol)> = self
                    .workspace_symbols
                    .iter()
                    .filter_map(|s| {
                        let hay = match &s.container {
                            Some(c) => format!("{} {}", s.name, c),
                            None => s.name.clone(),
                        };
                        Some((self.matcher.fuzzy_match(&hay, &effective_query)?, s))
                    })
                    .collect();
                // Stable: equal scores keep the server's order.
                scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
                self.entries.extend(
                    scored
                        .into_iter()
                        .take(100)
                        .map(|(_, s)| PaletteEntry::WorkspaceSymbol(s.clone())),
                );
            }
        } else if mode == PaletteMode::DocumentSymbols {
            if effective_query.is_empty() {
                self.entries.extend(
                    self.document_symbols
                        .iter()
                        .map(|s| PaletteEntry::DocumentSymbol(s.clone())),
                );
            } else {
                let mut scored: Vec<(i64, &DocumentSymbol)> = self
                    .document_symbols
                    .iter()
                    .filter_map(|s| Some((self.matcher.fuzzy_match(&s.name, &effective_query)?, s)))
                    .collect();
                scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
                self.entries.extend(
                    scored
                        .into_iter()
                        .map(|(_, s)| PaletteEntry::DocumentSymbol(s.clone())),
                );
            }
        } else if commands_only {
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
                    PaletteEntry::WorkspaceSymbol(_) | PaletteEntry::DocumentSymbol(_) => {
                        if let Some(loc) = self.symbol_location(entry) {
                            self.picked_location = Some(loc);
                            close = true;
                        }
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
                    let hint = match mode {
                        PaletteMode::Commands => {
                            "Run command (> to search commands, clear for files)…"
                        }
                        PaletteMode::WorkspaceSymbols => "Search symbols in the workspace…",
                        PaletteMode::DocumentSymbols => "Go to symbol in the current file…",
                        PaletteMode::Files => {
                            "Search files… (> commands, # workspace symbols, @ symbols in file)"
                        }
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
                    if let Some(msg) = self.empty_message(mode, &effective_query) {
                        ui.label(egui::RichText::new(msg).color(egui::Color32::GRAY));
                    }
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
                                PaletteEntry::WorkspaceSymbol(_)
                                | PaletteEntry::DocumentSymbol(_) => {
                                    let label = symbol_label(entry, workspace.as_deref());
                                    let resp = ui.selectable_label(is_selected, label);
                                    if is_selected {
                                        resp.scroll_to_me(None);
                                    }
                                    if resp.clicked() {
                                        if let Some(loc) = self.symbol_location(entry) {
                                            self.picked_location = Some(loc);
                                            close = true;
                                        }
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

    /// Where a symbol entry points: `(path, line, col)`, 0-based.
    fn symbol_location(&self, entry: &PaletteEntry) -> Option<(PathBuf, usize, usize)> {
        match entry {
            PaletteEntry::WorkspaceSymbol(s) => {
                Some((s.path.clone(), s.line as usize, s.col as usize))
            }
            PaletteEntry::DocumentSymbol(s) => self
                .document_symbols_path
                .clone()
                .map(|p| (p, s.line as usize, 0)),
            _ => None,
        }
    }

    /// Placeholder shown above an empty symbol list.
    fn empty_message(&self, mode: PaletteMode, query: &str) -> Option<&'static str> {
        if !self.entries.is_empty() {
            return None;
        }
        match mode {
            PaletteMode::WorkspaceSymbols if query.is_empty() => {
                Some("Type to search symbols (needs a running language server)")
            }
            PaletteMode::WorkspaceSymbols
                if self.workspace_symbols_pending || self.workspace_symbol_debounce_pending() =>
            {
                Some("Searching…")
            }
            PaletteMode::WorkspaceSymbols => Some("No matching symbols"),
            PaletteMode::DocumentSymbols if self.document_symbols_path.is_none() => {
                Some("No file is open")
            }
            PaletteMode::DocumentSymbols => Some("No symbols found in this file"),
            _ => None,
        }
    }
}

fn symbol_icon(kind: &str) -> &'static str {
    match kind {
        "Function" | "Method" | "Constructor" => "ƒ",
        "Class" | "Struct" => "◻",
        "Enum" | "EnumMember" => "⊞",
        "Variable" | "Constant" | "Field" | "Property" => "≡",
        "Interface" => "Ι",
        "Module" | "Namespace" | "Package" => "▤",
        _ => "•",
    }
}

/// Two-line label: `icon name   kind · container` / `  relative/file.rs:line`.
fn symbol_label(entry: &PaletteEntry, workspace: Option<&Path>) -> String {
    match entry {
        PaletteEntry::WorkspaceSymbol(s) => {
            let file = workspace
                .and_then(|ws| s.path.strip_prefix(ws).ok())
                .unwrap_or(&s.path)
                .to_string_lossy()
                .replace('\\', "/");
            let container = s
                .container
                .as_deref()
                .map(|c| format!(" · {c}"))
                .unwrap_or_default();
            format!(
                "{} {}    {}{}\n  {}:{}",
                symbol_icon(&s.kind),
                s.name,
                s.kind,
                container,
                file,
                s.line + 1
            )
        }
        PaletteEntry::DocumentSymbol(s) => format!(
            "{} {}    {}  :{}",
            symbol_icon(&s.kind),
            s.name,
            s.kind,
            s.line + 1
        ),
        _ => String::new(),
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
        assert_eq!(all.len(), 15);
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
                PaletteCommand::GoToWorkspaceSymbol => 13,
                PaletteCommand::GoToFileSymbol => 14,
            }
        }
        let mut seen = [false; 15];
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

    // ---- symbol modes ----

    fn wsym(name: &str, container: Option<&str>, file: &str, line: u32) -> WorkspaceSymbol {
        WorkspaceSymbol {
            name: name.into(),
            kind: "Function".into(),
            container: container.map(Into::into),
            path: PathBuf::from(file),
            line,
            col: 3,
        }
    }

    fn dsym(name: &str, line: u32) -> DocumentSymbol {
        DocumentSymbol {
            name: name.into(),
            kind: "Struct".into(),
            line,
        }
    }

    impl Harness {
        fn ws_symbols(&self) -> Vec<String> {
            self.p
                .entries
                .iter()
                .filter_map(|e| match e {
                    PaletteEntry::WorkspaceSymbol(s) => Some(s.name.clone()),
                    _ => None,
                })
                .collect()
        }
        fn doc_symbols(&self) -> Vec<String> {
            self.p
                .entries
                .iter()
                .filter_map(|e| match e {
                    PaletteEntry::DocumentSymbol(s) => Some(s.name.clone()),
                    _ => None,
                })
                .collect()
        }
    }

    #[test]
    fn mode_is_picked_by_prefix() {
        assert_eq!(PaletteMode::parse("main"), (PaletteMode::Files, "main"));
        assert_eq!(PaletteMode::parse(""), (PaletteMode::Files, ""));
        assert_eq!(PaletteMode::parse("> x "), (PaletteMode::Commands, "x"));
        assert_eq!(
            PaletteMode::parse("# Foo"),
            (PaletteMode::WorkspaceSymbols, "Foo")
        );
        assert_eq!(PaletteMode::parse("@"), (PaletteMode::DocumentSymbols, ""));
    }

    #[test]
    fn open_with_sets_prefix_and_resets_symbol_state() {
        let mut p = CommandPalette::new();
        p.workspace_symbols = vec![wsym("old", None, "a.rs", 0)];
        p.ws_query_sent = Some("old".into());
        p.open_with("#");
        assert!(p.is_open());
        assert_eq!(p.query, "#");
        assert_eq!(p.mode(), PaletteMode::WorkspaceSymbols);
        assert!(p.workspace_symbols.is_empty());
        assert!(p.ws_query_sent.is_none());
        // Already open: re-targets instead of closing.
        p.open_with("@");
        assert!(p.is_open());
        assert_eq!(p.mode(), PaletteMode::DocumentSymbols);
    }

    #[test]
    fn workspace_symbol_query_is_debounced_and_sent_once() {
        let mut h = Harness::new(&["a.rs"]);
        h.p.open_with("#");
        h.query("#");
        let later = Instant::now() + WORKSPACE_SYMBOL_DEBOUNCE * 2;
        assert_eq!(h.p.take_workspace_symbol_query(later), None, "empty query");

        h.query("#Foo");
        assert!(h.p.workspace_symbol_debounce_pending());
        assert_eq!(
            h.p.take_workspace_symbol_query(Instant::now()),
            None,
            "too soon after typing"
        );
        let later = Instant::now() + WORKSPACE_SYMBOL_DEBOUNCE * 2;
        assert_eq!(h.p.take_workspace_symbol_query(later), Some("Foo".into()));
        assert!(h.p.workspace_symbols_pending);
        assert!(!h.p.workspace_symbol_debounce_pending());
        assert_eq!(h.p.take_workspace_symbol_query(later), None, "sent once");

        h.query("#Foob");
        let later = Instant::now() + WORKSPACE_SYMBOL_DEBOUNCE * 2;
        assert_eq!(h.p.take_workspace_symbol_query(later), Some("Foob".into()));

        // Other modes never ask.
        h.query("Foo");
        assert_eq!(h.p.take_workspace_symbol_query(later), None);
    }

    #[test]
    fn workspace_symbol_answers_replace_then_append() {
        let mut p = CommandPalette::new();
        p.open_with("#");
        p.workspace_symbols = vec![wsym("stale", None, "a.rs", 0)];
        p.replace_workspace_symbols = true;
        p.workspace_symbols_pending = true;
        p.receive_workspace_symbols(vec![wsym("a", None, "a.rs", 1)], true);
        assert_eq!(p.workspace_symbols.len(), 1);
        assert!(p.workspace_symbols_pending);
        p.receive_workspace_symbols(
            vec![wsym("a", None, "a.rs", 1), wsym("b", None, "b.rs", 2)],
            false,
        );
        let names: Vec<&str> = p
            .workspace_symbols
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["a", "b"],
            "second server appends, duplicates dropped"
        );
        assert!(!p.workspace_symbols_pending);
    }

    #[test]
    fn workspace_symbols_are_listed_and_enter_navigates() {
        let mut h = Harness::new(&["a.rs"]);
        h.p.open_with("#");
        h.p.workspace_symbols = vec![
            wsym("parse_args", Some("cli"), "src/cli.rs", 9),
            wsym("Parser", Some("syntax"), "src/syntax.rs", 2),
            wsym("unrelated", None, "src/x.rs", 0),
        ];
        h.query("#");
        assert!(h.ws_symbols().is_empty(), "empty query lists nothing");
        h.query("#pars");
        let listed = h.ws_symbols();
        assert_eq!(listed.len(), 2, "{listed:?}");
        assert!(!listed.contains(&"unrelated".to_string()));
        assert!(h.files().is_empty() && h.commands().is_empty());
        // Container names are searchable too.
        h.query("#syntax");
        assert_eq!(h.ws_symbols(), ["Parser"]);
        let (file, cmd) = h.key(Key::Enter);
        assert_eq!((file, cmd), (None, None));
        assert_eq!(
            h.p.picked_location,
            Some((PathBuf::from("src/syntax.rs"), 2, 3))
        );
        assert!(!h.p.is_open());
    }

    #[test]
    fn document_symbols_mode_filters_and_navigates() {
        let mut h = Harness::new(&["a.rs"]);
        h.p.open_with("@");
        assert!(h.p.take_document_symbols_request());
        assert!(!h.p.take_document_symbols_request(), "once per opening");
        h.p.document_symbols = vec![dsym("Alpha", 1), dsym("beta", 5), dsym("Gamma", 9)];
        h.query("@");
        assert_eq!(h.doc_symbols(), ["Alpha", "beta", "Gamma"], "file order");
        h.query("@gam");
        assert_eq!(h.doc_symbols(), ["Gamma"]);

        // No file known → Enter does nothing.
        assert_eq!(h.key(Key::Enter), (None, None));
        assert!(h.p.is_open());
        assert_eq!(h.p.picked_location, None);

        h.p.document_symbols_path = Some(PathBuf::from("cur.rs"));
        h.query("@gam");
        h.key(Key::Enter);
        assert_eq!(h.p.picked_location, Some((PathBuf::from("cur.rs"), 9, 0)));
        assert!(!h.p.is_open());

        h.p.open_with("@");
        assert!(
            h.p.take_document_symbols_request(),
            "re-requested on reopen"
        );
    }

    #[test]
    fn symbol_labels_show_kind_container_and_relative_file() {
        let ws = PathBuf::from("ws");
        let e = PaletteEntry::WorkspaceSymbol(WorkspaceSymbol {
            path: ws.join("src").join("a.rs"),
            ..wsym("run", Some("app"), "", 4)
        });
        assert_eq!(
            symbol_label(&e, Some(&ws)),
            "ƒ run    Function · app\n  src/a.rs:5"
        );
        let e = PaletteEntry::DocumentSymbol(dsym("S", 0));
        assert_eq!(symbol_label(&e, None), "◻ S    Struct  :1");
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
