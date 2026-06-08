# Claude Code Integration — Design Spec

**Date:** 2026-06-08
**Status:** Approved (design), pending implementation plan
**Target:** Coding Unicorns editor (`writingUnicorns`)

## 1. Goal

Add a native chat/agent panel inside the editor that drives the installed
`claude` CLI (Claude Code) to provide an **agentic assistant with per-action
confirmation**: Claude can read the workspace, propose and apply edits, and run
commands, but every sensitive tool use is gated by an in-app allow/deny dialog.

Non-goals: no Anthropic API key handling (reuse the user's existing `claude`
login), no Node/Python runtime, no reimplementation of Claude Code features.

## 2. Decisions (locked)

| Topic | Decision |
|---|---|
| Shape | Native egui chat panel (not a raw PTY terminal) |
| Capability | Agentic **with per-action confirmation** |
| Architecture | **A** — `claude` CLI + Rust-hosted MCP permission tool |
| Turn model | One `claude -p` process per turn, continuity via `--resume <session_id>` |
| Streaming | `--output-format stream-json --verbose` (documented output schema) |
| Placement | Right dock (`SidePanel::right`), resizable |
| Auto-context | Current file path + current selection (with line numbers) |
| Auth | Reuse existing `claude` login; no API key in-app |
| Runtime deps | None new — `serde_json`, `std::net`, `std::process` only |

CLI verified present: `claude` v2.1.168.

## 3. Why per-turn + MCP permission (constraints)

- `--output-format stream-json` (stdout) is documented: emits JSON-lines events
  — `system/init` (carries `session_id`), assistant text deltas, `tool_use`,
  and a final `result`.
- `--input-format stream-json` (stdin, multi-turn) is **undocumented and
  fragile**. We avoid it: each user message is a fresh `claude -p "<msg>"`
  invocation, with conversation continuity via `--resume <session_id>`.
- Per-action confirmation, CLI-native, uses `--permission-prompt-tool
  <mcp_tool>`: Claude calls a host-provided MCP tool to request permission.
  Because that MCP tool runs as a process spawned **by `claude`**, the editor
  re-invokes **its own binary** in a "permission server" mode; that child
  relays the request to the main editor process over local IPC and returns the
  user's decision to `claude`.

## 4. Architecture

```
[egui panel] --message--> [ClaudeSession] --spawn--> claude -p
     ^                          ^                       --output-format stream-json --verbose
     |                     events (stdout)              --resume <session_id>
     |                          |                       --permission-prompt-tool mcp__editor__approve
  transcript <--- parse stream-json <--------------     --mcp-config <tmp.json>
     |                                                  --add-dir <workspace>
     |                                                         |
     |                                                  spawns MCP server:
 [allow/deny dialog] <--IPC (localhost TCP + token)--  `coding-unicorns --claude-permission-server
                                                            --port <P> --token <T>`
```

Process tree: editor → `claude` (child) → permission MCP server (grandchild,
= the editor binary re-invoked). The grandchild connects back to the editor's
IPC listener to surface each permission request.

## 5. Components

New module `src/claude/`:

### 5.1 `protocol.rs`
Serde types + parser for the stream-json events the host consumes:
- `init { session_id, model, tools }`
- assistant text delta (incremental text)
- `tool_use { id, name, input }`
- `result { result, total_cost_usd, session_id }`
- unknown event types are ignored (forward-compatible).
Pure functions; unit-tested against captured sample lines.

### 5.2 `process.rs`
- `spawn_turn(req) -> TurnHandle`: builds the `claude` argv, sets cwd =
  workspace, env `NO_COLOR=1` + IPC `port`/`token`, `stdin=null`,
  `stdout=piped`, `stderr=piped`.
- Reader thread parses stdout lines → `ClaudeEvent` over a
  `crossbeam_channel` (same pattern as `lsp/transport.rs`).
- stderr drained to `log` (shown only on error).
- `cancel()` kills the child (turn abort).

### 5.3 `session.rs`
- `ClaudeSession`: ordered transcript of `Message { role, content, tool_calls }`,
  `session_id: Option<String>`, current turn state (idle/running), accumulated
  cost.
- `build_prompt(user_text, ctx)`: prepends a compact context header — current
  file path and, when a selection exists, the selected text with start/end line
  numbers.
- Captures `session_id` from the first turn's `init` event and reuses it via
  `--resume` on later turns.
- `poll(events)`: applies channel events to the transcript.
Pure logic (prompt building, session_id capture) is unit-tested.

### 5.4 `permission.rs`
- Editor side: `PermissionListener` — binds `127.0.0.1:0` (ephemeral port),
  authenticates clients with a per-session random token, receives
  `{tool, input}` requests, and exposes them to the UI as pending
  `PermissionRequest`s; the UI's decision is sent back.
- Server side: `run_permission_mcp_server(port, token)` — a minimal stdio MCP
  server (hand-rolled JSON-RPC: `initialize`, `tools/list`, `tools/call`)
  exposing one tool `approve`. On `tools/call` it forwards to the editor over
  IPC and returns the allow/deny result to `claude`.
- `write_mcp_config(path, port, token) -> PathBuf`: emits the temp
  `--mcp-config` JSON pointing at `coding-unicorns --claude-permission-server
  --port <P> --token <T>`.

### 5.5 `src/ui/claude_panel.rs`
`ClaudePanel::show(ctx, &mut ClaudeSession, editor_ctx)` rendering a
`SidePanel::right`:
- Transcript with lightweight markdown rendering; assistant text streams in.
- Tool-use entries shown compactly (tool name + summary of input / diff /
  command).
- Inline **permission dialog** when a request is pending: tool name + input
  detail, buttons **Allow** / **Deny** / **Always allow this tool (session)**.
- Input box (Enter to send, Shift+Enter newline), Cancel button while running,
  "New conversation" button (drops `session_id`), status line (idle / running /
  cost).

## 6. Wiring into the app

- `main.rs`: detect `--claude-permission-server` early; if present, run
  `claude::permission::run_permission_mcp_server(...)` and **exit without
  starting the GUI** (also skip the panic-logger GUI assumptions / icon).
- `app/mod.rs`: own `claude_session: ClaudeSession`, `claude_panel`
  state, and `show_claude: bool`. Each frame: poll the events channel, drain
  pending permission requests, and `ctx.request_repaint()` while a turn runs.
- `ui/layout.rs`: render the right dock when `show_claude`; place it as a
  `SidePanel::right` so it composes with the existing central/editor area.
- Toggle: command palette entry `ToggleClaude` + keybinding (default
  `Ctrl+Shift+I`), mirroring `ToggleTerminal`.
- `config`: add `claude_binary: String` (default `"claude"`) and
  `claude_auto_allow_read: bool` (default `true` — auto-approve the read-only
  `Read`/`Glob`/`Grep` tools; everything else prompts).

## 7. Data flow (one turn)

1. User types a message and presses Enter.
2. `session.build_prompt` prepends the context header (current file path +
   selection with line numbers).
3. Editor ensures the `PermissionListener` is up; writes the temp mcp-config.
4. `process.spawn_turn` launches `claude -p "<prompt>"
   --output-format stream-json --verbose [--resume <id>]
   --permission-prompt-tool mcp__editor__approve --mcp-config <file>
   --add-dir <workspace>` with cwd = workspace.
5. Reader thread streams events → channel; the app appends assistant text to
   the transcript and captures `session_id` from `init`.
6. On a sensitive `tool_use`, the MCP grandchild calls back over IPC → the app
   shows the permission dialog → user decides → reply flows back → `claude`
   proceeds or skips the tool.
7. The `result` event ends the turn; the child exits; the panel returns to idle
   (cost updated).

## 8. Error handling

- `claude` not found on PATH → friendly panel message + install link (reuse the
  `shell_exists` PATH-probe pattern from `terminal/shell.rs`).
- Non-zero exit or unparseable output → error line in the transcript; the
  session stays usable.
- Permission IPC timeout (no UI response within a bound, e.g. 5 min) →
  **default deny**.
- "Always allow this tool" decisions are remembered for the current session
  only (in-memory), never persisted.
- Cancel → kill the child process; mark the turn aborted.
- stderr is logged, surfaced to the user only when a turn fails.

## 9. Testing

- `protocol.rs`: unit tests parsing representative stream-json lines (init,
  text delta, tool_use, result, unknown) into `ClaudeEvent`s.
- `session.rs`: unit tests for `build_prompt` (file-only, file+selection, no
  context) and `session_id` capture/reuse.
- `permission.rs`: round-trip allow/deny/always over the IPC framing; mcp-config
  JSON generation.
- End-to-end interactive behavior (real `claude`, real permission dialog) is
  verified manually — documented as a manual test checklist, not automated.

## 10. Lightness

No new heavyweight dependencies. Reuses `serde_json` (already present),
`std::net` for IPC, `std::process` for spawning. The MCP JSON-RPC server is a
minimal hand-rolled implementation (no MCP crate). Consistent with the project's
"lightest possible" goal — the only new runtime requirement is the `claude` CLI
the user already has.

## 11. Out of scope (YAGNI)

- Streaming user input over stdin (`--input-format stream-json`).
- Persisting "always allow" decisions across app restarts.
- Diagnostics auto-context (only file + selection were chosen).
- Multiple concurrent Claude sessions / tabs (one session per panel for now).
- Rendering Claude's full TUI; this is a native panel, not a PTY.
