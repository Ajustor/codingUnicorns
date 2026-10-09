# Coding Unicorns — Plugin System

Coding Unicorns supports a lightweight plugin system inspired by VSCode extensions. This document covers **built-in plugins**: Rust types compiled into the editor that implement the `Plugin` trait and run in-process with the editor loop.

> **Language extensions are different.** Syntax highlighting, hover and LSP for a language come from *extensions*: native modules (`cdylib`) described by a `manifest.toml`, installed from the online registry, git, a ZIP or a local folder, and loaded at runtime through `libloading`. Each one is wrapped in an `FfiLangPlugin` (`src/extension/ffi_plugin.rs`) that implements this same trait. Modules can also contribute sidebar panels and pages, described as JSON views and rendered by `src/extension/ui_host.rs`. Their manifest format, exported C functions, view format and install methods are documented in the README, section *Extensions de langage*; the [official modules](https://github.com/Ajustor/coding-unicorns-modules) are working examples.

---

## Creating a Plugin

Implement the `Plugin` trait from `crate::plugin`:

```rust
use crate::plugin::{Plugin, PluginCommand, PluginContext, PluginResponse, SidebarPanel};

pub struct MyPlugin;

impl Plugin for MyPlugin {
    fn name(&self) -> &str { "My Plugin" }
    fn version(&self) -> &str { "1.0.0" }
}
```

---

## Available Hooks

### `update(&mut self, ctx: &PluginContext) -> PluginResponse`

Called **every frame**. Use it to react to the current editor state (buffer contents, cursor position, etc.) and optionally return a status bar message.

```rust
fn update(&mut self, ctx: &PluginContext) -> PluginResponse {
    PluginResponse {
        status_text: Some(format!("chars: {}", ctx.buffer_text.len())),
        ..Default::default()
    }
}
```

### `execute_command(&mut self, command_id: &str, ctx: &PluginContext) -> PluginResponse`

Called when the user triggers one of your plugin's commands (e.g. from the command palette). Match on `command_id` to handle each command.

```rust
fn execute_command(&mut self, command_id: &str, ctx: &PluginContext) -> PluginResponse {
    if command_id == "my-plugin.greet" {
        return PluginResponse {
            notifications: vec!["Hello from My Plugin!".into()],
            ..Default::default()
        };
    }
    PluginResponse::default()
}
```

### `render_sidebar(&mut self, panel_id: &str, ui: &mut egui::Ui)`

Called to draw your plugin's sidebar panel using egui. Return `sidebar_panels()` to register panels.

```rust
fn render_sidebar(&mut self, _panel_id: &str, ui: &mut egui::Ui) {
    ui.label("Hello from the sidebar!");
}
```

### `tokenize_line(&self, lang: &str, line: &str) -> Option<Vec<Token>>`

Optional. Return `Some(tokens)` to provide syntax highlighting for a given language and line. Return `None` when the plugin doesn't handle that language.

```rust
fn tokenize_line(&self, lang: &str, line: &str) -> Option<Vec<Token>> {
    if lang == "mylang" {
        // produce your own Token vec
    }
    None
}
```

---

### Language support methods

All optional, with inert defaults (see `src/plugin/mod.rs`):

| Method | Purpose |
|--------|---------|
| `hover_info(lang, word, file_content) -> Option<String>` | Hover text for `word` (e.g. a code-fenced signature) |
| `tokenize_document(lang, text) -> Option<Vec<Vec<Token>>>` | Whole-document tokens, one vector per line, for multi-line constructs |
| `reset_tokenizer()` | Reset multi-line tokenizer state before a new document |
| `file_extensions() -> &[&str]` | Extensions handled (e.g. `&["ts", "tsx"]`), used to pick the plugin and LSP server for a file |
| `lsp_server_command() -> Option<(String, Vec<String>)>` | LSP server binary and arguments |

Debug adapters are not part of this trait: a language module declares its own in the `[debugger]` section of its `manifest.toml` (see the README), read by `src/dap/adapters.rs`. The IDE ships no language-specific debugger.

---

## Registering a Plugin

In `src/app/mod.rs`, inside `CodingUnicorns::new()`:

```rust
let mut plugin_manager = PluginManager::new();
plugin_manager.register(Box::new(MyPlugin::new()));
```

Registering a plugin under a name already in use replaces the previous one.

---

## Key Types

### `PluginCommand`

Registered commands appear in the command palette.

```rust
pub struct PluginCommand {
    pub id: String,              // unique id, e.g. "my-plugin.action"
    pub title: String,           // shown in command palette
    pub keybinding: Option<String>, // e.g. "Ctrl+Shift+M"
}
```

### `SidebarPanel`

```rust
pub struct SidebarPanel {
    pub id: String,         // unique id, e.g. "my-plugin.panel"
    pub title: String,      // shown in sidebar header
    pub icon: &'static str, // Phosphor icon char (egui_phosphor::regular::*)
}
```

### `PluginContext<'a>`

Read-only snapshot of editor state passed to every hook.

```rust
pub struct PluginContext<'a> {
    pub buffer_text: &'a str,
    pub filename: Option<&'a str>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub is_modified: bool,
}
```

### `PluginResponse`

Returned from `update()` and `execute_command()`.

```rust
pub struct PluginResponse {
    pub status_text: Option<String>, // shown in status bar
    pub notifications: Vec<String>,  // popup messages (future)
}
```

---

## Built-in Plugins

| Plugin | Commands | Sidebar Panel |
|--------|----------|---------------|
| Word Count | `word-count.show` — Show Statistics | `word-count.panel` — word / line / char counts |

---

## Future Plans

- **Plugin configuration** — per-plugin settings stored in the workspace config.
- **Event bus** — subscribe to editor events (file open, save, cursor move) instead of polling in `update()`.
- **Async plugins** — plugins that can spawn background tasks (e.g. linters, formatters).
