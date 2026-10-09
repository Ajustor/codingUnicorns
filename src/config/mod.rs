use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod session;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyBinding {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyBinding {
    pub fn new(key: &str, ctrl: bool, shift: bool, alt: bool) -> Self {
        Self {
            key: key.to_string(),
            ctrl,
            shift,
            alt,
        }
    }

    pub fn matches(&self, i: &egui::InputState) -> bool {
        if let Some(k) = self.parse_key() {
            i.key_pressed(k)
                && i.modifiers.ctrl == self.ctrl
                && i.modifiers.shift == self.shift
                && i.modifiers.alt == self.alt
        } else {
            false
        }
    }

    pub fn parse_key(&self) -> Option<egui::Key> {
        match self.key.as_str() {
            "A" => Some(egui::Key::A),
            "B" => Some(egui::Key::B),
            "C" => Some(egui::Key::C),
            "D" => Some(egui::Key::D),
            "E" => Some(egui::Key::E),
            "F" => Some(egui::Key::F),
            "G" => Some(egui::Key::G),
            "H" => Some(egui::Key::H),
            "I" => Some(egui::Key::I),
            "J" => Some(egui::Key::J),
            "K" => Some(egui::Key::K),
            "L" => Some(egui::Key::L),
            "M" => Some(egui::Key::M),
            "N" => Some(egui::Key::N),
            "O" => Some(egui::Key::O),
            "P" => Some(egui::Key::P),
            "Q" => Some(egui::Key::Q),
            "R" => Some(egui::Key::R),
            "S" => Some(egui::Key::S),
            "T" => Some(egui::Key::T),
            "U" => Some(egui::Key::U),
            "V" => Some(egui::Key::V),
            "W" => Some(egui::Key::W),
            "X" => Some(egui::Key::X),
            "Y" => Some(egui::Key::Y),
            "Z" => Some(egui::Key::Z),
            "Backtick" => Some(egui::Key::Backtick),
            "Comma" => Some(egui::Key::Comma),
            "F1" => Some(egui::Key::F1),
            "F2" => Some(egui::Key::F2),
            "F3" => Some(egui::Key::F3),
            "F4" => Some(egui::Key::F4),
            "F5" => Some(egui::Key::F5),
            "F6" => Some(egui::Key::F6),
            "F7" => Some(egui::Key::F7),
            "F8" => Some(egui::Key::F8),
            "F9" => Some(egui::Key::F9),
            "F10" => Some(egui::Key::F10),
            "F11" => Some(egui::Key::F11),
            "F12" => Some(egui::Key::F12),
            "Enter" => Some(egui::Key::Enter),
            "Escape" => Some(egui::Key::Escape),
            "Tab" => Some(egui::Key::Tab),
            "Space" => Some(egui::Key::Space),
            "Delete" => Some(egui::Key::Delete),
            "Backspace" => Some(egui::Key::Backspace),
            "ArrowUp" => Some(egui::Key::ArrowUp),
            "ArrowDown" => Some(egui::Key::ArrowDown),
            "ArrowLeft" => Some(egui::Key::ArrowLeft),
            "ArrowRight" => Some(egui::Key::ArrowRight),
            "Home" => Some(egui::Key::Home),
            "End" => Some(egui::Key::End),
            "PageUp" => Some(egui::Key::PageUp),
            "PageDown" => Some(egui::Key::PageDown),
            "OpenBracket" | "[" => Some(egui::Key::OpenBracket),
            "CloseBracket" | "]" => Some(egui::Key::CloseBracket),
            "Slash" | "/" => Some(egui::Key::Slash),
            "Backslash" | "\\" => Some(egui::Key::Backslash),
            "Period" | "." => Some(egui::Key::Period),
            _ => None,
        }
    }

    pub fn display(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push("Alt");
        }
        parts.push(&self.key);
        parts.join("+")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyBindings {
    pub new_file: KeyBinding,
    pub open_folder: KeyBinding,
    pub open_file: KeyBinding,
    pub save: KeyBinding,
    pub command_palette: KeyBinding,
    pub toggle_sidebar: KeyBinding,
    pub toggle_terminal: KeyBinding,
    pub shortcuts_help: KeyBinding,
    pub settings: KeyBinding,
    pub find: KeyBinding,
    pub close_tab: KeyBinding,
    pub go_to_line: KeyBinding,
    pub indent: KeyBinding,
    pub unindent: KeyBinding,
    // Editor operations
    pub select_next_occurrence: KeyBinding,
    pub select_all_occurrences: KeyBinding,
    pub toggle_comment: KeyBinding,
    pub delete_line: KeyBinding,
    pub duplicate_line: KeyBinding,
    pub insert_line_below: KeyBinding,
    pub insert_line_above: KeyBinding,
    pub move_line_up: KeyBinding,
    pub move_line_down: KeyBinding,
    pub add_cursor_above: KeyBinding,
    pub add_cursor_below: KeyBinding,
    pub trigger_completion: KeyBinding,
    pub find_replace: KeyBinding,
    pub undo: KeyBinding,
    pub redo: KeyBinding,
    pub select_all: KeyBinding,
    // Navigation
    pub goto_definition: KeyBinding,
    pub navigate_back: KeyBinding,
    pub navigate_forward: KeyBinding,
    pub toggle_split: KeyBinding,
    // Code actions
    pub find_references: KeyBinding,
    pub rename_symbol: KeyBinding,
    pub code_actions: KeyBinding,
    pub toggle_blame: KeyBinding,
    /// Opens the workspace search (Ctrl+Shift+F). The name is historical and
    /// kept so saved configs still load; formatting is Shift+Alt+F.
    pub format_document: KeyBinding,
    // Debug
    pub debug_start: KeyBinding,
    pub debug_toggle_breakpoint: KeyBinding,
    pub debug_step_over: KeyBinding,
    pub debug_step_into: KeyBinding,
    pub debug_step_out: KeyBinding,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            new_file: KeyBinding::new("N", true, false, false),
            open_folder: KeyBinding::new("O", true, false, false),
            open_file: KeyBinding::new("O", true, true, false),
            save: KeyBinding::new("S", true, false, false),
            command_palette: KeyBinding::new("P", true, false, false),
            toggle_sidebar: KeyBinding::new("B", true, false, false),
            toggle_terminal: KeyBinding::new("Backtick", true, false, false),
            shortcuts_help: KeyBinding::new("F1", false, false, false),
            settings: KeyBinding::new("Comma", true, false, false),
            find: KeyBinding::new("F", true, false, false),
            close_tab: KeyBinding::new("W", true, false, false),
            go_to_line: KeyBinding::new("G", true, false, false),
            indent: KeyBinding::new("CloseBracket", true, false, false),
            unindent: KeyBinding::new("OpenBracket", true, false, false),
            select_next_occurrence: KeyBinding::new("D", true, false, false),
            select_all_occurrences: KeyBinding::new("L", true, true, false),
            toggle_comment: KeyBinding::new("Slash", true, false, false),
            delete_line: KeyBinding::new("K", true, true, false),
            duplicate_line: KeyBinding::new("D", true, true, false),
            insert_line_below: KeyBinding::new("Enter", true, false, false),
            insert_line_above: KeyBinding::new("Enter", true, true, false),
            move_line_up: KeyBinding::new("ArrowUp", false, false, true),
            move_line_down: KeyBinding::new("ArrowDown", false, false, true),
            add_cursor_above: KeyBinding::new("ArrowUp", true, false, true),
            add_cursor_below: KeyBinding::new("ArrowDown", true, false, true),
            trigger_completion: KeyBinding::new("Space", true, false, false),
            find_replace: KeyBinding::new("H", true, false, false),
            undo: KeyBinding::new("Z", true, false, false),
            redo: KeyBinding::new("Z", true, true, false),
            select_all: KeyBinding::new("A", true, false, false),
            goto_definition: KeyBinding::new("F12", false, false, false),
            navigate_back: KeyBinding::new("ArrowLeft", false, false, true),
            navigate_forward: KeyBinding::new("ArrowRight", false, false, true),
            toggle_split: KeyBinding::new("Backslash", true, false, false),
            find_references: KeyBinding::new("F12", false, true, false),
            rename_symbol: KeyBinding::new("F2", false, false, false),
            code_actions: KeyBinding::new("Period", true, false, false),
            toggle_blame: KeyBinding::new("B", true, false, true),
            format_document: KeyBinding::new("F", true, true, false),
            debug_start: KeyBinding::new("F5", false, false, false),
            debug_toggle_breakpoint: KeyBinding::new("F9", false, false, false),
            debug_step_over: KeyBinding::new("F10", false, false, false),
            debug_step_into: KeyBinding::new("F11", false, false, false),
            debug_step_out: KeyBinding::new("F11", false, true, false),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub theme: Theme,
    pub editor: EditorConfig,
    pub font: FontConfig,
    #[serde(default)]
    pub keybindings: KeyBindings,
    #[serde(default)]
    pub last_workspace: Option<String>,
    #[serde(default)]
    pub last_file: Option<String>,
    #[serde(default = "default_terminal_height")]
    pub terminal_height: f32,
    /// Custom shell command (e.g. "pwsh.exe", "cmd.exe", "/bin/zsh").
    /// When empty, auto-detects the best available shell.
    #[serde(default)]
    pub shell: String,
    /// Command used to launch Claude Code (default "claude").
    #[serde(default = "default_claude_binary")]
    pub claude_binary: String,
    /// Auto-approve read-only tools (Read/Glob/Grep) without a prompt.
    #[serde(default = "default_true")]
    pub claude_auto_allow_read: bool,
    /// Check GitHub for a newer release on startup.
    #[serde(default = "default_true")]
    pub check_updates: bool,
    /// Release the user chose to skip; startup checks stay quiet about it.
    #[serde(default)]
    pub skipped_update_version: Option<String>,
    /// Recently opened workspace folders, most recent first (deduplicated,
    /// at most [`MAX_RECENT_WORKSPACES`]).
    #[serde(default)]
    pub recent_workspaces: Vec<String>,
    /// Labels of the palette commands last run, most recent first (at most
    /// [`MAX_RECENT_COMMANDS`]).
    #[serde(default)]
    pub recent_commands: Vec<String>,
    #[serde(default)]
    pub extensions: ExtensionsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionsConfig {
    /// URL of the remote module registry index (`registry.json`).
    /// Empty disables the "Browse registry" section.
    #[serde(
        default = "default_registry_url",
        deserialize_with = "deserialize_registry_url"
    )]
    pub registry_url: String,
}

impl Default for ExtensionsConfig {
    fn default() -> Self {
        Self {
            registry_url: default_registry_url(),
        }
    }
}

fn default_registry_url() -> String {
    crate::extension::remote_registry::DEFAULT_REGISTRY_URL.to_string()
}

/// Maps the pre-rename default URL to the current one; any other value is kept.
fn deserialize_registry_url<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    use crate::extension::remote_registry::{DEFAULT_REGISTRY_URL, LEGACY_REGISTRY_URL};
    let url = String::deserialize(d)?;
    Ok(if url.trim_end_matches('/') == LEGACY_REGISTRY_URL {
        DEFAULT_REGISTRY_URL.to_string()
    } else {
        url
    })
}

/// Maximum number of entries kept in [`Config::recent_workspaces`].
pub const MAX_RECENT_WORKSPACES: usize = 10;

/// Maximum number of entries kept in [`Config::recent_commands`].
pub const MAX_RECENT_COMMANDS: usize = 5;

/// Whether two workspace path strings designate the same folder. Ignores
/// trailing separators and, on Windows, case and `/` vs `\`.
pub fn same_workspace(a: &str, b: &str) -> bool {
    fn norm(s: &str) -> String {
        let s = s.trim_end_matches(['/', '\\']);
        if cfg!(windows) {
            s.replace('/', "\\").to_lowercase()
        } else {
            s.to_string()
        }
    }
    norm(a) == norm(b)
}

fn default_terminal_height() -> f32 {
    200.0
}

fn default_claude_binary() -> String {
    "claude".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub background: [u8; 3],
    pub foreground: [u8; 3],
    pub accent: [u8; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorConfig {
    pub tab_size: usize,
    pub insert_spaces: bool,
    pub word_wrap: bool,
    pub line_numbers: bool,
    pub auto_save: bool,
    #[serde(default = "default_true")]
    pub auto_close_brackets: bool,
    #[serde(default)]
    pub show_gitignored: bool,
    #[serde(default = "default_true")]
    pub show_minimap: bool,
    #[serde(default = "default_true")]
    pub highlight_current_line: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontConfig {
    pub size: f32,
    pub family: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme {
                name: "dark".to_string(),
                background: [30, 30, 30],
                foreground: [212, 212, 212],
                accent: [0, 122, 204],
            },
            editor: EditorConfig {
                tab_size: 4,
                insert_spaces: true,
                word_wrap: false,
                line_numbers: true,
                auto_save: false,
                auto_close_brackets: true,
                show_gitignored: false,
                show_minimap: true,
                highlight_current_line: true,
            },
            font: FontConfig {
                size: 14.0,
                family: "monospace".to_string(),
            },
            keybindings: KeyBindings::default(),
            last_workspace: None,
            last_file: None,
            terminal_height: default_terminal_height(),
            shell: String::new(),
            claude_binary: default_claude_binary(),
            claude_auto_allow_read: true,
            check_updates: true,
            skipped_update_version: None,
            recent_workspaces: Vec::new(),
            recent_commands: Vec::new(),
            extensions: ExtensionsConfig::default(),
        }
    }
}

impl Config {
    pub fn config_path() -> PathBuf {
        let mut path = dirs_next::config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("coding-unicorns");
        path.push("config.toml");
        path
    }

    pub fn load() -> Self {
        Self::load_from(&Self::config_path())
    }

    fn load_from(path: &std::path::Path) -> Self {
        if let Ok(content) = std::fs::read_to_string(path) {
            toml::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self) {
        self.save_to(&Self::config_path());
    }

    fn save_to(&self, path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(content) = toml::to_string_pretty(self) {
            let _ = std::fs::write(path, content);
        }
    }

    /// Move `path` to the front of the recent-workspaces list (deduplicated,
    /// capped at [`MAX_RECENT_WORKSPACES`]).
    pub fn push_recent_workspace(&mut self, path: &str) {
        self.recent_workspaces.retain(|p| !same_workspace(p, path));
        self.recent_workspaces.insert(0, path.to_string());
        self.recent_workspaces.truncate(MAX_RECENT_WORKSPACES);
    }

    /// Move the palette command `label` to the front of the recent commands
    /// (deduplicated, capped at [`MAX_RECENT_COMMANDS`]).
    pub fn push_recent_command(&mut self, label: &str) {
        self.recent_commands.retain(|l| l != label);
        self.recent_commands.insert(0, label.to_string());
        self.recent_commands.truncate(MAX_RECENT_COMMANDS);
    }

    /// Drop recent workspaces whose folder no longer exists. Returns true if
    /// anything was removed (so the caller knows to persist the change).
    pub fn prune_recent_workspaces(&mut self) -> bool {
        let before = self.recent_workspaces.len();
        self.recent_workspaces
            .retain(|p| std::path::Path::new(p).is_dir());
        self.recent_workspaces.len() != before
    }

    /// Remove one entry from the recent-workspaces list.
    pub fn remove_recent_workspace(&mut self, path: &str) {
        self.recent_workspaces.retain(|p| !same_workspace(p, path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_KEYS: &[(&str, egui::Key)] = &[
        ("A", egui::Key::A),
        ("B", egui::Key::B),
        ("C", egui::Key::C),
        ("D", egui::Key::D),
        ("E", egui::Key::E),
        ("F", egui::Key::F),
        ("G", egui::Key::G),
        ("H", egui::Key::H),
        ("I", egui::Key::I),
        ("J", egui::Key::J),
        ("K", egui::Key::K),
        ("L", egui::Key::L),
        ("M", egui::Key::M),
        ("N", egui::Key::N),
        ("O", egui::Key::O),
        ("P", egui::Key::P),
        ("Q", egui::Key::Q),
        ("R", egui::Key::R),
        ("S", egui::Key::S),
        ("T", egui::Key::T),
        ("U", egui::Key::U),
        ("V", egui::Key::V),
        ("W", egui::Key::W),
        ("X", egui::Key::X),
        ("Y", egui::Key::Y),
        ("Z", egui::Key::Z),
        ("Backtick", egui::Key::Backtick),
        ("Comma", egui::Key::Comma),
        ("F1", egui::Key::F1),
        ("F2", egui::Key::F2),
        ("F3", egui::Key::F3),
        ("F4", egui::Key::F4),
        ("F5", egui::Key::F5),
        ("F6", egui::Key::F6),
        ("F7", egui::Key::F7),
        ("F8", egui::Key::F8),
        ("F9", egui::Key::F9),
        ("F10", egui::Key::F10),
        ("F11", egui::Key::F11),
        ("F12", egui::Key::F12),
        ("Enter", egui::Key::Enter),
        ("Escape", egui::Key::Escape),
        ("Tab", egui::Key::Tab),
        ("Space", egui::Key::Space),
        ("Delete", egui::Key::Delete),
        ("Backspace", egui::Key::Backspace),
        ("ArrowUp", egui::Key::ArrowUp),
        ("ArrowDown", egui::Key::ArrowDown),
        ("ArrowLeft", egui::Key::ArrowLeft),
        ("ArrowRight", egui::Key::ArrowRight),
        ("Home", egui::Key::Home),
        ("End", egui::Key::End),
        ("PageUp", egui::Key::PageUp),
        ("PageDown", egui::Key::PageDown),
        ("OpenBracket", egui::Key::OpenBracket),
        ("[", egui::Key::OpenBracket),
        ("CloseBracket", egui::Key::CloseBracket),
        ("]", egui::Key::CloseBracket),
        ("Slash", egui::Key::Slash),
        ("/", egui::Key::Slash),
        ("Backslash", egui::Key::Backslash),
        ("\\", egui::Key::Backslash),
        ("Period", egui::Key::Period),
        (".", egui::Key::Period),
    ];

    fn kb(key: &str) -> KeyBinding {
        KeyBinding::new(key, false, false, false)
    }

    /// Run a headless egui frame with `key` pressed and `mods` held, then
    /// evaluate `binding.matches` against its input state.
    fn matches_with(binding: &KeyBinding, key: Option<egui::Key>, mods: egui::Modifiers) -> bool {
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput {
            modifiers: mods,
            ..Default::default()
        };
        if let Some(key) = key {
            raw.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: mods,
            });
        }
        let mut out = false;
        let _ = ctx.run(raw, |ctx| out = ctx.input(|i| binding.matches(i)));
        out
    }

    /// Every `KeyBinding` field of `KeyBindings`, discovered via serde so new
    /// fields are covered automatically.
    fn all_bindings(kbs: &KeyBindings) -> Vec<(String, KeyBinding)> {
        let value = toml::Value::try_from(kbs).unwrap();
        value
            .as_table()
            .unwrap()
            .iter()
            .map(|(name, v)| (name.clone(), v.clone().try_into().unwrap()))
            .collect()
    }

    #[test]
    fn parse_key_knows_every_supported_name() {
        for (name, key) in ALL_KEYS {
            assert_eq!(kb(name).parse_key(), Some(*key), "key name {name:?}");
        }
    }

    #[test]
    fn parse_key_rejects_unknown_and_is_case_sensitive() {
        assert_eq!(kb("").parse_key(), None);
        assert_eq!(kb("a").parse_key(), None);
        assert_eq!(kb("F13").parse_key(), None);
        assert_eq!(kb("Ctrl+S").parse_key(), None);
    }

    #[test]
    fn display_joins_modifiers_in_order() {
        assert_eq!(kb("F5").display(), "F5");
        assert_eq!(KeyBinding::new("S", true, false, false).display(), "Ctrl+S");
        assert_eq!(
            KeyBinding::new("Z", true, true, false).display(),
            "Ctrl+Shift+Z"
        );
        assert_eq!(
            KeyBinding::new("ArrowUp", true, true, true).display(),
            "Ctrl+Shift+Alt+ArrowUp"
        );
        assert_eq!(KeyBinding::new("B", false, false, true).display(), "Alt+B");
    }

    #[test]
    fn matches_requires_key_and_exact_modifiers() {
        use egui::Modifiers as M;
        let save = KeyBinding::new("S", true, false, false);
        assert!(matches_with(&save, Some(egui::Key::S), M::CTRL));
        assert!(!matches_with(&save, Some(egui::Key::S), M::NONE));
        assert!(!matches_with(&save, Some(egui::Key::S), M::CTRL | M::SHIFT));
        assert!(!matches_with(&save, Some(egui::Key::S), M::CTRL | M::ALT));
        assert!(!matches_with(&save, Some(egui::Key::A), M::CTRL));
        assert!(!matches_with(&save, None, M::CTRL));

        let redo = KeyBinding::new("Z", true, true, false);
        assert!(matches_with(&redo, Some(egui::Key::Z), M::CTRL | M::SHIFT));
        assert!(!matches_with(&redo, Some(egui::Key::Z), M::CTRL));

        let back = KeyBinding::new("ArrowLeft", false, false, true);
        assert!(matches_with(&back, Some(egui::Key::ArrowLeft), M::ALT));
    }

    #[test]
    fn matches_is_false_for_unparseable_key() {
        let bad = KeyBinding::new("Nope", false, false, false);
        assert!(!matches_with(
            &bad,
            Some(egui::Key::N),
            egui::Modifiers::NONE
        ));
    }

    #[test]
    fn every_default_keybinding_uses_a_parseable_key() {
        let all = all_bindings(&KeyBindings::default());
        assert!(all.len() >= 40, "expected all bindings, got {}", all.len());
        for (name, b) in all {
            assert!(
                b.parse_key().is_some(),
                "default binding {name} uses unknown key {:?}",
                b.key
            );
        }
    }

    #[test]
    fn default_keybindings_spot_check() {
        let k = KeyBindings::default();
        assert_eq!(k.save.display(), "Ctrl+S");
        assert_eq!(k.open_file.display(), "Ctrl+Shift+O");
        assert_eq!(k.toggle_terminal.display(), "Ctrl+Backtick");
        assert_eq!(k.redo.display(), "Ctrl+Shift+Z");
        assert_eq!(k.move_line_up.display(), "Alt+ArrowUp");
        assert_eq!(k.goto_definition.display(), "F12");
        assert_eq!(k.debug_step_out.display(), "Shift+F11");
    }

    #[test]
    fn default_config_values() {
        let c = Config::default();
        assert_eq!(c.theme.name, "dark");
        assert_eq!(c.theme.background, [30, 30, 30]);
        assert_eq!(c.editor.tab_size, 4);
        assert!(c.editor.insert_spaces);
        assert!(!c.editor.word_wrap);
        assert!(c.editor.line_numbers);
        assert!(!c.editor.auto_save);
        assert!(c.editor.auto_close_brackets);
        assert!(!c.editor.show_gitignored);
        assert!(c.editor.show_minimap);
        assert!(c.editor.highlight_current_line);
        assert_eq!(c.font.size, 14.0);
        assert_eq!(c.font.family, "monospace");
        assert_eq!(c.last_workspace, None);
        assert_eq!(c.last_file, None);
        assert_eq!(c.terminal_height, 200.0);
        assert_eq!(c.shell, "");
        assert_eq!(c.claude_binary, "claude");
        assert!(c.claude_auto_allow_read);
        assert!(c.check_updates);
        assert_eq!(c.skipped_update_version, None);
    }

    #[test]
    fn toml_round_trip_preserves_all_fields() {
        let mut c = Config::default();
        c.theme.name = "light".into();
        c.theme.accent = [1, 2, 3];
        c.editor.tab_size = 2;
        c.editor.word_wrap = true;
        c.editor.show_minimap = false;
        c.editor.show_gitignored = true;
        c.font.size = 18.5;
        c.font.family = "Fira Code".into();
        c.keybindings.save = KeyBinding::new("F2", false, true, true);
        c.last_workspace = Some("C:\\work\\proj".into());
        c.last_file = Some("/tmp/x.rs".into());
        c.terminal_height = 321.0;
        c.shell = "pwsh.exe".into();
        c.claude_binary = "claude-dev".into();
        c.claude_auto_allow_read = false;
        c.check_updates = false;
        c.skipped_update_version = Some("1.2.3".into());

        let s = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(toml::to_string_pretty(&back).unwrap(), s);
        assert_eq!(back.theme.name, "light");
        assert_eq!(back.theme.accent, [1, 2, 3]);
        assert_eq!(back.editor.tab_size, 2);
        assert!(back.editor.word_wrap);
        assert!(!back.editor.show_minimap);
        assert_eq!(back.font.family, "Fira Code");
        assert_eq!(back.keybindings.save.display(), "Shift+Alt+F2");
        assert_eq!(back.last_workspace.as_deref(), Some("C:\\work\\proj"));
        assert_eq!(back.terminal_height, 321.0);
        assert_eq!(back.shell, "pwsh.exe");
        assert_eq!(back.claude_binary, "claude-dev");
        assert!(!back.claude_auto_allow_read);
        assert!(!back.check_updates);
        assert_eq!(back.skipped_update_version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn old_config_missing_new_fields_gets_defaults() {
        // A config written by an early version: no keybindings, no
        // terminal/claude/update settings, and an older [editor] section.
        let old = r#"
            [theme]
            name = "custom"
            background = [1, 1, 1]
            foreground = [2, 2, 2]
            accent = [3, 3, 3]

            [editor]
            tab_size = 8
            insert_spaces = false
            word_wrap = true
            line_numbers = false
            auto_save = true

            [font]
            size = 12.0
            family = "Consolas"
        "#;
        let c: Config = toml::from_str(old).unwrap();
        assert_eq!(
            c.extensions.registry_url,
            crate::extension::remote_registry::DEFAULT_REGISTRY_URL
        );
        // Explicit values are kept.
        assert_eq!(c.theme.name, "custom");
        assert_eq!(c.editor.tab_size, 8);
        assert!(!c.editor.insert_spaces);
        assert!(c.editor.auto_save);
        assert_eq!(c.font.family, "Consolas");
        // Missing fields take their documented defaults (not bool::default()).
        assert!(c.editor.auto_close_brackets);
        assert!(!c.editor.show_gitignored);
        assert!(c.editor.show_minimap);
        assert!(c.editor.highlight_current_line);
        assert_eq!(c.terminal_height, 200.0);
        assert_eq!(c.shell, "");
        assert_eq!(c.claude_binary, "claude");
        assert!(c.claude_auto_allow_read);
        assert!(c.check_updates);
        assert_eq!(c.last_workspace, None);
        assert_eq!(c.skipped_update_version, None);
        assert_eq!(c.keybindings.save.display(), "Ctrl+S");
        assert_eq!(c.keybindings.debug_step_out.display(), "Shift+F11");
    }

    #[test]
    fn config_missing_required_section_fails_to_parse() {
        assert!(toml::from_str::<Config>("[theme]\nname = \"x\"").is_err());
    }

    #[test]
    fn save_to_creates_parent_dirs_and_load_from_reads_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deeper").join("config.toml");
        let mut c = Config::default();
        c.font.size = 22.0;
        c.last_file = Some("main.rs".into());
        c.save_to(&path);
        assert!(path.exists());
        let back = Config::load_from(&path);
        assert_eq!(back.font.size, 22.0);
        assert_eq!(back.last_file.as_deref(), Some("main.rs"));
    }

    #[test]
    fn load_from_missing_or_corrupt_file_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let missing = Config::load_from(&dir.path().join("nope.toml"));
        assert_eq!(missing.theme.name, "dark");

        let corrupt = dir.path().join("bad.toml");
        std::fs::write(&corrupt, "this is = = not toml [").unwrap();
        let c = Config::load_from(&corrupt);
        assert_eq!(c.font.size, 14.0);
        assert_eq!(c.claude_binary, "claude");
    }

    #[test]
    fn push_recent_command_is_mru_deduplicated_and_capped() {
        let mut c = Config::default();
        for l in ["a", "b", "a", "c", "d", "e", "f"] {
            c.push_recent_command(l);
        }
        assert_eq!(c.recent_commands, ["f", "e", "d", "c", "a"]);
    }

    #[test]
    fn push_recent_workspace_is_mru_deduplicated_and_capped() {
        let mut c = Config::default();
        c.push_recent_workspace("/a");
        c.push_recent_workspace("/b");
        c.push_recent_workspace("/a/"); // same folder, trailing separator
        assert_eq!(c.recent_workspaces, vec!["/a/", "/b"]);
        for i in 0..20 {
            c.push_recent_workspace(&format!("/w{i}"));
        }
        assert_eq!(c.recent_workspaces.len(), MAX_RECENT_WORKSPACES);
        assert_eq!(c.recent_workspaces[0], "/w19");
        assert_eq!(c.recent_workspaces[9], "/w10");
    }

    #[test]
    fn same_workspace_ignores_trailing_separators() {
        assert!(same_workspace("/a/b", "/a/b/"));
        assert!(!same_workspace("/a/b", "/a/c"));
        if cfg!(windows) {
            assert!(same_workspace("C:\\Work\\Proj", "c:/work/proj"));
        }
    }

    #[test]
    fn prune_and_remove_recent_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().to_string_lossy().to_string();
        let missing = dir.path().join("gone").to_string_lossy().to_string();
        let mut c = Config::default();
        c.push_recent_workspace(&missing);
        c.push_recent_workspace(&existing);
        assert!(c.prune_recent_workspaces());
        assert_eq!(c.recent_workspaces, vec![existing.clone()]);
        assert!(!c.prune_recent_workspaces());
        c.remove_recent_workspace(&existing);
        assert!(c.recent_workspaces.is_empty());
    }

    #[test]
    fn recent_workspaces_round_trip_and_default_empty() {
        let mut c = Config::default();
        c.push_recent_workspace("C:\\x\\y");
        let s = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.recent_workspaces, vec!["C:\\x\\y"]);
        assert!(Config::default().recent_workspaces.is_empty());
    }

    #[test]
    fn legacy_registry_url_is_migrated_after_the_repo_rename() {
        use crate::extension::remote_registry::{DEFAULT_REGISTRY_URL, LEGACY_REGISTRY_URL};
        let saved = toml::to_string_pretty(&Config::default()).unwrap();
        let with = |url: &str| -> Config {
            toml::from_str(&saved.replace(DEFAULT_REGISTRY_URL, url)).unwrap()
        };
        assert_eq!(
            with(LEGACY_REGISTRY_URL).extensions.registry_url,
            DEFAULT_REGISTRY_URL
        );
        let mirror = "https://mirror.example/r.json";
        assert_eq!(with(mirror).extensions.registry_url, mirror);
    }

    #[test]
    fn registry_url_round_trips_and_empty_disables() {
        let mut c = Config::default();
        assert!(c.extensions.registry_url.starts_with("https://"));
        c.extensions.registry_url = "http://mirror.invalid/registry.json".into();
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(
            back.extensions.registry_url,
            "http://mirror.invalid/registry.json"
        );
        // An explicitly cleared URL stays empty (registry disabled).
        c.extensions.registry_url.clear();
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert!(back.extensions.registry_url.is_empty());
    }

    #[test]
    fn config_path_ends_with_app_specific_file() {
        let p = Config::config_path();
        assert!(p.ends_with(std::path::Path::new("coding-unicorns").join("config.toml")));
    }
}
