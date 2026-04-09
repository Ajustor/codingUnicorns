use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
}

fn default_terminal_height() -> f32 {
    200.0
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
            },
            font: FontConfig {
                size: 14.0,
                family: "monospace".to_string(),
            },
            keybindings: KeyBindings::default(),
            last_workspace: None,
            last_file: None,
            terminal_height: default_terminal_height(),
        }
    }
}

impl Config {
    pub fn config_path() -> PathBuf {
        let mut path = dirs_next::config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("writing-unicorns");
        path.push("config.toml");
        path
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(content) = std::fs::read_to_string(&path) {
            toml::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(content) = toml::to_string_pretty(self) {
            let _ = std::fs::write(path, content);
        }
    }
}
