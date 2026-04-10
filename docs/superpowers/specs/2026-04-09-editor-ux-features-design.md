# Coding Unicorns - Editor UX Features Design

**Date:** 2026-04-09
**Scope:** 7 features to bring the editor closer to a VS Code-like experience

---

## Features Overview

| # | Feature | Status Before | Complexity |
|---|---------|---------------|------------|
| 1 | Split Editor (L/R) | Not implemented | High |
| 2 | Auto-close Brackets/Quotes | Not implemented | Low |
| 3 | Go to Definition (F12) + Nav History | Partial (go-to-def exists, no history) | Medium |
| 4 | Git Stage/Commit/Push UI + 3-Panel Merge Tool | Partial (status only) | High |
| 5 | Selection Highlighting | Not implemented | Low |
| 6 | Multi-cursor Paste | Not implemented | Low |

Note: Ctrl+Shift+F was already implemented (app.rs:935).

---

## 1. Split Editor

### Goal
Allow opening two files side-by-side with a single horizontal split (left/right).

### Architecture

New struct wrapping an editor + its tabs:

```rust
pub struct EditorPane {
    pub editor: Editor,
    pub tab_manager: TabManager,
}
```

App changes:
- Replace `app.editor` and `app.tab_manager` with `app.panes: Vec<EditorPane>` (1 or 2 elements)
- Add `app.active_pane: usize` (0 or 1)
- Shared services remain single-instance: LSP, Git, DAP, plugins

### Layout
- When 1 pane: current behavior (full-width central panel)
- When 2 panes: two equal-width columns in the central panel, separated by a 4px draggable divider
- Add `app.split_ratio: f32` (default 0.5) for the divider position
- Each pane renders its own tab bar + editor area

### Keyboard Shortcuts
- `Ctrl+\` : Split current file to the right (or close split if already split)
- Clicking in a pane sets it as active
- File tree double-click opens in the active pane
- Ctrl+click on file tree opens in the *other* pane

### Constraints
- Max 2 panes (no recursive splits)
- Same file can be open in both panes: each pane keeps its own independent Buffer copy with independent cursors. When switching focus between panes, if the file was modified in the other pane, reload from disk. This avoids the complexity of shared mutable state.
- When the last tab of a split pane is closed, the pane collapses back to single view
- LSP notifications (diagnostics, hover) are routed to the pane whose file matches

### Migration
This is the most invasive change. Every place in the codebase that references `app.editor` or `app.tab_manager` must be updated to go through `app.active_pane()` or `app.pane(index)`. This includes:
- `src/ui/layout.rs` (editor rendering)
- `src/app.rs` (all LSP response handling, file open, save, etc.)
- `src/ui/statusbar.rs` (cursor position, file info)
- `src/ui/palette.rs` (command palette file opening)
- Keyboard shortcut handling in `app.rs`

---

## 2. Auto-close Brackets/Quotes

### Goal
Automatically insert closing brackets/quotes and handle skip-close and surround behavior.

### Pairs
| Open | Close |
|------|-------|
| `(`  | `)`   |
| `[`  | `]`   |
| `{`  | `}`   |
| `"`  | `"`   |
| `'`  | `'`   |
| `` ` `` | `` ` `` |

### Behaviors

**Auto-insert:** When typing an opening character:
- Insert the closing character after the cursor
- Position cursor between the pair
- Only auto-insert if the next character is whitespace, a closing bracket, or end-of-line

**Skip-close:** When typing a closing character and the character under the cursor is that same closing character:
- Don't insert, just advance the cursor by 1

**Pair-delete:** When pressing Backspace with cursor between an empty pair (e.g., `(|)`):
- Delete both the opening and closing characters

**Surround:** When text is selected and an opening character is typed:
- Wrap the selection with the pair instead of replacing it

### Implementation Location
- `src/editor/mod.rs` in the `insert_char()` method (lines ~377-461)
- Backspace handling in the same file
- Add `auto_close_brackets: bool` to `Config.editor` (default: true)

### Edge Cases
- Inside strings/comments: still auto-close (VS Code does this too)
- Quotes: only auto-close `"` and `'` if the character before cursor is not alphanumeric (to avoid closing apostrophes in English text)

---

## 3. Go to Definition (F12) + Navigation History

### Goal
Add F12 shortcut for go-to-definition and a back/forward navigation stack.

### What Already Exists
- Ctrl+click triggers `go_to_definition_request` (editor/mod.rs:44)
- `handle_go_to_definition()` in app.rs:413-550 handles LSP lookup + regex fallback
- `open_file_at_line()` navigates to a file and line

### Navigation History

New struct:

```rust
pub struct NavigationHistory {
    stack: Vec<NavigationEntry>,
    index: usize, // points to the current position
}

pub struct NavigationEntry {
    pub path: PathBuf,
    pub row: usize,
    pub col: usize,
}
```

- Max 50 entries
- Push current position *before* navigating (go-to-def, file open from search, etc.)
- Alt+Left = go back (decrement index, navigate to stack[index])
- Alt+Right = go forward (increment index, navigate to stack[index])
- When navigating from the middle of the stack, truncate forward history (like browser)

### New Shortcuts
- `F12`: Go to definition (same as Ctrl+click but keyboard)
- `Alt+Left`: Navigate back
- `Alt+Right`: Navigate forward

### Push Points (where we record position before jumping)
- Go to definition (Ctrl+click or F12)
- Open file from command palette
- Open file from workspace search results
- Open file from references panel
- Go to line (Ctrl+G)

### Implementation Location
- New field `app.nav_history: NavigationHistory`
- Helper method `app.push_nav_and_goto(path, row, col)` that pushes then navigates
- Keyboard handling in app.rs for F12, Alt+Left, Alt+Right

---

## 4. Git Stage/Commit/Push UI + 3-Panel Merge Tool

### Goal
Interactive git operations from the sidebar + a 3-panel merge conflict resolver.

### 4a. Git Sidebar Panel

#### Stage/Unstage
- Each file in the status list gets a checkbox (or +/- button)
- Clicking stages (`git2::Index::add_path`) or unstages (`git2::Index::remove_path`) the file
- "Stage All" / "Unstage All" buttons at the top
- Staged files shown in a separate "Staged Changes" section above "Changes"

#### Commit
- Multi-line text input at the top of the Git panel for commit message
- "Commit" button (disabled when message is empty or no staged files)
- Uses `git2::Repository::commit()` with the current signature
- After commit: clear message, refresh status

#### Push / Pull
- "Push" button: `git2::Remote::push()`
- "Pull" button: `git2::Remote::fetch()` + fast-forward merge
- Show ahead/behind count next to branch name (compare local HEAD vs remote tracking)
- Disable push when nothing to push, disable pull when up-to-date

#### Conflicts
- Files with conflict status shown with a red icon and "C" badge
- Click opens the 3-panel merge tool

### 4b. 3-Panel Merge Tool

#### Layout
Full-screen modal replacing the editor area (like the settings panel):

```
+-------------------+-------------------+-------------------+
|   OURS (readonly) |  RESULT (edit)    | THEIRS (readonly) |
|   Current branch  |  Merged output    | Incoming branch   |
+-------------------+-------------------+-------------------+
| [Accept All Left] | [Save & Resolve]  | [Accept All Right]|
|                   | [Cancel]          |                   |
+-------------------+-------------------+-------------------+
```

#### Conflict Parsing
Parse the working tree file to extract conflict regions:

```rust
pub struct ConflictHunk {
    /// Line range in the original conflicted file
    pub file_line_start: usize,
    pub file_line_end: usize,
    /// Content from our side
    pub ours: Vec<String>,
    /// Content from their side
    pub theirs: Vec<String>,
    /// Resolution state
    pub resolution: HunkResolution,
}

pub enum HunkResolution {
    Unresolved,
    AcceptOurs,
    AcceptTheirs,
    Manual, // user edited the result directly
}
```

Markers to parse: `<<<<<<<`, `=======`, `>>>>>>>`

#### Panel Behavior
- Left panel: reconstructed "ours" version (non-conflict lines + ours hunks) — read-only Editor
- Right panel: reconstructed "theirs" version — read-only Editor
- Center panel: editable Editor initialized with the full file, conflict markers removed, unresolved hunks shown with a placeholder or "ours" version by default
- All 3 panels scroll synchronously (linked scroll offsets)
- Conflict hunks are highlighted:
  - Green background for additions
  - Red background for deletions
  - Yellow/orange for the conflict zone boundaries

#### Per-Hunk Actions
- "Accept Left" / "Accept Right" buttons rendered inline in the center panel at each conflict hunk
- Clicking replaces the hunk content in the result buffer

#### Toolbar Actions
- "Accept All Left": resolve all hunks with ours
- "Accept All Right": resolve all hunks with theirs
- "Save & Mark Resolved": write the center panel content to disk + `git add` the file
- "Cancel": discard changes, return to normal editor

#### Data Structure

```rust
pub struct MergeView {
    pub file_path: PathBuf,
    pub ours_buffer: Buffer,    // read-only
    pub theirs_buffer: Buffer,  // read-only
    pub result_editor: Editor,  // editable
    pub hunks: Vec<ConflictHunk>,
    pub scroll_offset: f32,     // synchronized scroll
    pub is_active: bool,
}
```

Added to app: `app.merge_view: Option<MergeView>`

When `merge_view.is_some()`, the layout renders the merge tool instead of the normal editor area.

---

## 5. Selection Highlighting

### Goal
When the cursor is on a word or a word is selected, highlight all other visible occurrences of that word.

### Behavior
- Extract the word under cursor (or the selected text if it's a single word)
- Minimum 2 characters
- Whole-word match only (bounded by non-alphanumeric characters)
- Scan only visible lines for performance
- Draw a semi-transparent rectangle (accent color at 30% opacity) behind each occurrence
- Exclude the current selection/cursor position itself

### Implementation

```rust
// In Editor struct
pub word_occurrences: Vec<(usize, usize, usize)>, // (row, col_start, col_end)
```

- Computed during the render pass, before drawing text
- Use `get_word_at(row, col)` (already exists in utils.rs) to get the word under cursor
- Simple substring search on each visible line, then filter for word boundaries
- Recompute only when cursor moves or buffer changes (use content_version)

### Rendering
- In the editor paint loop, after drawing line backgrounds but before drawing text
- Use `ui.painter().rect_filled()` with `Color32::from_rgba_unmultiplied(accent.r, accent.g, accent.b, 77)` (30% opacity)

---

## 6. Multi-cursor Paste

### Goal
When pasting with N cursors and N lines in clipboard, distribute one line per cursor.

### Logic

In the paste handler (`editor/mod.rs:1450-1459`):

```
let lines: Vec<&str> = clipboard_text.lines().collect();
let cursor_count = 1 + extra_cursors.len();

if lines.len() == cursor_count {
    // Distribute: line[0] to main cursor, line[1] to extra_cursors[0], etc.
    // Sort all cursors by position (row, col) first
} else {
    // Current behavior: paste full text at each cursor
}
```

### Edge Cases
- Cursors must be sorted by position (top to bottom) before distributing
- After pasting, adjust cursor positions for the inserted text (offset tracking)
- If clipboard ends with a newline, the last empty line is ignored for counting (so "a\nb\n" has 2 lines, not 3)
- Copy with multi-cursor should also copy one line per cursor (for round-trip)

### Multi-cursor Copy
When copying with multiple cursors active:
- Collect the selected text (or current line if no selection) from each cursor
- Join with newlines
- Store in clipboard
- This ensures copy+paste round-trips correctly with multi-cursor

---

## Implementation Order

Recommended order based on dependencies and risk:

1. **Selection Highlighting** — isolated, low risk, immediate visual impact
2. **Auto-close Brackets** — isolated, low risk, high usability impact
3. **Multi-cursor Paste** — isolated, small change
4. **Navigation History + F12** — medium scope, builds on existing go-to-def
5. **Git Stage/Commit/Push UI** — medium scope, new sidebar content
6. **3-Panel Merge Tool** — high scope, depends on Git panel
7. **Split Editor** — highest scope, touches the most code, should be last

Features 1-3 can be implemented in parallel. Feature 6 depends on 5. Feature 7 is independent but risky.

---

## Files Impacted

| File | Features |
|------|----------|
| `src/editor/mod.rs` | 1 (split), 2 (auto-close), 3 (F12), 5 (highlight), 6 (paste) |
| `src/app.rs` | 1 (split), 3 (nav history), 4 (git ops) |
| `src/ui/layout.rs` | 1 (split), 4 (git panel + merge), 5 (highlight render) |
| `src/git/mod.rs` | 4 (stage/commit/push/merge) |
| `src/config/mod.rs` | 2 (auto_close_brackets setting) |
| `src/tabs/mod.rs` | 1 (split - per-pane tabs) |
| `src/ui/statusbar.rs` | 1 (split - active pane info) |
| `src/ui/palette.rs` | 1 (split - open in active pane) |

### New Files
| File | Purpose |
|------|---------|
| `src/git/merge.rs` | Conflict parser + MergeView struct |
| `src/ui/merge_panel.rs` | 3-panel merge tool rendering |
| `src/nav_history.rs` | NavigationHistory struct and logic |
