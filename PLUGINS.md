# Coding Unicorns — Plugin System

Coding Unicorns supports a lightweight plugin system inspired by VSCode extensions. Plugins live in-process and integrate directly with the editor loop.

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

Optional. Return `Some(tokens)` to override syntax highlighting for a given language and line. Return `None` to fall back to the built-in highlighter.

```rust
fn tokenize_line(&self, lang: &str, line: &str) -> Option<Vec<Token>> {
    if lang == "mylang" {
        // produce your own Token vec
    }
    None
}
```

---

## Registering a Plugin

In `src/app.rs`, inside `CodingUnicorns::new()`:

```rust
let mut plugin_manager = PluginManager::new();
plugin_manager.register(Box::new(MyPlugin::new()));
```

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

- **Dynamic loading** via `libloading` — load `.so` / `.dll` plugin files at runtime without recompiling the editor.
- **Plugin configuration** — per-plugin settings stored in the workspace config.
- **Event bus** — subscribe to editor events (file open, save, cursor move) instead of polling in `update()`.
- **Async plugins** — plugins that can spawn background tasks (e.g. linters, formatters).
