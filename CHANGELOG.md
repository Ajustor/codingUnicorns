# Changelog

Notable changes to Coding Unicorns, version by version. Each section is also
shown in the app's update dialog and on the
[download page](https://ajustor.github.io/codingUnicorns/). The French
translation, [`CHANGELOG.fr.md`](CHANGELOG.fr.md), is kept in step with it.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
versions follow [Semantic Versioning](https://semver.org/).

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
