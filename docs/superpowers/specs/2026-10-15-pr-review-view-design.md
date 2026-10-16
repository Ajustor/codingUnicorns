# Pull Request Review View — Design

**Date:** 2026-10-15
**Scope:** Read the open pull requests of the workspace's GitHub repository from inside Coding Unicorns: a "Pull Requests" section in the Git panel lists them, and each one opens in a tab with its description, its changed files and a side-by-side diff, with the existing review comments shown under the lines they are about. Read-only.

---

## Context

The IDE already has the pieces for diffs, but no notion of a pull request:

- `src/editor/diff.rs` diffs a file against HEAD with `git2::Patch::from_buffers`, only to colour the gutter.
- `src/ui/merge_panel.rs` draws three panes side by side for conflicts; the split editor (`src/app/pane2.rs`) shows two editors side by side.
- Syntax colouring comes from the language modules through `Highlighter::tokenize_line` (`src/editor/highlight.rs`).
- Network git operations (`fetch_remote` in `src/git/remote.rs`) already authenticate over SSH and HTTPS, and run off the UI thread through `GitStatus::start_remote` / `poll_remote`.
- HTTP goes through `ureq` with the system trust store (`remote_registry::tls_config`).
- Nothing talks to the GitHub API, and there is no stored GitHub token. `origin` is typically an SSH URL.

## Goals

- A collapsible **Pull Requests (n)** section in the Git panel listing the open PRs of `origin`'s GitHub repository.
- Clicking a PR opens (or focuses) a tab for it: title, author, `head → base`, description (markdown), list of changed files, side-by-side diff.
- The diff is computed **locally by git** from the PR's commits, the same way GitHub computes it (against the merge base), so line numbers match GitHub's.
- Existing **review comments** are shown as threads under the line they target, on the side they target. Comments GitHub marks as outdated are grouped at the top of their file.
- Navigation: F7 / Shift+F7 jump to the next / previous change; double-click opens the file at that line.
- A "viewed" checkbox per file, to keep track during a review.
- Buttons: Refresh, Checkout, Open on GitHub.
- The UI never freezes: every network call runs on a background thread.

## Non-goals

- Writing comments, approving, requesting changes.
- CI check status.
- PR conversation comments (the non-inline ones) and review summaries.
- Using the diff view for local changes or for commits of the graph (the component is built generic so this can follow).
- GitLab, Gitea or any host other than GitHub.
- Keeping "viewed" checkboxes across restarts.

---

## Part 1 — GitHub client (`src/github/`)

New module, no new dependency (`ureq`, `serde`, `serde_json` are already used).

### Repository

`repo_from_remote_url(url) -> Option<RepoRef>` (`RepoRef { owner, name }`) accepts:

- `git@github.com:owner/name.git`
- `ssh://git@github.com/owner/name(.git)`
- `https://github.com/owner/name(.git)`, with or without credentials in the URL

The host may also be `www.github.com` or `ssh.github.com` (SSH over port 443). Owner and name must only use `[A-Za-z0-9._-]` and not be `.` or `..`: they go into API URLs. Anything else (another host, a local path, an SSH host alias from `~/.ssh/config`) is `None`. The URL is the one of the `origin` remote.

### Token (`auth.rs`)

Resolved in this order:

1. `github_token` in the config (new field, `#[serde(default)] pub github_token: String`), set in Settings in a masked text field.
2. The output of `gh auth token --hostname github.com` (spawned with `CREATE_NO_WINDOW` on Windows, 5-second timeout). Missing `gh` or a non-zero exit just means "no token". A token found is cached in memory for the session; "no token" is not, so a `gh auth login` done meanwhile is picked up by the next load or Refresh.
3. No token: requests go unauthenticated. Public repositories work, within GitHub's 60 requests/hour.

A 401 response clears the cached token so the next request resolves it again. The token never appears in logs, error messages or reports.

### API (`api.rs`)

Every request sends `Accept: application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28`, a `User-Agent: coding-unicorns/<version>` and, when there is one, `Authorization: Bearer <token>`. It goes through the same `ureq` agent configuration as the registry (system trust store, proxy rules).

| Call | Endpoint | Result |
|------|----------|--------|
| `list_open_pulls` | `GET /repos/{o}/{r}/pulls?state=open&per_page=100` (at most 3 pages: a Refresh stays cheap) | `Vec<PullSummary>` |
| `pull_detail` | `GET /repos/{o}/{r}/pulls/{n}` | `PullDetail` |
| `review_threads` | `GET /repos/{o}/{r}/pulls/{n}/comments?per_page=100` (all pages, at most 20) | `Vec<ReviewThread>` |

- `PullSummary`: `number`, `title`, `author` (`user.login`), `head_ref`, `base_ref`, `draft`.
- `PullDetail`: the summary fields plus `body` (may be empty), `head_sha`, `base_sha`, `head_repo` (`head.repo.full_name`, `None` when the fork was deleted), `html_url`.
- Pagination follows the `rel="next"` URL of the `Link` header, only when it is on the API (the token never leaves it); a truncated result is logged.
- Redirects keep the token only on the same host (`RedirectAuthHeaders::SameHost`): renamed repositories redirect within api.github.com.
- `ReviewThread`: `path`, `side` (`Left` = base, `Right` = PR), `line: Option<u32>` (1-based line in the file; `None` when GitHub reports `line: null`, i.e. the comment is outdated, or for a file-level comment), `original_line`, `file_level` (`subject_type: "file"`), and its `comments` (author, body, created_at) in order. Comments are sorted by id; a comment without `in_reply_to_id` starts a thread; the others are appended to the thread of the comment they reply to. A reply whose parent is missing starts a thread that its sibling replies join.

### Errors

`GithubError` turns responses into messages the panel and the tab show as is:

| Case | Message |
|------|---------|
| `origin` missing or not on GitHub | "The origin remote is not a GitHub repository." (no request is made) |
| 401 or 404 without a token | "This repository needs a GitHub token: run `gh auth login`, or set one in Settings → GitHub token." |
| 401 with a token | "GitHub refused the token: check it in Settings → GitHub token, or run `gh auth login` again." |
| 404 with a token | "Repository or pull request not found (does the token have access to it?)." |
| 403 with `x-ratelimit-remaining: 0`, or 429 | "GitHub rate limit reached, try again in N min." (minutes until `x-ratelimit-reset`; "try again later" without it) |
| Network failure | "Cannot reach GitHub: <reason>." |
| Unexpected JSON | "Unexpected response from GitHub." (details logged, without the token) |

---

## Part 2 — Local diff engine (`src/git/compare.rs`)

### Fetching the PR

On the `GitStatus`, run on a background thread:

1. Fetch `+refs/pull/{n}/head:refs/remotes/origin/pr/{n}` and `+refs/heads/{base_ref}:refs/remotes/origin/{base_ref}` through `fetch_remote` (same credentials and error messages as the Git panel's Fetch). This works for PRs from forks too.
2. Look up `head_sha` and `base_sha` from `PullDetail`. If either commit is missing after the fetch (a push landed between the API call and the fetch), the load fails with "The pull request changed while loading, refresh it."
3. `merge_base(base_sha, head_sha)` is the left side; `head_sha` is the right side. This is GitHub's three-dot diff, so comment line numbers line up.

### Diffing

`diff_tree_to_tree(merge_base_tree, head_tree)` with rename detection (`find_similar`), producing one `FileDiff` per file:

```rust
pub struct FileDiff {
    pub old_path: Option<String>,   // None when added
    pub new_path: Option<String>,   // None when deleted
    pub kind: FileDiffKind,         // Added, Modified, Deleted, Renamed
    pub additions: usize,
    pub deletions: usize,
    pub new_blob: Option<git2::Oid>, // keys the "viewed" checkbox
    pub content: DiffContent,
}

pub enum DiffContent {
    Text { old: Vec<String>, new: Vec<String>, rows: Vec<DiffRow> },
    Binary,
    TooLarge,
}
```

- Rename detection follows git: the rename limit is the number of deleted files, and past 1000×1000 candidates only exact renames are detected. Type changes (file ↔ symlink ↔ submodule) stay one `Modified` entry. Paths are taken as bytes (lossy UTF-8), never rewritten.
- A submodule side shows as `Subproject commit <oid>`, like `git diff`; `FileDiff::version()` (new blob or new submodule commit) keys the "viewed" mark.
- `Unavailable(reason)` when a blob is missing locally (shallow or partial clone): that file shows a notice, the others load normally. A shallow clone that doesn't reach the merge base asks for `git fetch --unshallow`.
- `Binary` when git flags either blob as binary or it is not UTF-8.
- `TooLarge` when either side is over `LARGE_FILE_BYTES` or `LARGE_FILE_LINES` (`src/editor/mod.rs`). The view offers "Show anyway", which recomputes it as `Text`.
- Line endings are normalised to `\n` before diffing (CRLF and LF compare equal). Tabs are diffed as they are (a tab↔spaces change shows) and only expanded to 4 spaces in the stored lines shown.
- Hunks use libgit2 with the indent heuristic (git's default). A single unchanged line between hunks is shown rather than folded.
- Word changes are computed within a per-file budget (summed `old.len() × new.len()` of the modified pairs); beyond it, the remaining rows only get the line colour.

### Alignment

`rows` pairs the two sides line by line, from the hunks of `git2::Patch::from_buffers` with 3 lines of context:

```rust
pub enum DiffRow {
    /// Same line on both sides (0-based indexes into `old` / `new`).
    Context { old: usize, new: usize },
    /// A removed line facing an added one: shown on one row, with word highlights.
    Modified { old: usize, new: usize },
    Removed { old: usize },   // empty filler on the right
    Added { new: usize },     // empty filler on the left
    /// Unchanged lines between hunks, folded: `len` lines starting at `old` / `new`.
    Fold { old: usize, new: usize, len: usize },
}
```

In each hunk, the first removed lines pair with the first added ones (`Modified`), the rest stay `Removed` or `Added` — the same rule as the gutter (`diff_statuses`). Unfolding a `Fold` replaces it with its `Context` rows.

Word highlights for `Modified` rows come from a longest-common-subsequence over word tokens (runs of word characters, runs of spaces, single punctuation), after trimming the tokens both lines start and end with. A single space between two changed runs joins them. Lines over 1,000 bytes, and pairs sharing less than half of the shorter line's non-blank text (unrelated lines paired by position), get no word highlight, only the line colour.

### Checkout

"Checkout" creates a local branch at `head_sha` and checks it out through the existing `checkout_branch` (which already refuses to overwrite local changes; a branch created for a refused checkout is deleted again). The branch is named `head_ref` when the PR comes from the same repository, `pr/{n}` when it comes from a fork. If a branch with that name already exists, it is checked out as is (not moved) and a toast says it may differ from the PR.

---

## Part 3 — UI

### Git panel section (`src/ui/git_panel.rs`)

A collapsible **PULL REQUESTS (n)** section after the stashes, collapsed by default.

- First expanded: loads the list in the background (spinner). The refresh icon in the header reloads it.
- Each entry: `#58` and the title (truncated, full title on hover; a click opens the PR's tab), then a small line with the author, `head → base` and a "Draft" tag when relevant.
- An error shows its message under the Refresh button (which retries); a previously loaded list stays visible under the error.
- Hidden when there is no repository. Shows the "not a GitHub repository" message when `origin` isn't on GitHub.

### Tab

`Tab` gets `pr: Option<u64>`, like `page`. `TabManager::open_pr(number, title)` opens or focuses the PR's single tab; its `path` is `__pr__{n}`, which isn't absolute, so sessions don't save it. The tab title is `#58 <title>`. The app keeps one `PrView` per open PR tab, dropped when the tab closes. The central panel routes PR tabs to `PrView::show` where it already routes page and settings tabs (`src/ui/layout.rs`).

### PR tab (`src/ui/pr_view.rs`)

- **Loading**: the PR detail first (the fetch needs its `base_ref` and SHAs), then the fetch + diff and the review threads in parallel, all on background threads. The tab shows a spinner, then the first error if any (with Retry), then the content.
- **Header**: `#58 title`, author, `head_ref → base_ref`, the buttons Refresh / Checkout / Open on GitHub (`ctx.open_url`), then the description rendered with `egui_commonmark`, collapsible ("No description" when empty).
- **Left column (files)**: one row per file — "viewed" checkbox, file-kind icon, path (`old → new` for renames), `+a −d`, number of comment threads. Clicking a row scrolls the diff to that file. The column is resizable.
- **Right column**: `DiffView` for every file, one after the other, each under a sticky header with its path and its "viewed" checkbox. A viewed file collapses to its header.
- **"Viewed"** is kept in memory per `(pr, path)` with the file's `new_blob`: after a refresh, a file whose content changed is unchecked again.
- **Keys** (unless a text field has focus): F7 / Shift+F7 move to the next / previous block of changed rows across files, stopping at the first and last (scrolling and briefly highlighting it). Double-click on a line of the right column opens the working-copy file at that line; when HEAD isn't the PR's head, a hint under the header says the working copy may differ.

### Diff view (`src/ui/diff_view.rs`)

A generic component: it takes a `FileDiff` (text variant), the comment threads of that file and the palette, and knows nothing about pull requests.

- Each row draws both sides: line number, `+`/`−` marker, code. Removed lines on a red tint, added on a green tint, fillers in the surface colour, word highlights in a stronger tint. Colours come from the theme palette so every theme works. Tabs are shown as 4 spaces. A line wider than its column is clipped; hovering it shows the whole line.
- Code is coloured with `Highlighter::tokenize_line` for the file's language (by extension), when a module provides it.
- `Fold` rows show "⋯ N unchanged lines"; a click unfolds them.
- Both columns are drawn in the same scroll area row by row, so they always scroll together. Only visible rows are laid out (`ScrollArea::show_viewport` with a cached height per row), so a long diff stays responsive.
- A comment thread is drawn as a block right after its row, under the side it targets: author, date (`YYYY-MM-DD`), body as markdown, replies indented. Threads whose `line` is outside the shown rows unfold the fold that hides them.
- Threads on the whole file (`subject_type: "file"`, no line) are listed first at the top of the file under "Comments on the file", full width.
- Outdated threads (`line: None`) are listed at the top of the file under "Outdated comments", with their `original_line`.
- The REST API doesn't say whether a thread was resolved: resolved threads show like open ones.

---

## Testing

No test touches the internet: the HTTP client is tested against a loopback server serving canned responses (pagination, auth header, foreign `Link`, 401, rate limit, same-host redirect).

- **`github`**
  - `repo_from_remote_url`: the SSH, `ssh://` and HTTPS forms, with and without `.git` and credentials; other hosts and local paths are `None`.
  - Parsing JSON fixtures: a PR list, a PR detail (with and without `head.repo`), review comments grouped into threads (replies, missing parent, `line: null` → outdated).
  - `Link` header pagination.
  - Token resolution order, with the config value and the `gh` call injected.
  - Error mapping: 401, 404 with and without a token, rate limit with its reset time.
- **`compare`**, on temporary repositories (the `TestRepo` / `bare_origin` helpers):
  - Changes made on the base after the branch point don't show in the diff.
  - Added, modified, deleted and renamed files, with their `+/−` counts.
  - Binary files; files over the size limit become `TooLarge`.
  - CRLF and LF compare equal.
  - Alignment: pure insertion, pure deletion, modification, more added than removed lines, folds between hunks and unfolding.
  - Fetching `refs/pull/{n}/head` from a bare origin that has such a ref.
  - Checkout names (`head_ref` vs `pr/{n}`), an existing branch left as is, local changes refused.
- **Word diff**: tokens, a changed word in the middle, long lines skipped.
- **Comment anchoring**: a thread lands on the row holding its `line` on its `side`; a thread inside a fold unfolds it; outdated threads go to the top section.
- **UI**: a headless egui test, like the editor's, renders a PR tab from fixture data and checks that F7 moves to the next change and that a "viewed" file collapses.
