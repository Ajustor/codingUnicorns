# Pull Request Review View Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Read the open pull requests of the workspace's GitHub repository from the Git panel, each in a tab with its description, changed files and a side-by-side diff showing the existing review comments.

**Architecture:** A small blocking GitHub client (`src/github/`) lists pull requests and reads their review comments, run on background threads. The diff itself is computed locally with git2 (`src/git/compare.rs`) against the merge base, like GitHub. The UI is split into a pure layout model (`src/ui/diff_layout.rs`), a virtualized painter (`src/ui/diff_view.rs`), the PR tab (`src/ui/pr_view.rs`) and a Git panel section (`src/ui/pr_list.rs`).

**Tech Stack:** Rust 2021 (rust-version 1.85), egui/eframe 0.31, git2 0.20, ureq 3, serde / serde_json, egui_commonmark 0.20, egui-phosphor 0.9.

**Spec:** `docs/superpowers/specs/2026-10-15-pr-review-view-design.md`

---

## Conventions

- Prefix every shell command with `rtk` (project rule): `rtk cargo test …`, `rtk cargo clippy …`.
- `sccache` isn't installed: run `export RUSTC_WRAPPER=` before cargo commands.
- The CI runs `cargo fmt --all --check`: every checkpoint also runs `rtk cargo fmt` then `cargo fmt --all --check`. The plan's code isn't always rustfmt-clean; formatting it is expected.
- **Do not commit.** The user commits themselves. Each task ends with a checkpoint (tests + clippy) instead of a commit.
- Tests go in a `#[cfg(test)] mod tests` at the bottom of the file they test, like the rest of the codebase. `rtk cargo test <path>` runs the tests whose path contains `<path>` (e.g. `rtk cargo test github::`).
- `src/main.rs` has `#![allow(dead_code)]`: items only used by a later task don't warn. Unused *imports* still warn, so each task only adds the imports it uses.
- UI strings are in English, like the rest of the app.

## File map

| File | Responsibility |
|------|----------------|
| `src/github/mod.rs` (new) | `RepoRef`, `repo_from_remote_url`, `origin_repo` |
| `src/github/error.rs` (new) | `GithubError`, user-facing messages |
| `src/github/auth.rs` (new) | Token: Settings, then `gh auth token`, then none |
| `src/github/api.rs` (new) | API types, JSON parsing, review threads, `Client` |
| `src/git/word_diff.rs` (new) | Changed words between two lines |
| `src/git/compare.rs` (new) | Line alignment, `TextDiff`, `FileDiff`, `diff_pull`, `fetch_pull`, `checkout_pull` |
| `src/ui/diff_layout.rs` (new) | Items, heights and offsets of the diff view; thread anchoring; F7 stops |
| `src/ui/diff_view.rs` (new) | Painting the side-by-side diff |
| `src/ui/pr_view.rs` (new) | The pull request tab |
| `src/ui/pr_list.rs` (new) | The PULL REQUESTS section of the Git panel |
| `src/app/pr_ops.rs` (new) | App glue: open a PR tab, apply its actions, drop closed views |
| `src/main.rs` | `mod github;` |
| `src/git/mod.rs` | `pub mod compare; pub mod word_diff;` |
| `src/ui/mod.rs` | `pub mod diff_layout; pub mod diff_view; pub mod pr_list; pub mod pr_view;` |
| `src/config/mod.rs` | `github_token` setting |
| `src/ui/settings.rs` | GitHub section |
| `src/tabs/mod.rs` | PR tabs (`pr`, `open_pr`, `active_pr`, `is_file`) |
| `src/app/file_ops.rs` | `load_active_tab` skips PR tabs |
| `src/app/mod.rs` | `pr_views`, `mod pr_ops`, drop closed views each frame |
| `src/ui/layout.rs` | Modified-flag sync guard, PR tab routing, Git panel call |
| `src/ui/git_panel.rs` | Hosts the PULL REQUESTS section |
| `README.md`, `CHANGELOG.md`, `CHANGELOG.fr.md` | Docs |

---

### Task 1: GitHub repository of `origin`

**Files:**
- Create: `src/github/mod.rs`, `src/github/error.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Register the module**

In `src/main.rs`, after `mod git;`:

```rust
mod github;
```

- [ ] **Step 2: Write the failing tests**

Create `src/github/error.rs` with only its tests:

```rust
#[cfg(test)]
mod tests {
    use super::GithubError;

    #[test]
    fn statuses_map_to_actionable_errors() {
        assert_eq!(GithubError::from_response(401, true, None, None, 0), GithubError::NeedsToken);
        assert_eq!(GithubError::from_response(404, false, None, None, 0), GithubError::NeedsToken);
        assert_eq!(GithubError::from_response(404, true, None, None, 0), GithubError::NotFound);
        assert_eq!(
            GithubError::from_response(403, true, Some("12"), None, 0),
            GithubError::Status(403)
        );
        assert_eq!(GithubError::from_response(500, true, None, None, 0), GithubError::Status(500));
    }

    #[test]
    fn rate_limit_says_when_it_resets() {
        assert_eq!(
            GithubError::from_response(403, false, Some("0"), Some("1000"), 1000 - 125),
            GithubError::RateLimited { minutes: Some(3) }
        );
        assert_eq!(
            GithubError::from_response(429, false, None, Some("junk"), 0),
            GithubError::RateLimited { minutes: None }
        );
        // Already past the reset time: at least one minute.
        assert_eq!(
            GithubError::from_response(403, false, Some("0"), Some("10"), 50),
            GithubError::RateLimited { minutes: Some(1) }
        );
    }

    #[test]
    fn messages_tell_what_to_do() {
        assert!(GithubError::NeedsToken.to_string().contains("gh auth login"));
        assert_eq!(
            GithubError::RateLimited { minutes: Some(3) }.to_string(),
            "GitHub rate limit reached, try again in 3 min."
        );
        assert_eq!(
            GithubError::NotGitHub.to_string(),
            "The origin remote is not a GitHub repository."
        );
    }
}
```

Create `src/github/mod.rs`:

```rust
//! Read-only access to the GitHub repository of the workspace: its open pull
//! requests and their review comments. Diffs are computed locally by git
//! (`crate::git::compare`); this module only talks to the GitHub API.

pub mod error;

pub use error::GithubError;

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(owner: &str, name: &str) -> Option<RepoRef> {
        Some(RepoRef {
            owner: owner.into(),
            name: name.into(),
        })
    }

    #[test]
    fn github_urls_in_every_form() {
        for url in [
            "git@github.com:Ajustor/writingUnicorns.git",
            "git@github.com:Ajustor/writingUnicorns",
            "ssh://git@github.com/Ajustor/writingUnicorns.git",
            "ssh://git@github.com:22/Ajustor/writingUnicorns",
            "https://github.com/Ajustor/writingUnicorns.git",
            "https://github.com/Ajustor/writingUnicorns/",
            "https://user:secret@github.com/Ajustor/writingUnicorns.git",
            "  https://GitHub.com/Ajustor/writingUnicorns  ",
        ] {
            assert_eq!(
                repo_from_remote_url(url),
                repo("Ajustor", "writingUnicorns"),
                "{url}"
            );
        }
    }

    #[test]
    fn other_hosts_and_paths_are_not_github() {
        for url in [
            "git@gitlab.com:me/repo.git",
            "https://gitlab.com/me/repo",
            "https://github.com.evil.example/me/repo",
            "https://github.com/only-owner",
            "https://github.com/a/b/c",
            "/home/me/repo",
            "C:\\Users\\me\\repo",
            "",
        ] {
            assert_eq!(repo_from_remote_url(url), None, "{url}");
        }
    }

    #[test]
    fn origin_repo_reads_the_origin_remote() {
        let r = crate::git::tests::TestRepo::new();
        assert_eq!(origin_repo(r.path()), Err(GithubError::NotGitHub));
        r.repo
            .remote("origin", "git@github.com:me/proj.git")
            .unwrap();
        assert_eq!(origin_repo(r.path()), Ok(repo("me", "proj").unwrap()));
        assert_eq!(origin_repo(r.path()).unwrap().full_name(), "me/proj");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `rtk cargo test github::`
Expected: compile errors: `GithubError`, `RepoRef`, `repo_from_remote_url`, `origin_repo` not found.

- [ ] **Step 4: Implement `GithubError`**

At the top of `src/github/error.rs`, above the tests:

```rust
//! Errors of GitHub requests, worded for the user.

/// Why a GitHub request failed. `Display` is the message the Git panel and
/// the pull request tab show as is; it never contains the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubError {
    /// `origin` is missing or isn't on GitHub: nothing is requested.
    NotGitHub,
    /// 401, or 404 without a token (GitHub hides private repositories).
    NeedsToken,
    /// 404 with a token.
    NotFound,
    /// The rate limit is reached; minutes until it resets, when known.
    RateLimited { minutes: Option<u64> },
    /// No response (DNS, TLS, timeout…).
    Network(String),
    /// Any other status.
    Status(u16),
    /// The body isn't the JSON expected.
    BadResponse,
}

impl GithubError {
    /// The error for a non-success `status`, given the rate-limit headers
    /// (`x-ratelimit-remaining`, and `x-ratelimit-reset`: a Unix time) and
    /// the current Unix time `now`.
    pub fn from_response(
        status: u16,
        has_token: bool,
        remaining: Option<&str>,
        reset: Option<&str>,
        now: u64,
    ) -> Self {
        let exhausted = remaining.map(str::trim) == Some("0");
        match status {
            401 => Self::NeedsToken,
            404 if has_token => Self::NotFound,
            404 => Self::NeedsToken,
            429 => Self::rate_limited(reset, now),
            403 if exhausted => Self::rate_limited(reset, now),
            _ => Self::Status(status),
        }
    }

    fn rate_limited(reset: Option<&str>, now: u64) -> Self {
        let minutes = reset
            .and_then(|r| r.trim().parse::<u64>().ok())
            .map(|at| at.saturating_sub(now).div_ceil(60).max(1));
        Self::RateLimited { minutes }
    }
}

impl std::fmt::Display for GithubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotGitHub => write!(f, "The origin remote is not a GitHub repository."),
            Self::NeedsToken => write!(
                f,
                "This repository needs a GitHub token: run `gh auth login`, or set one in Settings → GitHub token."
            ),
            Self::NotFound => write!(
                f,
                "Repository or pull request not found (does the token have access to it?)."
            ),
            Self::RateLimited { minutes: Some(m) } => {
                write!(f, "GitHub rate limit reached, try again in {m} min.")
            }
            Self::RateLimited { minutes: None } => {
                write!(f, "GitHub rate limit reached, try again later.")
            }
            Self::Network(reason) => write!(f, "Cannot reach GitHub: {reason}."),
            Self::Status(code) => write!(f, "GitHub answered with status {code}."),
            Self::BadResponse => write!(f, "Unexpected response from GitHub."),
        }
    }
}

impl std::error::Error for GithubError {}
```

- [ ] **Step 5: Implement the repository lookup**

In `src/github/mod.rs`, after `pub use error::GithubError;` and before the tests:

```rust
use std::path::Path;

/// `owner/name` of a GitHub repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    /// `owner/name`, as GitHub writes `full_name`.
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// The GitHub repository `url` points at: SSH (`git@github.com:o/r.git`,
/// `ssh://git@github.com/o/r`) or HTTPS, with or without `.git`, a port or
/// credentials. `None` for another host or a local path.
pub fn repo_from_remote_url(url: &str) -> Option<RepoRef> {
    let url = url.trim();
    let (authority, path) = match url.split_once("://") {
        Some((_scheme, rest)) => rest.split_once('/')?,
        // scp-like syntax: `[user@]host:path`.
        None => url.split_once(':')?,
    };
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = host.split(':').next().unwrap_or(host);
    if !host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com") {
        return None;
    }
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(RepoRef {
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

/// The GitHub repository of the `origin` remote of the repository holding `path`.
pub fn origin_repo(path: &Path) -> Result<RepoRef, GithubError> {
    let repo = git2::Repository::discover(path).map_err(|_| GithubError::NotGitHub)?;
    let remote = repo.find_remote("origin").map_err(|_| GithubError::NotGitHub)?;
    remote
        .url()
        .and_then(repo_from_remote_url)
        .ok_or(GithubError::NotGitHub)
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `rtk cargo test github::`
Expected: 6 tests pass.

- [ ] **Step 7: Checkpoint**

Run: `rtk cargo clippy --all-targets`
Expected: no new warnings in `src/github/`.

---

### Task 2: GitHub token

**Files:**
- Create: `src/github/auth.rs`
- Modify: `src/github/mod.rs`, `src/config/mod.rs`

- [ ] **Step 1: Write the failing tests**

Create `src/github/auth.rs` with its tests:

```rust
#[cfg(test)]
mod tests {
    use super::resolve_with;

    #[test]
    fn the_settings_token_comes_first() {
        assert_eq!(
            resolve_with(" ghp_settings ", || panic!("gh must not run")),
            Some("ghp_settings".to_string())
        );
    }

    #[test]
    fn falls_back_to_the_cli_then_to_none() {
        assert_eq!(resolve_with("", || Some("gho_cli".into())), Some("gho_cli".into()));
        assert_eq!(resolve_with("  ", || None), None);
        assert_eq!(resolve_with("", || Some(String::new())), None);
    }
}
```

In `src/github/mod.rs`, after `pub mod error;`:

```rust
pub mod auth;
```

In the `#[cfg(test)] mod tests` of `src/config/mod.rs`, add:

```rust
    #[test]
    fn github_token_is_only_written_when_set() {
        let mut c = Config::default();
        assert!(c.github_token.is_empty());
        assert!(!toml::to_string_pretty(&c).unwrap().contains("github_token"));
        c.github_token = "ghp_x".into();
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back.github_token, "ghp_x");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test github::auth` then `rtk cargo test github_token`
Expected: compile errors: `resolve_with` not found, no field `github_token`.

- [ ] **Step 3: Implement token resolution**

At the top of `src/github/auth.rs`:

```rust
//! The token GitHub requests are made with: the one set in Settings, else
//! the GitHub CLI's (`gh auth token`), else none (public repositories only).

use crate::process_ext::CommandExt;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A stuck `gh` must not hold a request back for long.
const GH_TIMEOUT: Duration = Duration::from_secs(5);

/// `gh auth token`'s answer, asked once per session (`None`: not asked yet).
static GH_TOKEN: Mutex<Option<Option<String>>> = Mutex::new(None);

/// The token for GitHub requests. The first time Settings has none, this
/// runs `gh`: call it off the UI thread.
pub fn resolve(config_token: &str) -> Option<String> {
    resolve_with(config_token, || {
        let mut cached = GH_TOKEN.lock().unwrap_or_else(|e| e.into_inner());
        cached.get_or_insert_with(gh_auth_token).clone()
    })
}

/// [`resolve`] with the GitHub CLI lookup injected.
pub(crate) fn resolve_with(
    config_token: &str,
    gh: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let configured = config_token.trim();
    if !configured.is_empty() {
        return Some(configured.to_string());
    }
    gh().filter(|t| !t.is_empty())
}

/// Forget the CLI's token (GitHub refused it): the next request asks `gh` again.
pub fn forget() {
    *GH_TOKEN.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// `gh auth token`, when the GitHub CLI is installed and logged in.
fn gh_auth_token() -> Option<String> {
    let mut child = Command::new("gh")
        .args(["auth", "token"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .no_window()
        .spawn()
        .ok()?;
    let deadline = Instant::now() + GH_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let token = out.trim();
    (!token.is_empty()).then(|| token.to_string())
}
```

- [ ] **Step 4: Add the setting**

In `src/config/mod.rs`, in `pub struct Config`, after the `report_url` field:

```rust
    /// Token for GitHub requests (the Git panel's pull requests). Empty: the
    /// GitHub CLI's login (`gh auth token`), else none. Not written while empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub github_token: String,
```

In `impl Default for Config`, after `report_url: default_report_url(),`:

```rust
            github_token: String::new(),
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `rtk cargo test github::auth` then `rtk cargo test config::`
Expected: all pass (including the existing config round-trip tests).

- [ ] **Step 6: Checkpoint**

Run: `rtk cargo clippy --all-targets`
Expected: no new warnings.

---

### Task 3: API types and JSON parsing

**Files:**
- Create: `src/github/api.rs`
- Modify: `src/github/mod.rs`

- [ ] **Step 1: Write the failing tests**

Create `src/github/api.rs` with its tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const PULLS: &str = r#"[
      {"number": 58, "title": "feat: connect servers", "user": {"login": "Ajustor"},
       "head": {"ref": "feat/connect-servers", "sha": "aaa", "repo": {"full_name": "Ajustor/writingUnicorns"}},
       "base": {"ref": "master", "sha": "bbb", "repo": {"full_name": "Ajustor/writingUnicorns"}},
       "draft": false, "body": "Body", "state": "open",
       "html_url": "https://github.com/Ajustor/writingUnicorns/pull/58"},
      {"number": 57, "title": "wip", "user": null,
       "head": {"ref": "wip", "sha": "ccc", "repo": null},
       "base": {"ref": "master", "sha": "bbb", "repo": {"full_name": "Ajustor/writingUnicorns"}},
       "draft": true, "body": null,
       "html_url": "https://github.com/Ajustor/writingUnicorns/pull/57"}
    ]"#;

    const COMMENTS: &str = r#"[
      {"id": 1, "path": "src/a.rs", "line": 12, "original_line": 12, "side": "RIGHT",
       "user": {"login": "rev"}, "body": "Why?", "created_at": "2026-10-14T09:00:00Z"},
      {"id": 2, "in_reply_to_id": 1, "path": "src/a.rs", "line": 12, "original_line": 12, "side": "RIGHT",
       "user": {"login": "Ajustor"}, "body": "Because.", "created_at": "2026-10-14T10:00:00Z"},
      {"id": 3, "path": "src/b.rs", "line": null, "original_line": 4, "side": "LEFT",
       "user": null, "body": "Old", "created_at": "2026-10-13T08:00:00Z"},
      {"id": 4, "in_reply_to_id": 99, "path": "src/c.rs", "line": 2, "original_line": 2, "side": "RIGHT",
       "user": {"login": "rev"}, "body": "Orphan", "created_at": "2026-10-14T11:00:00Z"}
    ]"#;

    #[test]
    fn pull_list_keeps_what_the_panel_shows() {
        let pulls = parse_pulls(PULLS).unwrap();
        assert_eq!(
            pulls[0],
            PullSummary {
                number: 58,
                title: "feat: connect servers".into(),
                author: "Ajustor".into(),
                head_ref: "feat/connect-servers".into(),
                base_ref: "master".into(),
                draft: false,
            }
        );
        // A deleted account is GitHub's "ghost".
        assert_eq!((pulls[1].author.as_str(), pulls[1].draft), ("ghost", true));
    }

    #[test]
    fn pull_detail_without_body_or_fork() {
        let list: Vec<serde_json::Value> = serde_json::from_str(PULLS).unwrap();
        let detail = parse_detail(&list[1].to_string()).unwrap();
        assert_eq!(detail.body, "");
        assert_eq!(detail.head_repo, None);
        assert_eq!((detail.head_sha.as_str(), detail.base_sha.as_str()), ("ccc", "bbb"));
        let detail = parse_detail(&list[0].to_string()).unwrap();
        assert_eq!(detail.head_repo.as_deref(), Some("Ajustor/writingUnicorns"));
        assert_eq!(detail.html_url, "https://github.com/Ajustor/writingUnicorns/pull/58");
    }

    #[test]
    fn unexpected_json_is_a_bad_response() {
        assert_eq!(parse_pulls("{}"), Err(GithubError::BadResponse));
        assert_eq!(parse_detail("[]"), Err(GithubError::BadResponse));
    }

    #[test]
    fn comments_are_grouped_into_threads() {
        let threads = group_threads(parse_comments(COMMENTS).unwrap());
        assert_eq!(threads.len(), 3);
        assert_eq!((threads[0].path.as_str(), threads[0].side, threads[0].line), ("src/a.rs", Side::Right, Some(12)));
        let bodies: Vec<&str> = threads[0].comments.iter().map(|c| c.body.as_str()).collect();
        assert_eq!(bodies, ["Why?", "Because."]);
        // Outdated: no line any more, but the original one is kept.
        assert_eq!((threads[1].side, threads[1].line, threads[1].original_line), (Side::Left, None, Some(4)));
        assert_eq!(threads[1].comments[0].author, "ghost");
        // A reply whose parent is missing starts its own thread.
        assert_eq!(threads[2].comments[0].body, "Orphan");
        assert_eq!(threads[0].comments[0].date(), "2026-10-14");
    }

    #[test]
    fn next_page_comes_from_the_link_header() {
        let link = r#"<https://api.github.com/x?page=2>; rel="next", <https://api.github.com/x?page=5>; rel="last""#;
        assert_eq!(next_link(link).as_deref(), Some("https://api.github.com/x?page=2"));
        assert_eq!(next_link(r#"<https://api.github.com/x?page=1>; rel="prev""#), None);
    }
}
```

In `src/github/mod.rs`, after `pub mod auth;`:

```rust
pub mod api;
```

and after `pub use error::GithubError;`:

```rust
pub use api::{PullDetail, PullSummary, ReviewComment, ReviewThread, Side};
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test github::api`
Expected: compile errors: `parse_pulls`, `PullSummary`… not found.

- [ ] **Step 3: Implement the types and parsing**

At the top of `src/github/api.rs`:

```rust
//! GitHub REST API: the calls the pull request view needs, and their JSON.

use super::error::GithubError;
use serde::Deserialize;
use std::collections::HashMap;

/// An open pull request, as the Git panel lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullSummary {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub head_ref: String,
    pub base_ref: String,
    pub draft: bool,
}

/// A pull request with what its tab needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullDetail {
    pub summary: PullSummary,
    /// Markdown; empty when there is none.
    pub body: String,
    pub head_sha: String,
    pub base_sha: String,
    /// `owner/name` the head branch lives in; `None` when that fork was deleted.
    pub head_repo: Option<String>,
    pub html_url: String,
}

/// Which version of a file a review comment is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// The base: left column.
    Left,
    /// The pull request: right column.
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewComment {
    pub author: String,
    /// Markdown.
    pub body: String,
    /// ISO 8601, as GitHub sends it.
    pub created_at: String,
}

impl ReviewComment {
    /// `YYYY-MM-DD` of `created_at`.
    pub fn date(&self) -> &str {
        self.created_at.get(..10).unwrap_or(&self.created_at)
    }
}

/// A review comment and its replies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewThread {
    pub path: String,
    pub side: Side,
    /// 1-based line on `side` in the pull request's current diff; `None`
    /// when GitHub reports the thread as outdated.
    pub line: Option<u32>,
    /// The line the thread was first written on.
    pub original_line: Option<u32>,
    pub comments: Vec<ReviewComment>,
}

// ── Wire format ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct WireUser {
    login: String,
}

#[derive(Deserialize)]
struct WireRepo {
    full_name: String,
}

#[derive(Deserialize)]
struct WireRef {
    #[serde(rename = "ref")]
    name: String,
    sha: String,
    repo: Option<WireRepo>,
}

#[derive(Deserialize)]
struct WirePull {
    number: u64,
    title: String,
    user: Option<WireUser>,
    head: WireRef,
    base: WireRef,
    #[serde(default)]
    draft: bool,
    body: Option<String>,
    html_url: String,
}

#[derive(Deserialize)]
pub(crate) struct WireComment {
    id: u64,
    in_reply_to_id: Option<u64>,
    path: String,
    line: Option<u32>,
    original_line: Option<u32>,
    side: Option<String>,
    user: Option<WireUser>,
    body: String,
    created_at: String,
}

impl WirePull {
    fn into_detail(self) -> PullDetail {
        PullDetail {
            summary: PullSummary {
                number: self.number,
                title: self.title,
                author: login(self.user),
                head_ref: self.head.name,
                base_ref: self.base.name,
                draft: self.draft,
            },
            body: self.body.unwrap_or_default(),
            head_sha: self.head.sha,
            base_sha: self.base.sha,
            head_repo: self.head.repo.map(|r| r.full_name),
            html_url: self.html_url,
        }
    }
}

/// Login of `user`; GitHub sends `null` for deleted accounts.
fn login(user: Option<WireUser>) -> String {
    user.map_or_else(|| "ghost".to_string(), |u| u.login)
}

fn parse<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, GithubError> {
    serde_json::from_str(json).map_err(|e| {
        log::warn!("GitHub: unexpected response: {e}");
        GithubError::BadResponse
    })
}

pub(crate) fn parse_pulls(json: &str) -> Result<Vec<PullSummary>, GithubError> {
    let pulls: Vec<WirePull> = parse(json)?;
    Ok(pulls.into_iter().map(|p| p.into_detail().summary).collect())
}

pub(crate) fn parse_detail(json: &str) -> Result<PullDetail, GithubError> {
    Ok(parse::<WirePull>(json)?.into_detail())
}

pub(crate) fn parse_comments(json: &str) -> Result<Vec<WireComment>, GithubError> {
    parse(json)
}

/// Group review comments (in GitHub's order) into threads: a comment without
/// `in_reply_to_id` starts one, a reply joins the thread of the comment it
/// answers, and a reply whose parent is missing starts its own.
pub(crate) fn group_threads(comments: Vec<WireComment>) -> Vec<ReviewThread> {
    let mut threads: Vec<ReviewThread> = Vec::new();
    let mut thread_of: HashMap<u64, usize> = HashMap::new();
    for c in comments {
        let comment = ReviewComment {
            author: login(c.user),
            body: c.body,
            created_at: c.created_at,
        };
        let parent = c.in_reply_to_id.and_then(|id| thread_of.get(&id).copied());
        let index = match parent {
            Some(t) => {
                threads[t].comments.push(comment);
                t
            }
            None => {
                threads.push(ReviewThread {
                    path: c.path,
                    side: if c.side.as_deref() == Some("LEFT") {
                        Side::Left
                    } else {
                        Side::Right
                    },
                    line: c.line,
                    original_line: c.original_line,
                    comments: vec![comment],
                });
                threads.len() - 1
            }
        };
        thread_of.insert(c.id, index);
    }
    threads
}

/// The `rel="next"` URL of a `Link` header.
pub(crate) fn next_link(header: &str) -> Option<String> {
    header.split(',').find_map(|part| {
        let mut pieces = part.split(';');
        let url = pieces.next()?.trim().strip_prefix('<')?.strip_suffix('>')?;
        pieces
            .any(|p| p.trim() == "rel=\"next\"")
            .then(|| url.to_string())
    })
}

/// A one-comment thread, for tests.
#[cfg(test)]
pub(crate) fn thread(path: &str, side: Side, line: Option<u32>) -> ReviewThread {
    ReviewThread {
        path: path.to_string(),
        side,
        line,
        original_line: line.or(Some(1)),
        comments: vec![ReviewComment {
            author: "reviewer".into(),
            body: "Why?".into(),
            created_at: "2026-10-14T09:00:00Z".into(),
        }],
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `rtk cargo test github::api`
Expected: 5 tests pass.

- [ ] **Step 5: Checkpoint**

Run: `rtk cargo clippy --all-targets`
Expected: no new warnings.

---

### Task 4: API client

**Files:**
- Modify: `src/github/api.rs`, `src/github/mod.rs`

- [ ] **Step 1: Write the failing test**

In the tests of `src/github/api.rs`:

```rust
    #[test]
    fn requests_stay_on_the_repository_api() {
        let repo = crate::github::RepoRef {
            owner: "me".into(),
            name: "proj".into(),
        };
        let client = Client::new(repo, None);
        assert_eq!(
            client.url("pulls/7"),
            "https://api.github.com/repos/me/proj/pulls/7"
        );
        let page = "https://api.github.com/repos/me/proj/pulls?page=2".to_string();
        assert_eq!(same_api(page.clone()), Some(page));
        // A `Link` leading elsewhere would receive the token.
        assert_eq!(same_api("https://evil.example/steal".into()), None);
        assert_eq!(same_api("https://api.github.com.evil.example/x".into()), None);
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `rtk cargo test github::api::tests::requests_stay`
Expected: compile errors: `Client`, `same_api` not found.

- [ ] **Step 3: Implement the client**

In `src/github/api.rs`, replace the imports at the top with:

```rust
use super::error::GithubError;
use super::RepoRef;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
```

Then, after `next_link` (before the `#[cfg(test)] pub(crate) fn thread`):

```rust
// ── HTTP ────────────────────────────────────────────────────────────────────

const API: &str = "https://api.github.com";
const USER_AGENT: &str = concat!("coding-unicorns/", env!("CARGO_PKG_VERSION"));
/// Cap on one response body, against a runaway response.
const MAX_BODY: u64 = 10 * 1024 * 1024;
/// Cap on followed pages (100 items each).
const MAX_PAGES: usize = 20;

/// Blocking client for one repository: use it off the UI thread.
pub struct Client {
    repo: RepoRef,
    token: Option<String>,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(repo: RepoRef, token: Option<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .tls_config(crate::extension::remote_registry::tls_config())
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        Self { repo, token, agent }
    }

    /// The open pull requests, most recent first.
    pub fn list_open_pulls(&self) -> Result<Vec<PullSummary>, GithubError> {
        let mut pulls = Vec::new();
        for page in self.get_pages(self.url("pulls?state=open&per_page=100"))? {
            pulls.extend(parse_pulls(&page)?);
        }
        Ok(pulls)
    }

    pub fn pull_detail(&self, number: u64) -> Result<PullDetail, GithubError> {
        let (body, _) = self.get(&self.url(&format!("pulls/{number}")))?;
        parse_detail(&body)
    }

    /// The review comments of pull request `number`, grouped into threads.
    pub fn review_threads(&self, number: u64) -> Result<Vec<ReviewThread>, GithubError> {
        let mut comments = Vec::new();
        let first = self.url(&format!("pulls/{number}/comments?per_page=100"));
        for page in self.get_pages(first)? {
            comments.extend(parse_comments(&page)?);
        }
        Ok(group_threads(comments))
    }

    fn url(&self, rest: &str) -> String {
        format!("{API}/repos/{}/{}/{rest}", self.repo.owner, self.repo.name)
    }

    /// Bodies of `first` and of the pages its `Link` headers lead to.
    fn get_pages(&self, first: String) -> Result<Vec<String>, GithubError> {
        let mut pages = Vec::new();
        let mut next = Some(first);
        while let Some(url) = next.take() {
            if pages.len() == MAX_PAGES {
                break;
            }
            let (body, link) = self.get(&url)?;
            pages.push(body);
            next = link;
        }
        Ok(pages)
    }

    /// GET `url`: its body, and the URL of the next page if any.
    fn get(&self, url: &str) -> Result<(String, Option<String>), GithubError> {
        let mut request = self
            .agent
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", USER_AGENT);
        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let mut response = request
            .call()
            .map_err(|e| GithubError::Network(e.to_string()))?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            // A refused token (401), or one that can't see the repository
            // (404, e.g. after `gh auth switch`): ask `gh` again next time.
            if status == 401 || (status == 404 && self.token.is_some()) {
                super::auth::forget();
            }
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            return Err(GithubError::from_response(
                status,
                self.token.is_some(),
                header("x-ratelimit-remaining").as_deref(),
                header("x-ratelimit-reset").as_deref(),
                now,
            ));
        }
        let next = header("link")
            .as_deref()
            .and_then(next_link)
            .and_then(same_api);
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_string()
            .map_err(|e| GithubError::Network(e.to_string()))?;
        Ok((body, next))
    }
}

/// `url` if it is on the GitHub API: the token must never leave it.
fn same_api(url: String) -> Option<String> {
    url.starts_with(&format!("{API}/")).then_some(url)
}
```

In `src/github/mod.rs`, change the `api` re-export to:

```rust
pub use api::{Client, PullDetail, PullSummary, ReviewThread, Side};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `rtk cargo test github::`
Expected: all `github::` tests pass.

- [ ] **Step 5: Checkpoint**

Run: `rtk cargo clippy --all-targets`
Expected: no new warnings.

---

### Task 5: Changed words of a modified line

**Files:**
- Create: `src/git/word_diff.rs`
- Modify: `src/git/mod.rs`

- [ ] **Step 1: Write the failing tests**

Create `src/git/word_diff.rs` with its tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_word_in_the_middle() {
        let w = changed_ranges("let x = 1;", "let y = 1;").unwrap();
        assert_eq!((w.old, w.new), (vec![4..5], vec![4..5]));
    }

    #[test]
    fn identical_lines_have_no_changes() {
        assert_eq!(changed_ranges("a b", "a b").unwrap(), WordChanges::default());
    }

    #[test]
    fn appended_words_merge_into_one_range() {
        let w = changed_ranges("a b", "a b c").unwrap();
        assert_eq!((w.old, w.new), (vec![], vec![3..5]));
    }

    #[test]
    fn ranges_are_byte_offsets_of_whole_characters() {
        let w = changed_ranges("café x", "café y").unwrap();
        assert_eq!((w.old, w.new), (vec![6..7], vec![6..7]));
    }

    #[test]
    fn long_lines_are_not_compared() {
        assert_eq!(changed_ranges(&"x".repeat(MAX_LINE + 1), "x"), None);
    }
}
```

In `src/git/mod.rs`, after `pub mod watch;`:

```rust
pub mod word_diff;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test git::word_diff`
Expected: compile errors: `changed_ranges`, `WordChanges`, `MAX_LINE` not found.

- [ ] **Step 3: Implement the word diff**

At the top of `src/git/word_diff.rs`:

```rust
//! Which words changed between two versions of a line, for the highlights of
//! the pull request diff.

use std::ops::Range;

/// Lines longer than this (in bytes) get no word highlights.
pub const MAX_LINE: usize = 1000;

/// Byte ranges of a line pair that are not part of their longest common
/// word subsequence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WordChanges {
    pub old: Vec<Range<usize>>,
    pub new: Vec<Range<usize>>,
}

/// The changed words of `old` → `new`; `None` when a line is too long to compare.
pub fn changed_ranges(old: &str, new: &str) -> Option<WordChanges> {
    if old.len() > MAX_LINE || new.len() > MAX_LINE {
        return None;
    }
    let (a, b) = (tokens(old), tokens(new));
    let (n, m) = (a.len(), b.len());
    let same = |i: usize, j: usize| old[a[i].clone()] == new[b[j].clone()];
    // lcs[i][j]: length of the longest common subsequence of a[i..] and b[j..].
    let mut lcs = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if same(i, j) {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut keep_a, mut keep_b) = (vec![false; n], vec![false; m]);
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if same(i, j) {
            keep_a[i] = true;
            keep_b[j] = true;
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    Some(WordChanges {
        old: merge(&a, &keep_a),
        new: merge(&b, &keep_b),
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Other,
}

fn class(c: char) -> Class {
    if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else if c.is_whitespace() {
        Class::Space
    } else {
        Class::Other
    }
}

/// Runs of word characters, runs of spaces, and single other characters.
fn tokens(s: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut chars = s.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let kind = class(c);
        let mut end = start + c.len_utf8();
        if kind != Class::Other {
            while let Some(&(i, d)) = chars.peek() {
                if class(d) != kind {
                    break;
                }
                end = i + d.len_utf8();
                chars.next();
            }
        }
        out.push(start..end);
    }
    out
}

/// The tokens not kept, adjacent ones merged.
fn merge(tokens: &[Range<usize>], keep: &[bool]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for (t, &kept) in tokens.iter().zip(keep) {
        if kept {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.end == t.start => last.end = t.end,
            _ => out.push(t.clone()),
        }
    }
    out
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `rtk cargo test git::word_diff`
Expected: 5 tests pass.

- [ ] **Step 5: Checkpoint**

Run: `rtk cargo clippy --all-targets`
Expected: no new warnings.

---

### Task 6: Side-by-side line alignment

**Files:**
- Create: `src/git/compare.rs`
- Modify: `src/git/mod.rs`

- [ ] **Step 1: Write the failing tests**

Create `src/git/compare.rs` with its tests:

```rust
#[cfg(test)]
mod tests {
    use super::DiffRow::*;
    use super::*;

    fn rows(old: &str, new: &str) -> Vec<DiffRow> {
        TextDiff::new(old, new).rows
    }

    #[test]
    fn an_insertion_faces_a_filler() {
        assert_eq!(
            rows("a\nb\nc\n", "a\nb\nX\nc\n"),
            vec![
                Context { old: 0, new: 0 },
                Context { old: 1, new: 1 },
                Added { new: 2 },
                Context { old: 2, new: 3 },
            ]
        );
    }

    #[test]
    fn a_deletion_faces_a_filler() {
        assert_eq!(
            rows("a\nb\nc\n", "a\nc\n"),
            vec![Context { old: 0, new: 0 }, Removed { old: 1 }, Context { old: 2, new: 1 }]
        );
    }

    #[test]
    fn replaced_lines_pair_up_and_extra_ones_are_added() {
        assert_eq!(
            rows("a\nb\nc\n", "a\nB\nB2\nc\n"),
            vec![
                Context { old: 0, new: 0 },
                Modified { old: 1, new: 1 },
                Added { new: 2 },
                Context { old: 2, new: 3 },
            ]
        );
    }

    #[test]
    fn unchanged_runs_fold_beyond_the_context() {
        let old: String = (0..10).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l5\n", "L5\n");
        assert_eq!(
            rows(&old, &new),
            vec![
                Fold { old: 0, new: 0, len: 2 },
                Context { old: 2, new: 2 },
                Context { old: 3, new: 3 },
                Context { old: 4, new: 4 },
                Modified { old: 5, new: 5 },
                Context { old: 6, new: 6 },
                Context { old: 7, new: 7 },
                Context { old: 8, new: 8 },
                Fold { old: 9, new: 9, len: 1 },
            ]
        );
    }

    #[test]
    fn identical_files_are_one_fold_and_new_files_all_added() {
        assert_eq!(rows("a\nb\n", "a\nb\n"), vec![Fold { old: 0, new: 0, len: 2 }]);
        assert_eq!(rows("", "a\nb\n"), vec![Added { new: 0 }, Added { new: 1 }]);
    }

    #[test]
    fn unfold_replaces_a_fold_by_its_lines() {
        let mut d = TextDiff::new("a\nb\n", "a\nb\n");
        assert!(d.unfold(0));
        assert_eq!(d.rows, vec![Context { old: 0, new: 0 }, Context { old: 1, new: 1 }]);
        assert!(!d.unfold(0), "not a fold any more");
    }

    #[test]
    fn modified_rows_carry_their_changed_words_and_count_both_ways() {
        let d = TextDiff::new("let x = 1;\n", "let y = 1;\nmore\n");
        assert_eq!(d.words[&0].new, vec![4..5]);
        assert_eq!(d.counts(), (2, 1));
    }

    #[test]
    fn crlf_and_lf_compare_equal_and_tabs_become_spaces() {
        let DiffContent::Text(d) = text_content(b"a\r\nb\r\n", b"a\nb\n", false) else {
            panic!("not text");
        };
        assert_eq!(d.counts(), (0, 0));
        let DiffContent::Text(d) = text_content(b"", b"\tx\n", false) else {
            panic!("not text");
        };
        assert_eq!(d.new, vec!["    x".to_string()]);
    }

    #[test]
    fn nul_bytes_and_invalid_utf8_are_binary() {
        assert_eq!(text_content(b"a\0b", b"a", false), DiffContent::Binary);
        assert_eq!(text_content(b"a", &[0xff, 0xfe], false), DiffContent::Binary);
    }

    #[test]
    fn huge_files_wait_for_show_anyway() {
        let big = "x\n".repeat(crate::editor::LARGE_FILE_LINES + 1);
        assert_eq!(text_content(b"", big.as_bytes(), false), DiffContent::TooLarge);
        assert!(matches!(text_content(b"", big.as_bytes(), true), DiffContent::Text(_)));
    }
}
```

In `src/git/mod.rs`, after `pub mod commit;`:

```rust
pub mod compare;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test git::compare`
Expected: compile errors: `TextDiff`, `DiffRow`, `text_content`, `DiffContent` not found.

- [ ] **Step 3: Implement the alignment**

At the top of `src/git/compare.rs`:

```rust
//! The diff of a pull request, computed locally from its commits: the files
//! it changes and, for each text file, its lines paired side by side.

use super::word_diff::{changed_ranges, WordChanges};
use crate::editor::{LARGE_FILE_BYTES, LARGE_FILE_LINES};
use std::collections::HashMap;

/// Unchanged lines shown around each change; the rest is folded.
pub const CONTEXT: usize = 3;

/// One row of a side-by-side diff. Indexes are 0-based lines of
/// [`TextDiff::old`] and [`TextDiff::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffRow {
    /// The same line on both sides.
    Context { old: usize, new: usize },
    /// A removed line facing the added line that replaces it.
    Modified { old: usize, new: usize },
    /// A removed line facing a filler.
    Removed { old: usize },
    /// An added line facing a filler.
    Added { new: usize },
    /// `len` unchanged lines starting at `old` / `new`, folded.
    Fold { old: usize, new: usize, len: usize },
}

impl DiffRow {
    pub fn is_change(self) -> bool {
        matches!(
            self,
            Self::Modified { .. } | Self::Removed { .. } | Self::Added { .. }
        )
    }
}

/// A text file's two versions and their rows.
#[derive(Debug, Clone, PartialEq)]
pub struct TextDiff {
    pub old: Vec<String>,
    pub new: Vec<String>,
    pub rows: Vec<DiffRow>,
    /// Changed words of each `Modified` row, by its old line.
    pub words: HashMap<usize, WordChanges>,
}

impl TextDiff {
    /// Diff `old` against `new` (both with `\n` line endings).
    pub fn new(old: &str, new: &str) -> Self {
        let rows = align(old, new);
        let old: Vec<String> = old.lines().map(str::to_string).collect();
        let new: Vec<String> = new.lines().map(str::to_string).collect();
        let words = rows
            .iter()
            .filter_map(|row| match *row {
                DiffRow::Modified { old: o, new: n } => {
                    Some((o, changed_ranges(old.get(o)?, new.get(n)?)?))
                }
                _ => None,
            })
            .collect();
        Self {
            old,
            new,
            rows,
            words,
        }
    }

    /// Replace the fold at `row` by its lines. False when `row` isn't a fold.
    pub fn unfold(&mut self, row: usize) -> bool {
        let Some(&DiffRow::Fold { old, new, len }) = self.rows.get(row) else {
            return false;
        };
        self.rows.splice(
            row..=row,
            (0..len).map(|k| DiffRow::Context {
                old: old + k,
                new: new + k,
            }),
        );
        true
    }

    /// Lines added and removed; a modified line counts as both.
    pub fn counts(&self) -> (usize, usize) {
        self.rows.iter().fold((0, 0), |(add, del), row| match row {
            DiffRow::Modified { .. } => (add + 1, del + 1),
            DiffRow::Added { .. } => (add + 1, del),
            DiffRow::Removed { .. } => (add, del + 1),
            _ => (add, del),
        })
    }
}

/// What the view shows for a file.
#[derive(Debug, Clone, PartialEq)]
pub enum DiffContent {
    Text(TextDiff),
    /// NUL bytes or not UTF-8.
    Binary,
    /// Over the editor's large-file limits: shown on demand ("Show anyway").
    TooLarge,
}

/// The content to show for a file going from `old` to `new`; `force` ignores
/// the size limits.
pub fn text_content(old: &[u8], new: &[u8], force: bool) -> DiffContent {
    let (Some(old), Some(new)) = (as_text(old), as_text(new)) else {
        return DiffContent::Binary;
    };
    if !force && (too_large(&old) || too_large(&new)) {
        return DiffContent::TooLarge;
    }
    DiffContent::Text(TextDiff::new(&old, &new))
}

/// UTF-8 text without NUL bytes, with `\n` line endings and tabs as 4 spaces.
fn as_text(bytes: &[u8]) -> Option<String> {
    if bytes.contains(&0) {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    Some(text.replace("\r\n", "\n").replace('\t', "    "))
}

fn too_large(text: &str) -> bool {
    text.len() > LARGE_FILE_BYTES || text.lines().count() > LARGE_FILE_LINES
}

/// Rows of `old` → `new`: each hunk paired line by line, unchanged runs
/// shown [`CONTEXT`] lines around the hunks and folded beyond.
fn align(old: &str, new: &str) -> Vec<DiffRow> {
    let old_len = old.lines().count();
    let mut rows = Vec::new();
    let (mut o, mut n) = (0, 0);
    let hunks = hunks(old, new);
    for (index, &(old_start, old_count, new_start, new_count)) in hunks.iter().enumerate() {
        push_gap(&mut rows, o, n, old_start.saturating_sub(o), index == 0, false);
        let paired = old_count.min(new_count);
        rows.extend((0..paired).map(|k| DiffRow::Modified {
            old: old_start + k,
            new: new_start + k,
        }));
        rows.extend((paired..old_count).map(|k| DiffRow::Removed { old: old_start + k }));
        rows.extend((paired..new_count).map(|k| DiffRow::Added { new: new_start + k }));
        o = old_start + old_count;
        n = new_start + new_count;
    }
    push_gap(&mut rows, o, n, old_len.saturating_sub(o), hunks.is_empty(), true);
    rows
}

/// Hunks of `old` → `new` without context, as 0-based
/// `(old_start, old_count, new_start, new_count)`.
fn hunks(old: &str, new: &str) -> Vec<(usize, usize, usize, usize)> {
    let mut opts = git2::DiffOptions::new();
    opts.context_lines(0);
    let patch = match git2::Patch::from_buffers(
        old.as_bytes(),
        None,
        new.as_bytes(),
        None,
        Some(&mut opts),
    ) {
        Ok(patch) => patch,
        Err(e) => {
            log::warn!("diff failed: {e}");
            return Vec::new();
        }
    };
    (0..patch.num_hunks())
        .filter_map(|h| patch.hunk(h).ok())
        .map(|(hunk, _)| {
            let (old_count, new_count) = (hunk.old_lines() as usize, hunk.new_lines() as usize);
            // An empty side's start is the line *before* the change (0 = top).
            let start = |start: u32, count: usize| {
                if count == 0 {
                    start as usize
                } else {
                    start as usize - 1
                }
            };
            (
                start(hunk.old_start(), old_count),
                old_count,
                start(hunk.new_start(), new_count),
                new_count,
            )
        })
        .collect()
}

/// `len` unchanged lines starting at `old` / `new`: [`CONTEXT`] lines after
/// the previous hunk (none at the top of the file) and before the next one
/// (none at the end), the rest folded.
fn push_gap(rows: &mut Vec<DiffRow>, old: usize, new: usize, len: usize, at_start: bool, at_end: bool) {
    let head = if at_start { 0 } else { CONTEXT.min(len) };
    let tail = if at_end { 0 } else { CONTEXT.min(len - head) };
    let folded = len - head - tail;
    rows.extend((0..head).map(|k| DiffRow::Context { old: old + k, new: new + k }));
    if folded > 0 {
        rows.push(DiffRow::Fold {
            old: old + head,
            new: new + head,
            len: folded,
        });
    }
    rows.extend((len - tail..len).map(|k| DiffRow::Context { old: old + k, new: new + k }));
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `rtk cargo test git::compare`
Expected: 10 tests pass. If `an_insertion_faces_a_filler` or `a_deletion_faces_a_filler` fails, print `hunks(...)` for the case: the start rule in `hunks` (an empty side's start is the line before the change) is what lines the rows up.

- [ ] **Step 5: Checkpoint**

Run: `rtk cargo clippy --all-targets`
Expected: no new warnings.

---

### Task 7: Files of a pull request

**Files:**
- Modify: `src/git/compare.rs`

- [ ] **Step 1: Write the failing tests**

In the tests of `src/git/compare.rs`, add:

```rust
    use crate::git::tests::TestRepo;

    /// `main` and a `feature` branch made from it; then `main` moves on.
    /// Returns the repo, `main`'s tip and `feature`'s tip.
    fn pull_repo() -> (TestRepo, String, String) {
        let r = TestRepo::new();
        r.write("a.txt", "one\ntwo\nthree\n");
        r.write("b.txt", "gone\n");
        r.write("r.txt", &"same\n".repeat(10));
        r.commit_all("init");
        r.branch("feature");
        r.switch("feature");
        r.write("a.txt", "one\nTWO\nthree\n");
        r.remove("b.txt");
        r.write("c.txt", "new\n");
        r.remove("r.txt");
        r.write("s.txt", &"same\n".repeat(10));
        let head = r.commit_all("feature");
        r.switch("main");
        r.write("d.txt", "base only\n");
        let base = r.commit_all("main moves on");
        (r, base.to_string(), head.to_string())
    }

    #[test]
    fn diff_pull_compares_against_the_merge_base() {
        let (r, base, head) = pull_repo();
        let files = diff_pull(r.path(), &base, &head).unwrap();
        let paths: Vec<&str> = files.iter().map(FileDiff::path).collect();
        let find = |p: &str| {
            files
                .iter()
                .find(|f| f.path() == p)
                .unwrap_or_else(|| panic!("{p} not in {paths:?}"))
        };
        assert!(!paths.contains(&"d.txt"), "a change of the base is shown");
        assert_eq!(files.len(), 4, "{paths:?}");
        let a = find("a.txt");
        assert_eq!((a.kind, a.additions, a.deletions), (FileDiffKind::Modified, 1, 1));
        let b = find("b.txt");
        assert_eq!((b.kind, b.new_path.as_deref(), b.deletions), (FileDiffKind::Deleted, None, 1));
        let c = find("c.txt");
        assert_eq!((c.kind, c.old_path.as_deref(), c.additions), (FileDiffKind::Added, None, 1));
        let s = find("s.txt");
        assert_eq!(
            (s.kind, s.old_path.as_deref(), s.display_path()),
            (FileDiffKind::Renamed, Some("r.txt"), "r.txt → s.txt".to_string())
        );
    }

    #[test]
    fn binary_files_are_flagged() {
        let r = TestRepo::new();
        r.write("x.bin", "a\0b");
        let base = r.commit_all("init");
        r.write("x.bin", "a\0c");
        let head = r.commit_all("change");
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        assert_eq!(files[0].content, DiffContent::Binary);
        assert_eq!((files[0].additions, files[0].deletions), (0, 0));
    }

    #[test]
    fn unknown_commits_ask_for_a_refresh() {
        let r = TestRepo::new();
        r.write("a", "1");
        let c = r.commit_all("init");
        let err = diff_pull(r.path(), &c.to_string(), "0123456789abcdef0123456789abcdef01234567")
            .unwrap_err();
        assert_eq!(err, CHANGED);
    }

    #[test]
    fn load_text_ignores_the_size_limit() {
        let r = TestRepo::new();
        r.write("a", "1\n");
        let base = r.commit_all("init");
        r.write("a", &"x\n".repeat(crate::editor::LARGE_FILE_LINES + 1));
        let head = r.commit_all("big");
        let files = diff_pull(r.path(), &base.to_string(), &head.to_string()).unwrap();
        assert_eq!(files[0].content, DiffContent::TooLarge);
        let content = load_text(r.path(), files[0].old_blob, files[0].new_blob).unwrap();
        assert!(matches!(content, DiffContent::Text(_)));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test git::compare`
Expected: compile errors: `diff_pull`, `FileDiff`, `FileDiffKind`, `CHANGED`, `load_text` not found.

- [ ] **Step 3: Implement the file list**

In `src/git/compare.rs`, add `use std::path::Path;` to the imports, then add after `push_gap`:

```rust
// ── Files of a pull request ─────────────────────────────────────────────────

/// Why a commit of the pull request is missing locally.
pub const CHANGED: &str = "The pull request changed while loading, refresh it.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDiffKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// One file the pull request changes.
#[derive(Debug, Clone, PartialEq)]
pub struct FileDiff {
    /// `None` for an added file.
    pub old_path: Option<String>,
    /// `None` for a deleted file.
    pub new_path: Option<String>,
    pub kind: FileDiffKind,
    pub additions: usize,
    pub deletions: usize,
    pub old_blob: Option<git2::Oid>,
    /// Content in the pull request: a file marked viewed stays viewed while it is the same.
    pub new_blob: Option<git2::Oid>,
    pub content: DiffContent,
}

impl FileDiff {
    /// The path in the pull request (the old one for a deleted file).
    pub fn path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or("")
    }

    /// `old → new` for a rename, the path otherwise.
    pub fn display_path(&self) -> String {
        match (self.kind, &self.old_path, &self.new_path) {
            (FileDiffKind::Renamed, Some(old), Some(new)) => format!("{old} → {new}"),
            _ => self.path().to_string(),
        }
    }

    /// Set the content, and the counts that go with it.
    pub fn set_content(&mut self, content: DiffContent) {
        (self.additions, self.deletions) = match &content {
            DiffContent::Text(text) => text.counts(),
            _ => (0, 0),
        };
        self.content = content;
    }
}

/// The files `head_sha` changes since its merge base with `base_sha`
/// (GitHub's three-dot diff), renames detected.
pub fn diff_pull(workdir: &Path, base_sha: &str, head_sha: &str) -> Result<Vec<FileDiff>, String> {
    let repo = git2::Repository::discover(workdir).map_err(|e| format!("Repo error: {e}"))?;
    let base = find_commit(&repo, base_sha)?;
    let head = find_commit(&repo, head_sha)?;
    let merge_base = repo.merge_base(base.id(), head.id()).map_err(|e| {
        format!(
            "The pull request has no history in common with its base: {}",
            e.message()
        )
    })?;
    let old_tree = repo
        .find_commit(merge_base)
        .and_then(|c| c.tree())
        .map_err(|e| e.message().to_string())?;
    let new_tree = head.tree().map_err(|e| e.message().to_string())?;
    let mut diff = repo
        .diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None)
        .map_err(|e| e.message().to_string())?;
    diff.find_similar(Some(git2::DiffFindOptions::new().renames(true)))
        .map_err(|e| e.message().to_string())?;
    diff.deltas().map(|delta| file_diff(&repo, &delta)).collect()
}

/// Reload a file's content as text whatever its size ("Show anyway").
pub fn load_text(
    workdir: &Path,
    old_blob: Option<git2::Oid>,
    new_blob: Option<git2::Oid>,
) -> Result<DiffContent, String> {
    let repo = git2::Repository::discover(workdir).map_err(|e| format!("Repo error: {e}"))?;
    Ok(text_content(
        &blob_bytes(&repo, old_blob)?,
        &blob_bytes(&repo, new_blob)?,
        true,
    ))
}

fn find_commit<'r>(repo: &'r git2::Repository, sha: &str) -> Result<git2::Commit<'r>, String> {
    git2::Oid::from_str(sha)
        .ok()
        .and_then(|oid| repo.find_commit(oid).ok())
        .ok_or_else(|| CHANGED.to_string())
}

fn file_diff(repo: &git2::Repository, delta: &git2::DiffDelta) -> Result<FileDiff, String> {
    let kind = match delta.status() {
        git2::Delta::Added => FileDiffKind::Added,
        git2::Delta::Deleted => FileDiffKind::Deleted,
        git2::Delta::Renamed => FileDiffKind::Renamed,
        _ => FileDiffKind::Modified,
    };
    let path = |file: git2::DiffFile| file.path().map(|p| p.to_string_lossy().replace('\\', "/"));
    let blob = |file: git2::DiffFile| (!file.id().is_zero()).then(|| file.id());
    let mut file = FileDiff {
        old_path: if kind == FileDiffKind::Added {
            None
        } else {
            path(delta.old_file())
        },
        new_path: if kind == FileDiffKind::Deleted {
            None
        } else {
            path(delta.new_file())
        },
        kind,
        additions: 0,
        deletions: 0,
        old_blob: blob(delta.old_file()),
        new_blob: blob(delta.new_file()),
        content: DiffContent::Binary,
    };
    let old = blob_bytes(repo, file.old_blob)?;
    let new = blob_bytes(repo, file.new_blob)?;
    file.set_content(text_content(&old, &new, false));
    Ok(file)
}

fn blob_bytes(repo: &git2::Repository, blob: Option<git2::Oid>) -> Result<Vec<u8>, String> {
    match blob {
        None => Ok(Vec::new()),
        Some(oid) => repo
            .find_blob(oid)
            .map(|b| b.content().to_vec())
            .map_err(|e| e.message().to_string()),
    }
}

/// A modified text file, for tests.
#[cfg(test)]
pub(crate) fn text_file(path: &str, old: &str, new: &str) -> FileDiff {
    let mut file = binary_file(path);
    file.set_content(DiffContent::Text(TextDiff::new(old, new)));
    file
}

/// A modified binary file, for tests.
#[cfg(test)]
pub(crate) fn binary_file(path: &str) -> FileDiff {
    FileDiff {
        old_path: Some(path.into()),
        new_path: Some(path.into()),
        kind: FileDiffKind::Modified,
        additions: 0,
        deletions: 0,
        old_blob: None,
        new_blob: None,
        content: DiffContent::Binary,
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `rtk cargo test git::compare`
Expected: 14 tests pass.

- [ ] **Step 5: Checkpoint**

Run: `rtk cargo clippy --all-targets`
Expected: no new warnings.

---

### Task 8: Fetching and checking out a pull request

**Files:**
- Modify: `src/git/compare.rs`

- [ ] **Step 1: Write the failing tests**

In the tests of `src/git/compare.rs`, add:

```rust
    #[test]
    fn fetch_pull_brings_the_pull_ref_and_the_base() {
        let r = TestRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        let (_bare, url) = crate::git::tests::bare_origin();
        r.repo.remote("origin", &url).unwrap();
        // Someone's pull request: a commit only GitHub's `refs/pull/7/head` points at.
        r.branch("feature");
        r.switch("feature");
        r.write("a.txt", "2\n");
        let pr_tip = r.commit_all("change");
        r.repo
            .find_remote("origin")
            .unwrap()
            .push(
                &["refs/heads/main:refs/heads/main", "refs/heads/feature:refs/pull/7/head"],
                None,
            )
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let clone = git2::Repository::clone(&url, dir.path()).unwrap();
        assert!(clone.find_reference("refs/remotes/origin/pr/7").is_err());

        fetch_pull(dir.path(), 7, "main").unwrap();

        let pr = clone.find_reference("refs/remotes/origin/pr/7").unwrap();
        assert_eq!(pr.target(), Some(pr_tip));
        assert!(clone.find_reference("refs/remotes/origin/main").is_ok());
    }

    #[test]
    fn branch_name_is_the_head_branch_or_the_number_for_forks() {
        assert_eq!(pull_branch_name(58, "feat/x", false), "feat/x");
        assert_eq!(pull_branch_name(58, "main", true), "pr/58");
    }

    /// A repo on `main` with a `work` branch one commit ahead; returns `work`'s tip.
    fn repo_with_work() -> (TestRepo, git2::Oid) {
        let r = TestRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        r.branch("work");
        r.switch("work");
        r.write("a.txt", "2\n");
        let head = r.commit_all("pr");
        r.switch("main");
        (r, head)
    }

    #[test]
    fn checkout_pull_creates_the_branch_at_the_head() {
        let (r, head) = repo_with_work();
        let mut s = r.status();
        assert_eq!(s.checkout_pull("pr/7", &head.to_string()), Ok(false));
        assert_eq!(r.head_name(), "pr/7");
        assert_eq!(r.head_oid(), head);
        assert_eq!(r.read("a.txt"), "2\n");
    }

    #[test]
    fn checkout_pull_keeps_an_existing_branch_as_is() {
        let r = TestRepo::new();
        r.write("a.txt", "1\n");
        let init = r.commit_all("init");
        r.branch("feat");
        r.write("a.txt", "2\n");
        let newer = r.commit_all("newer on main");
        let mut s = r.status();
        assert_eq!(s.checkout_pull("feat", &newer.to_string()), Ok(true));
        assert_eq!((r.head_name(), r.head_oid()), ("feat".to_string(), init));
    }

    #[test]
    fn a_refused_checkout_leaves_no_branch_behind() {
        let (r, head) = repo_with_work();
        r.write("a.txt", "local edit\n");
        let mut s = r.status();
        assert!(s.checkout_pull("pr/7", &head.to_string()).is_err());
        assert!(r.repo.find_branch("pr/7", git2::BranchType::Local).is_err());
        assert_eq!(r.read("a.txt"), "local edit\n");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test git::compare`
Expected: compile errors: `fetch_pull`, `pull_branch_name`, `checkout_pull` not found.

- [ ] **Step 3: Implement fetch and checkout**

In `src/git/compare.rs`, add `use super::GitStatus;` to the imports, then after `blob_bytes` (before the `#[cfg(test)]` helpers):

```rust
// ── Fetch and checkout ──────────────────────────────────────────────────────

/// Fetch pull request `number`'s head into `refs/remotes/origin/pr/<number>`,
/// and its base branch, from `origin`. Works for pull requests from forks:
/// GitHub keeps every pull request's head under `refs/pull/` in the base repo.
pub fn fetch_pull(workdir: &Path, number: u64, base_ref: &str) -> Result<(), String> {
    let repo = git2::Repository::discover(workdir).map_err(|e| format!("Repo error: {e}"))?;
    let mut remote = repo
        .find_remote("origin")
        .map_err(|e| format!("Remote error: {e}"))?;
    let head = format!("+refs/pull/{number}/head:refs/remotes/origin/pr/{number}");
    let base = format!("+refs/heads/{base_ref}:refs/remotes/origin/{base_ref}");
    super::remote::fetch_remote(&mut remote, &[head.as_str(), base.as_str()])
}

/// Local branch to check pull request `number` out on: its own branch name
/// when it comes from the same repository, `pr/<number>` from a fork.
pub fn pull_branch_name(number: u64, head_ref: &str, from_fork: bool) -> String {
    if from_fork {
        format!("pr/{number}")
    } else {
        head_ref.to_string()
    }
}

impl GitStatus {
    /// Check a pull request out on local branch `branch`, created at
    /// `head_sha` unless it exists (then checked out as is). `Ok(true)` when
    /// the branch already existed. Local changes the checkout would
    /// overwrite refuse it, and a branch created for it is deleted again.
    pub fn checkout_pull(&mut self, branch: &str, head_sha: &str) -> Result<bool, String> {
        let repo = self.open_repo()?;
        let existed = repo.find_branch(branch, git2::BranchType::Local).is_ok();
        if !existed {
            let commit = find_commit(&repo, head_sha)?;
            repo.branch(branch, &commit, false)
                .map_err(|e| e.message().to_string())?;
        }
        if let Err(e) = self.checkout_branch(branch) {
            if !existed {
                if let Ok(mut created) = repo.find_branch(branch, git2::BranchType::Local) {
                    let _ = created.delete();
                }
            }
            return Err(e);
        }
        Ok(existed)
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `rtk cargo test git::`
Expected: all `git::` tests pass (the new ones and the existing ones).

- [ ] **Step 5: Checkpoint**

Run: `rtk cargo clippy --all-targets`
Expected: no new warnings.

---

### Task 9: Diff layout model

**Files:**
- Create: `src/ui/diff_layout.rs`
- Modify: `src/ui/mod.rs`

- [ ] **Step 1: Write the failing tests**

Create `src/ui/diff_layout.rs` with its tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::compare::{binary_file, text_file};
    use crate::github::api::thread;

    fn build(files: &[FileDiff], threads: &[ReviewThread], viewed: &[bool]) -> DiffLayout {
        DiffLayout::build(files, threads, viewed, 20.0, &HashMap::new())
    }

    #[test]
    fn threads_sit_under_their_line_on_their_side() {
        // Rows: Context(0,0) Modified(1,1) Added(2) Context(2,3).
        let files = vec![text_file("a.rs", "a\nb\nc\n", "a\nB\nX\nc\n")];
        let threads = vec![
            thread("a.rs", Side::Left, Some(2)),
            thread("a.rs", Side::Right, Some(3)),
            thread("a.rs", Side::Right, None),
            thread("other.rs", Side::Right, Some(1)),
        ];
        let a = anchor_threads(&files, &threads);
        assert_eq!(a[0].by_row.get(&1), Some(&vec![0]));
        assert_eq!(a[0].by_row.get(&2), Some(&vec![1]));
        assert_eq!(a[0].outdated, vec![2]);
        assert_eq!(a.len(), 1, "a thread on a file outside the diff is left out");
    }

    #[test]
    fn a_thread_inside_a_fold_unfolds_it() {
        let old: String = (0..12).map(|i| format!("l{i}\n")).collect();
        let new = old.replace("l11\n", "L11\n");
        let mut files = vec![text_file("a.rs", &old, &new)];
        let threads = vec![thread("a.rs", Side::Right, Some(2))];
        assert_eq!(anchor_threads(&files, &threads)[0].outdated, vec![0]);
        unfold_for_threads(&mut files, &threads);
        assert_eq!(anchor_threads(&files, &threads)[0].by_row.get(&1), Some(&vec![0]));
    }

    #[test]
    fn items_list_headers_threads_and_rows_in_order() {
        let files = vec![text_file("a.rs", "a\nb\nc\n", "a\nB\nc\n"), binary_file("img.png")];
        let threads = vec![thread("a.rs", Side::Right, Some(2)), thread("a.rs", Side::Left, None)];
        let l = build(&files, &threads, &[false, false]);
        assert_eq!(
            l.items,
            vec![
                Item::FileHeader { file: 0 },
                Item::OutdatedTitle { file: 0 },
                Item::Thread { file: 0, thread: 1, outdated: true },
                Item::Row { file: 0, row: 0 },
                Item::Row { file: 0, row: 1 },
                Item::Thread { file: 0, thread: 0, outdated: false },
                Item::Row { file: 0, row: 2 },
                Item::FileHeader { file: 1 },
                Item::Notice { file: 1 },
            ]
        );
        assert_eq!(l.change_starts, vec![4]);
        assert_eq!(l.header_item(1), Some(7));
        assert_eq!(l.items[5].file(), 0);
    }

    #[test]
    fn file_level_threads_have_their_own_section() {
        let files = vec![text_file("a.rs", "a\n", "b\n")];
        let mut on_file = thread("a.rs", Side::Right, None);
        on_file.file_level = true;
        let threads = vec![on_file, thread("a.rs", Side::Right, None)];
        assert_eq!(anchor_threads(&files, &threads)[0].file_level, vec![0]);
        let l = build(&files, &threads, &[false]);
        assert_eq!(
            l.items[..5],
            [
                Item::FileHeader { file: 0 },
                Item::FileCommentsTitle { file: 0 },
                Item::Thread { file: 0, thread: 0, outdated: false },
                Item::OutdatedTitle { file: 0 },
                Item::Thread { file: 0, thread: 1, outdated: true },
            ]
        );
    }

    #[test]
    fn a_viewed_file_is_only_its_header() {
        let files = vec![text_file("a.rs", "a\n", "b\n")];
        let l = build(&files, &[], &[true]);
        assert_eq!(l.items, vec![Item::FileHeader { file: 0 }]);
        assert!(l.change_starts.is_empty());
    }

    #[test]
    fn each_block_of_changes_is_one_stop() {
        // File a: Modified, Context, Context, Removed. File b: Modified, Modified, Added.
        let files = vec![
            text_file("a", "a\nb\nc\nd\n", "A\nb\nc\n"),
            text_file("b", "a\nb\n", "A\nB\nC\n"),
        ];
        let l = build(&files, &[], &[false, false]);
        assert_eq!(l.change_starts, vec![1, 4, 6]);
    }

    #[test]
    fn change_navigation_stops_at_both_ends() {
        let files = vec![text_file("a", "a\nb\nc\nd\n", "A\nb\nc\n")];
        let l = build(&files, &[], &[false]);
        assert_eq!(l.next_change(None), Some(0));
        assert_eq!(l.next_change(Some(0)), Some(1));
        assert_eq!(l.next_change(Some(1)), Some(1));
        assert_eq!(l.prev_change(Some(1)), Some(0));
        assert_eq!(l.prev_change(Some(0)), Some(0));
        assert_eq!(l.prev_change(Some(9)), Some(1), "a stale index is clamped");
        assert_eq!(build(&[], &[], &[]).next_change(None), None);
    }

    #[test]
    fn offsets_follow_heights_and_measured_threads() {
        let files = vec![text_file("a.rs", "a\nb\nc\n", "a\nB\nc\n")];
        let threads = vec![thread("a.rs", Side::Right, Some(2))];
        // Items: header, row 0, row 1, thread, row 2.
        let mut l = build(&files, &threads, &[false]);
        assert_eq!(l.top(1), HEADER_HEIGHT);
        assert_eq!(l.total_height(), HEADER_HEIGHT + 60.0 + THREAD_ESTIMATE);
        assert_eq!(l.visible(0.0, 1.0), 0..1);
        assert_eq!(l.visible(HEADER_HEIGHT + 25.0, HEADER_HEIGHT + 45.0), 2..4);
        assert!(l.set_height(3, 50.0));
        assert!(!l.set_height(3, 50.2), "sub-pixel changes are ignored");
        assert_eq!(l.total_height(), HEADER_HEIGHT + 60.0 + 50.0);
        let measured = HashMap::from([(0, 42.0)]);
        let l = DiffLayout::build(&files, &threads, &[false], 20.0, &measured);
        assert_eq!(l.height(3), 42.0);
        assert_eq!(build(&[], &[], &[]).visible(0.0, 100.0), 0..0);
    }
}
```

In `src/ui/mod.rs`, after `pub mod debugger;`:

```rust
pub mod diff_layout;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test ui::diff_layout`
Expected: compile errors: `DiffLayout`, `Item`, `anchor_threads`… not found.

- [ ] **Step 3: Implement the layout model**

At the top of `src/ui/diff_layout.rs`:

```rust
//! Where each band of a pull request's diff sits. The diff view only lays
//! out what is on screen, so it needs every item's height up front: rows
//! have a fixed height, threads are estimated until drawn and measured.

use crate::git::compare::{DiffContent, DiffRow, FileDiff, TextDiff};
use crate::github::{ReviewThread, Side};
use std::collections::HashMap;
use std::ops::Range;

pub const HEADER_HEIGHT: f32 = 30.0;
pub const NOTICE_HEIGHT: f32 = 28.0;
pub const TITLE_HEIGHT: f32 = 24.0;
/// Height of a thread until it is drawn and measured.
pub const THREAD_ESTIMATE: f32 = 80.0;

/// One horizontal band of the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    /// Path, counts and "viewed" checkbox of `file`.
    FileHeader { file: usize },
    /// A binary or large file: a line of text instead of its diff.
    Notice { file: usize },
    /// Title above the threads on the whole of `file` (no line).
    FileCommentsTitle { file: usize },
    /// Title above the outdated threads of `file`.
    OutdatedTitle { file: usize },
    /// Row `row` of the file's [`TextDiff::rows`].
    Row { file: usize, row: usize },
    /// Review thread `thread` (index in the pull request's threads).
    /// `outdated`: listed above the rows because its line is gone (threads
    /// on the whole file are listed above the rows too, with `false`).
    Thread { file: usize, thread: usize, outdated: bool },
}

impl Item {
    pub fn file(self) -> usize {
        match self {
            Self::FileHeader { file }
            | Self::Notice { file }
            | Self::FileCommentsTitle { file }
            | Self::OutdatedTitle { file }
            | Self::Row { file, .. }
            | Self::Thread { file, .. } => file,
        }
    }
}

/// The threads of one file: on the whole file, under the row they target,
/// or outdated.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Anchors {
    pub file_level: Vec<usize>,
    pub by_row: HashMap<usize, Vec<usize>>,
    pub outdated: Vec<usize>,
}

/// Place each thread in the file it is about (one `Anchors` per file). A
/// thread GitHub reports outdated, or whose line has no row, is outdated;
/// a thread on a file absent from the diff is left out.
pub fn anchor_threads(files: &[FileDiff], threads: &[ReviewThread]) -> Vec<Anchors> {
    let mut anchors = vec![Anchors::default(); files.len()];
    for (index, thread) in threads.iter().enumerate() {
        let Some(file) = files.iter().position(|f| f.path() == thread.path) else {
            continue;
        };
        if thread.file_level {
            anchors[file].file_level.push(index);
            continue;
        }
        let row = match (&files[file].content, thread_line(thread)) {
            (DiffContent::Text(text), Some(line)) => row_of_line(text, thread.side, line),
            _ => None,
        };
        match row {
            Some(row) => anchors[file].by_row.entry(row).or_default().push(index),
            None => anchors[file].outdated.push(index),
        }
    }
    anchors
}

/// Unfold the folds hiding a thread's line, so that it gets a row.
pub fn unfold_for_threads(files: &mut [FileDiff], threads: &[ReviewThread]) {
    for thread in threads {
        let Some(line) = thread_line(thread) else {
            continue;
        };
        let Some(file) = files.iter_mut().find(|f| f.path() == thread.path) else {
            continue;
        };
        let DiffContent::Text(text) = &mut file.content else {
            continue;
        };
        if row_of_line(text, thread.side, line).is_some() {
            continue;
        }
        let fold = text.rows.iter().position(|row| match (thread.side, *row) {
            (Side::Left, DiffRow::Fold { old, len, .. }) => (old..old + len).contains(&line),
            (Side::Right, DiffRow::Fold { new, len, .. }) => (new..new + len).contains(&line),
            _ => false,
        });
        if let Some(row) = fold {
            text.unfold(row);
        }
    }
}

/// 0-based line of a thread, unless it is outdated.
fn thread_line(thread: &ReviewThread) -> Option<usize> {
    thread.line.and_then(|l| (l as usize).checked_sub(1))
}

/// The row showing 0-based `line` of `side`, if any.
fn row_of_line(text: &TextDiff, side: Side, line: usize) -> Option<usize> {
    text.rows.iter().position(|row| match (side, *row) {
        (
            Side::Left,
            DiffRow::Context { old, .. } | DiffRow::Modified { old, .. } | DiffRow::Removed { old },
        ) => old == line,
        (
            Side::Right,
            DiffRow::Context { new, .. } | DiffRow::Modified { new, .. } | DiffRow::Added { new },
        ) => new == line,
        _ => false,
    })
}

/// The items of the view and their geometry.
pub struct DiffLayout {
    pub items: Vec<Item>,
    heights: Vec<f32>,
    /// `offsets[i]`: top of item `i`; the last entry is the total height.
    offsets: Vec<f32>,
    /// Items starting a block of changed rows, in order: the F7 stops.
    pub change_starts: Vec<usize>,
}

impl DiffLayout {
    /// `viewed[f]`: file `f` collapses to its header. `thread_heights`:
    /// measured heights by thread index.
    pub fn build(
        files: &[FileDiff],
        threads: &[ReviewThread],
        viewed: &[bool],
        row_height: f32,
        thread_heights: &HashMap<usize, f32>,
    ) -> Self {
        let anchors = anchor_threads(files, threads);
        let mut items = Vec::new();
        let mut change_starts = Vec::new();
        for (file, f) in files.iter().enumerate() {
            items.push(Item::FileHeader { file });
            if viewed.get(file).copied().unwrap_or(false) {
                continue;
            }
            let a = &anchors[file];
            if !a.file_level.is_empty() {
                items.push(Item::FileCommentsTitle { file });
                items.extend(a.file_level.iter().map(|&thread| Item::Thread {
                    file,
                    thread,
                    outdated: false,
                }));
            }
            if !a.outdated.is_empty() {
                items.push(Item::OutdatedTitle { file });
                items.extend(a.outdated.iter().map(|&thread| Item::Thread {
                    file,
                    thread,
                    outdated: true,
                }));
            }
            let DiffContent::Text(text) = &f.content else {
                items.push(Item::Notice { file });
                continue;
            };
            let mut in_change = false;
            for (row, r) in text.rows.iter().enumerate() {
                if r.is_change() && !in_change {
                    change_starts.push(items.len());
                }
                in_change = r.is_change();
                items.push(Item::Row { file, row });
                if let Some(anchored) = a.by_row.get(&row) {
                    items.extend(anchored.iter().map(|&thread| Item::Thread {
                        file,
                        thread,
                        outdated: false,
                    }));
                }
            }
        }
        let heights = items
            .iter()
            .map(|item| match *item {
                Item::FileHeader { .. } => HEADER_HEIGHT,
                Item::Notice { .. } => NOTICE_HEIGHT,
                Item::FileCommentsTitle { .. } | Item::OutdatedTitle { .. } => TITLE_HEIGHT,
                Item::Row { .. } => row_height,
                Item::Thread { thread, .. } => {
                    thread_heights.get(&thread).copied().unwrap_or(THREAD_ESTIMATE)
                }
            })
            .collect();
        let mut layout = Self {
            items,
            heights,
            offsets: Vec::new(),
            change_starts,
        };
        layout.recompute_offsets();
        layout
    }

    pub fn total_height(&self) -> f32 {
        self.offsets.last().copied().unwrap_or(0.0)
    }

    pub fn top(&self, item: usize) -> f32 {
        self.offsets[item]
    }

    pub fn height(&self, item: usize) -> f32 {
        self.heights[item]
    }

    /// The items overlapping `[top, bottom)`.
    pub fn visible(&self, top: f32, bottom: f32) -> Range<usize> {
        let n = self.items.len();
        let start = self.offsets[1..].partition_point(|&b| b <= top).min(n);
        let end = self.offsets[..n].partition_point(|&t| t < bottom).max(start);
        start..end
    }

    /// Record the measured height of `item`; true when it changed.
    pub fn set_height(&mut self, item: usize, height: f32) -> bool {
        if (self.heights[item] - height).abs() <= 0.5 {
            return false;
        }
        self.heights[item] = height;
        self.recompute_offsets();
        true
    }

    pub fn header_item(&self, file: usize) -> Option<usize> {
        self.items
            .iter()
            .position(|item| *item == Item::FileHeader { file })
    }

    /// The F7 stop after `current` (the first one when none), staying on the last.
    pub fn next_change(&self, current: Option<usize>) -> Option<usize> {
        let last = self.change_starts.len().checked_sub(1)?;
        Some(current.map_or(0, |c| (c + 1).min(last)))
    }

    /// The F7 stop before `current` (the first one when none), staying on the first.
    pub fn prev_change(&self, current: Option<usize>) -> Option<usize> {
        let last = self.change_starts.len().checked_sub(1)?;
        Some(current.map_or(0, |c| c.saturating_sub(1).min(last)))
    }

    fn recompute_offsets(&mut self) {
        self.offsets.clear();
        let mut y = 0.0;
        for h in &self.heights {
            self.offsets.push(y);
            y += h;
        }
        self.offsets.push(y);
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `rtk cargo test ui::diff_layout`
Expected: 7 tests pass.

- [ ] **Step 5: Checkpoint**

Run: `rtk cargo clippy --all-targets`
Expected: no new warnings.

---

### Task 10: Painting the diff

> **Amendments from the reviews of Tasks 7–9. Where they differ from the code below, the amendments win.**
>
> 1. **`diff_layout` API changes:**
>    - `DiffLayout`'s fields are private. Use `items()` and `change_starts()`.
>    - `Item::file()` returns `Option<usize>`.
>    - Thread items are `Item::Thread { file, thread, placement: Placement }`, with `Placement::{Line, File, Outdated}`.
>    - New items, listed first in the view:
>      - `Item::OrphanTitle`, painted with `group_title(.., "Comments on files not in this diff", ..)`;
>      - `Item::OrphanThread { thread }`.
>    - Navigation takes a position: `next_change(current: Option<usize>, from_y: f32)` and `prev_change(current, from_y)`. Both return an index into `change_starts()`.
> 2. **`DiffContent` has a fourth variant, `Unavailable(String)`.** The header and notice code below already handles it.
> 3. **F7 navigation lives in `DiffView` and runs inside `show()`, after the layout exists,** so a key press is never lost on a rebuild frame.
>    - New fields:
>      - `current: Option<usize>`, the stop index;
>      - `viewport: Option<(f32, f32)>`, the last viewport top/bottom in content coordinates, stored every frame;
>      - `pending: Option<Nav>` with `enum Nav { Next, Prev }`.
>    - `pub fn request_next(&mut self)` / `request_prev(&mut self)` set `pending`; the tab calls them on F7 / Shift+F7.
>    - In `show()`, after `get_or_insert_with` and before the scroll area is built, take `pending`:
>      - **cursor still visible:** the cursor is valid when `current` is `Some(c)`, `c < change_starts().len()`, and the stop's top `top(change_starts()[c])` lies in the last viewport `[top, bottom)`. Then step from it: `layout.next_change(Some(c), _)` / `prev_change(Some(c), _)`.
>      - **otherwise:** search from the position, `layout.next_change(None, viewport_top)` / `prev_change(None, viewport_top)`, with `viewport_top = 0.0` before the first frame.
>      - **then:** `go_to_change(t)`.
>    - `go_to_change(t)` is private. It sets `current = Some(t)`, `scroll_to` (with the usual one-third lead) and `flash`.
>    - `pub fn current_change(&self) -> Option<usize>`.
>    - **Two invalidations:**
>      - `invalidate()` rebuilds and **keeps** `current`. Use it on Unfold: a fold never holds a change, so the stops keep their order.
>      - `invalidate_stops()` rebuilds and **clears** `current`. Use it on a viewed toggle and on Show anyway, which add or remove stops.
>      - `reset()` clears everything.
>    - **`collapse_anchor(file)`:** the tab calls it with a viewed toggle. On the next build, it scrolls that file's header to the **top** of the viewport, with no lead, so the reader stays on the file they just toggled. Support this with `scroll_to: Option<(usize, bool /* at_top */)>`.
>    - **Scroll stability (optional but cheap):** when a thread is measured and `top(i) < viewport.min.y`, add `new_height - old_height` to the scroll offset on the next frame, so content above the viewport doesn't push what the reader is looking at.
> 4. **Painting threads.** The signature becomes `paint_thread(ui, top_left, width, thread, full_width: bool, label: Option<String>, p, md_cache, salt)`, and the caller decides both values:
>
>    | Item | Width | Label |
>    |---|---|---|
>    | `Thread { placement: Line }` | half width, under its side | none |
>    | `Thread { placement: File }` | full | `Some(format!("Line {l}"))` when `thread.line` is `Some(l)` (a line thread on a file without rows), otherwise none |
>    | `Thread { placement: Outdated }` | full | `"Line {original_line or ?} of the {base / pull request} version (outdated)"` |
>    | `OrphanThread` | full | `format!("On {}, which this diff doesn't show", thread.path)` |
> 5. **Sticky header.** Draw it only when the first visible item has `file() == Some(f)` and is not a `FileHeader`.
> 6. **Header thread count.** Count the threads whose `path` equals the file's `path()` or its `old_path`.

**Files:**
- Create: `src/ui/diff_view.rs`
- Modify: `src/ui/mod.rs`

- [ ] **Step 1: Write the failing tests**

Create `src/ui/diff_view.rs` with its tests (the painting itself is covered by the pull request tab's UI tests in Task 12):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn parts(job: &LayoutJob, strong: Color32) -> Vec<(&str, bool)> {
        job.sections
            .iter()
            .map(|s| (&job.text[s.byte_range.clone()], s.format.background == strong))
            .collect()
    }

    #[test]
    fn changed_words_get_the_strong_background_and_tokens_keep_their_colour() {
        let tokens = vec![
            Token { text: "let".into(), kind: TokenKind::Keyword },
            Token { text: " y = 1;".into(), kind: TokenKind::Normal },
        ];
        let job = line_job("let y = 1;", &tokens, &[4..5], FontId::monospace(12.0), Color32::RED);
        assert_eq!(
            parts(&job, Color32::RED),
            vec![("let", false), (" ", false), ("y", true), (" = 1;", false)]
        );
        assert_eq!(job.sections[0].format.color, TokenKind::Keyword.color());
    }

    #[test]
    fn tokens_that_do_not_spell_the_line_fall_back_to_plain_text() {
        let tokens = vec![Token { text: "other".into(), kind: TokenKind::Keyword }];
        let job = line_job("abc", &tokens, &[], FontId::monospace(12.0), Color32::RED);
        assert_eq!(parts(&job, Color32::RED), vec![("abc", false)]);
        assert_eq!(job.sections[0].format.color, TokenKind::Normal.color());
    }
}
```

In `src/ui/mod.rs`, after `pub mod diff_layout;`:

```rust
pub mod diff_view;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test ui::diff_view`
Expected: compile errors: `line_job`, `LayoutJob`, `Token`… not found.

- [ ] **Step 3: Implement the painter**

At the top of `src/ui/diff_view.rs`:

```rust
//! Side-by-side diff of a pull request's files: the base on the left, the
//! pull request on the right, review threads under the lines they are
//! about. Only the items on screen are laid out (see [`DiffLayout`]).

use super::diff_layout::{DiffLayout, Item, HEADER_HEIGHT};
use super::theme::Palette;
use crate::editor::highlight::{Highlighter, Token, TokenKind};
use crate::git::compare::{DiffContent, DiffRow, FileDiff, FileDiffKind, TextDiff};
use crate::github::{ReviewThread, Side};
use crate::plugin::manager::PluginManager;
use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke, TextFormat, Ui,
    UiBuilder,
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use egui_phosphor::regular;
use std::collections::HashMap;
use std::hash::Hash;
use std::ops::Range;

/// Width of each side's line-number gutter.
const GUTTER: f32 = 44.0;
/// Width of the `+` / `−` marker after the gutter.
const MARKER: f32 = 14.0;
/// How long a change reached with F7 stays outlined, in seconds.
const FLASH_SECONDS: f64 = 0.8;
/// Room a thread may take while it is measured.
const THREAD_MAX_HEIGHT: f32 = 4000.0;

/// Something the user did in the view, for the pull request tab to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffEvent {
    ToggleViewed(usize),
    Unfold { file: usize, row: usize },
    ShowAnyway(usize),
    /// Double-click on 0-based line `line` of the pull request's version of `file`.
    OpenLine { file: usize, line: usize },
}

/// What the view shows; owned by the pull request tab.
pub struct DiffInput<'a> {
    pub files: &'a [FileDiff],
    pub threads: &'a [ReviewThread],
    /// `viewed[f]`: file `f` collapses to its header.
    pub viewed: &'a [bool],
    pub palette: Palette,
    pub plugins: Option<&'a PluginManager>,
}

/// Syntax tokens of each file side, by line.
type TokenCache = HashMap<(usize, Side), Vec<Vec<Token>>>;

#[derive(Default)]
pub struct DiffView {
    layout: Option<DiffLayout>,
    /// Measured height of each thread (by index), kept across rebuilds.
    thread_heights: HashMap<usize, f32>,
    /// Computed the first time a file is drawn.
    tokens: TokenCache,
    /// Item to bring into view on the next frame.
    scroll_to: Option<usize>,
    /// Item outlined after F7, and when the outline started.
    flash: Option<(usize, Option<f64>)>,
}

impl DiffView {
    /// Rebuild the layout on the next frame (folds or "viewed" changed).
    pub fn invalidate(&mut self) {
        self.layout = None;
    }

    /// Forget everything computed from the files (new data loaded).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn layout(&self) -> Option<&DiffLayout> {
        self.layout.as_ref()
    }

    /// Bring the header of `file` into view.
    pub fn scroll_to_file(&mut self, file: usize) {
        self.scroll_to = self.layout.as_ref().and_then(|l| l.header_item(file));
    }

    /// Bring F7 stop `index` into view and outline it.
    pub fn go_to_change(&mut self, index: usize) {
        if let Some(&item) = self.layout.as_ref().and_then(|l| l.change_starts.get(index)) {
            self.scroll_to = Some(item);
            self.flash = Some((item, None));
        }
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        input: &DiffInput<'_>,
        md_cache: &mut CommonMarkCache,
    ) -> Vec<DiffEvent> {
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let row_height = ui.fonts(|f| f.row_height(&font)) + 4.0;
        let Self {
            layout,
            thread_heights,
            tokens,
            scroll_to,
            flash,
        } = self;
        let layout = layout.get_or_insert_with(|| {
            DiffLayout::build(input.files, input.threads, input.viewed, row_height, thread_heights)
        });
        let now = ui.input(|i| i.time);
        let mut events = Vec::new();
        let mut area = egui::ScrollArea::vertical()
            .id_salt("pr-diff")
            .auto_shrink([false, false]);
        if let Some(item) = scroll_to.take().filter(|&i| i < layout.items.len()) {
            let lead = ui.available_height() / 3.0;
            area = area.vertical_scroll_offset((layout.top(item) - lead).max(0.0));
        }
        area.show_viewport(ui, |ui, viewport| {
            ui.set_height(layout.total_height());
            let origin = ui.min_rect().min;
            let width = ui.max_rect().width();
            let visible = layout.visible(viewport.min.y, viewport.max.y);
            let mut resized = false;
            for i in visible.clone() {
                let rect = Rect::from_min_size(
                    origin + egui::vec2(0.0, layout.top(i)),
                    egui::vec2(width, layout.height(i)),
                );
                let item = layout.items[i];
                let event = match item {
                    Item::FileHeader { file } => header(ui, rect, file, input, ("diff-header", i)),
                    Item::Notice { file } => {
                        notice(ui, rect, file, &input.files[file], &input.palette, ("diff-notice", i))
                    }
                    Item::FileCommentsTitle { .. } => {
                        group_title(ui, rect, "Comments on the file", &input.palette);
                        None
                    }
                    Item::OutdatedTitle { .. } => {
                        group_title(ui, rect, "Outdated comments", &input.palette);
                        None
                    }
                    Item::Row { file, row: index } => {
                        let f = &input.files[file];
                        let DiffContent::Text(text) = &f.content else {
                            continue;
                        };
                        ensure_tokens(tokens, file, f, text, input.plugins);
                        let flashing = match flash {
                            Some((at, started)) if *at == i => {
                                now - *started.get_or_insert(now) < FLASH_SECONDS
                            }
                            _ => false,
                        };
                        let side_tokens = [&tokens[&(file, Side::Left)], &tokens[&(file, Side::Right)]];
                        let id = ui.id().with(("diff-row", i));
                        let cell = RowCell { rect, file, index, text, flashing, id };
                        row(ui, &cell, side_tokens, &input.palette, &font)
                    }
                    Item::Thread { thread, outdated, .. } => {
                        let height = paint_thread(
                            ui,
                            rect.min,
                            width,
                            &input.threads[thread],
                            outdated,
                            &input.palette,
                            md_cache,
                            ("diff-thread", i),
                        );
                        thread_heights.insert(thread, height);
                        resized |= layout.set_height(i, height);
                        None
                    }
                };
                events.extend(event);
            }
            // The header of the file being read stays at the top.
            if let Some(first) = visible.clone().next() {
                if !matches!(layout.items[first], Item::FileHeader { .. }) {
                    let rect = Rect::from_min_size(
                        origin + egui::vec2(0.0, viewport.min.y),
                        egui::vec2(width, HEADER_HEIGHT),
                    );
                    events.extend(header(ui, rect, layout.items[first].file(), input, "diff-sticky"));
                }
            }
            if resized {
                ui.ctx().request_repaint();
            }
        });
        let expired = matches!(flash, Some((_, Some(t0))) if now - *t0 >= FLASH_SECONDS);
        if expired {
            *flash = None;
        } else if flash.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }
        events
    }
}

/// Letter and colour of a file's change, like the Git panel.
pub fn kind_icon(kind: FileDiffKind, p: &Palette) -> (&'static str, Color32) {
    match kind {
        FileDiffKind::Added => ("A", p.git_added),
        FileDiffKind::Modified => ("M", p.git_modified),
        FileDiffKind::Deleted => ("D", p.git_removed),
        FileDiffKind::Renamed => ("R", p.info),
    }
}

fn header(
    ui: &mut Ui,
    rect: Rect,
    file: usize,
    input: &DiffInput<'_>,
    salt: impl Hash,
) -> Option<DiffEvent> {
    let p = &input.palette;
    let f = &input.files[file];
    ui.painter().rect_filled(rect, 0.0, p.surface_raised);
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.border));
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(salt)
            .max_rect(rect.shrink2(egui::vec2(8.0, 2.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    let mut viewed = input.viewed.get(file).copied().unwrap_or(false);
    let toggled = child.checkbox(&mut viewed, "").on_hover_text("Viewed").changed();
    let (icon, color) = kind_icon(f.kind, p);
    child.label(RichText::new(icon).monospace().color(color));
    child.label(RichText::new(f.display_path()).monospace().color(p.text));
    match &f.content {
        DiffContent::Text(_) => {
            child.label(RichText::new(format!("+{}", f.additions)).small().color(p.git_added));
            child.label(RichText::new(format!("−{}", f.deletions)).small().color(p.git_removed));
        }
        DiffContent::Binary => {
            child.label(RichText::new("binary").small().color(p.text_muted));
        }
        DiffContent::TooLarge => {
            child.label(RichText::new("large file").small().color(p.text_muted));
        }
        DiffContent::Unavailable(_) => {
            child.label(RichText::new("unavailable").small().color(p.warning));
        }
    }
    let threads = input.threads.iter().filter(|t| t.path == f.path()).count();
    if threads > 0 {
        child.label(
            RichText::new(format!("{} {threads}", regular::CHAT_CIRCLE))
                .small()
                .color(p.text_muted),
        );
    }
    toggled.then_some(DiffEvent::ToggleViewed(file))
}

fn notice(
    ui: &mut Ui,
    rect: Rect,
    file: usize,
    f: &FileDiff,
    p: &Palette,
    salt: impl Hash,
) -> Option<DiffEvent> {
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(salt)
            .max_rect(rect.shrink2(egui::vec2(12.0, 2.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    match &f.content {
        DiffContent::Binary => {
            child.label(RichText::new("Binary file changed").italics().color(p.text_muted));
            None
        }
        DiffContent::TooLarge => {
            child.label(RichText::new("Large file, not shown.").italics().color(p.text_muted));
            child
                .small_button("Show anyway")
                .clicked()
                .then_some(DiffEvent::ShowAnyway(file))
        }
        DiffContent::Unavailable(reason) => {
            child.label(
                RichText::new(format!("Not available locally: {reason}"))
                    .italics()
                    .color(p.warning),
            );
            None
        }
        DiffContent::Text(_) => None,
    }
}

/// Title above a group of threads listed before a file's rows.
fn group_title(ui: &Ui, rect: Rect, title: &str, p: &Palette) {
    ui.painter().text(
        rect.left_center() + egui::vec2(12.0, 0.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(12.0),
        p.text_muted,
    );
}

/// Tokens of both sides of `file`, computed once.
fn ensure_tokens(
    tokens: &mut TokenCache,
    file: usize,
    f: &FileDiff,
    text: &TextDiff,
    plugins: Option<&PluginManager>,
) {
    let sides = [
        (Side::Left, &text.old, f.old_path.as_deref()),
        (Side::Right, &text.new, f.new_path.as_deref()),
    ];
    for (side, lines, path) in sides {
        tokens.entry((file, side)).or_insert_with(|| {
            let mut highlighter = Highlighter::new();
            highlighter.set_language_from_filename(path.unwrap_or(f.path()));
            highlighter.highlight_document(&lines.join("\n"), 0, plugins);
            lines
                .iter()
                .enumerate()
                .map(|(i, line)| highlighter.tokens_for_line(i, line, plugins))
                .collect()
        });
    }
}

#[derive(Clone, Copy)]
enum LineKind {
    Context,
    Removed,
    Added,
}

/// What one side of a row shows.
struct SideLine<'a> {
    /// 0-based.
    number: usize,
    text: &'a str,
    tokens: &'a [Token],
    changed: &'a [Range<usize>],
    kind: LineKind,
}

fn side_line<'a>(
    lines: &'a [String],
    tokens: &'a [Vec<Token>],
    changed: Option<&'a [Range<usize>]>,
    at: Option<(usize, LineKind)>,
) -> Option<SideLine<'a>> {
    let (number, kind) = at?;
    Some(SideLine {
        number,
        text: lines.get(number)?.as_str(),
        tokens: tokens.get(number).map_or(&[][..], Vec::as_slice),
        changed: changed.unwrap_or(&[][..]),
        kind,
    })
}

/// Where and what row is drawn.
struct RowCell<'a> {
    rect: Rect,
    file: usize,
    /// Index in `text.rows`.
    index: usize,
    text: &'a TextDiff,
    flashing: bool,
    id: egui::Id,
}

fn row(
    ui: &mut Ui,
    cell: &RowCell<'_>,
    tokens: [&Vec<Vec<Token>>; 2],
    p: &Palette,
    font: &FontId,
) -> Option<DiffEvent> {
    let RowCell { rect, file, index, text, flashing, id } = *cell;
    let r = text.rows[index];
    let (old, new) = match r {
        DiffRow::Fold { len, .. } => {
            let resp = ui.interact(rect, id, Sense::click());
            let bg = if resp.hovered() { p.surface_raised } else { p.surface };
            ui.painter().rect_filled(rect, 0.0, bg);
            let s = if len == 1 { "" } else { "s" };
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                format!("⋯ {len} unchanged line{s}"),
                FontId::proportional(12.0),
                p.text_muted,
            );
            return resp
                .on_hover_text("Show these lines")
                .clicked()
                .then_some(DiffEvent::Unfold { file, row: index });
        }
        DiffRow::Context { old, new } => {
            (Some((old, LineKind::Context)), Some((new, LineKind::Context)))
        }
        DiffRow::Modified { old, new } => {
            (Some((old, LineKind::Removed)), Some((new, LineKind::Added)))
        }
        DiffRow::Removed { old } => (Some((old, LineKind::Removed)), None),
        DiffRow::Added { new } => (None, Some((new, LineKind::Added))),
    };
    let words = match r {
        DiffRow::Modified { old, .. } => text.words.get(&old),
        _ => None,
    };
    let half = rect.width() / 2.0;
    let left = Rect::from_min_size(rect.min, egui::vec2(half, rect.height()));
    let right = Rect::from_min_size(rect.min + egui::vec2(half, 0.0), egui::vec2(half, rect.height()));
    let old_line = side_line(&text.old, tokens[0], words.map(|w| w.old.as_slice()), old);
    let new_line = side_line(&text.new, tokens[1], words.map(|w| w.new.as_slice()), new);
    side_cell(ui, left, old_line, p, font, id.with("left"));
    let right_resp = side_cell(ui, right, new_line, p, font, id.with("right"));
    ui.painter()
        .vline(rect.center().x, rect.y_range(), Stroke::new(1.0, p.border));
    if flashing {
        ui.painter()
            .rect_stroke(rect, 0.0, Stroke::new(1.5, p.accent), egui::StrokeKind::Inside);
    }
    match new {
        Some((line, _)) if right_resp.double_clicked() => Some(DiffEvent::OpenLine { file, line }),
        _ => None,
    }
}

fn side_cell(
    ui: &mut Ui,
    rect: Rect,
    line: Option<SideLine<'_>>,
    p: &Palette,
    font: &FontId,
    id: egui::Id,
) -> egui::Response {
    let resp = ui.interact(rect, id, Sense::click());
    let painter = ui.painter_at(rect);
    let Some(line) = line else {
        // Filler facing a line that only exists on the other side.
        painter.rect_filled(rect, 0.0, p.surface);
        return resp;
    };
    let (bg, strong, marker) = match line.kind {
        LineKind::Context => (Color32::TRANSPARENT, Color32::TRANSPARENT, ""),
        LineKind::Removed => (
            p.git_removed.gamma_multiply(0.15),
            p.git_removed.gamma_multiply(0.45),
            "−",
        ),
        LineKind::Added => (
            p.git_added.gamma_multiply(0.15),
            p.git_added.gamma_multiply(0.45),
            "+",
        ),
    };
    painter.rect_filled(rect, 0.0, bg);
    let y = rect.center().y;
    painter.text(
        Pos2::new(rect.left() + GUTTER - 6.0, y),
        Align2::RIGHT_CENTER,
        (line.number + 1).to_string(),
        font.clone(),
        p.text_faint,
    );
    painter.text(
        Pos2::new(rect.left() + GUTTER + 2.0, y),
        Align2::LEFT_CENTER,
        marker,
        font.clone(),
        p.text_muted,
    );
    let job = line_job(line.text, line.tokens, line.changed, font.clone(), strong);
    let galley = ui.fonts(|f| f.layout_job(job));
    let left = rect.left() + GUTTER + MARKER;
    let clipped = left + galley.size().x > rect.right();
    painter.galley(Pos2::new(left, y - galley.size().y / 2.0), galley, p.text);
    if clipped {
        resp.on_hover_text(line.text.to_string())
    } else {
        resp
    }
}

/// The layout of one line: token colours, and a `strong` background on the
/// `changed` byte ranges. Tokens that don't spell the line exactly are ignored.
fn line_job(
    text: &str,
    tokens: &[Token],
    changed: &[Range<usize>],
    font: FontId,
    strong: Color32,
) -> LayoutJob {
    let mut spans = Vec::with_capacity(tokens.len());
    let mut pos = 0;
    let mut spelled = true;
    for t in tokens {
        let end = pos + t.text.len();
        if text.get(pos..end) != Some(t.text.as_str()) {
            spelled = false;
            break;
        }
        spans.push((pos..end, t.kind.color()));
        pos = end;
    }
    if !spelled || pos != text.len() {
        spans = vec![(0..text.len(), TokenKind::Normal.color())];
    }
    let mut job = LayoutJob::default();
    for (range, color) in spans {
        let mut start = range.start;
        while start < range.end {
            let (end, marked) = next_boundary(start, range.end, changed);
            job.append(
                &text[start..end],
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color,
                    background: if marked { strong } else { Color32::TRANSPARENT },
                    ..Default::default()
                },
            );
            start = end;
        }
    }
    job
}

/// End of the run from `start` (at most `end`) that is uniformly inside or
/// outside `changed`, and whether it is inside.
fn next_boundary(start: usize, end: usize, changed: &[Range<usize>]) -> (usize, bool) {
    if let Some(r) = changed.iter().find(|r| r.contains(&start)) {
        return (r.end.min(end), true);
    }
    let next = changed
        .iter()
        .map(|r| r.start)
        .filter(|&s| s > start)
        .min()
        .unwrap_or(end);
    (next.min(end), false)
}

/// Draw a thread under its side (full width when outdated or on the whole
/// file); its height.
#[allow(clippy::too_many_arguments)]
fn paint_thread(
    ui: &mut Ui,
    top_left: Pos2,
    width: f32,
    thread: &ReviewThread,
    outdated: bool,
    p: &Palette,
    md_cache: &mut CommonMarkCache,
    salt: impl Hash,
) -> f32 {
    let (x, w) = match (outdated || thread.file_level, thread.side) {
        (true, _) => (0.0, width),
        (false, Side::Left) => (0.0, width / 2.0),
        (false, Side::Right) => (width / 2.0, width / 2.0),
    };
    let max = Rect::from_min_size(
        top_left + egui::vec2(x + 8.0, 4.0),
        egui::vec2((w - 16.0).max(120.0), THREAD_MAX_HEIGHT),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(salt)
            .max_rect(max)
            .layout(Layout::top_down(Align::Min)),
    );
    egui::Frame::new()
        .fill(p.surface_raised)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::same(8))
        .show(&mut child, |ui| {
            ui.set_width(max.width() - 18.0);
            if outdated {
                let version = match thread.side {
                    Side::Left => "base",
                    Side::Right => "pull request",
                };
                let line = thread
                    .original_line
                    .map_or_else(|| "?".to_string(), |l| l.to_string());
                ui.label(
                    RichText::new(format!("Line {line} of the {version} version (outdated)"))
                        .small()
                        .color(p.text_faint),
                );
            }
            for (k, comment) in thread.comments.iter().enumerate() {
                if k > 0 {
                    ui.add_space(4.0);
                }
                ui.horizontal(|ui| {
                    if k > 0 {
                        ui.add_space(16.0);
                    }
                    ui.label(RichText::new(&comment.author).strong());
                    ui.label(RichText::new(comment.date()).small().color(p.text_muted));
                });
                if k == 0 {
                    CommonMarkViewer::new().show(ui, md_cache, &comment.body);
                } else {
                    ui.indent(("reply", k), |ui| {
                        CommonMarkViewer::new().show(ui, md_cache, &comment.body);
                    });
                }
            }
        });
    child.min_rect().height() + 8.0
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `rtk cargo test ui::diff_view`
Expected: 2 tests pass.

- [ ] **Step 5: Checkpoint**

Run: `rtk cargo clippy --all-targets`
Expected: no new warnings. If clippy flags `too_many_arguments` elsewhere, group the arguments in a struct like `RowCell` rather than adding more `allow`s.

---

### Task 11: Pull request tabs

**Files:**
- Modify: `src/tabs/mod.rs`, `src/app/file_ops.rs:376-379`, `src/ui/layout.rs:155-160`

- [ ] **Step 1: Write the failing tests**

In the tests of `src/tabs/mod.rs`, add:

```rust
    #[test]
    fn pull_request_tabs_are_singletons_and_not_files() {
        let mut tm = TabManager::new();
        let f = tm.open(PathBuf::from("/w/a.rs"), String::new());
        let p = tm.open_pr(58, "feat: connect servers");
        assert_eq!(tm.active_pr(), Some(58));
        assert_eq!(tm.open_pr(58, "feat: connect servers"), p);
        assert_eq!(tm.tabs.len(), 2);
        let tab = tm.tabs.iter().find(|t| t.id == p).unwrap();
        assert_eq!(tab.title, "#58 feat: connect servers");
        assert!(!tab.is_file());
        assert!(tm.tabs.iter().find(|t| t.id == f).unwrap().is_file());
        tm.active_tab = Some(f);
        assert_eq!(tm.active_pr(), None);
    }

    #[test]
    fn long_pull_request_titles_are_cut() {
        let mut tm = TabManager::new();
        tm.open_pr(1, &"x".repeat(50));
        assert_eq!(tm.tabs[0].title, format!("#1 {}…", "x".repeat(40)));
    }

    #[test]
    fn clicking_a_pull_request_tab_activates_it_without_opening_a_file() {
        let mut tm = manager_with(&["/w/a.rs"]);
        let p = tm.open_pr(7, "Fix");
        tm.active_tab = Some(tm.tabs[0].id);
        let mut h = Harness::new();
        assert_eq!(h.click(&mut tm, "#7 Fix", egui::PointerButton::Primary), None);
        assert_eq!(tm.active_tab, Some(p));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test tabs::`
Expected: compile errors: `open_pr`, `active_pr`, `is_file` not found.

- [ ] **Step 3: Add the PR tab kind**

In `src/tabs/mod.rs`, in `pub struct Tab`, after the `page` field:

```rust
    /// Pull request shown in this tab (its number), instead of a file.
    pub pr: Option<u64>,
```

After the struct:

```rust
impl Tab {
    /// Shows a file (or an untitled buffer), not settings, a module page or
    /// a pull request.
    pub fn is_file(&self) -> bool {
        !self.is_settings && self.page.is_none() && self.pr.is_none()
    }
}
```

Add `pr: None,` to each of the four existing `Tab { … }` literals (in `open`, `open_untitled`, `open_settings`, `open_page`), after `page: …,`.

After `active_page`, add:

```rust
    /// Open (or focus) the tab of pull request `number`.
    pub fn open_pr(&mut self, number: u64, title: &str) -> usize {
        if let Some(existing) = self.tabs.iter().find(|t| t.pr == Some(number)) {
            let id = existing.id;
            self.active_tab = Some(id);
            return id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab {
            id,
            path: PathBuf::from(format!("__pr__{number}")),
            title: pr_tab_title(number, title),
            is_modified: false,
            is_settings: false,
            page: None,
            pr: Some(number),
            is_deleted: false,
        });
        self.active_tab = Some(id);
        id
    }

    /// Pull request of the active tab, if it shows one.
    pub fn active_pr(&self) -> Option<u64> {
        let id = self.active_tab?;
        self.tabs.iter().find(|t| t.id == id)?.pr
    }
```

After `impl TabManager { … }` (before the tests):

```rust
/// `#58 title`, the title cut to 40 characters.
fn pr_tab_title(number: u64, title: &str) -> String {
    const MAX: usize = 40;
    let short: String = title.chars().take(MAX).collect();
    let ellipsis = if title.chars().count() > MAX { "…" } else { "" };
    format!("#{number} {short}{ellipsis}")
}
```

In `TabManager::show`:
- In `tabs_data`, replace `t.is_settings || t.page.is_some(),` with `!t.is_file(),`.
- Replace the icon choice

```rust
                            let icon = if tab_path.starts_with("__page__") {
                                egui_phosphor::regular::PUZZLE_PIECE
                            } else {
                                egui_phosphor::regular::GEAR
                            };
```

with

```rust
                            let icon = if tab_path.starts_with("__page__") {
                                egui_phosphor::regular::PUZZLE_PIECE
                            } else if tab_path.starts_with("__pr__") {
                                egui_phosphor::regular::GIT_PULL_REQUEST
                            } else {
                                egui_phosphor::regular::GEAR
                            };
```

- In the close handling, replace `.filter(|t| !t.is_settings && t.page.is_none())` with `.filter(|t| t.is_file())`.

- [ ] **Step 4: Keep PR tabs away from the editor**

In `src/app/file_ops.rs`, `load_active_tab`, replace:

```rust
                if tab.page.is_some() {
                    return; // drawn by its module, no file to load
                }
```

with:

```rust
                if tab.page.is_some() || tab.pr.is_some() {
                    return; // drawn by its module or the pull request view: no file to load
                }
```

In `src/ui/layout.rs` (around line 155), replace:

```rust
    // Sync editor modified state → active tab
    if let Some(active_id) = app.tab_manager.active_tab {
        if let Some(tab) = app.tab_manager.tabs.iter_mut().find(|t| t.id == active_id) {
            tab.is_modified = app.editor.is_modified;
        }
    }
```

with:

```rust
    // Sync editor modified state → active tab. Only a file tab: while a
    // settings, module or pull request tab is shown, the editor still holds
    // the last file, whose changes must not mark that tab modified.
    if let Some(active_id) = app.tab_manager.active_tab {
        if let Some(tab) = app
            .tab_manager
            .tabs
            .iter_mut()
            .find(|t| t.id == active_id && t.is_file())
        {
            tab.is_modified = app.editor.is_modified;
        }
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `rtk cargo test tabs::` then `rtk cargo test`
Expected: the new tab tests pass and the whole suite stays green.

- [ ] **Step 6: Checkpoint**

Run: `rtk cargo clippy --all-targets`
Expected: no new warnings.

---

### Task 12: The pull request tab

> **Amendments from the reviews of Tasks 7–11. Where the code below disagrees with them, the amendments win.**
>
> 1. **F7 cursor.** `PrView` has no `current_change` field.
>    - When no text field has focus, F7 calls `diff.request_next()` and Shift+F7 calls `diff.request_prev()`. Call them even when no layout exists yet: `DiffView` handles the request inside `show()`.
>    - `PrView::current_change()` returns `self.diff.current_change()`.
>    - `poll` no longer resets the cursor; `diff.reset()` does.
>    - Invalidation calls:
>      - Unfold calls `diff.invalidate()`, which keeps the cursor.
>      - `toggle_viewed(file)` calls `diff.invalidate_stops()`, then `diff.collapse_anchor(file)`.
>      - A finished "Show anyway" calls `diff.invalidate_stops()`.
>    - The F7 UI test runs one frame per key press, and still expects `Some(0)`, `Some(1)`, `Some(0)`.
> 2. **`PrAction::Checkout`** carries `merge_ref`, built with `compare::pull_merge_ref(number, head_ref, from_fork)`. The code below already does this.
> 3. **"Viewed" key** is `FileDiff::version()`. The code below already uses it.
> 4. **Tests** call `view.diff().layout().unwrap().items()`, the accessor.
> 5. **`compare::fetch_pull` error.** It can now fail with "origin doesn't publish refs/pull/N/head …". Show it as the load error, like the other errors.

**Files:**
- Create: `src/ui/pr_view.rs`
- Modify: `src/ui/mod.rs`

- [ ] **Step 1: Write the failing tests**

Create `src/ui/pr_view.rs` with its tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::compare::text_file;
    use crate::ui::diff_layout::Item;

    fn fixture() -> PrView {
        let data = PrData {
            detail: PullDetail {
                summary: PullSummary {
                    number: 7,
                    title: "Fix".into(),
                    author: "me".into(),
                    head_ref: "fix".into(),
                    base_ref: "main".into(),
                    draft: false,
                },
                body: "Body".into(),
                head_sha: "a".repeat(40),
                base_sha: "b".repeat(40),
                head_repo: Some("me/proj".into()),
                html_url: "https://github.com/me/proj/pull/7".into(),
            },
            // Two stops in the first file, one in the second.
            files: vec![
                text_file("a.rs", "a\nb\nc\nd\n", "A\nb\nc\n"),
                text_file("b.rs", "x\n", "y\n"),
            ],
            threads: vec![],
            checked_out: false,
        };
        let repo = RepoRef {
            owner: "me".into(),
            name: "proj".into(),
        };
        PrView::from_data(PathBuf::from("."), repo, data)
    }

    fn frame(
        ctx: &egui::Context,
        view: &mut PrView,
        cache: &mut CommonMarkCache,
        events: Vec<egui::Event>,
    ) -> Option<PrAction> {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let palette = Palette::from_theme(&crate::config::Theme::default());
        let mut action = None;
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                action = view.show(ui, palette, None, cache);
            });
        });
        action
    }

    fn f7(shift: bool) -> egui::Event {
        egui::Event::Key {
            key: egui::Key::F7,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: if shift {
                egui::Modifiers::SHIFT
            } else {
                egui::Modifiers::NONE
            },
        }
    }

    #[test]
    fn f7_walks_through_the_changes() {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let mut view = fixture();
        frame(&ctx, &mut view, &mut cache, vec![]); // lays the diff out
        frame(&ctx, &mut view, &mut cache, vec![f7(false)]);
        assert_eq!(view.current_change(), Some(0));
        frame(&ctx, &mut view, &mut cache, vec![f7(false)]);
        assert_eq!(view.current_change(), Some(1));
        frame(&ctx, &mut view, &mut cache, vec![f7(true)]);
        assert_eq!(view.current_change(), Some(0));
    }

    #[test]
    fn a_viewed_file_collapses_to_its_header() {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let mut view = fixture();
        frame(&ctx, &mut view, &mut cache, vec![]);
        view.toggle_viewed(0);
        frame(&ctx, &mut view, &mut cache, vec![]);
        let items = &view.diff().layout().unwrap().items;
        assert!(!items.iter().any(|i| matches!(i, Item::Row { file: 0, .. })));
        assert!(items.iter().any(|i| matches!(i, Item::Row { file: 1, .. })));
        view.toggle_viewed(0);
        frame(&ctx, &mut view, &mut cache, vec![]);
        let items = &view.diff().layout().unwrap().items;
        assert!(items.iter().any(|i| matches!(i, Item::Row { file: 0, .. })));
    }

    #[test]
    fn a_new_version_of_a_viewed_file_is_unchecked() {
        let mut view = fixture();
        view.toggle_viewed(0);
        let State::Ready(data) = &mut view.state else { panic!("not ready") };
        assert!(is_viewed(&view.viewed, &data.files[0]));
        data.files[0].new_blob = Some(git2::Oid::from_str(&"c".repeat(40)).unwrap());
        assert!(!is_viewed(&view.viewed, &data.files[0]));
    }
}
```

In `src/ui/mod.rs`, after `pub mod palette;`:

```rust
pub mod pr_view;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `rtk cargo test ui::pr_view`
Expected: compile errors: `PrView`, `PrData`, `PrAction` not found.

- [ ] **Step 3: Implement the tab**

At the top of `src/ui/pr_view.rs`:

```rust
//! The tab of a pull request: its description, the files it changes and
//! their side-by-side diff with the review threads. Loads in the background.

use super::diff_layout::unfold_for_threads;
use super::diff_view::{kind_icon, DiffEvent, DiffInput, DiffView};
use super::theme::Palette;
use crate::git::compare::{self, DiffContent, FileDiff};
use crate::github::{self, PullDetail, PullSummary, RepoRef, ReviewThread};
use crate::plugin::manager::PluginManager;
use egui::RichText;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use egui_phosphor::regular;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

/// Everything a pull request tab shows.
pub struct PrData {
    pub detail: PullDetail,
    pub files: Vec<FileDiff>,
    pub threads: Vec<ReviewThread>,
    /// HEAD is the pull request's head: opened files match the diff.
    pub checked_out: bool,
}

/// What the tab asks the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrAction {
    /// Open `path` (relative to the repository root) at 0-based `line`.
    OpenFile { path: String, line: usize },
    /// Check the pull request out on local branch `branch`, tracking
    /// `merge_ref` on origin (see `compare::pull_merge_ref`).
    Checkout {
        branch: String,
        head_sha: String,
        merge_ref: String,
    },
}

enum State {
    Loading(Receiver<Result<PrData, String>>),
    Failed(String),
    Ready(PrData),
}

pub struct PrView {
    pub number: u64,
    workdir: PathBuf,
    repo: RepoRef,
    config_token: String,
    state: State,
    /// Files marked viewed: path → the content they were viewed with.
    viewed: HashMap<String, Option<git2::Oid>>,
    diff: DiffView,
    /// F7 stop reached last.
    current_change: Option<usize>,
    /// "Show anyway" of a large file in flight: file index and result.
    full_text: Option<(usize, Receiver<Result<DiffContent, String>>)>,
    /// A problem to show under the header (a large file that failed to load).
    notice: Option<String>,
}

impl PrView {
    /// Open pull request `pull` of `repo` and start loading it.
    pub fn open(workdir: PathBuf, repo: RepoRef, pull: &PullSummary, config_token: String) -> Self {
        let mut view = Self::with_state(workdir, repo, pull.number, State::Failed(String::new()));
        view.config_token = config_token;
        view.reload();
        view
    }

    /// A view showing `data`, loading nothing.
    pub fn from_data(workdir: PathBuf, repo: RepoRef, data: PrData) -> Self {
        let number = data.detail.summary.number;
        Self::with_state(workdir, repo, number, State::Ready(data))
    }

    fn with_state(workdir: PathBuf, repo: RepoRef, number: u64, state: State) -> Self {
        Self {
            number,
            workdir,
            repo,
            config_token: String::new(),
            state,
            viewed: HashMap::new(),
            diff: DiffView::default(),
            current_change: None,
            full_text: None,
            notice: None,
        }
    }

    /// Load (again) the pull request in the background; "viewed" marks are kept.
    pub fn reload(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (workdir, repo, token, number) = (
            self.workdir.clone(),
            self.repo.clone(),
            self.config_token.clone(),
            self.number,
        );
        let spawned = std::thread::Builder::new()
            .name(format!("pull-{number}"))
            .spawn(move || {
                let _ = tx.send(load(&workdir, repo, number, &token));
            });
        self.state = match spawned {
            Ok(_) => State::Loading(rx),
            Err(e) => State::Failed(format!("Could not start loading: {e}")),
        };
        self.full_text = None;
        self.notice = None;
    }

    pub fn current_change(&self) -> Option<usize> {
        self.current_change
    }

    pub fn diff(&self) -> &DiffView {
        &self.diff
    }

    /// After a checkout: whether HEAD is now the pull request's head.
    pub fn set_checked_out(&mut self, on_head: bool) {
        if let State::Ready(data) = &mut self.state {
            data.checked_out = on_head;
        }
    }

    /// Mark file `file` viewed, or not viewed any more.
    pub fn toggle_viewed(&mut self, file: usize) {
        let State::Ready(data) = &self.state else {
            return;
        };
        let Some(f) = data.files.get(file) else {
            return;
        };
        if is_viewed(&self.viewed, f) {
            self.viewed.remove(f.path());
        } else {
            self.viewed.insert(f.path().to_string(), f.version());
        }
        self.diff.invalidate();
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        plugins: Option<&PluginManager>,
        md_cache: &mut CommonMarkCache,
    ) -> Option<PrAction> {
        self.poll();
        match &self.state {
            State::Loading(_) => {
                ui.add_space(24.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Loading pull request #{}…", self.number));
                });
                ui.ctx().request_repaint_after(Duration::from_millis(100));
                return None;
            }
            State::Failed(message) => {
                let message = message.clone();
                ui.add_space(24.0);
                ui.label(RichText::new(message).color(palette.error));
                if ui.button("Retry").clicked() {
                    self.reload();
                }
                return None;
            }
            State::Ready(_) => {}
        }
        if self.full_text.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        let mut action = None;
        let mut refresh = false;
        let mut toggled = None;
        let mut show_anyway = None;
        {
            let Self {
                number,
                repo,
                state: State::Ready(data),
                viewed,
                diff,
                current_change,
                notice,
                ..
            } = self
            else {
                return None;
            };
            let number = *number;
            let viewed_flags: Vec<bool> = data.files.iter().map(|f| is_viewed(viewed, f)).collect();
            let d = &data.detail;

            // ── Header ──
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!("#{} {}", d.summary.number, d.summary.title))
                    .heading()
                    .strong(),
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new(&d.summary.author).strong());
                ui.label(
                    RichText::new(format!(
                        "wants to merge {} into {}",
                        d.summary.head_ref, d.summary.base_ref
                    ))
                    .color(palette.text_muted),
                );
                if d.summary.draft {
                    ui.label(RichText::new("Draft").small().color(palette.warning));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(format!("{} Open on GitHub", regular::ARROW_SQUARE_OUT))
                        .clicked()
                    {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(&d.html_url));
                    }
                    if ui
                        .button(format!("{} Checkout", regular::GIT_BRANCH))
                        .on_hover_text("Check the pull request out on a local branch")
                        .clicked()
                    {
                        let from_fork = d
                            .head_repo
                            .as_deref()
                            .is_none_or(|r| !r.eq_ignore_ascii_case(&repo.full_name()));
                        let (n, head) = (d.summary.number, d.summary.head_ref.as_str());
                        action = Some(PrAction::Checkout {
                            branch: compare::pull_branch_name(n, head, from_fork),
                            head_sha: d.head_sha.clone(),
                            merge_ref: compare::pull_merge_ref(n, head, from_fork),
                        });
                    }
                    if ui
                        .button(format!("{} Refresh", regular::ARROW_CLOCKWISE))
                        .clicked()
                    {
                        refresh = true;
                    }
                });
            });
            let hint = if data.checked_out {
                "Double-click a line to open it in the editor."
            } else {
                "Double-click a line to open it in the editor. Your working copy is not on this pull request's head: the file may differ."
            };
            ui.label(RichText::new(hint).small().color(palette.text_faint));
            if let Some(n) = notice.as_deref() {
                ui.label(RichText::new(n).small().color(palette.warning));
            }
            egui::CollapsingHeader::new("Description")
                .id_salt(("pr-description", number))
                .default_open(true)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("pr-description-scroll", number))
                        .max_height(220.0)
                        .show(ui, |ui| {
                            if d.body.trim().is_empty() {
                                ui.label(
                                    RichText::new("No description")
                                        .italics()
                                        .color(palette.text_muted),
                                );
                            } else {
                                CommonMarkViewer::new().show(ui, md_cache, &d.body);
                            }
                        });
                });
            ui.separator();

            // ── F7 / Shift+F7 ──
            if !ui.ctx().wants_keyboard_input() {
                let (next, prev) = ui.input(|i| {
                    let f7 = i.key_pressed(egui::Key::F7);
                    (f7 && !i.modifiers.shift, f7 && i.modifiers.shift)
                });
                if let Some(layout) = diff.layout().filter(|_| next || prev) {
                    let target = if next {
                        layout.next_change(*current_change)
                    } else {
                        layout.prev_change(*current_change)
                    };
                    if let Some(t) = target {
                        *current_change = Some(t);
                        diff.go_to_change(t);
                    }
                }
            }

            // ── Files ──
            egui::SidePanel::left(egui::Id::new(("pr-files", number)))
                .resizable(true)
                .default_width(260.0)
                .width_range(160.0..=600.0)
                .show_inside(ui, |ui| {
                    ui.label(
                        RichText::new(format!("FILES ({})", data.files.len()))
                            .strong()
                            .small(),
                    );
                    egui::ScrollArea::vertical()
                        .id_salt(("pr-files-scroll", number))
                        .show(ui, |ui| {
                            for (i, f) in data.files.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    let mut seen = viewed_flags[i];
                                    if ui.checkbox(&mut seen, "").on_hover_text("Viewed").changed() {
                                        toggled = Some(i);
                                    }
                                    let (icon, color) = kind_icon(f.kind, &palette);
                                    ui.label(RichText::new(icon).monospace().small().color(color));
                                    if matches!(f.content, DiffContent::Text(_)) {
                                        ui.label(
                                            RichText::new(format!("+{}", f.additions))
                                                .small()
                                                .color(palette.git_added),
                                        );
                                        ui.label(
                                            RichText::new(format!("−{}", f.deletions))
                                                .small()
                                                .color(palette.git_removed),
                                        );
                                    }
                                    let threads =
                                        data.threads.iter().filter(|t| t.path == f.path()).count();
                                    if threads > 0 {
                                        ui.label(
                                            RichText::new(format!("{} {threads}", regular::CHAT_CIRCLE))
                                                .small()
                                                .color(palette.text_muted),
                                        );
                                    }
                                    let name = ui
                                        .add(
                                            egui::Label::new(RichText::new(f.display_path()).small())
                                                .truncate()
                                                .sense(egui::Sense::click()),
                                        )
                                        .on_hover_text(f.display_path());
                                    if name.clicked() {
                                        diff.scroll_to_file(i);
                                    }
                                });
                            }
                        });
                });

            // ── Diff ──
            let mut events = Vec::new();
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    let input = DiffInput {
                        files: &data.files,
                        threads: &data.threads,
                        viewed: &viewed_flags,
                        palette,
                        plugins,
                    };
                    events = diff.show(ui, &input, md_cache);
                });
            for event in events {
                match event {
                    DiffEvent::ToggleViewed(i) => toggled = Some(i),
                    DiffEvent::Unfold { file, row } => {
                        if let Some(DiffContent::Text(text)) =
                            data.files.get_mut(file).map(|f| &mut f.content)
                        {
                            text.unfold(row);
                        }
                        diff.invalidate();
                    }
                    DiffEvent::ShowAnyway(file) => show_anyway = Some(file),
                    DiffEvent::OpenLine { file, line } => {
                        if let Some(path) = data.files.get(file).and_then(|f| f.new_path.clone()) {
                            action = Some(PrAction::OpenFile { path, line });
                        }
                    }
                }
            }
        }
        if let Some(file) = toggled {
            self.toggle_viewed(file);
        }
        if let Some(file) = show_anyway {
            self.start_full_text(file);
        }
        if refresh {
            self.reload();
        }
        action
    }

    /// Take the finished background loads, if any.
    fn poll(&mut self) {
        let loaded = match &self.state {
            State::Loading(rx) => match rx.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err("Loading stopped unexpectedly.".into())),
            },
            _ => None,
        };
        match loaded {
            Some(Ok(data)) => {
                self.state = State::Ready(data);
                self.diff.reset();
                self.current_change = None;
            }
            Some(Err(e)) => self.state = State::Failed(e),
            None => {}
        }
        let finished = match &self.full_text {
            Some((file, rx)) => match rx.try_recv() {
                Ok(result) => Some((*file, result)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some((*file, Err("Loading the file stopped unexpectedly.".into())))
                }
            },
            None => None,
        };
        let Some((file, result)) = finished else {
            return;
        };
        self.full_text = None;
        let State::Ready(data) = &mut self.state else {
            return;
        };
        match result {
            Ok(content) => {
                if let Some(f) = data.files.get_mut(file) {
                    f.set_content(content);
                    unfold_for_threads(std::slice::from_mut(f), &data.threads);
                }
                self.diff.invalidate();
            }
            Err(e) => self.notice = Some(e),
        }
    }

    /// Load a large file's diff anyway, in the background.
    fn start_full_text(&mut self, file: usize) {
        let State::Ready(data) = &self.state else {
            return;
        };
        let Some(f) = data.files.get(file) else {
            return;
        };
        let (workdir, old, new) = (self.workdir.clone(), f.old_blob, f.new_blob);
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("pull-file".into())
            .spawn(move || {
                let _ = tx.send(compare::load_text(&workdir, old, new));
            });
        match spawned {
            Ok(_) => {
                self.full_text = Some((file, rx));
                self.notice = None;
            }
            Err(e) => self.notice = Some(format!("Could not load the file: {e}")),
        }
    }
}

/// `f` is marked viewed, with the content it has now.
fn is_viewed(viewed: &HashMap<String, Option<git2::Oid>>, f: &FileDiff) -> bool {
    viewed.get(f.path()).is_some_and(|version| *version == f.version())
}

/// Fetch and diff pull request `number`, with its review threads.
fn load(workdir: &Path, repo: RepoRef, number: u64, config_token: &str) -> Result<PrData, String> {
    let client = github::Client::new(repo, github::auth::resolve(config_token));
    let detail = client.pull_detail(number).map_err(|e| e.to_string())?;
    let (threads, files) = std::thread::scope(|s| {
        let threads = s.spawn(|| client.review_threads(number));
        let files = compare::fetch_pull(workdir, number, &detail.summary.base_ref)
            .and_then(|()| compare::diff_pull(workdir, &detail.base_sha, &detail.head_sha));
        (threads.join(), files)
    });
    let mut files = files?;
    let threads = threads
        .map_err(|_| "Loading the review comments stopped unexpectedly.".to_string())?
        .map_err(|e| e.to_string())?;
    unfold_for_threads(&mut files, &threads);
    let checked_out = git2::Repository::discover(workdir)
        .ok()
        .and_then(|r| r.head().ok()?.target())
        .is_some_and(|oid| oid.to_string() == detail.head_sha);
    Ok(PrData {
        detail,
        files,
        threads,
        checked_out,
    })
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `rtk cargo test ui::pr_view`
Expected: 3 tests pass. If `f7_walks_through_the_changes` sees `None`, check that the first frame built the layout (`view.diff().layout().is_some()` after it): F7 is ignored until the diff has been laid out once.

- [ ] **Step 5: Checkpoint**

Run: `rtk cargo clippy --all-targets`
Expected: no new warnings.

---

### Task 13: PULL REQUESTS section of the Git panel

**Files:**
- Create: `src/ui/pr_list.rs`
- Modify: `src/ui/mod.rs`, `src/ui/git_panel.rs`, `src/ui/layout.rs` (Git panel call and its test)

- [ ] **Step 1: Write the failing test**

Create `src/ui/pr_list.rs` with its test:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_repository_without_a_github_origin_says_so() {
        let r = crate::git::tests::TestRepo::new();
        let mut list = PrList::new();
        list.start(r.path().to_path_buf(), String::new());
        let deadline = Instant::now() + Duration::from_secs(5);
        while list.loading.is_some() && Instant::now() < deadline {
            list.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            list.error.as_deref(),
            Some("The origin remote is not a GitHub repository.")
        );
        assert!(list.pulls.is_empty());
        assert!(list.open_request.is_none());
    }
}
```

In `src/ui/mod.rs`, after `pub mod palette;`:

```rust
pub mod pr_list;
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `rtk cargo test ui::pr_list`
Expected: compile errors: `PrList` not found.

- [ ] **Step 3: Implement the section**

At the top of `src/ui/pr_list.rs`:

```rust
//! The PULL REQUESTS section of the Git panel: the open pull requests of
//! `origin`'s GitHub repository, loaded in the background when the section
//! is first expanded.

use crate::git::GitStatus;
use crate::github::{self, GithubError, PullSummary, RepoRef};
use egui_phosphor::regular;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

type Loaded = Result<(RepoRef, Vec<PullSummary>), GithubError>;

pub struct PrList {
    /// Folder the list belongs to; another workspace starts over.
    loaded_for: Option<PathBuf>,
    loading: Option<Receiver<Loaded>>,
    repo: Option<RepoRef>,
    pulls: Vec<PullSummary>,
    error: Option<String>,
    /// The pull request clicked, for the app to open.
    pub open_request: Option<(RepoRef, PullSummary)>,
}

impl PrList {
    pub fn new() -> Self {
        Self {
            loaded_for: None,
            loading: None,
            repo: None,
            pulls: Vec::new(),
            error: None,
            open_request: None,
        }
    }

    /// Load the list for the repository holding `path`, in the background.
    pub fn start(&mut self, path: PathBuf, config_token: String) {
        let (tx, rx) = std::sync::mpsc::channel();
        let folder = path.clone();
        let spawned = std::thread::Builder::new()
            .name("github-pulls".into())
            .spawn(move || {
                let result = github::origin_repo(&folder).and_then(|repo| {
                    let client = github::Client::new(repo.clone(), github::auth::resolve(&config_token));
                    client.list_open_pulls().map(|pulls| (repo, pulls))
                });
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => {
                self.loading = Some(rx);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Could not start loading: {e}")),
        }
        self.loaded_for = Some(path);
    }

    /// Take the finished load, if any. A failure keeps the previous list.
    fn poll(&mut self) {
        let Some(rx) = &self.loading else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err(GithubError::Network("loading stopped unexpectedly".into()))
            }
        };
        self.loading = None;
        match result {
            Ok((repo, pulls)) => {
                self.repo = Some(repo);
                self.pulls = pulls;
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, git: &GitStatus, config_token: &str) {
        let Some(path) = git.repo_path.clone() else {
            return;
        };
        if self.loaded_for.as_ref().is_some_and(|p| p != &path) {
            // Another workspace was opened: its pull requests, not these.
            *self = Self::new();
        }
        self.poll();
        let title = match (&self.loading, self.pulls.len()) {
            (Some(_), 0) => "PULL REQUESTS".to_string(),
            (_, n) => format!("PULL REQUESTS ({n})"),
        };
        egui::CollapsingHeader::new(egui::RichText::new(title).strong().small())
            .id_salt("pull-requests")
            .default_open(false)
            .show(ui, |ui| {
                if self.loaded_for.is_none() {
                    self.start(path.clone(), config_token.to_string());
                }
                ui.horizontal(|ui| {
                    let idle = self.loading.is_none();
                    let refresh = egui::Button::new(format!("{} Refresh", regular::ARROW_CLOCKWISE)).small();
                    if ui.add_enabled(idle, refresh).clicked() {
                        self.start(path.clone(), config_token.to_string());
                    }
                    if !idle {
                        ui.spinner();
                        ui.ctx().request_repaint_after(Duration::from_millis(100));
                    }
                });
                if let Some(err) = &self.error {
                    ui.label(
                        egui::RichText::new(err)
                            .small()
                            .color(egui::Color32::from_rgb(220, 80, 80)),
                    );
                }
                if self.pulls.is_empty() && self.error.is_none() && self.loading.is_none() {
                    ui.label(
                        egui::RichText::new("No open pull requests")
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                }
                let mut clicked = None;
                for pull in &self.pulls {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("#{}", pull.number))
                                .monospace()
                                .small()
                                .color(egui::Color32::GRAY),
                        );
                        let title = ui
                            .add(
                                egui::Label::new(egui::RichText::new(&pull.title).small())
                                    .truncate()
                                    .sense(egui::Sense::click()),
                            )
                            .on_hover_text(pull.title.as_str());
                        if title.clicked() {
                            clicked = Some(pull.clone());
                        }
                    });
                    let draft = if pull.draft { " · Draft" } else { "" };
                    ui.label(
                        egui::RichText::new(format!(
                            "{} · {} → {}{draft}",
                            pull.author, pull.head_ref, pull.base_ref
                        ))
                        .small()
                        .color(egui::Color32::GRAY),
                    );
                    ui.add_space(2.0);
                }
                if let (Some(pull), Some(repo)) = (clicked, &self.repo) {
                    self.open_request = Some((repo.clone(), pull));
                }
            });
    }
}
```

- [ ] **Step 4: Host it in the Git panel**

In `src/ui/git_panel.rs`:
- In `pub struct GitPanel`, after `stash_message`:

```rust
    /// The PULL REQUESTS section.
    pub pulls: crate::ui::pr_list::PrList,
```

- In `GitPanel::new`, after `stash_message: String::new(),`:

```rust
            pulls: crate::ui::pr_list::PrList::new(),
```

- Change the signature of `show` to take the token:

```rust
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        git: &mut GitStatus,
        github_token: &str,
    ) -> Option<String> {
```

- In `show`, replace

```rust
        self.show_stashes(ui, git);
        ui.separator();
```

with

```rust
        self.show_stashes(ui, git);
        ui.separator();
        self.pulls.show(ui, git, github_token);
        ui.separator();
```

In `src/ui/layout.rs`:
- In the `SidebarTab::Git` arm (around line 804), replace

```rust
                        let merge_file = app.git_panel.show(ui, &mut app.git_status);
```

with

```rust
                        let merge_file =
                            app.git_panel
                                .show(ui, &mut app.git_status, &app.config.github_token);
```

- In the layout test that renders the Git panel (around line 1820), replace `git_panel.show(ui, &mut git);` with `git_panel.show(ui, &mut git, "");`.

A click on a pull request only sets `open_request` for now; Task 14 opens the tab.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `rtk cargo test ui::pr_list` then `rtk cargo test`
Expected: the new test passes and the whole suite stays green.

- [ ] **Step 6: Checkpoint**

Run: `rtk cargo clippy --all-targets`
Expected: no new warnings.

---

### Task 14: Wiring the tab into the app

**Files:**
- Create: `src/app/pr_ops.rs`
- Modify: `src/app/mod.rs`, `src/ui/layout.rs` (two central-panel routings)

- [ ] **Step 1: Add the app glue**

Create `src/app/pr_ops.rs`:

```rust
//! Pull request tabs: opening them, applying what they ask for, and
//! dropping the views of closed tabs.

use super::CodingUnicorns;
use crate::github::{PullSummary, RepoRef};
use crate::ui::pr_view::{PrAction, PrView};

impl CodingUnicorns {
    /// Open (or focus) the tab of pull request `pull` of `repo`.
    pub fn open_pull_request(&mut self, repo: RepoRef, pull: PullSummary) {
        let Some(workdir) = self.git_status.workdir() else {
            return;
        };
        let token = self.config.github_token.clone();
        self.pr_views
            .entry(pull.number)
            .or_insert_with(|| PrView::open(workdir, repo, &pull, token));
        self.tab_manager.open_pr(pull.number, &pull.title);
    }

    /// Apply what the tab of pull request `number` asked for.
    pub fn apply_pr_action(&mut self, number: u64, action: PrAction) {
        match action {
            PrAction::OpenFile { path, line } => {
                let Some(root) = self.git_status.workdir() else {
                    return;
                };
                self.active_pane = 0;
                self.open_file_at_line(root.join(path), line);
            }
            PrAction::Checkout {
                branch,
                head_sha,
                merge_ref,
            } => {
                match self.git_status.checkout_pull(&branch, &head_sha, &merge_ref) {
                    Ok(existed) => {
                        let on_head = self
                            .git_status
                            .open_repo()
                            .ok()
                            .and_then(|r| r.head().ok()?.target())
                            .is_some_and(|oid| oid.to_string() == head_sha);
                        if let Some(view) = self.pr_views.get_mut(&number) {
                            view.set_checked_out(on_head);
                        }
                        self.toast(if existed && !on_head {
                            format!("Checked out the existing branch {branch}: it may differ from the pull request.")
                        } else {
                            format!("Checked out {branch}.")
                        });
                    }
                    Err(e) => self.toast(format!("Checkout failed: {e}")),
                }
            }
        }
    }

    /// Forget the views of closed pull request tabs.
    pub fn drop_closed_pr_views(&mut self) {
        let tabs = &self.tab_manager.tabs;
        self.pr_views
            .retain(|number, _| tabs.iter().any(|t| t.pr == Some(*number)));
    }
}
```

- [ ] **Step 2: Hold the views in the app**

In `src/app/mod.rs`:
- After `mod navigation;`, add `mod pr_ops;`.
- In `pub struct CodingUnicorns`, after the `merge_view` field:

```rust
    /// Views of the open pull request tabs, by pull request number.
    pub pr_views: std::collections::HashMap<u64, crate::ui::pr_view::PrView>,
```

- In the constructor's struct literal, after `close_tab_id_pending: None,`:

```rust
            pr_views: std::collections::HashMap::new(),
```

- In `update`, right after

```rust
        if let Some((scope, anchor)) = self.tab_manager.bulk_close.take() {
            self.close_tabs(scope, Some(anchor));
        }
```

add

```rust
        self.drop_closed_pr_views();
```

- [ ] **Step 3: Open clicked pull requests and route PR tabs to their view**

In `src/ui/layout.rs`, in the `SidebarTab::Git` arm, right after the `app.git_panel.show(…)` call changed in Task 13:

```rust
                        if let Some((repo, pull)) = app.git_panel.pulls.open_request.take() {
                            app.open_pull_request(repo, pull);
                        }
```

Add this function next to the other free functions of the file (`CodingUnicorns` is already imported there):

```rust
/// Draw the tab of pull request `number` and apply what it asks for.
fn show_pr_tab(app: &mut CodingUnicorns, ui: &mut egui::Ui, number: u64) {
    let Some(view) = app.pr_views.get_mut(&number) else {
        ui.label("This pull request is not loaded any more: open it again from the Git panel.");
        return;
    };
    if let Some(action) = view.show(ui, app.palette, Some(&app.plugin_manager), &mut app.md_cache) {
        app.apply_pr_action(number, action);
    }
}
```

In both central-panel routings (the split one around line 1228 using `left_ui`, and the single-pane one around line 1446 using `ui`), put a PR branch first. Replace

```rust
                if let Some(page) = app.tab_manager.active_page().map(str::to_string) {
```

with, in the split routing:

```rust
                if let Some(number) = app.tab_manager.active_pr() {
                    show_pr_tab(app, &mut left_ui, number);
                } else if let Some(page) = app.tab_manager.active_page().map(str::to_string) {
```

and in the single-pane routing:

```rust
                if let Some(number) = app.tab_manager.active_pr() {
                    show_pr_tab(app, ui, number);
                } else if let Some(page) = app.tab_manager.active_page().map(str::to_string) {
```

- [ ] **Step 4: Build and run the whole suite**

Run: `rtk cargo build` then `rtk cargo test`
Expected: builds; all tests pass.

- [ ] **Step 5: Checkpoint**

Run: `rtk cargo clippy --all-targets`
Expected: no new warnings.

---

### Task 15: GitHub section in Settings

**Files:**
- Modify: `src/ui/settings.rs`

- [ ] **Step 1: Add the section**

In `SettingsPanel::show_inline`, right after the EXTENSIONS section (the block ending with `ui.add_space(16.0);` after the registry URL hint), add:

```rust
            // ═══════════════════════════════════════════════════════════════
            // GITHUB
            // ═══════════════════════════════════════════════════════════════
            if setting_matches(&q, &["github", "token", "pull", "request", "review"]) {
                section_heading(ui, "GitHub");
                ui.label("GitHub token");
                ui.add_space(2.0);
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut config.github_token)
                            .password(true)
                            .hint_text("(GitHub CLI login)")
                            .desired_width(ui.available_width()),
                    )
                    .changed();
                hint(
                    ui,
                    "Lists the pull requests of the Git panel and reads their review comments. Leave empty to use the GitHub CLI's login (gh auth login); public repositories also work without a token. Stored in the config file.",
                );
                ui.add_space(16.0);
            }
```

- [ ] **Step 2: Build**

Run: `rtk cargo build`
Expected: builds.

- [ ] **Step 3: Checkpoint**

Run: `rtk cargo test` then `rtk cargo clippy --all-targets`
Expected: all tests pass, no new warnings.

---

### Task 16: Docs and final verification

**Files:**
- Modify: `README.md`, `CHANGELOG.md`, `CHANGELOG.fr.md`

- [ ] **Step 1: README**

In `README.md`, in the `### Git` list, after the `🔀 3-panel merge tool` line:

```markdown
- 🔍 Revue des pull requests GitHub : liste dans le panneau Git, diff côte à côte avec les commentaires de review, navigation F7 / Maj+F7, fichiers marqués comme vus, checkout sur une branche locale (lecture seule)
```

- [ ] **Step 2: Changelogs**

In `CHANGELOG.md`, above `## [0.21.0] - 2026-10-15`:

```markdown
## [Unreleased]

### Added

- Pull requests: the Git panel lists the open pull requests of the repository's GitHub `origin`. Opening one shows its description, the files it changes and a side-by-side diff with the review comments under the lines they are about. F7 / Shift+F7 jump between changes, a checkbox marks each file as viewed, and Checkout puts the pull request on a local branch. Set a token in Settings → GitHub, or let the IDE use the GitHub CLI's login (`gh auth login`).

```

In `CHANGELOG.fr.md`, above `## [0.21.0] - 2026-10-15`:

```markdown
## [Non publié]

### Ajouts

- Pull requests : le panneau Git liste les pull requests ouvertes du dépôt GitHub `origin`. En ouvrir une affiche sa description, les fichiers modifiés et un diff côte à côte avec les commentaires de review sous les lignes concernées. F7 / Maj+F7 passent d'un changement à l'autre, une case marque chaque fichier comme vu, et Checkout place la pull request sur une branche locale. Renseignez un token dans Settings → GitHub, ou laissez l'IDE utiliser la connexion de la CLI GitHub (`gh auth login`).

```

- [ ] **Step 3: Format, lint, test**

Run: `rtk cargo fmt` then `rtk cargo clippy --all-targets` then `rtk cargo test`
Expected: no clippy warnings in the new code; every test passes.

- [ ] **Step 4: Try it in the app**

Use the `run` skill to start the IDE on this repository (origin `git@github.com:Ajustor/writingUnicorns.git`, `gh` logged in), then check:

1. Git panel → expand PULL REQUESTS: the open PRs load (spinner, then the list). With none open, "No open pull requests".
2. Click a PR: a `#N title` tab opens with a spinner, then the header, the description, the file list and the diff.
3. Rows: removed lines red on the left, added green on the right, fillers facing them, changed words stronger. Syntax colours when the language module is installed.
4. A `⋯ N unchanged lines` row unfolds on click.
5. F7 / Shift+F7 move between change blocks, scrolling and outlining them; they stop at the first and last.
6. Ticking "viewed" (file list or header) collapses the file; unticking brings it back.
7. Review comments appear under their line, on their side; comments on a whole file under "Comments on the file" and outdated ones under "Outdated comments", at the top of their file.
8. Double-click a right-hand line: the file opens at that line.
9. Checkout: the toast names the branch, the Git panel shows it as current. With local changes in the way, the toast says the checkout failed and no branch is left.
10. Close the tab, reopen the PR: it loads again.
11. With `gh` logged out and no token, on a private repository: the message asks for `gh auth login` or a token.

Report what was checked and anything that didn't work.

- [ ] **Step 5: Hand over**

Tell the user the work is ready for their review and commit. Do not commit.
