# GDScript / Godot Support — Design

**Date:** 2026-10-15
**Scope:** Work on Godot projects in Coding Unicorns: highlighting for GDScript, scene/resource files and shaders, plus completion, hover, diagnostics and debugging through the Godot editor's built-in language server and debug adapter. Two repositories: the IDE (`writingUnicorns`) learns to *connect* to a server that is already running, and a new `gdscript-lang` module (`writing-unicorns-modules`) uses it.

---

## Context

Godot ships no stand-alone language server or debug adapter. Both live **inside the running Godot editor** and listen on TCP:

| Service | Default port (Godot 4) | Editor setting |
|---------|------------------------|----------------|
| Language server (LSP) | 6005 | Network → Language Server → Remote Port |
| Debug adapter (DAP) | 6006 | Network → Debug Adapter → Remote Port |

The IDE can only *start* processes today:

- LSP: `LspTransport::spawn` (`src/lsp/transport.rs`) runs the server on stdio.
- DAP: `DapTransport::open` (`src/dap/transport.rs`) runs the adapter on stdio, or runs it and connects to the free port it was told to listen on (`spawn_tcp`).

Neither can attach to a server that is already listening on a fixed address.

## Goals

- `.gd` files: highlighting, completion, hover, go-to-definition, references, diagnostics (Godot LSP).
- F5 runs the project's main scene through the Godot editor, with breakpoints, stepping, call stack and variables (Godot DAP).
- `.tscn`, `.tres`, `project.godot`, `.import`, `.gdextension`: highlighting.
- `.gdshader`, `.gdshaderinc`: highlighting.
- The IDE stays language-agnostic: no Godot-specific code, only a generic "connect to this address" option in module manifests.

## Non-goals

- Starting Godot (headless or not) from the IDE. When the editor is closed, the IDE waits for it.
- User-configurable ports. The module uses Godot's defaults; a project with other ports is out of scope for this version.
- LSP for scenes, resources or shaders (Godot provides none).
- A minimum-IDE-version field in manifests.

---

## Part 1 — IDE: connect to a running language server

### Manifest

`[capabilities.lsp_servers.<language>]` accepts `connect = "host:port"` instead of `command`:

```toml
[capabilities.lsp_servers.gd]
connect = "127.0.0.1:6005"
```

- `LanguageServer.command` becomes `Option<String>`; a new `connect: Option<String>` is added.
- Exactly one of `command` and `connect` must be set. Both or neither is a manifest error, reported when the module is loaded (same path as other manifest errors). Installed manifests are validated with `ExtensionManifest::parse`; an invalid one is skipped with a warning.
- `args` with `connect` is an error (nothing is started); `init_options` still applies (sent in `initialize`).
- The module-wide `lsp_server` / `lsp_args` keys are unchanged; `connect` exists only per language. A module that wants an LSP for one of its languages only declares that language in `lsp_servers` and no `lsp_server`.

### Resolution

A new type describes how to reach a server:

```rust
pub enum LspTarget {
    Spawn { command: String, args: Vec<String> },
    Connect(String), // "host:port"
}
```

Since 0.15, servers declared in manifests are resolved by `ModuleServer` (`src/lsp/server.rs`); the `Plugin` trait path in `ensure_lsp_for_language` (`src/app/lsp_ops.rs`) is only a fallback and keeps its `(command, args)` tuple:

- `ModuleServer` reads `connect` from the per-language entry, never downloads for it, and gains `target(workspace) -> LspTarget` (the address, or its expanded command line).
- `ensure_lsp_for_language` → `LspManager::ensure_started_with(ext, &LspTarget, …)` (replaces `ensure_started_with_cmd`); the fallback wraps its tuple in `LspTarget::Spawn`.
- `FfiLangPlugin` keeps connected languages as `None` entries (no fallback to the module-wide server); `Capabilities::lsp_command` returns `None` for a connected server.

### Transport

`LspTransport::connect(addr: &str)`: `connect_tcp` resolves the address with `ToSocketAddrs` and tries every resolved address with `TcpStream::connect_timeout` (1 s each); the writer is the stream, and the existing `read_messages` reader thread runs on a `try_clone` of it. `_child` is `None`. EOF on the socket clears `is_alive`, exactly like a server process exiting. The read timeout of the connection is only used during the handshake, so a handshake that gives up can be aborted. TCP connections are closed with `Shutdown::Write` on the original stream handle (FIN, queued data is delivered; `Both` makes Windows reset the connection and lose the last `disconnect`). Spawned servers are killed 2 s after their transport is dropped. `initialize` sends `rootPath` along `rootUri`.

`LspClient::start` / `spawn_handshake` take the `LspTarget` and pick `spawn` or `connect`. The `initialize` / `initialized` handshake is unchanged.

### Waiting for the server

**Existing bug, fixed for every server:** when the very first start fails (command missing, server dies during `initialize`), the handshake thread returns without sending, `reconnect_rx` stays set and `last_crash_time` stays `None`, so `try_restart` never retries. `poll` will detect a disconnected `reconnect_rx` (`TryRecvError::Disconnected`), clear it and set `last_crash_time`, so a failed start follows the same retry path as a crash.

**Retry delay:** spawned servers keep the exponential back-off (2 s doubling, capped at 30 s). Connected servers retry every **3 s**, forever: a refused local connection costs nothing, and the user expects completion as soon as Godot is open. When Godot closes, the socket hits EOF and the client goes back to waiting through the same path.

### Status bar

A new `LspStatus::Waiting(addr)` variant, shown with the same orange dot as `Connecting`; its tooltip reads "Waiting for the language server at <addr>".

---

## Part 2 — IDE: connect to a running debug adapter

### Manifest

`[debugger]` accepts `connect = "host:port"` instead of `command`:

```toml
[debugger]
types = ["godot"]
connect = "127.0.0.1:6006"
install_hint = "open the project in the Godot editor (Editor Settings → Network → Debug Adapter, port 6006)"

[debugger.default_launch]
scene = "main"
```

- `DebuggerSpec.command` becomes `Option<OneOrMany>`; a new `connect: Option<String>` is added.
- Exactly one of `command` and `connect` must be set. With `connect`, `args`, a `transport` other than `stdio` (`tcp`, `lsp`), `download` and `lsp_resolve` are errors (nothing is started or downloaded).
- `install_hint` keeps its role: shown when the adapter cannot be reached.

### Adapter resolution (`src/dap/adapters.rs`)

- `name()` returns the address in connect mode.
- `needs_download()` is `false` and `locate()` is not used in connect mode.
- `config()` copies `connect` into `DapConfig` (new field `connect: Option<String>`).
- `src/dap/launcher.rs` skips the download / locate steps when the adapter connects.

### Transport (`src/dap/transport.rs`)

`DapTransport::open` checks `cfg.connect` first: `connect_tcp` (every resolved address, `ATTACH_TIMEOUT` of 3 s), then the existing `over_tcp`. `port` is set so child sessions (`startDebugging`) could open more connections; `child` is `None`, so `Drop` sends `disconnect` and kills nothing — the Godot editor stays open.

A refused or timed-out connection fails the start with: `Cannot reach the debug adapter at <addr>: <install_hint>`. No retry: F5 is an explicit action.

### Godot launch behaviour (no IDE code, documented)

The IDE already sends `launch` before `configurationDone` (`src/dap/client.rs`), which is what Godot needs: it stores the `launch` request and plays the scene on `configurationDone`.

- Without a launch configuration, F5 on a `.gd` file uses `default_launch`: `scene = "main"` plays the project's main scene.
- A `launch.toml` / `.vscode/launch.json` entry with `type = "godot"` can set `scene = "current"` (scene open in the Godot editor) or a `res://` path, plus Godot's `noDebug`, `playArgs`, `platform`.
- `project` is **not** sent by default: Godot compares it with its own project path and rejects a mismatch, and on Windows the comparison is fragile (backslashes, drive-letter case). When a configuration provides it, it is passed through untouched. Extra fields from godot-tools (VS Code) configurations are ignored by Godot.
- Breakpoints are sent with absolute paths; Godot localises them to `res://` itself. The `cwd` the IDE injects is ignored by Godot.

---

## Part 3 — `gdscript-lang` module

In `writing-unicorns-modules`, on a branch from `origin/master`.

### Files

```
gdscript-lang/
  Cargo.toml
  manifest.toml
  queries/gdscript.scm
  queries/godot-resource.scm
  src/lib.rs
  tests/highlight.rs
  tests/samples/gd/…, tscn/…, tres/…, godot/…, gdshader/… (+ .tokens snapshots)
```

### Dependencies

`unicorns-highlight` (shared), `tree-sitter-highlight` 0.25, `tree-sitter-gdscript` 6.1, `tree-sitter-godot-resource` 0.7, `tree-sitter-gdshader` 0.4 (all on `tree-sitter-language` 0.1, compatible with tree-sitter 0.25). Dev: `unicorns-highlight-test`.

### Grammars and queries

| Language key | Extensions | Grammar | Highlight query |
|--------------|------------|---------|-----------------|
| `gd` | `.gd` | tree-sitter-gdscript | `queries/gdscript.scm` (written for this module) |
| `tscn`, `tres`, `godot`, `import`, `gdextension` | same | tree-sitter-godot-resource | `queries/godot-resource.scm` (written for this module) |
| `gdshader`, `gdshaderinc` | same | tree-sitter-gdshader | the crate's `HIGHLIGHTS_QUERY` |

The two Prestonknopp crates ship no highlight query (their `HIGHLIGHTS_QUERY` constants are commented out), so the module writes its own from the grammars' `node-types.json`, using the capture names `unicorns-highlight` maps to the IDE's kinds — the same approach as `csharp-lang`, `java-lang` and `powershell-lang`. The gdshader query is used as-is; captures the shared mapping does not know render as plain text. If the snapshot shows keywords, types or comments left plain, the module ships its own `queries/gdshader.scm` instead.

### FFI

- `language_id()` → `gdscript`; `file_extensions()` → all extensions above.
- `tokenize_document_lang_ffi(lang, text)` picks the grammar from the language key (as `docker-lang` does); `tokenize_document_ffi` falls back to GDScript. `tokenize_line_ffi` returns null (document tokenizing only), `reset_tokenizer` is a no-op, `free_string` as usual.
- No hover export: hover comes from the Godot LSP.

### Manifest

```toml
[extension]
id = "unicorns.gdscript-lang"
name = "GDScript"
version = "0.1.0"
description = "Godot support — GDScript, scenes, resources and shaders highlighting; completion, diagnostics and debugging through the Godot editor"
author = "Writing Unicorns"
repository = ""
documentation = "https://docs.godotengine.org/en/stable/tutorials/scripting/gdscript/"

[capabilities]
languages = ["gd", "tscn", "tres", "godot", "import", "gdextension", "gdshader", "gdshaderinc"]
language_ids = { gd = "gdscript" }

[capabilities.lsp_servers.gd]
connect = "127.0.0.1:6005"

[debugger]
types = ["godot"]
connect = "127.0.0.1:6006"
install_hint = "open the project in the Godot editor (Editor Settings → Network → Debug Adapter, port 6006)"

[debugger.default_launch]
scene = "main"
```

`project.godot` is matched by its `.godot` extension; `file_names` is not needed.

### Compatibility

An IDE without `connect` support cannot parse this manifest (`command` is required there) and refuses the module with a manifest error. The module README states the minimum IDE version, like the `file_names` and `panels` notes. The IDE is released first, then the module.

---

## Testing

### IDE (unit tests, `cargo test`)

- Manifest: `lsp_servers` entry with `connect` only, `command` only, both (error), neither (error); `[debugger]` with `connect` only, `command` only, both, neither, `connect` + `download` (error).
- Resolution: `lsp_server_for_ext` returns `LspTarget::Connect` for the language that declares it and `None` for the module's other languages.
- `LspTransport::connect` against a local `TcpListener` that answers `initialize`: the client becomes connected.
- Wait-and-connect: a connect-mode client started with nothing listening stays disconnected, then connects once a listener appears on that port. The connect retry delay is a field of `LspClient` (3 s by default) that tests set to zero.
- Failed first start of a spawned server now schedules a retry (`last_crash_time` set).
- `DapTransport::open` in connect mode against a local listener; refused connection yields the message with the install hint; `needs_download()` is false in connect mode.

### Module (`cargo test -p gdscript-lang`)

- Snapshot tests on samples for each grammar (`tests/samples/<lang>/…` + `.tokens`).
- Unit test: `tokenize_document_lang_ffi` dispatches each language key to the right grammar.

### End to end (manual, Windows, Godot 4)

1. Godot editor open on a sample project; module installed from the local folder.
2. Open a `.gd` file: status goes from `Waiting` to ready; completion, hover, go-to-definition and a diagnostic on a deliberate error work.
3. Close Godot: status goes back to `Waiting` ("Waiting for the language server at 127.0.0.1:6005"); reopen it: the IDE reconnects without action.
4. Breakpoint in a script of the main scene, F5: the game starts, stops on the breakpoint, call stack and variables show, continue and stop work.
5. F5 with Godot closed: the error shows the install hint.
6. `.tscn`, `.tres`, `project.godot` and a `.gdshader` are highlighted.

---

## Documentation

- IDE `README.md`: `connect` in the manifest reference (`lsp_servers` and `[debugger]`), and a `gdscript-lang` row in the official extensions table.
- IDE `CHANGELOG.md` and `CHANGELOG.fr.md`: entries for `connect` and the first-start retry fix.
- Modules `README.md`: `gdscript-lang` row, a note that LSP and debugging need the Godot editor open on the project (ports 6005 / 6006), and the minimum IDE version.

## Delivery

Two branches, two pull requests: the IDE first (`connect` support + retry fix), then the module. Nothing is committed without the user's go-ahead.
