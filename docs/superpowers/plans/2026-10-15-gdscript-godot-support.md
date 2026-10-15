# GDScript / Godot Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **⚠ Commit policy (user override):** never `git add` / `git commit` without the owner's explicit go-ahead. Each task ends with a **Checkpoint**: leave the changes uncommitted; the suggested commit message is for when the owner asks. Commit messages and PR bodies carry **no** Claude attribution (no `Co-Authored-By`, no "Generated with Claude Code").

> **Shell:** run `cargo` from **PowerShell** (`RUSTC_WRAPPER=sccache` is set and `sccache` is only on the PowerShell `PATH`; from Git Bash prefix with `RUSTC_WRAPPER=`). Prefix commands with `rtk` (project rule).

> **Base:** IDE `master` at 0.20.0 (`38eebe1`). Module language servers are resolved by `ModuleServer` (`src/lsp/server.rs`) from the manifests; the `Plugin` trait path in `src/app/lsp_ops.rs` is only a fallback and is not changed.

**Goal:** Work on Godot projects in Coding Unicorns: GDScript, scene/resource and shader highlighting, plus completion, diagnostics and debugging through the Godot editor's built-in LSP (TCP 6005) and DAP (TCP 6006).

**Architecture:** The IDE gains a generic `connect = "host:port"` manifest key (per-language LSP servers and `[debugger]`) so it can attach to a server another program already runs, instead of starting one; a connected LSP client keeps retrying every 3 s until the server is reachable. A new `gdscript-lang` module in `writing-unicorns-modules` uses it, with three tree-sitter grammars for highlighting.

**Tech Stack:** Rust, egui 0.31, tree-sitter 0.25 / tree-sitter-highlight 0.25, tree-sitter-gdscript 6.1, tree-sitter-godot-resource 0.7, tree-sitter-gdshader 0.4.

**Reference spec:** `docs/superpowers/specs/2026-10-15-gdscript-godot-support-design.md`

---

## File map

**IDE (`C:\Users\alexa\dev\perso\writingUnicorns`, branch `feat/connect-servers`, already created from `master`)**

| File | Change |
|------|--------|
| `src/lsp/target.rs` | **New.** `LspTarget` (spawn a command, or connect to `host:port`) |
| `src/lsp/mod.rs` | Register `target`, re-export `LspTarget` |
| `src/lsp/transport.rs` | `LspTransport::connect(addr)` |
| `src/lsp/client.rs` | `start(LspTarget, …)`, failed first start retried, fixed 3 s retry for connect targets, `waiting_for()` |
| `src/lsp/manager.rs` | `ensure_started_with(ext, &LspTarget, …)` replaces `ensure_started_with_cmd` + `start_client` |
| `src/lsp/server.rs` | `ModuleServer`: `connect`, `target()`, no download when connecting |
| `src/extension/manifest.rs` | `LanguageServer.connect`, `DebuggerSpec.connect`, validation in `parse` |
| `src/extension/ffi_plugin.rs` | Skips per-language servers without a command |
| `src/app/lsp_ops.rs` | Call sites |
| `src/ui/statusbar.rs`, `src/ui/layout.rs` | `LspStatus::Waiting(addr)` |
| `src/dap/types.rs` | `DapConfig.connect` |
| `src/dap/transport.rs` | `DapTransport::attach(addr)`, used by `open` |
| `src/dap/adapters.rs` | Connect mode: `connects()`, `unreachable_message()`, no download/locate |
| `src/dap/launcher.rs` | Skips download/locate when connecting; clear error |
| `src/dap/lsp_session.rs` | Reads the command through `commands()` |
| `src/dap/client.rs`, `src/dap/manager.rs` | `connect: None` in test `DapConfig` literals; e2e test start call |
| `README.md`, `CHANGELOG.md`, `CHANGELOG.fr.md` | Docs |

**Modules (`C:\Users\alexa\dev\perso\writing-unicorns-modules`)**

| File | Change |
|------|--------|
| `Cargo.toml` | Add `gdscript-lang` member |
| `gdscript-lang/Cargo.toml`, `manifest.toml`, `src/lib.rs` | **New** module |
| `gdscript-lang/queries/gdscript.scm`, `godot-resource.scm` | **New** highlight queries |
| `gdscript-lang/tests/highlight.rs`, `tests/samples/**` | **New** snapshot tests |
| `scripts/build_registry.py` | `lsp_server` falls back to a `connect` address |
| `README.md` | Module row, Godot notes |

---

# Part A — IDE

### Task A1: `LspTarget` and `LspTransport::connect`

**Files:**
- Create: `src/lsp/target.rs`
- Modify: `src/lsp/mod.rs`, `src/lsp/transport.rs`

- [ ] **Step 1: Write the failing transport tests**

Append inside `mod tests` of `src/lsp/transport.rs` (before its closing `}`):

```rust
    #[test]
    fn connect_talks_to_a_listening_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let (tx, rx) = unbounded();
            std::thread::spawn(move || read_messages(BufReader::new(stream), &tx));
            let msg = rx.recv_timeout(Duration::from_secs(10)).unwrap();
            write_message(&mut writer, &msg).unwrap();
            writer.shutdown(std::net::Shutdown::Both).unwrap();
        });
        let mut t = LspTransport::connect(&addr).unwrap();
        t.send(&json!({"id": 1})).unwrap();
        assert_eq!(
            t.receiver.recv_timeout(Duration::from_secs(10)).unwrap(),
            json!({"id": 1})
        );
        server.join().unwrap();
        // The server closed the connection: the transport is dead.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while t.is_alive.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!t.is_alive.load(Ordering::Relaxed));
    }

    #[test]
    fn connect_fails_without_a_server() {
        // A port that was free a moment ago: nothing listens on it.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(LspTransport::connect(&format!("127.0.0.1:{port}")).is_err());
        assert!(LspTransport::connect("not an address").is_err());
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `rtk cargo test lsp::transport`
Expected: compile error `no function or associated item named 'connect' found for struct 'LspTransport'`.

- [ ] **Step 3: Implement `connect`**

In `src/lsp/transport.rs`, extend the imports at the top so they read:

```rust
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
```

(keep any other existing `use` lines as they are). Add after the `LspTransport` struct definition:

```rust
/// How long connecting to a server that is already running may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
```

Add this method in `impl LspTransport`, after `spawn`:

```rust
    /// Connect to a server already listening on `addr` (`host:port`), such
    /// as the Godot editor's. Nothing is started.
    pub fn connect(addr: &str) -> anyhow::Result<Self> {
        let socket = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| anyhow::anyhow!("no address for `{addr}`"))?;
        let stream = TcpStream::connect_timeout(&socket, CONNECT_TIMEOUT)?;
        let reader = stream.try_clone()?;
        let (tx, rx) = unbounded::<Value>();
        let alive = Arc::new(AtomicBool::new(true));
        let alive_clone = alive.clone();
        std::thread::spawn(move || {
            read_messages(BufReader::new(reader), &tx);
            // The server closed the connection.
            alive_clone.store(false, Ordering::Relaxed);
        });
        Ok(Self {
            stdin: Box::new(stream),
            receiver: rx,
            is_alive: alive,
            _child: None,
        })
    }
```

- [ ] **Step 4: Create `src/lsp/target.rs`**

```rust
//! How the IDE reaches a language server.

/// A language server the IDE starts, or one another program already runs
/// (the Godot editor runs its own on a TCP port).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspTarget {
    /// Start `command` with `args` and talk to it over stdio.
    Spawn { command: String, args: Vec<String> },
    /// Connect to a server already listening on `host:port`.
    Connect(String),
}

impl LspTarget {
    /// The address of a server the IDE connects to; `None` for one it starts.
    pub fn connect_addr(&self) -> Option<&str> {
        match self {
            Self::Connect(addr) => Some(addr),
            Self::Spawn { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_connected_servers_have_an_address() {
        assert_eq!(
            LspTarget::Connect("127.0.0.1:6005".into()).connect_addr(),
            Some("127.0.0.1:6005")
        );
        let spawn = LspTarget::Spawn {
            command: "pylsp".into(),
            args: vec![],
        };
        assert_eq!(spawn.connect_addr(), None);
    }
}
```

Replace `src/lsp/mod.rs` with:

```rust
pub mod client;
pub mod manager;
pub mod server;
pub mod target;
pub mod transport;

pub use client::LspClient;
pub use manager::LspManager;
pub use target::LspTarget;
```

- [ ] **Step 5: Run the tests**

Run: `rtk cargo test lsp::`
Expected: all `lsp::` tests pass, including `connect_talks_to_a_listening_server`, `connect_fails_without_a_server`, `only_connected_servers_have_an_address`. (Unused-code warnings for `LspTarget` / `connect` are expected until Task A2.)

- [ ] **Step 6: Checkpoint** — no commit. Suggested message: `feat(lsp): connect to a language server over TCP`

---

### Task A2: `LspClient` / `LspManager` take an `LspTarget`; retry a failed first start

**Files:**
- Modify: `src/lsp/client.rs`, `src/lsp/manager.rs`, `src/app/lsp_ops.rs` (`ensure_lsp_for_language`), `src/lsp/server.rs` (e2e test), `src/dap/manager.rs` (e2e test)

- [ ] **Step 1: Write the failing client tests**

In `mod tests` of `src/lsp/client.rs`:

1. In `crash_is_detected_and_restart_backs_off`, replace

```rust
        h.client.restart_cmd = Some((
            "definitely-not-a-real-lsp-binary-xyz".into(),
            vec![],
            std::env::temp_dir(),
        ));
```

with

```rust
        h.client.restart_target = Some((
            LspTarget::Spawn {
                command: "definitely-not-a-real-lsp-binary-xyz".into(),
                args: vec![],
            },
            std::env::temp_dir(),
        ));
```

2. Replace the whole `start_records_restart_command_and_fails_quietly_for_missing_binary` test with:

```rust
    #[test]
    fn failed_first_start_schedules_a_retry() {
        let mut c = LspClient::new();
        let dir = tempfile::tempdir().unwrap();
        let target = LspTarget::Spawn {
            command: "definitely-not-a-real-lsp-binary-xyz".into(),
            args: vec!["--stdio".into()],
        };
        c.start(target.clone(), dir.path());
        assert_eq!(c.restart_target, Some((target, dir.path().to_path_buf())));
        assert!(c.reconnect_rx.is_some());
        assert_eq!(c.waiting_for(), None, "a started server is not waited for");
        // The handshake thread gives up; the client stays disconnected and
        // retries later, as after a crash.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while c.reconnect_rx.is_some() && std::time::Instant::now() < deadline {
            c.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!c.is_connected);
        assert!(c.reconnect_rx.is_none(), "the handshake gave up");
        assert!(c.last_crash_time.is_some(), "a retry is scheduled");
    }

    #[test]
    fn connect_targets_retry_at_a_fixed_pace() {
        let ago = std::time::Instant::now().checked_sub(std::time::Duration::from_secs(4));
        // A server the IDE starts backs off: 30 s after many attempts.
        let mut spawned = LspClient::new();
        spawned.restart_target = Some((
            LspTarget::Spawn {
                command: "definitely-not-a-real-lsp-binary-xyz".into(),
                args: vec![],
            },
            std::env::temp_dir(),
        ));
        spawned.restart_attempts = 10;
        spawned.last_crash_time = ago;
        spawned.try_restart();
        assert!(spawned.reconnect_rx.is_none());
        // A server the IDE connects to is tried again after 3 s, always.
        let mut connected = LspClient::new();
        connected.restart_target = Some((
            LspTarget::Connect("127.0.0.1:1".into()),
            std::env::temp_dir(),
        ));
        connected.restart_attempts = 10;
        connected.last_crash_time = ago;
        connected.try_restart();
        assert!(connected.reconnect_rx.is_some());
    }

    /// A language server on `listener` that answers `initialize`.
    fn fake_server(listener: std::net::TcpListener) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let (tx, rx) = unbounded();
            std::thread::spawn(move || {
                crate::lsp::transport::read_messages(std::io::BufReader::new(stream), &tx)
            });
            let wait = std::time::Duration::from_secs(10);
            let init = rx.recv_timeout(wait).unwrap();
            assert_eq!(init["method"], "initialize");
            crate::lsp::transport::write_message(
                &mut writer,
                &json!({"jsonrpc": "2.0", "id": init["id"], "result": {"capabilities": {}}}),
            )
            .unwrap();
            assert_eq!(rx.recv_timeout(wait).unwrap()["method"], "initialized");
        })
    }

    #[test]
    fn connect_target_waits_for_the_server_then_connects() {
        // A free port, with nothing listening yet.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let addr = format!("127.0.0.1:{port}");
        let mut c = LspClient::new();
        c.connect_retry = std::time::Duration::ZERO;
        c.start(LspTarget::Connect(addr.clone()), &std::env::temp_dir());
        assert_eq!(c.waiting_for(), Some(addr.as_str()));

        // Nothing listens: the attempt fails and another one is scheduled.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while c.last_crash_time.is_none() && std::time::Instant::now() < deadline {
            c.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(c.last_crash_time.is_some(), "first attempt failed");
        assert!(!c.is_connected);

        // The server starts: a later attempt connects.
        let server = fake_server(std::net::TcpListener::bind(&addr).unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !c.is_connected && std::time::Instant::now() < deadline {
            c.poll();
            c.try_restart();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(c.is_connected);
        assert_eq!(c.waiting_for(), None);
        server.join().unwrap();
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `rtk cargo test lsp::client`
Expected: compile errors (`no field 'restart_target'`, `no method named 'waiting_for'`, `start` takes 3 arguments…).

- [ ] **Step 3: Implement in `src/lsp/client.rs`**

a. Imports — after `use super::transport::LspTransport;` add:

```rust
use super::target::LspTarget;
```

b. Just above `pub struct LspClient {` add:

```rust
/// Delay between attempts to reach a server the IDE connects to
/// ([`LspTarget::Connect`]): an attempt is cheap (at most a second, on the
/// handshake thread).
const CONNECT_RETRY: std::time::Duration = std::time::Duration::from_secs(3);
```

c. In `pub struct LspClient`, replace

```rust
    /// Command + args + workspace stored for auto-restart.
    restart_cmd: Option<(String, Vec<String>, PathBuf)>,
```

with

```rust
    /// Server and workspace, kept to restart or reconnect.
    restart_target: Option<(LspTarget, PathBuf)>,
```

and add after the `init_options` field:

```rust
    /// Delay between attempts to reach a server the IDE connects to
    /// ([`CONNECT_RETRY`]; tests shorten it).
    connect_retry: std::time::Duration,
```

d. In `LspClient::new()`, replace `restart_cmd: None,` with `restart_target: None,` and add `connect_retry: CONNECT_RETRY,` after `init_options: None,`.

e. After `pub fn is_busy(&self) -> bool { … }` add:

```rust
    /// The address this client waits for: set while a server the IDE
    /// connects to ([`LspTarget::Connect`]) is not reachable.
    pub fn waiting_for(&self) -> Option<&str> {
        if self.is_connected {
            return None;
        }
        self.restart_target.as_ref()?.0.connect_addr()
    }
```

f. Replace `start` (doc comment included) with:

```rust
    /// Start the language server `target` (or connect to it) and perform the
    /// initialize handshake.
    ///
    /// Non-blocking: the (potentially slow) handshake runs on a background thread so
    /// the UI never freezes while a server like rust-analyzer or csharp-ls boots.
    /// `poll()` installs the client and flips `is_connected` once it's ready; the app
    /// treats that as a (re)connect and re-opens the current file so diagnostics flow.
    /// A handshake that fails is retried like a crash (see [`Self::try_restart`]).
    pub fn start(&mut self, target: LspTarget, workspace: &Path) {
        let workspace = workspace.to_path_buf();
        self.restart_target = Some((target.clone(), workspace.clone()));
        self.reconnect_rx = Some(Self::spawn_handshake(
            target,
            workspace,
            self.init_options.clone(),
        ));
    }
```

g. In `spawn_handshake`, replace its doc comment, signature and the first lines of the thread up to and including the `let Ok(mut transport) … else { return; };` statement with:

```rust
    /// Start (or connect to) the server and run the `initialize`/`initialized`
    /// handshake on a background thread, delivering the ready `LspClientInner`
    /// through the returned channel. The channel closes without a value when the
    /// server cannot be started or reached, or does not answer. Shared by the
    /// initial start and the restart path.
    fn spawn_handshake(
        target: LspTarget,
        workspace: PathBuf,
        init_options: Option<Value>,
    ) -> mpsc::Receiver<LspClientInner> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let transport = match &target {
                LspTarget::Spawn { command, args } => {
                    let args: Vec<&str> = args.iter().map(String::as_str).collect();
                    LspTransport::spawn(command, &args, &workspace.to_string_lossy())
                }
                LspTarget::Connect(addr) => LspTransport::connect(addr),
            };
            let Ok(mut transport) = transport else {
                return;
            };
```

(The rest of the thread body — `let id = 1u64;` onwards — is unchanged; it uses `workspace` for `rootUri`.)

h. In `poll`, replace the block

```rust
        // Check if a background reconnect thread has finished.
        if let Some(rx) = &self.reconnect_rx {
            if let Ok(new_inner) = rx.try_recv() {
                self.inner = Some(new_inner);
                self.is_connected = true;
                self.restart_attempts = 0;
                self.reconnect_rx = None;
                self.last_crash_time = None;
                self.work_done_active = 0;
            }
        }
```

with

```rust
        // Check if a background reconnect thread has finished.
        if let Some(rx) = &self.reconnect_rx {
            match rx.try_recv() {
                Ok(new_inner) => {
                    self.inner = Some(new_inner);
                    self.is_connected = true;
                    self.restart_attempts = 0;
                    self.reconnect_rx = None;
                    self.last_crash_time = None;
                    self.work_done_active = 0;
                }
                // The handshake gave up (server missing, unreachable or
                // silent): retry later, as after a crash.
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.reconnect_rx = None;
                    self.last_crash_time = Some(std::time::Instant::now());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
```

i. Replace `try_restart` (doc comment included) with:

```rust
    /// Called each frame. Schedules a non-blocking reconnect: after an
    /// exponential back-off for a server the IDE starts, every
    /// `connect_retry` for one it connects to.
    /// Returns `true` when a reconnect just succeeded (caller should re-open the current file).
    pub fn try_restart(&mut self) -> bool {
        // Already reconnecting or connected — nothing to do.
        if self.is_connected || self.reconnect_rx.is_some() {
            return false;
        }
        let Some(crash_time) = self.last_crash_time else {
            return false;
        };
        let Some((target, workspace)) = self.restart_target.clone() else {
            return false;
        };
        let delay = match target {
            LspTarget::Connect(_) => self.connect_retry,
            LspTarget::Spawn { .. } => {
                std::time::Duration::from_secs((2u64 << self.restart_attempts.min(4)).min(30))
            }
        };
        if crash_time.elapsed() < delay {
            return false;
        }

        self.restart_attempts += 1;
        self.reconnect_rx = Some(Self::spawn_handshake(
            target,
            workspace,
            self.init_options.clone(),
        ));

        false
    }
```

- [ ] **Step 4: Update `src/lsp/manager.rs`**

a. Imports: after `use super::client::{…};` add `use super::target::LspTarget;`.

b. In the doc of `ensure_started`, replace `` `ensure_started_with_cmd()` `` with `` `ensure_started_with()` ``.

c. Replace `ensure_started_with_cmd` and `start_client` (both functions, with their docs) with:

```rust
    /// Ensure the language server `target` (from an installed extension)
    /// runs for files of `ext`. `init_options` are its `initializationOptions`.
    pub fn ensure_started_with(
        &mut self,
        ext: &str,
        target: &LspTarget,
        workspace: &Path,
        init_options: Option<serde_json::Value>,
    ) {
        if self.clients.contains_key(ext) {
            return;
        }
        let mut client = LspClient::new();
        client.set_init_options(init_options);
        client.start(target.clone(), workspace);
        self.clients.insert(ext.to_string(), client);
    }
```

d. Replace the test `ensure_started_with_cmd_registers_once` with:

```rust
    #[test]
    fn ensure_started_with_registers_once() {
        let mut m = LspManager::new();
        let dir = tempfile::tempdir().unwrap();
        let missing = LspTarget::Spawn {
            command: "definitely-not-a-real-lsp-binary-xyz".into(),
            args: vec!["--stdio".into()],
        };
        m.ensure_started_with("rs", &missing, dir.path(), None);
        assert!(m.get("rs").is_some());
        assert!(!m.get("rs").unwrap().is_connected);
        // Second call for the same extension is a no-op (does not replace the client).
        m.get_mut("rs")
            .unwrap()
            .completions
            .push(crate::lsp::client::CompletionItem {
                label: "marker".into(),
                detail: None,
                kind: "Text".into(),
                insert_text: None,
            });
        let other = LspTarget::Spawn {
            command: "other".into(),
            args: vec![],
        };
        m.ensure_started_with("rs", &other, dir.path(), None);
        assert_eq!(m.get("rs").unwrap().completions.len(), 1);

        m.restart_all();
        assert!(m.get("rs").is_none());
    }
```

(If the existing test pushes a `CompletionItem` with other fields, keep the struct literal exactly as the existing test writes it.)

- [ ] **Step 5: Update the call sites**

a. `src/app/lsp_ops.rs`, in `ensure_lsp_for_language`, replace

```rust
            let (cmd, args) = server.command_line(&workspace);
            let init = server.init_options(&workspace);
            self.lsp
                .ensure_started_with_cmd(ext, &cmd, &args, &workspace, init);
            return;
        }
        // Servers named by a module's library (`lsp_server_command_ffi`).
        if let Some((cmd, args)) = self.plugin_manager.lsp_server_for_ext(ext) {
            let init = self.extension_registry.lsp_init_options(ext);
            self.lsp
                .ensure_started_with_cmd(ext, &cmd, &args, &workspace, init);
```

with

```rust
            let (command, args) = server.command_line(&workspace);
            let target = crate::lsp::LspTarget::Spawn { command, args };
            let init = server.init_options(&workspace);
            self.lsp.ensure_started_with(ext, &target, &workspace, init);
            return;
        }
        // Servers named by a module's library (`lsp_server_command_ffi`).
        if let Some((command, args)) = self.plugin_manager.lsp_server_for_ext(ext) {
            let init = self.extension_registry.lsp_init_options(ext);
            let target = crate::lsp::LspTarget::Spawn { command, args };
            self.lsp.ensure_started_with(ext, &target, &workspace, init);
```

(Task A3 replaces the first one with `server.target(&workspace)`.)

b. `src/lsp/server.rs`, test `e2e_module_language_server_starts`: replace

```rust
        let (cmd, args) = server.command_line(&project);
        eprintln!("{cmd} {args:?}");
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut client = crate::lsp::LspClient::new();
        client.start(&cmd, &args, &project).unwrap();
```

with

```rust
        let (command, args) = server.command_line(&project);
        eprintln!("{command} {args:?}");
        let mut client = crate::lsp::LspClient::new();
        client.start(crate::lsp::LspTarget::Spawn { command, args }, &project);
```

c. `src/dap/manager.rs`, the e2e test that starts a language server (around line 935): replace

```rust
        let (cmd, args) = server.command_line(&project);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut lsp = crate::lsp::LspClient::new();
        lsp.set_init_options(server.init_options(&project));
        lsp.start(&cmd, &args, &project).unwrap();
```

with

```rust
        let (command, args) = server.command_line(&project);
        let mut lsp = crate::lsp::LspClient::new();
        lsp.set_init_options(server.init_options(&project));
        lsp.start(crate::lsp::LspTarget::Spawn { command, args }, &project);
```

- [ ] **Step 6: Run the tests**

Run: `rtk cargo test lsp::` then `rtk cargo build --all-targets`
Expected: all `lsp::` tests pass, including `failed_first_start_schedules_a_retry`, `connect_targets_retry_at_a_fixed_pace`, `connect_target_waits_for_the_server_then_connects`, `ensure_started_with_registers_once`, and the unchanged `crash_is_detected_and_restart_backs_off`, `restart_without_command_does_nothing`, `poll_installs_completed_reconnect`; the whole crate (with tests) builds.

- [ ] **Step 7: Checkpoint** — no commit. Suggested message: `fix(lsp): retry a server that fails on its first start; wait for servers reached over TCP`

---

### Task A3: `connect` for per-language servers in the manifest

**Files:**
- Modify: `src/extension/manifest.rs`, `src/lsp/server.rs`, `src/extension/ffi_plugin.rs`, `src/app/lsp_ops.rs`

- [ ] **Step 1: Write the failing tests**

a. In `mod tests` of `src/extension/manifest.rs`, add:

```rust
    #[test]
    fn per_language_server_can_connect_instead_of_starting() {
        let with = |entry: &str| {
            ExtensionManifest::parse(&format!(
                "{FULL}\n[capabilities.lsp_servers.pyw]\n{entry}\n"
            ))
        };
        let m = with("connect = \"127.0.0.1:6005\"").unwrap();
        let s = &m.capabilities.lsp_servers["pyw"];
        assert_eq!(s.connect.as_deref(), Some("127.0.0.1:6005"));
        assert_eq!(s.command, None);
        // A connected server has no command to name.
        assert_eq!(m.capabilities.lsp_command("pyw"), None);
        for bad in [
            "command = \"x\"\nconnect = \"127.0.0.1:6005\"",
            "args = [\"--stdio\"]",
            "connect = \"127.0.0.1:6005\"\nargs = [\"x\"]",
            "connect = \"127.0.0.1:6005\"\ndownload = { binary = \"x\", urls = { \"*\" = \"https://example.invalid/x.zip\" } }",
            "connect = \"localhost\"",
            "connect = \"127.0.0.1:0\"",
            "connect = \":6005\"",
        ] {
            let err = with(bad).unwrap_err().to_string();
            assert!(err.contains("[capabilities.lsp_servers.pyw]"), "{bad}: {err}");
        }
    }
```

b. In `mod tests` of `src/lsp/server.rs`, add:

```rust
    #[test]
    fn connected_server_is_reached_not_started() {
        let dir = tempfile::tempdir().unwrap();
        let m = module(
            dir.path(),
            r#"languages = ["gd", "tscn"]
[capabilities.lsp_servers.gd]
connect = "127.0.0.1:6005""#,
        );
        let gd = ModuleServer::for_language([&m], "gd").unwrap();
        assert!(!gd.needs_download());
        assert_eq!(
            gd.target(dir.path()),
            crate::lsp::LspTarget::Connect("127.0.0.1:6005".into())
        );
        assert!(
            ModuleServer::for_language([&m], "tscn").is_none(),
            "no server for the module's other languages"
        );

        let py = module(dir.path(), "languages = [\"py\"]\nlsp_server = \"pylsp\"");
        let target = ModuleServer::for_language([&py], "py")
            .unwrap()
            .target(dir.path());
        assert!(
            matches!(&target, crate::lsp::LspTarget::Spawn { command, .. } if command.starts_with("pylsp")),
            "{target:?}"
        );
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `rtk cargo test extension::manifest` then `rtk cargo test lsp::server`
Expected: compile errors (`no field 'connect'`, `no method named 'target'`).

- [ ] **Step 3: Implement in `src/extension/manifest.rs`**

a. In `pub struct LanguageServer`, replace

```rust
/// Language server of one language, when a module's languages need
/// different servers (`[capabilities.lsp_servers.<language>]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguageServer {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
```

with

```rust
/// Language server of one language, when a module's languages need
/// different servers (`[capabilities.lsp_servers.<language>]`): a command to
/// start, or the address of a server already running.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguageServer {
    /// Executable started on stdio. Exclusive with `connect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// `host:port` of a server already listening (e.g. inside the Godot
    /// editor), reached over TCP. Exclusive with `command`, `args` and
    /// `download`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect: Option<String>,
```

(the `init_options` and `download` fields stay).

b. After the `LanguageServer` struct add:

```rust
impl LanguageServer {
    fn validate(&self, language: &str) -> anyhow::Result<()> {
        let section = format!("[capabilities.lsp_servers.{language}]");
        match (&self.command, &self.connect) {
            (Some(_), None) => Ok(()),
            (None, Some(addr)) if self.args.is_empty() && self.download.is_none() => {
                validate_connect(&section, addr)
            }
            (None, Some(_)) => {
                anyhow::bail!("{section}: `args` and `download` cannot be used with `connect`")
            }
            (Some(_), Some(_)) => anyhow::bail!("{section}: set `command` or `connect`, not both"),
            (None, None) => anyhow::bail!("{section}: `command` or `connect` is required"),
        }
    }
}

/// Check the `connect` address of `section`: `host:port`, port 1–65535.
fn validate_connect(section: &str, addr: &str) -> anyhow::Result<()> {
    let valid = addr.rsplit_once(':').is_some_and(|(host, port)| {
        !host.is_empty() && port.parse::<u16>().is_ok_and(|p| p != 0)
    });
    if !valid {
        anyhow::bail!("{section}: `connect` must be `host:port`, got `{addr}`");
    }
    Ok(())
}
```

c. Replace `ExtensionManifest::parse` with:

```rust
    /// Parse a `manifest.toml` and validate it. Every install path must use
    /// this (not raw `toml::from_str`) because `extension.id` becomes a
    /// directory name under the extensions dir, and server sections must
    /// name exactly one way to reach the server.
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        let manifest: Self = toml::from_str(s)?;
        validate_extension_id(&manifest.extension.id)?;
        for (language, server) in &manifest.capabilities.lsp_servers {
            server.validate(language)?;
        }
        Ok(manifest)
    }
```

d. Replace `Capabilities::lsp_command` with:

```rust
    /// Language server command for `language`: its own entry in
    /// `lsp_servers`, else the module's `lsp_server`. `None` for a server
    /// reached with `connect`.
    pub fn lsp_command(&self, language: &str) -> Option<(String, Vec<String>)> {
        match self.lsp_servers.get(language) {
            Some(s) => Some((s.command.clone()?, s.args.clone())),
            None => Some((self.lsp_server.clone()?, self.lsp_args.clone())),
        }
    }
```

- [ ] **Step 4: Implement in `src/lsp/server.rs`**

a. In `pub struct ModuleServer`, after the `command: String,` / `args: Vec<String>,` fields add:

```rust
    /// `host:port` of a server already running (`connect`): nothing is
    /// started or downloaded.
    connect: Option<String>,
```

b. In `for_language`, replace

```rust
            let (command, args, download, per_language) = match caps.lsp_servers.get(language) {
                Some(s) => (s.command.clone(), s.args.clone(), s.download.clone(), true),
                None if caps.languages.iter().any(|l| l == language) => (
                    caps.lsp_server.clone()?,
                    caps.lsp_args.clone(),
                    caps.lsp_download.clone(),
                    false,
                ),
                None => return None,
            };
```

with

```rust
            let (command, args, download, connect, per_language) =
                match caps.lsp_servers.get(language) {
                    Some(s) => (
                        s.command.clone().unwrap_or_default(),
                        s.args.clone(),
                        s.download.clone(),
                        s.connect.clone(),
                        true,
                    ),
                    None if caps.languages.iter().any(|l| l == language) => (
                        caps.lsp_server.clone()?,
                        caps.lsp_args.clone(),
                        caps.lsp_download.clone(),
                        None,
                        false,
                    ),
                    None => return None,
                };
```

and add `connect,` to the `Some(Self { … })` literal (after `args,`).

c. Replace `needs_download` with:

```rust
    /// Whether files must be fetched before starting the server: its own
    /// archive, or the debugger files it uses. Never for a server the IDE
    /// connects to.
    pub fn needs_download(&self) -> bool {
        self.connect.is_none() && (self.needs_server_files() || self.needs_debugger_files())
    }
```

d. After `command_line` add:

```rust
    /// How to reach the server for `workspace`: the address of one already
    /// running (`connect`), else its command line.
    pub fn target(&self, workspace: &Path) -> crate::lsp::LspTarget {
        match &self.connect {
            Some(addr) => crate::lsp::LspTarget::Connect(addr.clone()),
            None => {
                let (command, args) = self.command_line(workspace);
                crate::lsp::LspTarget::Spawn { command, args }
            }
        }
    }
```

e. If any test in this file builds a `ModuleServer { … }` literal, add `connect: None,` to it.

- [ ] **Step 5: `src/extension/ffi_plugin.rs`**

In `load`, replace

```rust
        let lsp_servers = caps
            .lsp_servers
            .iter()
            .map(|(lang, s)| (lang.clone(), (s.command.clone(), s.args.clone())))
            .collect();
```

with

```rust
        // A server reached with `connect` has no command: `ModuleServer`
        // starts every manifest-declared server, this map is a fallback.
        let lsp_servers = caps
            .lsp_servers
            .iter()
            .filter_map(|(lang, s)| Some((lang.clone(), (s.command.clone()?, s.args.clone()))))
            .collect();
```

- [ ] **Step 6: Use the target (`src/app/lsp_ops.rs`, `ensure_lsp_for_language`)**

Replace

```rust
            let (command, args) = server.command_line(&workspace);
            let target = crate::lsp::LspTarget::Spawn { command, args };
            let init = server.init_options(&workspace);
```

with

```rust
            let target = server.target(&workspace);
            let init = server.init_options(&workspace);
```

- [ ] **Step 7: Run the tests**

Run: `rtk cargo test`
Expected: everything passes, including `per_language_server_can_connect_instead_of_starting` and `connected_server_is_reached_not_started`.

- [ ] **Step 8: Checkpoint** — no commit. Suggested message: `feat(extensions): per-language servers can connect to a running server (connect = "host:port")`

---

### Task A4: Status bar shows the address it waits for

**Files:**
- Modify: `src/ui/statusbar.rs`, `src/ui/layout.rs` (the `lsp_status` match, around line 335)

- [ ] **Step 1: Write the failing test**

In `src/ui/statusbar.rs` test `lsp_status_indicator`, add to the array:

```rust
            (LspStatus::Waiting("127.0.0.1:6005".into()), "⬤ LSP"),
```

- [ ] **Step 2: Run it to see it fail**

Run: `rtk cargo test ui::statusbar`
Expected: compile error `no variant named 'Waiting'`.

- [ ] **Step 3: Implement**

In `pub enum LspStatus`, after `Connecting,` add:

```rust
    /// Waiting for a server another program runs (e.g. the Godot editor's)
    /// to be reachable at this address.
    Waiting(String),
```

In `show`, after the `LspStatus::Connecting => { … }` arm add:

```rust
                        LspStatus::Waiting(addr) => {
                            ui.label(egui::RichText::new("⬤ LSP").color(palette.warning).small())
                                .on_hover_text(format!(
                                    "Waiting for the language server at {addr}…"
                                ));
                            ui.separator();
                        }
```

In `src/ui/layout.rs`, replace

```rust
                    Some(c) if !c.is_connected => LspStatus::Connecting,
```

with

```rust
                    Some(c) if !c.is_connected => match c.waiting_for() {
                        Some(addr) => LspStatus::Waiting(addr.to_string()),
                        None => LspStatus::Connecting,
                    },
```

- [ ] **Step 4: Run the tests**

Run: `rtk cargo test ui::`
Expected: pass.

- [ ] **Step 5: Checkpoint** — no commit. Suggested message: `feat(statusbar): show the address of a language server the IDE waits for`

---

### Task A5: `connect` in `[debugger]`

**Files:**
- Modify: `src/extension/manifest.rs`, `src/dap/adapters.rs` (`name`, `command_on_path`), `src/dap/lsp_session.rs` (around line 83)

- [ ] **Step 1: Write the failing test**

In `mod tests` of `src/extension/manifest.rs`, replace the two `d.command.as_slice()` assertions (`["netcoredbg"]` and `["python", "python3"]`) with `d.commands()`:

```rust
        assert_eq!(d.commands(), ["netcoredbg"]);
```
```rust
        assert_eq!(d.commands(), ["python", "python3"]);
```

and add:

```rust
    #[test]
    fn debugger_can_connect_to_a_running_adapter() {
        let with = |body: &str| ExtensionManifest::parse(&format!("{FULL}\n[debugger]\n{body}\n"));
        let d = with("types = [\"godot\"]\nconnect = \"127.0.0.1:6006\"")
            .unwrap()
            .debugger
            .unwrap();
        assert_eq!(d.connect.as_deref(), Some("127.0.0.1:6006"));
        assert!(d.commands().is_empty());
        for bad in [
            "command = \"x\"\nconnect = \"127.0.0.1:6006\"",
            "types = [\"godot\"]",
            "connect = \"127.0.0.1:6006\"\nargs = [\"--x\"]",
            "connect = \"127.0.0.1:6006\"\ntransport = \"tcp\"",
            "connect = \"127.0.0.1:6006\"\ntransport = \"lsp\"",
            "connect = \"127.0.0.1\"",
            "connect = \"127.0.0.1:6006\"\n[debugger.download]\nbinary = \"x\"\n[debugger.download.urls]\nwindows-x86_64 = \"https://example.invalid/x.zip\"",
            "connect = \"127.0.0.1:6006\"\n[[debugger.lsp_resolve]]\ncommand = \"x\"",
        ] {
            let err = with(bad).unwrap_err().to_string();
            assert!(err.contains("[debugger]"), "{bad}: {err}");
        }
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `rtk cargo test extension::manifest`
Expected: compile errors (`no method named 'commands'`, `no field 'connect'`).

- [ ] **Step 3: Implement in `src/extension/manifest.rs`**

a. In `pub struct DebuggerSpec`, replace

```rust
    /// Executable, or candidates tried in order, looked up on `PATH`. With
    /// `transport = "lsp"`: the language server command (`workspace/
    /// executeCommand`) that starts a debug session and returns its port.
    pub command: OneOrMany,
```

with

```rust
    /// Executable, or candidates tried in order, looked up on `PATH`. With
    /// `transport = "lsp"`: the language server command (`workspace/
    /// executeCommand`) that starts a debug session and returns its port.
    /// Exclusive with `connect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<OneOrMany>,
    /// `host:port` of an adapter already running (e.g. inside the Godot
    /// editor): the IDE connects to it and starts nothing. Exclusive with
    /// `command`, `args`, `transport`, `download` and `lsp_resolve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect: Option<String>,
```

b. After the `DebuggerSpec` struct add:

```rust
impl DebuggerSpec {
    /// Executables to try, in order (with `transport = "lsp"`: the server
    /// command); none for an adapter the IDE connects to.
    pub fn commands(&self) -> &[String] {
        self.command
            .as_ref()
            .map(OneOrMany::as_slice)
            .unwrap_or(&[])
    }

    fn validate(&self) -> anyhow::Result<()> {
        const SECTION: &str = "[debugger]";
        match (&self.command, &self.connect) {
            (Some(_), None) => Ok(()),
            (None, Some(addr)) => {
                if !self.args.is_empty()
                    || self.transport != DebuggerTransport::Stdio
                    || self.download.is_some()
                    || !self.lsp_resolve.is_empty()
                {
                    anyhow::bail!(
                        "{SECTION}: `args`, `transport`, `download` and `lsp_resolve` cannot be used with `connect`"
                    );
                }
                validate_connect(SECTION, addr)
            }
            (Some(_), Some(_)) => anyhow::bail!("{SECTION}: set `command` or `connect`, not both"),
            (None, None) => anyhow::bail!("{SECTION}: `command` or `connect` is required"),
        }
    }
}
```

c. In `ExtensionManifest::parse`, after the `lsp_servers` loop add:

```rust
        if let Some(debugger) = &manifest.debugger {
            debugger.validate()?;
        }
```

- [ ] **Step 4: Read the command through `commands()`**

`src/dap/adapters.rs`, replace `name()` and `command_on_path()` with:

```rust
    /// Name shown in messages (the first command).
    pub fn name(&self) -> &str {
        self.spec
            .commands()
            .first()
            .map(String::as_str)
            .unwrap_or("debugger")
    }
```

```rust
    fn command_on_path(&self) -> Option<PathBuf> {
        self.spec.commands().iter().find_map(|c| find_on_path(c))
    }
```

`src/dap/lsp_session.rs`, replace

```rust
    let command = adapter
        .spec
        .command
        .as_slice()
        .first()
        .ok_or("the module's debugger has no command")?;
```

with

```rust
    let command = adapter
        .spec
        .commands()
        .first()
        .ok_or("the module's debugger has no command")?;
```

- [ ] **Step 5: Run the tests**

Run: `rtk cargo test`
Expected: all pass, including `debugger_can_connect_to_a_running_adapter`.

- [ ] **Step 6: Checkpoint** — no commit. Suggested message: `feat(extensions): [debugger] can connect to a running adapter`

---

### Task A6: Attach to a running debug adapter

**Files:**
- Modify: `src/dap/types.rs`, `src/dap/transport.rs`, `src/dap/adapters.rs`, `src/dap/launcher.rs`, `src/dap/client.rs` (tests), `src/dap/manager.rs` (tests)

- [ ] **Step 1: Add the `DapConfig` field (needed by the tests)**

In `src/dap/types.rs`, add at the end of `pub struct DapConfig`:

```rust
    /// `host:port` of an adapter already running: connect to it instead of
    /// starting `adapter_cmd`.
    pub connect: Option<String>,
```

Add `connect: None,` to every `DapConfig { … }` literal that lists all its fields (literals ending in `..Default::default()` stay as they are): `src/dap/types.rs` (test `types_are_cloneable_and_debuggable`), `src/dap/client.rs` (test helper `cfg` and test `start_with_missing_adapter_fails`), `src/dap/manager.rs` (test helper `cfg`, the literal in the test around line 688, `exiting_adapter`), `src/dap/launcher.rs` (test helper `plan`). In `src/dap/adapters.rs` `config()`, add `connect: self.spec.connect.clone(),` to the returned `DapConfig`.

Run: `rtk cargo test dap::`
Expected: compiles, all existing `dap::` tests pass.

- [ ] **Step 2: Write the failing tests**

`src/dap/transport.rs`, inside `mod tests`:

```rust
    #[test]
    fn attach_connects_to_a_running_adapter_and_owns_no_process() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let reader = stream.try_clone().unwrap();
            let (tx, rx) = unbounded();
            std::thread::spawn(move || read_messages(BufReader::new(reader), &tx));
            let msg = rx.recv_timeout(Duration::from_secs(10)).unwrap();
            write_message(&mut stream, &msg).unwrap();
        });
        let cfg = DapConfig {
            connect: Some(format!("127.0.0.1:{port}")),
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let mut t = DapTransport::open(&cfg, dir.path()).unwrap();
        assert_eq!(t.port, Some(port));
        assert!(t.child.is_none(), "nothing started");
        t.send(&json!({"seq": 1})).unwrap();
        assert_eq!(
            t.receiver.recv_timeout(Duration::from_secs(10)).unwrap()["seq"],
            1
        );
        server.join().unwrap();
    }

    #[test]
    fn attach_fails_when_nothing_listens() {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(DapTransport::attach(&format!("127.0.0.1:{port}")).is_err());
    }
```

`src/dap/adapters.rs`, inside `mod tests`:

```rust
    const GODOT: &str = r#"[debugger]
types = ["godot"]
connect = "127.0.0.1:6006"
install_hint = "open the project in the Godot editor"
[debugger.default_launch]
scene = "main"
"#;

    #[test]
    fn running_adapter_is_connected_to_not_started() {
        let dir = tempfile::tempdir().unwrap();
        let gd = DebugAdapter::find(&[module(dir.path(), "gd", r#"["gd"]"#, GODOT)], None, "gd")
            .unwrap();
        assert!(gd.connects());
        assert_eq!(gd.name(), "127.0.0.1:6006");
        assert!(!gd.needs_download());
        let cfg = gd.config(PathBuf::new(), gd.default_launch().unwrap());
        assert_eq!(cfg.connect.as_deref(), Some("127.0.0.1:6006"));
        assert_eq!(cfg.launch_config["scene"], "main");
        let msg = gd.unreachable_message("connection refused");
        assert!(
            msg.contains("127.0.0.1:6006")
                && msg.contains("Godot editor")
                && msg.contains("connection refused"),
            "{msg}"
        );
        let py = DebugAdapter::find(&[module(dir.path(), "py", r#"["py"]"#, PYTHON)], None, "py")
            .unwrap();
        assert!(!py.connects());
    }
```

`src/dap/launcher.rs`, inside `mod tests`:

```rust
    #[test]
    fn unreachable_running_adapter_is_reported_with_its_hint() {
        let dir = tempfile::tempdir().unwrap();
        // A port that was free a moment ago: nothing listens on it.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let module = super::super::adapters::tests::module(
            dir.path(),
            "gd",
            r#"["gd"]"#,
            &format!(
                "[debugger]\nconnect = \"127.0.0.1:{port}\"\ninstall_hint = \"open the project in the Godot editor\""
            ),
        );
        let adapter = DebugAdapter::find(&[module], None, "gd").unwrap();
        let mut p = plan(vec![], dir.path());
        p.config = adapter.config(std::path::PathBuf::new(), json!({}));
        p.adapter = Some(adapter);
        let (logs, res) = finish(prepare(p));
        let err = res.unwrap_err();
        assert!(
            err.contains(&format!("127.0.0.1:{port}")) && err.contains("Godot editor"),
            "{err}"
        );
        assert!(logs.iter().all(|l| !l.contains("Downloading")), "{logs:?}");
    }
```

- [ ] **Step 3: Run them to see them fail**

Run: `rtk cargo test dap::`
Expected: compile errors (`no function 'attach'`, `no method 'connects'`, `no method 'unreachable_message'`).

- [ ] **Step 4: Implement the transport (`src/dap/transport.rs`)**

a. (No import change: `attach` resolves addresses through `connect_tcp`.)

b. Below `const CONNECT_TIMEOUT …` add:

```rust
/// How long connecting to an adapter that is already running may take.
const ATTACH_TIMEOUT: Duration = Duration::from_secs(2);
```

c. At the start of `open`, before `let args = …`, add:

```rust
        if let Some(addr) = &cfg.connect {
            return Self::attach(addr);
        }
```

and extend its doc comment with: `` /// An adapter already running at `cfg.connect` is connected to, not started. ``

d. After `connect(port)` add (it reuses `crate::lsp::transport::connect_tcp`, which tries every address the name resolves to):

```rust
    /// Connect to an adapter already listening on `addr` (`host:port`), such
    /// as the Godot editor's. Nothing is started, so nothing is stopped when
    /// the session ends.
    pub fn attach(addr: &str) -> anyhow::Result<Self> {
        let stream = crate::lsp::transport::connect_tcp(addr, ATTACH_TIMEOUT)?;
        let peer = stream.peer_addr()?;
        let (tx, rx) = unbounded::<Value>();
        let mut transport = Self::over_tcp(stream, tx, rx)?;
        // Child sessions (`startDebugging`) reconnect to 127.0.0.1:port.
        if peer.ip().is_loopback() {
            transport.port = Some(peer.port());
        }
        Ok(transport)
    }
```

e. **Close TCP connections on drop.** `over_tcp` gives the reader thread a `try_clone` of the stream, so dropping the transport leaves the connection open (the adapter never sees EOF and keeps its client). Add a field `socket: Option<TcpStream>` to `DapTransport` (`None` in `spawn` and `from_parts`). In `over_tcp`, keep the **original** stream in `socket` and give the writer and the reader thread `try_clone`s of it — on Windows, shutting down a cloned handle sometimes fails with `NotConnected` and sends no FIN (seen in `LspTransport`, see the comment in its `connect`). At the start of `impl Drop for DapTransport`'s `drop`, before the child handling:

```rust
        if let Some(socket) = &self.socket {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
```

Test (in `src/dap/transport.rs` tests): `dropping_an_attached_transport_closes_the_connection` — listener on `127.0.0.1:0`, `DapTransport::attach`, accept, drop the transport, then reading the accepted stream (`set_read_timeout(Some(5 s))`) returns `Ok(0)`.

- [ ] **Step 5: Implement the adapter (`src/dap/adapters.rs`)**

a. After `handles_type` add:

```rust
    /// Whether the adapter already runs (`connect`): the IDE starts and
    /// downloads nothing.
    pub fn connects(&self) -> bool {
        self.spec.connect.is_some()
    }
```

b. Replace `name()` with:

```rust
    /// Name shown in messages: the address of an adapter the IDE connects
    /// to, else the first command.
    pub fn name(&self) -> &str {
        if let Some(addr) = &self.spec.connect {
            return addr;
        }
        self.spec
            .commands()
            .first()
            .map(String::as_str)
            .unwrap_or("debugger")
    }
```

c. In `needs_download`, make `!self.connects()` the first condition:

```rust
    pub fn needs_download(&self) -> bool {
        !self.connects()
            && self.download_url().is_some()
            && self.installed_file().is_none()
            && (self.needs_files() || self.command_on_path().is_none())
    }
```

d. After `missing_message` add:

```rust
    /// Why an adapter the IDE connects to could not be reached.
    pub fn unreachable_message(&self, error: &str) -> String {
        let mut msg = format!("Cannot reach the debug adapter at {}", self.name());
        if let Some(hint) = &self.spec.install_hint {
            msg.push_str(": ");
            msg.push_str(hint);
        }
        msg.push_str(&format!(" ({error})"));
        msg
    }
```

- [ ] **Step 6: Implement the launcher (`src/dap/launcher.rs`, `run`)**

Replace

```rust
    if let (Some(adapter), None) = (&plan.adapter, by_lsp) {
```

with

```rust
    // An adapter the IDE connects to is neither downloaded nor located.
    let to_start = plan.adapter.as_ref().filter(|a| !a.connects());
    if let (Some(adapter), None) = (to_start, by_lsp) {
```

and replace

```rust
    let transport = DapTransport::open(&config, &plan.workspace)
        .map_err(|e| format!("could not start the debugger `{}`: {e}", config.adapter_cmd))?;
```

with

```rust
    let transport = DapTransport::open(&config, &plan.workspace).map_err(|e| match &plan.adapter {
        Some(adapter) if adapter.connects() => adapter.unreachable_message(&e.to_string()),
        _ => format!("could not start the debugger `{}`: {e}", config.adapter_cmd),
    })?;
```

- [ ] **Step 7: Run the tests**

Run: `rtk cargo test dap::`
Expected: all pass, including `attach_connects_to_a_running_adapter_and_owns_no_process`, `attach_fails_when_nothing_listens`, `running_adapter_is_connected_to_not_started`, `unreachable_running_adapter_is_reported_with_its_hint`.

- [ ] **Step 8: Checkpoint** — no commit. Suggested message: `feat(dap): attach to a debug adapter that is already running`

---

### Task A7: Documentation

**Files:**
- Modify: `README.md`, `CHANGELOG.md`, `CHANGELOG.fr.md`

- [ ] **Step 1: `README.md` — manifest reference**

In the `### Créer une extension` TOML block, replace

```toml
download = { binary = "bin/srv", urls = { "*" = "https://…/srv.zip" } }  # optionnel, comme lsp_download
```

with

```toml
download = { binary = "bin/srv", urls = { "*" = "https://…/srv.zip" } }  # optionnel, comme lsp_download
# connect = "127.0.0.1:6005"    # à la place de command/args/download : serveur déjà lancé, joint en TCP
```

and replace

```toml
command = ["mylang-dap"]    # binaire (ou candidats dans l'ordre), cherché dans le PATH
```

with

```toml
command = ["mylang-dap"]    # binaire (ou candidats dans l'ordre), cherché dans le PATH
# connect = "127.0.0.1:6006"  # à la place de command : adaptateur déjà lancé (sans args, transport, download ni lsp_resolve)
```

After the paragraph starting with `Le serveur LSP et le débogueur sont déclarés dans le manifeste, pas exportés` add a new paragraph:

```markdown
Un serveur qu'un autre programme fait déjà tourner (l'éditeur Godot embarque son serveur de langage et son débogueur) se déclare avec `connect = "hôte:port"` à la place de `command` (dans `[capabilities.lsp_servers.<langage>]` ou `[debugger]`) : l'IDE s'y connecte en TCP sans rien lancer ni télécharger. Tant que le serveur de langage ne répond pas, l'IDE réessaie toutes les 3 secondes (au survol, la pastille LSP de la barre d'état indique l'adresse attendue) ; un débogueur injoignable fait échouer F5 avec le message de `install_hint`.
```

- [ ] **Step 2: `README.md` — official modules**

In the official extensions table, after the `**docker-lang**` row add:

```markdown
| **gdscript-lang** | `.gd`, `.tscn` `.tres` `project.godot` `.import` `.gdextension`, `.gdshader` `.gdshaderinc` | éditeur Godot (TCP 6005) | éditeur Godot (TCP 6006) | — (Godot 4 ouvert sur le projet) |
```

After the `**docker-lang** ajoute aussi un panneau **Docker**…` paragraph add:

```markdown
**gdscript-lang** se branche sur l'éditeur Godot 4 : la complétion, les diagnostics et le débogage marchent tant que Godot est ouvert sur le projet, et l'IDE s'y reconnecte tout seul quand on le rouvre. F5 lance la scène principale ; une configuration de type `godot` peut choisir `scene = "current"` (la scène ouverte dans Godot) ou un chemin `res://`.
```

In the Debug (DAP) bullet near the top, replace `**Go** (Delve) et **Rust** (lldb-dap)` with `**Go** (Delve), **Rust** (lldb-dap) et **GDScript** (celui de l'éditeur Godot, joint en TCP)`.

- [ ] **Step 3: `CHANGELOG.md`**

Insert above `## [0.20.0] - 2026-10-14`:

```markdown
## [Unreleased]

### Added

- Language modules can connect to a language server or debug adapter that another program already runs, such as the ones built into the Godot editor (`connect = "host:port"` instead of `command` in the manifest). While the server is not reachable, the LSP indicator of the status bar shows the address it waits for, and the IDE connects as soon as it answers.

### Fixed

- A language server that fails on its very first start is retried, like one that crashes later, instead of staying off until the IDE restarts.

```

- [ ] **Step 4: `CHANGELOG.fr.md`**

Insert above `## [0.20.0] - 2026-10-14`:

```markdown
## [Non publié]

### Nouveautés

- Les modules de langage peuvent se connecter à un serveur de langage ou à un débogueur qu'un autre programme fait déjà tourner, comme ceux de l'éditeur Godot (`connect = "hôte:port"` à la place de `command` dans le manifeste). Tant que le serveur ne répond pas, la pastille LSP de la barre d'état indique l'adresse attendue, et l'IDE s'y connecte dès qu'il répond.

### Corrections

- Un serveur de langage qui échoue à son tout premier démarrage est relancé, comme celui qui plante plus tard, au lieu de rester éteint jusqu'au redémarrage de l'IDE.

```

- [ ] **Step 5: Checkpoint** — no commit. Suggested message: `docs: connect to running language servers and debuggers; gdscript-lang`

---

### Task A8: Verify Part A

- [ ] **Step 1: Format, lint, test (as CI does)**

Run, from PowerShell in the IDE repo:

```
rtk cargo fmt --all --check
rtk cargo clippy --workspace --all-targets -- -D warnings
rtk cargo test --workspace
```

Expected: no formatting diff (run `rtk cargo fmt --all` and re-check if there is one), no clippy warning, all tests pass.

- [ ] **Step 2: Build the IDE for the end-to-end check**

Run: `rtk cargo build --release`
Expected: `target\release\coding-unicorns.exe` is built.

- [ ] **Step 3: Checkpoint** — report results to the owner; no commit.

---

# Part B — `gdscript-lang` module

Repository: `C:\Users\alexa\dev\perso\writing-unicorns-modules`.

### Task B1: Branch

- [ ] **Step 1: Start from the latest `master`**

```
rtk git status
rtk git fetch origin
rtk git switch -c feat/gdscript origin/master
```

Expected: `git status` clean before switching (the local checkout is on `feat/docker-logs-height`; if it is not clean, stop and ask the owner). After the switch, `Cargo.toml` lists `config-lang` and `shared/highlight` exists.

### Task B2: Crate, manifest, grammars

**Files:**
- Create: `gdscript-lang/Cargo.toml`, `gdscript-lang/manifest.toml`, `gdscript-lang/src/lib.rs`, `gdscript-lang/queries/gdscript.scm`, `gdscript-lang/queries/godot-resource.scm`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Register the crate**

In the root `Cargo.toml`, add `"gdscript-lang",` after `"config-lang",` in `members`.

- [ ] **Step 2: `gdscript-lang/Cargo.toml`**

```toml
[package]
name = "gdscript-lang"
version = "0.1.0"
edition = "2021"
description = "Godot support module for Writing Unicorns IDE: GDScript, scenes, resources and shaders"

[lib]
crate-type = ["rlib", "cdylib"]

[dependencies]
unicorns-highlight = { path = "../shared/highlight" }
tree-sitter-highlight = "0.25"
tree-sitter-gdscript = "6.1"
tree-sitter-godot-resource = "0.7"
tree-sitter-gdshader = "0.4"

[dev-dependencies]
unicorns-highlight-test = { path = "../shared/highlight-test" }
```

- [ ] **Step 3: `gdscript-lang/manifest.toml`**

```toml
[extension]
id = "unicorns.gdscript-lang"
name = "GDScript"
version = "0.1.0"
description = "Godot support — GDScript, scenes, resources and shaders highlighting; completion, diagnostics and debugging through the Godot editor (open on the project)"
author = "Writing Unicorns"
repository = ""
documentation = "https://docs.godotengine.org/en/stable/tutorials/scripting/gdscript/"

[capabilities]
languages = ["gd", "tscn", "tres", "godot", "import", "gdextension", "gdshader", "gdshaderinc"]
# LSP languageId of extensions whose id differs from the extension.
language_ids = { gd = "gdscript" }

# The Godot editor runs the language server (Editor Settings → Network →
# Language Server, port 6005): the IDE connects to it (Coding Unicorns 0.21.0+).
# Scenes, resources and shaders have none.
[capabilities.lsp_servers.gd]
connect = "127.0.0.1:6005"

[dependencies]

# The Godot editor also runs the debug adapter (Network → Debug Adapter, port
# 6006). Without a launch configuration, F5 plays the main scene.
[debugger]
types = ["godot"]
connect = "127.0.0.1:6006"
install_hint = "open the project in the Godot editor (Editor Settings → Network → Debug Adapter, port 6006)"

[debugger.default_launch]
scene = "main"
```

- [ ] **Step 4: `gdscript-lang/queries/gdscript.scm`**

(Validated against tree-sitter-gdscript 6.1.0 on a sample during planning.)

```scheme
; GDScript highlighting for Coding Unicorns, written for tree-sitter-gdscript
; (whose crate ships no query). Capture names: see unicorns-highlight. When
; several patterns capture the same node, the first one wins.

(comment) @comment
[(region_start) (region_end)] @comment

; "text", &"StringName", ^"NodePath", $Node, %UniqueNode
[(string) (string_name) (node_path) (get_node)] @string
(escape_sequence) @escape

[(integer) (float)] @number
[(true) (false) (null)] @constant.builtin

((identifier) @variable.builtin
  (#any-of? @variable.builtin "self" "super"))

; @export, @onready, @tool…
(annotation "@" @attribute)
(annotation (identifier) @attribute)

(function_definition name: (name) @function)
(constructor_definition "_init" @function)
(signal_statement name: (name) @function)
(class_name_statement name: (name) @type)
(class_definition name: (name) @type)
(enum_definition name: (name) @type)
(const_statement name: (name) @constant)
(enumerator left: (identifier) @constant)
(type) @type

; foo(), obj.foo(), super.foo()
(call (identifier) @function)
(attribute_call (identifier) @function)
(base_call (identifier) @function)

[
  "and" "as" "await" "break" "class" "class_name" "const" "continue"
  "elif" "else" "enum" "extends" "for" "func" "if" "in" "is" "match"
  "not" "or" "pass" "return" "signal" "var" "when" "while" "set" "get"
] @keyword
[(static_keyword) (remote_keyword) (breakpoint_statement)] @keyword
```

- [ ] **Step 5: `gdscript-lang/queries/godot-resource.scm`**

(tree-sitter-godot-resource 0.7.0 has no `string_name` node: do not add one.)

```scheme
; Godot scene and resource files (.tscn, .tres, project.godot, .import,
; .gdextension), written for tree-sitter-godot-resource (whose crate ships no
; query). Capture names: see unicorns-highlight.

(comment) @comment
(string) @string
[(integer) (float)] @number
[(true) (false) (null)] @constant.builtin

; [gd_scene load_steps=3 format=3], [node name="Player" type="Node2D"]
(section (identifier) @keyword)
(attribute (identifier) @property)

; key = value, config/name = "…"
(property (path) @property)
(property (identifier) @property)

; Vector2(0, 0), ExtResource("1_abc"), PackedStringArray(…)
(constructor (identifier) @type)
```

- [ ] **Step 6: `gdscript-lang/src/lib.rs` with its unit tests**

```rust
//! Godot support: GDScript (`.gd`), scene and resource files (`.tscn`,
//! `.tres`, `project.godot`, `.import`, `.gdextension`) and shaders
//! (`.gdshader`, `.gdshaderinc`), highlighted with tree-sitter.
//!
//! Completion, diagnostics and debugging come from the Godot editor, which
//! runs a language server and a debug adapter the IDE connects to (`connect`
//! in `manifest.toml`).

use std::ffi::{c_char, CStr, CString};
use std::sync::OnceLock;
use tree_sitter_highlight::HighlightConfiguration;

fn gdscript() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        unicorns_highlight::configure(
            tree_sitter_gdscript::LANGUAGE.into(),
            "gdscript",
            include_str!("../queries/gdscript.scm"),
            "",
        )
    })
}

fn resource() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        unicorns_highlight::configure(
            tree_sitter_godot_resource::LANGUAGE.into(),
            "godot_resource",
            include_str!("../queries/godot-resource.scm"),
            "",
        )
    })
}

fn shader() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        unicorns_highlight::configure(
            tree_sitter_gdshader::LANGUAGE.into(),
            "gdshader",
            tree_sitter_gdshader::HIGHLIGHTS_QUERY,
            "",
        )
    })
}

/// The grammar of language `lang`, one of the keys of [`file_extensions`].
fn config_for(lang: &str) -> &'static HighlightConfiguration {
    match lang {
        "tscn" | "tres" | "godot" | "import" | "gdextension" => resource(),
        "gdshader" | "gdshaderinc" => shader(),
        _ => gdscript(),
    }
}

/// The document in language `lang` as the IDE's token lines.
pub fn document_to_json(lang: &str, source: &str) -> String {
    unicorns_highlight::document(config_for(lang), source)
}

// ── FFI ───────────────────────────────────────────────────────────────────────

unsafe fn from_c<'a>(ptr: *const c_char) -> &'a str {
    if ptr.is_null() {
        return "";
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().unwrap_or("")
}

fn into_c(s: String) -> *mut c_char {
    CString::new(s).unwrap_or_default().into_raw()
}

#[no_mangle]
pub extern "C" fn language_id() -> *const c_char {
    c"gdscript".as_ptr()
}

#[no_mangle]
pub extern "C" fn file_extensions() -> *const c_char {
    c"gd,tscn,tres,godot,import,gdextension,gdshader,gdshaderinc".as_ptr()
}

#[no_mangle]
pub extern "C" fn reset_tokenizer() {}

/// The whole document in language `lang`: `[[{text,kind},…],…]`, freed with
/// [`free_string`].
///
/// # Safety
/// Both pointers must be null or valid, nul-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn tokenize_document_lang_ffi(
    lang_ptr: *const c_char,
    text_ptr: *const c_char,
) -> *mut c_char {
    let (lang, text) = unsafe { (from_c(lang_ptr), from_c(text_ptr)) };
    into_c(document_to_json(lang, text))
}

/// The whole document as GDScript, for IDEs without the language-aware export.
///
/// # Safety
/// `text_ptr` must be null or a valid, nul-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn tokenize_document_ffi(text_ptr: *const c_char) -> *mut c_char {
    into_c(document_to_json("gd", unsafe { from_c(text_ptr) }))
}

/// Line tokenizing is not offered: the IDE uses the document tokenizers.
#[no_mangle]
pub extern "C" fn tokenize_line_ffi(_line_ptr: *const c_char) -> *mut c_char {
    std::ptr::null_mut()
}

/// # Safety
/// `ptr` must come from this module and not be freed yet.
#[no_mangle]
pub unsafe extern "C" fn free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(unsafe { CString::from_raw(ptr) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kind of the first token whose text is `text`.
    fn kind_of(json: &str, text: &str) -> Option<String> {
        let needle = format!(r#"{{"text":"{}","kind":""#, text.replace('"', "\\\""));
        let start = json.find(&needle)? + needle.len();
        let end = json[start..].find('"')? + start;
        Some(json[start..end].to_string())
    }

    #[test]
    fn each_language_gets_its_grammar() {
        let gd = document_to_json("gd", "var speed := 1\n");
        assert_eq!(kind_of(&gd, "var").as_deref(), Some("keyword"));
        let scene = document_to_json("tscn", "[node name=\"A\"]\nspeed = 1\n");
        assert_eq!(kind_of(&scene, "node").as_deref(), Some("keyword"));
        assert_eq!(kind_of(&scene, "speed").as_deref(), Some("property"));
        for lang in ["tres", "godot", "import", "gdextension"] {
            let json = document_to_json(lang, "[remap]\n");
            assert_eq!(kind_of(&json, "remap").as_deref(), Some("keyword"), "{lang}");
        }
        for lang in ["gdshader", "gdshaderinc"] {
            let json = document_to_json(lang, "uniform float a;\n");
            assert_eq!(kind_of(&json, "uniform").as_deref(), Some("keyword"), "{lang}");
        }
    }

    #[test]
    fn language_less_export_reads_gdscript() {
        let text = CString::new("func _ready():\n\tpass\n").unwrap();
        let out = unsafe { tokenize_document_ffi(text.as_ptr()) };
        let json = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        unsafe { free_string(out) };
        assert_eq!(kind_of(&json, "func").as_deref(), Some("keyword"));
        assert_eq!(kind_of(&json, "_ready").as_deref(), Some("function"));
    }
}
```

- [ ] **Step 7: Run the unit tests**

Run: `rtk cargo test -p gdscript-lang --lib`
Expected: `each_language_gets_its_grammar` and `language_less_export_reads_gdscript` pass. A query error shows up as a panic `gdscript highlight query: …` naming the bad node: fix the `.scm` line it points at.

### Task B3: Highlight snapshots

**Files:**
- Create: `gdscript-lang/tests/highlight.rs`, `gdscript-lang/tests/samples/gd/player.gd`, `tests/samples/tscn/main.tscn`, `tests/samples/godot/project.godot`, `tests/samples/import/icon.svg.import`, `tests/samples/gdshader/water.gdshader` (+ generated `.tokens`)

- [ ] **Step 1: `gdscript-lang/tests/highlight.rs`**

```rust
//! Highlighting snapshots of `tests/samples/` (see `unicorns-highlight-test`).
//! After an intended change: `UPDATE_HIGHLIGHT=1 cargo test -p gdscript-lang`.

use unicorns_highlight_test::{check_samples, Ffi};

#[test]
fn samples() {
    let ffi = Ffi {
        tokenize_document_lang: Some(gdscript_lang::tokenize_document_lang_ffi),
        tokenize_document: Some(gdscript_lang::tokenize_document_ffi),
        tokenize_line: Some(gdscript_lang::tokenize_line_ffi),
        reset_tokenizer: Some(gdscript_lang::reset_tokenizer),
        ..Ffi::new(gdscript_lang::file_extensions, gdscript_lang::free_string)
    };
    check_samples(env!("CARGO_MANIFEST_DIR"), &ffi);
}
```

- [ ] **Step 2: Samples**

`gdscript-lang/tests/samples/gd/player.gd` (indent with tabs, as Godot does):

```gdscript
@tool
class_name Player
extends CharacterBody2D
## The player: moves and jumps.

signal hit(damage: int)

enum State { IDLE, RUN }

const SPEED := 300.0
@export var max_health: int = 3
@onready var sprite: Sprite2D = $Sprite2D

var state := State.IDLE

#region Movement
func _physics_process(delta: float) -> void:
	var direction := Input.get_axis(&"move_left", &"move_right")
	velocity.x = direction * SPEED
	if not is_on_floor():
		velocity += get_gravity() * delta
	move_and_slide()
#endregion

func _init() -> void:
	pass

func take_damage(amount: int) -> void:
	max_health -= amount
	hit.emit(amount)
	match state:
		State.IDLE:
			print("ouch\n")
		_:
			pass
	await get_tree().create_timer(0.5).timeout
	var scene: PackedScene = preload("res://scenes/explosion.tscn")
	add_child(scene.instantiate())
	super.take_damage(amount)
	print(self, null, true)
	var path := ^"Sprite2D:modulate"
```

`gdscript-lang/tests/samples/tscn/main.tscn`:

```
[gd_scene load_steps=3 format=3 uid="uid://b6x3k2q1w0e8a"]

[ext_resource type="Script" path="res://player.gd" id="1_abcde"]
[ext_resource type="Texture2D" uid="uid://c7y4l3r2x1f9b" path="res://icon.svg" id="2_fghij"]

[sub_resource type="RectangleShape2D" id="RectangleShape2D_1"]
size = Vector2(32, 48)

[node name="Player" type="CharacterBody2D"]
script = ExtResource("1_abcde")
max_health = 5
metadata/_edit_group_ = true

[node name="Sprite2D" type="Sprite2D" parent="."]
texture = ExtResource("2_fghij")
modulate = Color(1, 0.5, 0.5, 1)

[node name="CollisionShape2D" type="CollisionShape2D" parent="."]
shape = SubResource("RectangleShape2D_1")

[connection signal="hit" from="." to="." method="_on_hit"]
```

`gdscript-lang/tests/samples/godot/project.godot`:

```
; Engine configuration file.
; It's best edited using the editor UI and not directly,

config_version=5

[application]

config/name="Karting"
run/main_scene="res://main.tscn"
config/features=PackedStringArray("4.3", "Forward Plus")

[autoload]

Globals="*res://globals.gd"

[display]

window/size/viewport_width=1280
window/stretch/mode="canvas_items"

[input]

move_left={
"deadzone": 0.5,
"events": []
}
```

`gdscript-lang/tests/samples/import/icon.svg.import`:

```
[remap]

importer="texture"
type="CompressedTexture2D"
uid="uid://c7y4l3r2x1f9b"
path="res://.godot/imported/icon.svg-218a8f2b3041327d8a5756f3a245f83b.ctex"
metadata={
"vram_texture": false
}

[deps]

source_file="res://icon.svg"
dest_files=["res://.godot/imported/icon.svg-218a8f2b3041327d8a5756f3a245f83b.ctex"]

[params]

compress/mode=0
compress/lossy_quality=0.7
```

`gdscript-lang/tests/samples/gdshader/water.gdshader`:

```glsl
shader_type canvas_item;
render_mode blend_mix;

/** Speed of the waves. */
uniform float speed : hint_range(0.0, 10.0) = 1.0;
uniform sampler2D noise : repeat_enable;
varying vec2 world_uv;

// Wobble the vertices.
void vertex() {
	world_uv = VERTEX * 0.01;
}

void fragment() {
	vec4 base = texture(TEXTURE, UV);
	float wave = sin(TIME * speed + world_uv.x * 6.28);
	COLOR = vec4(base.rgb * (0.8 + 0.2 * wave), base.a);
}
```

- [ ] **Step 3: Generate the snapshots and review them**

Run (PowerShell): `$env:UPDATE_HIGHLIGHT = "1"; rtk cargo test -p gdscript-lang --test highlight; Remove-Item Env:UPDATE_HIGHLIGHT`

Then read each generated `.tokens` file and check these lines (what the prototype produced during planning):
- `player.gd`: `@tool` / `@export` / `@onready` → property; `class_name`, `extends`, `signal`, `func`, `var`, `match`, `await`, `pass` → keyword; `Player`, `CharacterBody2D`, `int`, `void`, `PackedScene` → type; `_physics_process`, `_init`, `get_axis`, `preload`, `emit` → function; `&"move_left"`, `$Sprite2D`, `^"Sprite2D:modulate"` → string; `self`, `super`, `null`, `true`, `IDLE`, `SPEED` → keyword; `#region Movement`, `## The player…` → comment.
- `main.tscn`: `gd_scene`, `ext_resource`, `node`, `connection` → keyword; `load_steps`, `type`, `name`, `size`, `metadata/_edit_group_` → property; `Vector2`, `ExtResource`, `Color` → type; quoted values → string; numbers → number.
- `project.godot`: `; Engine configuration file.` → comment; `application`, `input` → keyword; `config/name`, `move_left` → property; `PackedStringArray` → type.
- `icon.svg.import`: `remap`, `deps`, `params` → keyword; `compress/mode` → property; `false` → keyword.
- `water.gdshader`: `shader_type`, `canvas_item`, `uniform`, `varying`, `VERTEX`, `TIME`, `COLOR` → keyword; `float`, `vec2`, `vec4`, `sampler2D`, `void` → type; `vertex`, `fragment`, `texture`, `sin` → function; `/** Speed of the waves. */` → comment.

- [ ] **Step 4: Run all module tests**

Run: `rtk cargo test -p gdscript-lang`
Expected: pass (snapshots match, invariants hold: tokens rebuild each line, one token line per source line, no sample left uncoloured).

### Task B4: Registry script and README

**Files:**
- Modify: `scripts/build_registry.py`, `README.md`

- [ ] **Step 1: `scripts/build_registry.py`** — in `module_entry`, replace

```python
        # A module whose servers are all per language (java-lang) names the first.
        "lsp_server": caps.get("lsp_server")
        or next((s.get("command") for s in caps.get("lsp_servers", {}).values()), None),
```

with

```python
        # A module whose servers are all per language (java-lang) names the
        # first; one reached over TCP (gdscript-lang) gives its address.
        "lsp_server": caps.get("lsp_server")
        or next(
            (s.get("command") or s.get("connect") for s in caps.get("lsp_servers", {}).values()),
            None,
        ),
```

Check it (Python 3.11+ for `tomllib`):

```
py -3.13 -I -c "import sys; sys.path.insert(0, 'scripts'); import build_registry as b; from pathlib import Path; e = b.module_entry(Path('.'), 'gdscript-lang', Path('no-zips'), 'v0', 'Ajustor/coding-unicorns-modules'); print(e['lsp_server'], e['languages'])"
```

Expected: `127.0.0.1:6005 ['gd', 'tscn', 'tres', 'godot', 'import', 'gdextension', 'gdshader', 'gdshaderinc']`.

- [ ] **Step 2: `README.md`**

In the modules table, after the `config-lang` row add:

```markdown
| `gdscript-lang` | GDScript, Godot scenes, resources and shaders | `.gd`, `.tscn`, `.tres`, `project.godot`, `.import`, `.gdextension`, `.gdshader`, `.gdshaderinc` | Godot editor | [docs](https://docs.godotengine.org/en/stable/tutorials/scripting/gdscript/) |
```

After the paragraph about `docker-lang`'s panel, add:

```markdown
`gdscript-lang` ships no language server or debugger: the Godot editor runs both, on TCP
ports 6005 (LSP) and 6006 (DAP). Its manifest declares them with `connect` (Coding
Unicorns 0.21.0+), so completion, diagnostics and F5 work while the Godot 4 editor is open
on the project, and the IDE connects as soon as it is. F5 without a launch configuration
plays the main scene; a `launch.json` entry of type `godot` can set `scene` to `current`
(the scene open in Godot) or a `res://` path. Scenes, resources and shaders are only
highlighted.
```

In `## Debuggers`, append to the examples sentence (`Examples: \`csharp-lang\` (downloaded binary), …`) the item `` `gdscript-lang` (`connect`: the adapter already runs in the Godot editor) ``, and in the sentence `Without a launch configuration, F5 debugs the current file for …` add `GDScript (the main scene)` to the list.

- [ ] **Step 3: Checkpoint** — no commit.

### Task B5: Verify Part B

- [ ] **Step 1: Format, lint, test, build**

From PowerShell in the modules repo:

```
rtk cargo fmt -p gdscript-lang --check
rtk cargo clippy -p gdscript-lang --all-targets -- -D warnings
rtk cargo test --workspace
rtk cargo build --release -p gdscript-lang
```

Expected: clean; all workspace tests pass; `target\release\gdscript_lang.dll` exists.

- [ ] **Step 2: Load the real module through the IDE's loader**

From the IDE repo (PowerShell):

```
New-Item -ItemType Directory -Force "$env:TEMP\cu-gd" | Out-Null
Copy-Item ..\writing-unicorns-modules\gdscript-lang\manifest.toml "$env:TEMP\cu-gd\"
Copy-Item ..\writing-unicorns-modules\target\release\gdscript_lang.dll "$env:TEMP\cu-gd\"
$env:CU_E2E_MODULE_DIR = "$env:TEMP\cu-gd"
rtk cargo test real_module_from_env -- --ignored --nocapture
Remove-Item Env:CU_E2E_MODULE_DIR
```

Expected: the manifest parses (connect validated) and the library loads; the test passes. (`real_module_from_env` unwraps `lsp_server_command_for(lang)` for every language: if it panics on `gd` because the plugin path has no server for a connected language, change that loop to print `p.lsp_server_command_for(lang)` with `{:?}` instead of unwrapping — the fallback path is expected to return `None` here.)

- [ ] **Step 3: Checkpoint** — report to the owner; no commit.

---

# Part C — End to end with Godot (with the owner)

Needs Godot 4 and a project; `C:\Users\alexa\dev\perso\games\karting` is one.

- [ ] **Step 1:** Start the release IDE built in Task A8 (`target\release\coding-unicorns.exe`). Extensions panel → **⚙ BUILD FROM SOURCES** → path `C:\Users\alexa\dev\perso\writing-unicorns-modules` → select `gdscript-lang` → install.
- [ ] **Step 2:** Open `karting` as the workspace, Godot closed. Open a `.gd` file: highlighting works; the LSP dot is orange and its tooltip reads `Waiting for the language server at 127.0.0.1:6005…`.
- [ ] **Step 3:** Open the project in the Godot editor. Within a few seconds the dot turns green; completion (`Input.` → members), hover on a built-in, go-to-definition on a project function, and a diagnostic on a deliberate error (e.g. `var x: int = "a"`) all work.
- [ ] **Step 4:** Close Godot: the dot goes back to orange (waiting). Reopen it: the IDE reconnects without any action.
- [ ] **Step 5:** Put a breakpoint in a script used by the main scene, press F5: the game starts, stops on the breakpoint, the call stack and variables show; continue and stop work.
- [ ] **Step 6:** Close Godot, press F5: the error reads `Cannot reach the debug adapter at 127.0.0.1:6006: open the project in the Godot editor (…)`.
- [ ] **Step 7:** Open a `.tscn`, a `.tres`, `project.godot` and a `.gdshader`: all highlighted; no LSP dot for them.
- [ ] **Step 8:** Report what worked and what did not to the owner. Problems found here (e.g. Godot rejecting a request) are fixed with `superpowers:systematic-debugging` before any PR.

---

## Delivery (only when the owner asks)

1. IDE: commit on `feat/connect-servers`, push, open a PR (body without attribution). The owner releases the next IDE version (its `[Unreleased]` changelog section becomes the version heading).
2. Modules: commit on `feat/gdscript`, push, open a PR. If the IDE release that ships `connect` is not 0.21.0, update the version in `gdscript-lang/manifest.toml`'s comment and the modules README first.
