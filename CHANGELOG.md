# Changelog

Notable changes to Coding Unicorns, version by version. Each section is also
shown in the app's update dialog and on the
[download page](https://ajustor.github.io/codingUnicorns/). The French
translation, [`CHANGELOG.fr.md`](CHANGELOG.fr.md), is kept in step with it.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
versions follow [Semantic Versioning](https://semver.org/).

## [0.11.4] - 2026-10-10

### Fixed

- macOS: the download page now prominently tells macOS 26 (Tahoe) users to drag the app to their Desktop or any folder **not** named Applications — both `/Applications` and `~/Applications` are blocked by `syspolicyd` for ad-hoc-signed apps on macOS 26. On macOS 15 and earlier, dragging to Applications still works.

## [0.11.3] - 2026-10-10

### Fixed

- macOS 26 (Tahoe): the app no longer hangs silently at launch. `syspolicyd` on macOS 26 blocks ad-hoc-signed apps (no Apple Developer ID) placed in any folder named "Applications" — `/Applications` or `~/Applications` — suspending the process in the dynamic linker before any Rust code runs. **On macOS 26, drag the app to any other location after opening the disk image** — for example `~/Desktop` or a `~/Apps` folder you create. On macOS 15 and earlier, `/Applications` and `~/Applications` still work.
- Build: added minimum Rust version (`rust-version = "1.85"` in `Cargo.toml`) so a build with an older toolchain fails with a clear message instead of a cryptic dependency error (`edition2024` not stabilised before Rust 1.85).

## [0.11.2] - 2026-10-10

### Fixed

- macOS: when the app cannot start, it no longer vanishes without a word. A dialog shows the error (a crash, or a window that cannot be opened), and every launch is logged step by step in `~/Library/Logs/Coding Unicorns/launch.log` (also listed in Console.app). Started from a terminal (`cu`), the app prints the error there if it stops within its first seconds.

## [0.11.1] - 2026-10-10

### Fixed

- macOS: the app could stay stuck at startup without opening its window, when a shell startup file (`.zshrc`…) left a process running in the background (ssh-agent, gpg-agent, a plugin's update check...).

## [0.11.0] - 2026-10-09

### Added

- macOS: a real app, **Coding Unicorns.app**, installed from a disk image (`coding-unicorns-macos-arm64.dmg`: drag it into Applications). Started from the Finder or the Dock, it takes the `PATH` of your login shell, so the terminal, the language servers and Claude find your tools (Homebrew, `~/.cargo/bin`...).
- Linux: an AppImage (`coding-unicorns-linux-x64.AppImage`), with its icon and menu entry, which updates itself in place.
- The macOS app and the AppImage put the `cu` command on your `PATH` when they start (a link in `/usr/local/bin` or `~/.local/bin`, added to your shell's startup file if needed): open a new terminal and run `cu .`.

## [0.10.10] - 2026-10-09

### Added

- The command palette lists the commands you ran last first (the last 5, kept across restarts): Enter runs the latest again.

### Changed

- Module logs (such as the Docker container logs) fill the height of their tab or panel, instead of a fixed 520-pixel box.

## [0.10.9] - 2026-10-09

### Fixed

- The Settings page opens instantly: listing the available shells no longer starts a `where` / `which` process per shell (about 100 ms each, more with an antivirus), which froze the window the first time the page opened. New terminals start faster for the same reason.
- **The split editor can be opened again.** Its only way in was Ctrl+\, which AZERTY and other layouts type with AltGr (Ctrl+Alt): it never matched there. A split button now sits at the right of the tab bar (a close button in the right pane), and the palette has *Split Editor: Toggle*.
- In the split view, the active pane is the one you type in: files opened from the explorer go to that pane. Before, clicking into an editor did not change it, so files opened in the wrong pane.
- Splitting without an open file says so, instead of opening and closing the split at once.
- Notifications ("Checking for updates…", "… is up to date") and the update dialog are readable: after the first frames of a window still small at startup, they kept that width and wrapped their text a few letters per line, in a narrow column in the middle of the editor.

## [0.10.8] - 2026-10-09

### Added

- **Update all** sits at the top of the Extensions panel, next to *Check for updates*, so it is visible without scrolling.
- Module views can show logs: a `log` element (monospace, selectable, scrolled to its end).
- The **Docker** module (0.2.0) gets **Containers & logs**: an editor tab listing containers with Start, Stop, Restart, Remove and Shell, and a logs tab that follows the chosen container.

### Fixed

- **Updates install again.** After *Install*, the update only applied if *Restart now* or *When I quit* was clicked; closing the window otherwise left the downloaded installer unused. It now installs whenever the app quits (*Restart now* still installs and reopens right away). Each step is written to `update.log` in the update folder, and the installer's own log to `install.log`.
- HTTPS requests (module registry, downloads, updates) trust the system's certificates. Behind a corporate proxy or an antivirus that inspects HTTPS, the registry failed with "invalid peer certificate: UnknownIssuer". Language server installs through npm use the system's certificates too (Node 22.15+ / 23.8+); git, pip, cargo, go and dotnet already did.

## [0.10.7] - 2026-10-09

### Added

- Modules can add interfaces to the IDE: sidebar panels with their own activity bar icon, and pages opened as editor tabs (`[[panels]]` in `manifest.toml`). The module returns JSON views (text, buttons, inputs, lists, tables…) that the IDE draws, receives the clicks, and can show toasts, run a command in a terminal or open a page.
- The **Docker** module gets a **Docker** panel: list local images, pull, run in a terminal, remove and prune them, with a full table in a page.
- One module can now use a different language server per language (`[capabilities.lsp_servers.<language>]`) and receive the file's language in its tokenizer and hover (`*_lang_ffi` exports). The Dockerfile and Docker Compose modules are merged into one **Docker** module.
- **Uninstall** buttons are easier to find: a red button in the *Installed* section, now open by default, and on installed modules in the registry. A failed uninstall is reported instead of only being logged.

### Fixed

- **macOS: the app starts again without Homebrew.** It required Homebrew's OpenSSL (`Library not loaded: /opt/homebrew/opt/openssl@3/lib/libssl.3.dylib`) and aborted at launch on machines without it. OpenSSL and libgit2 are now built into the macOS and Linux binaries, and the release checks that no such library is linked.
- The selected entry of the command palette is readable: it is drawn in the theme's contrast color on a solid accent background, instead of accent text on an accent tint.
- On Linux and macOS, cancelling a debug launch also stops the command its `preLaunchTask` started, instead of leaving it running.

## [0.10.6] - 2026-10-09

### Added

- **Update all** button in the Extensions panel (*Installed*), next to *Check for updates*: updates every module that has a newer version in one click.
- New **Dockerfile** module (`Dockerfile`, `Containerfile`, `Dockerfile.*`, `.dockerfile`): highlighting of instructions, flags, variables, parser directives and heredocs, hover docs for every instruction, and docker-langserver for validation, completion and formatting.
- New **Docker Compose** module (`compose.yaml`, `docker-compose.yml` and their `*.override.yml` variants): YAML highlighting with `${VAR}` interpolation, anchors and block scalars, hover docs for Compose keys, and Microsoft's compose-language-service for validation, completion and hover.
- Modules can claim files by name rather than extension (`file_names` in `manifest.toml`, `*` wildcards). The language server, status bar and debugger now follow it, so a `Dockerfile` gets its language server.

### Fixed

- Uninstalling a module whose npm dependency is scoped (`@scope/package`) now removes that package.

## [0.10.5] - 2026-10-08

### Added

- Language modules ship their debugger: a `[debugger]` section in `manifest.toml` names the adapter, the launch configuration types it serves, and optionally an archive to download it from. Adapters can talk over stdio or TCP, and the child sessions an adapter asks for (`startDebugging`) are opened automatically. The IDE itself contains no debugger.
- Official modules updated, offered in the Extensions panel: **C#** debugs with netcoredbg, **TypeScript, JavaScript, React, Vue, Svelte and HTML** with vscode-js-debug (Node programs, or pages in Chrome/Edge), **PowerShell** with PowerShell Editor Services (all three downloaded on first use), **Python** with debugpy (installed with the module), **Go** with Delve (installed with the module) and **Rust** with lldb-dap (from LLVM).
- Text a debugger prints outside the protocol shows in the debug output.
- New **JSON** module (`.json`, `.jsonc`, `.json5`, `.geojson`, `.webmanifest`): highlighting with keys distinct from values, comments and trailing commas, hover docs for `package.json` and `tsconfig.json` keys, and vscode-json-language-server for validation (including `$schema`) and formatting.
- Modules can color tokens as `property`, `operator` and `class`, and pass `initializationOptions` to their language server (`lsp_init_options`).

### Changed

- The LSP language id of a file extension (e.g. `cs` → `csharp`) now comes from its module (`language_ids` in `manifest.toml`) instead of a table in the IDE. **Update your modules** from the Extensions panel: older C#, Python and Rust modules don't declare it.

### Fixed

- **The debugger now starts.** F5 used to do nothing at all: no installed module provided a debug adapter, and the error was silent.
- The `preLaunchTask` of a launch configuration (e.g. `build` from `.vscode/tasks.json`) runs before the program starts, with its output in the debug panel. A failed build stops the launch.
- Launch problems (missing debugger, failed build, adapter error) are shown in the debug panel instead of being lost.
- The debug panel updates on its own when a breakpoint is hit, without having to move the mouse.
- A finished session can be restarted with F5 without pressing Stop first; Stop also cancels a running build.
- Debug adapters, and the programs they run, are stopped when the IDE quits.

## [0.10.4] - 2026-10-08

### Changed

- `Ctrl+Shift+F` searches the workspace for the selected text (single line, matched literally in regex mode) and focuses the search field.
- **Format document is now `Shift+Alt+F`**, as in VSCode: `Ctrl+Shift+F` used to format the file *and* open the workspace search at the same time.

## [0.10.3] - 2026-10-08

### Fixed

- Freezes every couple of seconds while editing or building in a large repository: the automatic git refresh added in 0.10.2 now runs in the background.
- Freeze while the mouse rested on an unresolved word: the hover lookup no longer reads the workspace's files, and is attempted once per word instead of on every frame.
- The terminal no longer redraws continuously while visible, and only draws the rows in view instead of its whole scrollback.
- Plugins (status bar word count) only run when the text, file or cursor changes, instead of copying the whole file on every frame.
- Workspace search no longer crashes on result lines with accented characters.

## [0.10.2] - 2026-10-08

### Added

- Click in the gutter, left of the fold markers, to add or remove a breakpoint (a faint dot previews it on hover); also in the right pane of a split editor.
- The current branch, its ahead/behind counts and the changed files refresh by themselves after a `git checkout`, `commit`, `pull`… made outside the IDE. Cheap: only a few files of the git dir are compared, at most every 2 seconds.

### Fixed

- Language servers, debug adapters, extension installs and the Claude CLI no longer open a console window on Windows (e.g. with the C# extension).
- "Restart now" after an MSI update always opens a new window, and reopens the right folder.
- A relative workspace saved by an older version (`cu .`) is ignored instead of opening the wrong folder (such as the install folder, with `cu.cmd`).

## [0.10.1] - 2026-10-08

### Fixed

- `cu` gives the terminal back right away: the IDE keeps running in the background, detached from the terminal (`--wait` keeps it in the foreground on Linux and macOS).
- `${workspaceFolder}` and `${workspaceRoot}` are the full absolute path of the opened folder, including after `cu .`, in run and debug configurations.

## [0.10.0] - 2026-10-08

### Added

- **Branch management in the Git panel**, with visible buttons on top of the right-click menu:
  - create a branch from HEAD (even detached), from another branch or from a commit of the graph, optionally checking it out right away;
  - rename any branch, including the current one;
  - delete a branch after confirmation, with a warning when its commits are not merged;
  - branch names checked as you type, `Enter` to confirm, `Esc` to cancel.
- New download page: light/dark theme, the file for your system highlighted, SHA-256 checksums and version history.

### Fixed

- Sidebar panels no longer grow wider on every frame.

## [0.9.0] - 2026-10-06

### Added

- Browse and install extensions from the online registry, right from the Extensions panel.
- Files open in the running window instead of a new one.
- Windows: Coding Unicorns is offered in "Open with" for text files.

### Fixed

- Follow the modules repository's new name (`coding-unicorns-modules`): existing configurations are migrated automatically.
- A plugin registered under a name already in use replaces the old one instead of being added next to it.

## [0.8.0] - 2026-10-06

### Added

- **Editor**: soft word wrap, auto-indent on `Enter` and dedent on a closing bracket.
- **Files**: external changes are detected and guarded before a save overwrites them; line endings and BOM are preserved, invalid UTF-8 is tolerated.
- **Session**: open tabs, cursor and scroll restored per workspace; "Open Recent" list.
- **Git**: stash (save, apply, pop, drop), Fetch button, merging diverged history on pull, authentication for fetch and push.
- **Search**: regex and whole-word modes in workspace search.
- **LSP**: file and workspace symbol search in the palette, workspace-wide Problems panel.
- **Terminal**: select and copy with the mouse, bracketed paste, `Ctrl+Shift+V` and right-click paste, PTY sized to the panel.
- **Debug**: watch expressions, every DAP scope with on-demand expansion.
- **Run**: imports `.vscode/launch.json` when there is no `launch.toml`.
- New unicorn icon, embedded in the Windows executable; executable and installer are signed.

### Fixed

- The terminal snaps back to the prompt when typing after scrolling up.
- Find in file takes focus and steps through every match.

## [0.7.2] - 2026-10-05

### Fixed

- The app restarts correctly after an MSI update.

## [0.7.1] - 2026-10-05

### Fixed

- Only the current platform's library is loaded for an extension.

## [0.7.0] - 2026-10-05

### Fixed

- **Editor**: undo granularity, go to line, duplicate line, CRLF join, multi-cursor delete, search and replace, find highlighting.
- **Copy, cut and paste** work in the editor and the terminal.
- **Git**: merge commits, staging new folders, checking out a remote or detached branch.
- **Navigation**: `Alt+←` / `Alt+→` walk the history correctly.
- **LSP and debug**: valid file URIs, `launch` sent after DAP initialization.
- Extension safety: ids and zip archives can no longer escape their folder.

## [0.6.0] - 2026-10-03

### Added

- **Automatic updates** distributed through GitHub Pages, checked with SHA-256.
- **Built-in Claude Code**: chat panel on the right, permissions, active account, interactive mode in a terminal.
- **Refreshed interface**: light theme, colors derived from the theme, toast notifications, themed status bar, confirmation before deleting files.
- **Markdown**: native side-by-side preview.
- **Editor**: current line and active indent guide highlighted, triple-click selects the line, horizontal scrolling, unified completion with match highlighting.
- Central keybinding registry with conflict detection.

### Changed

- Non-blocking LSP server startup, with a "Loading" status while they work.
- Much faster highlighting and minimap on large files.

## [0.5.0] - 2026-04-10

### Added

- Editing experience improvements: minimap, highlighting, scrolling and new aliases.

## [0.4.0] - 2026-03-20

### Fixed

- Windows installer fix.

## [0.3.0] - 2026-03-19

### Added

- First in-app update system.

## [0.2.0] - 2026-03-18

### Added

- Debugging through the Debug Adapter Protocol (DAP).

## [0.1.0] - 2026-03-15

- First release.
