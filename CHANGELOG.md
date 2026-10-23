# Changelog

Notable changes to Coding Unicorns, version by version. Each section is also
shown in the app's update dialog and on the
[download page](https://ajustor.github.io/codingUnicorns/). The French
translation, [`CHANGELOG.fr.md`](CHANGELOG.fr.md), is kept in step with it.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed

- macOS: shortcuts use Cmd instead of Ctrl (Cmd+S, Cmd+P, Cmd+C, Cmd+Z, Cmd+click to go to a definition…), and menus, the command palette and the shortcut list show Cmd. Ctrl+Space (completion), Ctrl+` (terminal) and Ctrl+Tab (next tab) stay on Ctrl, since macOS keeps Cmd+Space and Cmd+` for itself. In the terminal, Cmd+C copies and no longer interrupts the running command; Ctrl+C still does.

## [0.23.3] - 2026-10-23

### Fixed

- Tab with several lines selected indents them by one level instead of replacing the selection with a tab, and Shift+Tab removes one level from the selected lines (or the current line). Blank lines are left alone, the selection stays on the shifted text, and one Ctrl+Z undoes it.

## [0.23.2] - 2026-10-22

### Fixed

- Windows: "Restart now" in the update dialog installs the update even when the app was not started from a terminal. It looked PowerShell up on the `PATH`, and when a `PATH` entry was broken (stray quotes), nothing was installed; it now uses PowerShell's full path, and still installs the update if the relaunch can't be prepared.
- Selecting with the mouse lands under the pointer along the whole line. Columns were measured a fraction of a pixel too wide, so the caret drifted left of the pointer, about one column every 30 characters.
- After scrolling sideways, the caret, the selection and the underlines stay on the text instead of being shifted by the scrolled distance.

## [0.23.1] - 2026-10-22

### Fixed

- F2 in the explorer renames the selected file inline, starting from its current name. It used to open Rename Symbol, which did nothing without a language server; that dialog now says so instead of closing silently.
- In Report an Issue, picking Bug or Idea no longer sends what you type next into the open file: the title field keeps the focus. More generally, the editor no longer takes the keyboard back while a dialog is open over it.

## [0.23.0] - 2026-10-19

### Changed

- `cu .` (or `cu <folder>`) opens the folder in a new window, unless a window already has that folder open, which then comes to the front. It used to replace the project of the running window. A file still opens in the window whose folder holds it, else in the most recent window.

## [0.22.3] - 2026-10-19

### Fixed

- In the delete confirmation, Enter deletes (Tab moves to Cancel), instead of typing a new line in the open file.

## [0.22.2] - 2026-10-19

### Fixed

- In the explorer, Ctrl+V pastes a copied file once, instead of twice.
- In the explorer, the Delete key asks to delete the selected file, like Delete File in its context menu.

## [0.22.1] - 2026-10-19

### Fixed

- With the find bar open, selecting text in the editor and pressing Ctrl+C copies the selection instead of going to the find field.
- In workspace search, two files with the same name are listed separately, with their folder next to the name: opening one no longer folds or unfolds the other.
- In the explorer, Ctrl+C then Ctrl+V on a file copies it next to the original as `name-copie.ext`, instead of duplicating the editor's current line.

## [0.22.0] - 2026-10-16

### Added

- Pull requests: the Git panel lists the open pull requests of the repository's GitHub `origin`. Opening one shows its description, the files it changes and a side-by-side diff with the review comments under the lines they are about. F7 / Shift+F7 jump between changes, a checkbox marks each file as viewed (a chevron folds or unfolds a file either way), and Checkout puts the pull request on a local branch. Set a token in Settings → GitHub, or let the IDE use the GitHub CLI's login (`gh auth login`).

### Fixed

- Fetch, Pull, Push and opening a pull request no longer fail on SSH remotes with keys the built-in SSH client can't read (such as ECDSA keys on Windows): the IDE then goes through the installed `git`, like the terminal. Every private key in `~/.ssh` is tried, and the message only mentions a passphrase when the key really has one.

## [0.21.1] - 2026-10-15

### Fixed

- New File (Ctrl+N) opens an empty document you can type in right away, instead of showing the welcome screen again.
- Opening another folder (Open Folder, Open Recent, or `cu .` from another directory) closes the files of the previous one, even when the new folder has no saved tabs or a file has unsaved changes. Files with unsaved changes stay open, and a message says how many.
- On a line with an emoji or another character outside the Basic Multilingual Plane, hover, completion, go to definition, rename, formatting, code actions and diagnostic underlines no longer land one column off: positions are converted to the way the language server counts them (UTF-16 by default, characters when the server supports it).

## [0.21.0] - 2026-10-15

### Added

- Language modules can connect to a language server or debug adapter that another program already runs, such as the ones built into the Godot editor (`connect = "host:port"` instead of `command` in the manifest). While the server is not reachable, hovering the LSP indicator of the status bar shows the address it waits for, and the IDE connects as soon as it answers.

### Fixed

- A language server that fails on its very first start is retried, like one that crashes later, instead of staying off until the IDE restarts.
- Language servers that do not exit when the IDE stops them (Restart LSP, a start that gives up) are now killed after 2 seconds instead of being left running.

## [0.20.0] - 2026-10-14

### Added

- Report an Issue: the new Help menu (and the palette command "Help: Report an Issue…") opens a form to send a bug or an idea straight to the developers. The app version, the operating system and, if you leave the box checked, the end of the launch log are sent with it.

### Changed

- Welcome Tour and Keyboard Shortcuts moved from the View menu to the new Help menu.

## [0.19.0] - 2026-10-14

### Added

- Welcome tour: on the first launch, a short tour spotlights each part of the window (activity bar, Explorer, Extensions, editor, command palette, terminal, status bar) and explains how the IDE works: the editor ships bare, and languages come from extensions. Replay it any time with View → Welcome Tour or the palette command "Help: Welcome Tour".
- Close tabs in one go: right-click a tab for Close Others, Close to the Right, Close Saved and Close All, or press Ctrl+Shift+W to close every tab (also in the palette). Tabs with unsaved changes always stay open.

### Fixed

- Sidebar: the scrollbar of the Explorer (and of the Search, Git, Extensions and Outline views) sat in the middle of the panel and covered the end of long file names. It is now against the right edge.

## [0.18.0] - 2026-10-13

### Added

- Explorer and tabs: icons for many more files: Vue, Svelte, images, PDF, archives, CSV, audio and video, Office documents, config files, PowerShell, Java, Dockerfile and Compose files, `.gitignore`, `.env`, `LICENSE`, `package.json`, `Cargo.toml`, Makefile, Gradle.

### Fixed

- Large files no longer freeze the editor:
  - Typing in a 50,000-line file took about 190 ms per keystroke, now about 17 ms. Folding, the minimap and completion words no longer rescan the whole file on each keystroke.
  - Files over 10 MB or 200,000 lines open in large file mode: no language server, git markers, folding or minimap. A 50 MB file now takes about 16 ms per keystroke.
  - Very long lines (minified JSON…) are drawn only where visible: about 10 ms per keystroke on a 2 MB line, instead of 260 ms. Files with lines over 20,000 characters are not syntax highlighted, like in VS Code.

## [0.17.1] - 2026-10-13

### Changed

- Explorer: folders are no longer all yellow. Only the folders holding changes are colored, like the changed files themselves: yellow for modified, green for new, red for conflicts.
- Explorer: gitignored files are shown (dimmed) by default. The eye button still hides them.
- The launch log records each startup phase (modules, workspace, first frame), to tell what makes a launch slow.

## [0.17.0] - 2026-10-13

### Added

- Databases: a database icon in the activity bar (and the palette command "Databases: Open storingUnicorns") opens storingUnicorns, the Unicorns' database client. When it isn't installed, the IDE offers to open its download page.

## [0.16.1] - 2026-10-12

### Added

- The status bar shows the running version of Coding Unicorns, at the far left.

## [0.16.0] - 2026-10-12

### Added

- Java debugging, with the Java module: F5 on a `.java` file (or a `java` launch configuration) asks the language server for the main class of the open file, its classpath and the project's Java, then debugs it with breakpoints, stepping, the call stack and variables. The debugger (java-debug) is downloaded with the language server, nothing to install.
- Modules can have their debugger started by their language server (`transport = "lsp"`): the IDE runs the module's `[[debugger.lsp_resolve]]` server commands to complete the launch arguments, then the command that starts the session and returns its port. Language server `initializationOptions` can use the same variables as its arguments, and `${debuggerDir}`.

### Fixed

- Debugger: breakpoints in a file opened through a path with `/` separators on Windows (e.g. a folder opened with `cu C:/project`) are no longer ignored by debuggers such as java-debug.

## [0.15.0] - 2026-10-12

### Added

- Modules can have their language server downloaded on first use (`[capabilities.lsp_download]`, or `download` in a per-language server): the archive is fetched in the background when the first file of its languages is opened, then the server starts. Server arguments can use `${serverDir}` (the unpacked archive), `${dataDir}` (a data folder per module and workspace) and `${os}`, and a file name with a `*` picks the matching file. Downloads can be `.zip`, `.tar.gz`, a single `.gz` file or the executable itself. The Java, Rust, TOML, XML and Markdown modules use it: Eclipse JDT LS, rust-analyzer, taplo, lemminx and marksman no longer need to be installed by hand.

## [0.14.1] - 2026-10-12

### Added

- File tree: a button in the explorer header (eye icon) and the palette command "Explorer: Show/Hide Gitignored Files" show the files and folders excluded by `.gitignore` (build output, logs, local tool folders…). They are dimmed and in italics, and their tooltip says they are not part of commits. The setting is remembered, like "Show gitignored files" in the settings.

### Fixed

- File tree: "Reveal in File Manager" now works on Windows (Explorer opens with the file selected) and macOS (Finder); it only worked on Linux.

## [0.14.0] - 2026-10-12

### Added

- Image viewer tools: zoom with the mouse wheel (around the pointer), the toolbar or +/-, fit to the view (0) or actual size (1), double-click to switch between the two, and drag to pan. Images can be rotated and flipped for viewing (the file is not changed), shown on a checkerboard, dark or light background to judge transparency, and pixels stay crisp when zoomed in. The status line shows the pixel under the pointer with its colour (click to copy the hex code), and the toolbar the image's size, file size and format.
- The image viewer now opens GIF (first frame), and BMP and ICO files, which were listed but failed with "Unable to load image".

### Fixed

- File tree: dot-folders and dot-files (`.github`, `.vscode`, `.claude`, `.env`, `.gitignore`…) are listed again, following the .gitignore like the rest; only `.git` and OS clutter (`.DS_Store`, `Thumbs.db`) stay hidden.
- File tree: a folder collapsed and opened again now shows its current content instead of what it held when first opened. When the system drops file-change events (many changes at once, e.g. `npm install`), the tree is fully refreshed instead of missing them.
- File tree: names are sorted regardless of case, so `Zeta` no longer comes before `alpha`.

## [0.13.0] - 2026-10-11

### Added

- macOS: word shortcuts follow the Mac conventions. Option+Left/Right move by word (with Shift to select), Option+Backspace/Delete delete a word, and Cmd+Left/Right jump to the start/end of the line. In the editor, Option+arrows no longer trigger back/forward navigation on macOS; Windows and Linux are unchanged (Ctrl for words, Alt+arrows to navigate).

### Fixed

- Git: merging a branch that is ahead (fast-forward) from the branch list no longer discards uncommitted changes. Unrelated local edits are kept, and the merge is refused with a clear message when it would overwrite them. A merge is also refused while another one is in progress.
- Switching tabs no longer throws away unsaved changes: a modified file is kept in memory, with its undo history and cursors, until you come back to it. Clicking the current tab again, or going to a definition in the same file, no longer reloads it from disk either. "Save All & Quit" saves these files too, and a file changed on disk meanwhile still raises the conflict prompt.
- Closing a tab with unsaved changes with its × or a middle-click now asks to save, discard or cancel, like Ctrl+W, instead of closing it straight away.
- Split view: the right pane's unsaved changes are now protected. Its tab shows the modified dot, closing its tab or the split asks to save, discard or cancel, quitting includes it in "Save All & Quit", opening another file there is refused until it is saved, and it no longer overwrites a version saved from the left pane (it reloads it when it has no changes of its own).
- Language servers: hover, completion, outline, formatting and signature help no longer stop working until a restart when a server crashes or never answers a request (it is given up after 15 s or when the server restarts), and with several servers running, an answer from one is no longer taken for another's. Go to Definition no longer stays stuck after a "not found".
- Formatting a document through its language server (rustfmt, prettier, gofmt…) no longer destroys text when an edit spans several lines: each edit now replaces exactly its range, and the whole format is a single undo step.
- Renaming a symbol through the language server while the file has unsaved changes no longer loses them or lands the edits on the wrong lines: the open file is renamed in the editor (undoable, saved like any edit), other files on disk as before, and a file that can't be written is reported.
- Saving an untitled file (Ctrl+N) now asks where to save it, and its tab takes the file's name. Before, Ctrl+S, "Save" and "Save All & Quit" reported success without writing anything, so quitting lost the text. A save that fails now says so instead of showing "Saved".
- Renaming in the file tree no longer silently replaces an existing file of that name (a case-only rename is still allowed), refuses names with a path separator, and reports a failed rename. Open tabs follow the renamed file or folder, so saving no longer recreates the old name (e.g. New File, rename, Ctrl+S).
- Multiple cursors: every edit now applies at every cursor and moves all of them correctly. Backspace, Delete and Cut with selections no longer act on the main cursor only; typing at a cursor placed before the main one, auto-closed pairs and line joins no longer leave cursors at wrong positions or corrupt the text; a paste spread over the cursors replaces their selections; and copying with several cursors takes each exact selection, top to bottom.
- Pasting several lines is a single undo step again. Each pasted line used to add two, so a large paste could push everything before it out of the undo history.
- The git gutter now compares the file with HEAD line by line: inserting a line no longer marks every line below it as modified, changed lines and added lines are told apart, and removed lines show a small red marker where they were.
- Run configurations: their `env` variables (e.g. imported from launch.json) are now passed to the command, paths substituted for `${file}`, `${workspaceFolder}`… and arguments containing spaces are quoted for the terminal's shell (PowerShell, cmd, bash/zsh, fish), and cmd changes drive when needed. New variables `${fileBasename}` (file name with its extension) and `${fileExtname}`.
- Git: Push, Pull and Fetch no longer freeze the editor while the network (or a credential prompt) is slow. They run in the background with a spinner in the Git panel, the buttons wait until the operation ends, and an error is reported even if the panel was closed meanwhile.
- Extensions: a module without its own document tokenizer no longer leaks memory on every keystroke (its list of file extensions was copied and never freed each time it was asked for).
- Debugger: Continue and the step commands no longer leave the session shown as "Paused" with debuggers that don't announce it (lldb-dap, CodeLLDB…). The old call stack and variables are cleared until the next stop, and stepping again is no longer sent to a running thread.
- Language servers and debuggers that send a `Content-Type` header after `Content-Length` no longer have every message dropped, which left them looking frozen.
- Extensions: uninstalling or updating a module that only adds panels or commands now unloads it first, so its library is released on Windows. When its files can't be deleted, the module stays listed instead of disappearing half-removed.
- Merge view: "Save & Resolve" no longer stages a file it failed to write (the error is shown and the view stays open), keeps the file's CRLF line endings and final line break, and works when the opened folder is a subfolder of the repository.
- Extensions: a ZIP install that fails to extract a file now reports the module as failed instead of installing a truncated library, and an update that fails to copy its library puts the previous one back.
- Git panel: the conflicts list follows git's own merge state, so it updates as soon as a conflict is resolved and no longer lists files that merely contain `<<<<<<<`.
- Extensions: module updates compare versions properly: a pre-release such as `1.0.0-rc.1` comes before `1.0.0`, and a `v` prefix (`v1.3.0`) is understood.
- Claude panel: two editor windows running Claude at the same time no longer overwrite each other's permission server settings, which got tool permissions denied.
- Settings: a config file with a missing section or field keeps everything else it sets instead of falling back to the defaults. A file that can't be read at all is kept as `config.toml.invalid` before it gets overwritten.
- macOS: after an in-app update, the Finder shows the new version (the app's `Info.plist` is updated) and the app's ad hoc signature matches the new executable again.

## [0.12.2] - 2026-10-11

### Added

- macOS: a one-line Terminal installer, now the recommended way to install on a Mac: `curl -fsSL https://ajustor.github.io/codingUnicorns/install-macos.sh | bash`. It downloads the latest version, checks its SHA-256, installs the app in `~/Apps` (where Spotlight finds it) and puts the `cu` command on your `PATH`. Downloaded this way, the app shows no "dangerous app" warning and is not blocked by macOS 26. Run the command again to reinstall; the app keeps updating itself afterwards.

### Changed

- The download page shows the macOS Terminal installer first; the `.pkg` and `.dmg` remain available for installing without the Terminal.
## [0.12.1] - 2026-10-11

### Fixed

- With auto-save turned off, files are no longer saved on their own 2 seconds after the last edit. That idle save ignored the setting; it now only runs when auto-save is on, and the setting's label says it covers both the idle save and the save on focus loss.
## [0.12.0] - 2026-10-11

### Added

- Editor: word-by-word editing shortcuts. **Ctrl+Left / Ctrl+Right** jump over a whole word (`foo_bar` is one word, a punctuation run like `::` or `->` is one stop), **Ctrl+Shift+Left / Right** select word by word, and **Ctrl+Backspace / Ctrl+Delete** delete the word before / after the cursor. They work with multiple cursors, and each word deletion is a single undo step.

### Fixed

- Pressing Enter in a text field outside the editor (search, branch name, rename...) no longer also inserts a new line in the open file.
- The editor's blinking cursor is hidden while another field has the keyboard focus, so it is clear where typing goes.
## [0.11.5] - 2026-10-10

### Fixed

- macOS 26 (Tahoe): added a `.pkg` installer that handles the security restrictions automatically. The installer runs with admin rights, strips the quarantine attribute (which macOS 26 prevents removing otherwise), and places the app on the Desktop on macOS 26 or in `/Applications` on macOS 15 and earlier. The `.dmg` is still available for users who prefer it.

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
