//! GitHub REST API: the calls the pull request view needs, and their JSON.

use super::error::GithubError;
use super::RepoRef;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    /// The base side (the merge base of the three-dot diff, not the tip of the
    /// base branch): left column.
    Left,
    /// The pull request: right column.
    Right,
}

/// One comment of a review thread.
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
    /// 1-based line number in the file on `side` (not a diff position); `None`
    /// when GitHub reports the thread outdated, or for a file-level thread.
    pub line: Option<u32>,
    /// The line the thread was first written on.
    pub original_line: Option<u32>,
    /// On the whole file (`subject_type: "file"`), not on a line: `line` is
    /// `None` without the thread being outdated.
    pub file_level: bool,
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
    subject_type: Option<String>,
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

/// Group review comments into threads: a comment without `in_reply_to_id`
/// starts one, a reply joins the thread of the comment it answers, and replies
/// whose parent is missing share a thread of their own. Comments are ordered by
/// id first (ids only increase, so parents come before replies), whatever order
/// the API used.
pub(crate) fn group_threads(mut comments: Vec<WireComment>) -> Vec<ReviewThread> {
    comments.sort_unstable_by_key(|c| c.id);
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
                    file_level: c.subject_type.as_deref() == Some("file"),
                    comments: vec![comment],
                });
                threads.len() - 1
            }
        };
        // Siblings of a missing parent join the thread this one started.
        if let Some(p) = c.in_reply_to_id {
            thread_of.entry(p).or_insert(index);
        }
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

// ── HTTP ────────────────────────────────────────────────────────────────────

const API: &str = "https://api.github.com";
const USER_AGENT: &str = concat!("coding-unicorns/", env!("CARGO_PKG_VERSION"));
/// Cap on one response body, against a runaway response.
const MAX_BODY: u64 = 10 * 1024 * 1024;
/// Cap on followed pages of review comments (100 items each).
const MAX_PAGES: usize = 20;
/// Cap on followed pages of the open pull requests listed: a Refresh must stay
/// cheap, also without a token (60 requests/hour).
const MAX_LIST_PAGES: usize = 3;

/// Blocking client for one repository: use it off the UI thread.
pub struct Client {
    repo: RepoRef,
    token: Option<String>,
    agent: ureq::Agent,
    /// Origin of the API; only tests use another one.
    api: String,
}

fn agent_config() -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    ureq::Agent::config_builder()
        .tls_config(crate::extension::remote_registry::tls_config())
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        // Renamed or transferred repositories answer 301 to another path of
        // api.github.com: the token must follow that, and `SameHost` never
        // sends it to another host.
        .redirect_auth_headers(ureq::config::RedirectAuthHeaders::SameHost)
}

impl Client {
    pub fn new(repo: RepoRef, token: Option<String>) -> Self {
        let agent = agent_config().build().into();
        Self {
            repo,
            token,
            agent,
            api: API.to_string(),
        }
    }

    /// A client for a server at `api`, bypassing any proxy of the environment.
    #[cfg(test)]
    fn at(api: &str, repo: RepoRef, token: Option<String>) -> Self {
        let agent = agent_config().proxy(None).build().into();
        Self {
            repo,
            token,
            agent,
            api: api.to_string(),
        }
    }

    /// The open pull requests, most recent first.
    pub fn list_open_pulls(&self) -> Result<Vec<PullSummary>, GithubError> {
        let mut pulls = Vec::new();
        for page in self.get_pages(self.url("pulls?state=open&per_page=100"), MAX_LIST_PAGES)? {
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
        for page in self.get_pages(first, MAX_PAGES)? {
            comments.extend(parse_comments(&page)?);
        }
        Ok(group_threads(comments))
    }

    fn url(&self, rest: &str) -> String {
        format!(
            "{}/repos/{}/{}/{rest}",
            self.api, self.repo.owner, self.repo.name
        )
    }

    /// Bodies of `first` and of the pages its `Link` headers lead to.
    fn get_pages(&self, first: String, max_pages: usize) -> Result<Vec<String>, GithubError> {
        let mut pages = Vec::new();
        let mut next = Some(first);
        while let Some(url) = next.take() {
            if pages.len() == max_pages {
                log::warn!("GitHub: stopped after {max_pages} pages, not fetching {url}");
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
            .and_then(|url| {
                let kept = same_api(&self.api, url.clone());
                if kept.is_none() {
                    let rest = url.split_once("://").map_or("?", |(_, rest)| rest);
                    let host = rest.split(['/', '?', '#']).next().unwrap_or("?");
                    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
                    log::warn!("GitHub: ignored a next page outside the API ({host})");
                }
                kept
            });
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_string()
            .map_err(|e| match e {
                ureq::Error::BodyExceedsLimit(_) => {
                    log::warn!("GitHub: response larger than {MAX_BODY} bytes");
                    GithubError::BadResponse
                }
                e => GithubError::Network(e.to_string()),
            })?;
        Ok((body, next))
    }
}

/// `url` if it is under `api`: the token must never leave it.
fn same_api(api: &str, url: String) -> Option<String> {
    url.starts_with(&format!("{api}/")).then_some(url)
}

/// A one-comment thread, for tests.
#[cfg(test)]
pub(crate) fn thread(path: &str, side: Side, line: Option<u32>) -> ReviewThread {
    ReviewThread {
        path: path.to_string(),
        side,
        line,
        original_line: line.or(Some(1)),
        file_level: false,
        comments: vec![ReviewComment {
            author: "reviewer".into(),
            body: "Why?".into(),
            created_at: "2026-10-14T09:00:00Z".into(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

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
       "user": {"login": "rev"}, "body": "Orphan", "created_at": "2026-10-14T11:00:00Z"},
      {"id": 5, "in_reply_to_id": 99, "path": "src/c.rs", "line": 2, "original_line": 2, "side": "RIGHT",
       "user": {"login": "rev"}, "body": "Orphan 2", "created_at": "2026-10-14T11:30:00Z"},
      {"id": 6, "path": "src/d.rs", "subject_type": "file", "line": null, "original_line": null, "side": "RIGHT",
       "user": {"login": "rev"}, "body": "Whole file", "created_at": "2026-10-14T12:00:00Z"},
      {"id": 7, "in_reply_to_id": 2, "path": "src/a.rs", "line": 12, "original_line": 12, "side": "RIGHT",
       "user": {"login": "rev"}, "body": "Reply to reply", "created_at": "2026-10-14T13:00:00Z"},
      {"id": 8, "path": "src/e.rs", "line": 1, "original_line": 1, "side": null,
       "user": {"login": "rev"}, "body": "No side", "created_at": "2026-10-14T14:00:00Z"},
      {"id": 9, "path": "src/f.rs", "line": 1, "original_line": 1,
       "user": {"login": "rev"}, "body": "No side key", "created_at": "2026-10-14T15:00:00Z"}
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
        assert_eq!(
            (detail.head_sha.as_str(), detail.base_sha.as_str()),
            ("ccc", "bbb")
        );
        let detail = parse_detail(&list[0].to_string()).unwrap();
        assert_eq!(detail.head_repo.as_deref(), Some("Ajustor/writingUnicorns"));
        assert_eq!(
            detail.html_url,
            "https://github.com/Ajustor/writingUnicorns/pull/58"
        );
    }

    #[test]
    fn comment_order_does_not_matter() {
        let mut reversed = parse_comments(COMMENTS).unwrap();
        reversed.reverse();
        assert_eq!(
            group_threads(reversed),
            group_threads(parse_comments(COMMENTS).unwrap())
        );
    }

    #[test]
    fn pull_without_draft_key_is_not_a_draft() {
        let json = r#"{"number": 1, "title": "t", "user": {"login": "u"},
          "head": {"ref": "h", "sha": "a", "repo": null},
          "base": {"ref": "b", "sha": "b", "repo": null},
          "body": null, "html_url": "https://github.com/o/r/pull/1"}"#;
        assert!(!parse_detail(json).unwrap().summary.draft);
    }

    #[test]
    fn unexpected_json_is_a_bad_response() {
        assert_eq!(parse_pulls("{}"), Err(GithubError::BadResponse));
        assert_eq!(parse_detail("[]"), Err(GithubError::BadResponse));
    }

    #[test]
    fn comments_are_grouped_into_threads() {
        let threads = group_threads(parse_comments(COMMENTS).unwrap());
        assert_eq!(threads.len(), 6);
        assert_eq!(
            (threads[0].path.as_str(), threads[0].side, threads[0].line),
            ("src/a.rs", Side::Right, Some(12))
        );
        let bodies: Vec<&str> = threads[0]
            .comments
            .iter()
            .map(|c| c.body.as_str())
            .collect();
        assert_eq!(bodies, ["Why?", "Because.", "Reply to reply"]);
        // Outdated: no line any more, but the original one is kept.
        assert_eq!(
            (threads[1].side, threads[1].line, threads[1].original_line),
            (Side::Left, None, Some(4))
        );
        assert_eq!(threads[1].comments[0].author, "ghost");
        // Replies to a missing parent share a thread of their own.
        let orphans: Vec<&str> = threads[2]
            .comments
            .iter()
            .map(|c| c.body.as_str())
            .collect();
        assert_eq!(orphans, ["Orphan", "Orphan 2"]);
        // A comment on the whole file has no line without being outdated.
        assert_eq!(threads[3].comments[0].body, "Whole file");
        assert_eq!((threads[3].file_level, threads[3].line), (true, None));
        assert!([0, 1, 2, 4, 5].iter().all(|&i| !threads[i].file_level));
        // A missing or null side is the right column.
        assert_eq!(
            (threads[4].side, threads[5].side),
            (Side::Right, Side::Right)
        );
        assert_eq!(threads[0].comments[0].date(), "2026-10-14");
    }

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
        assert_eq!(same_api(API, page.clone()), Some(page));
        // A `Link` leading elsewhere would receive the token.
        assert_eq!(same_api(API, "https://evil.example/steal".into()), None);
        assert_eq!(
            same_api(API, "https://api.github.com.evil.example/x".into()),
            None
        );
        assert_eq!(
            same_api(API, "https://api.github.com@evil.example/x".into()),
            None
        );
        assert_eq!(same_api(API, "http://api.github.com/x".into()), None);
    }

    /// One canned answer of the test server.
    struct Canned {
        status: &'static str,
        headers: Vec<String>,
        body: String,
    }

    fn canned(status: &'static str, headers: &[&str], body: &str) -> Canned {
        Canned {
            status,
            headers: headers.iter().map(|h| h.to_string()).collect(),
            body: body.to_string(),
        }
    }

    /// Serves `answers(base)` in order on a local port, one per connection:
    /// the base URL, and the heads of the requests received.
    fn serve(answers: impl FnOnce(&str) -> Vec<Canned>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let answers = answers(&base);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            let served = answers
                .into_iter()
                .map_while(|a| listener.incoming().next().map(|s| (s, a)));
            for (stream, answer) in served {
                let Ok(mut stream) = stream else { continue };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                log.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf).to_string());
                let mut head = format!("HTTP/1.1 {}\r\n", answer.status);
                for h in &answer.headers {
                    head.push_str(h);
                    head.push_str("\r\n");
                }
                head.push_str(&format!(
                    "Content-Length: {}\r\nConnection: close\r\n\r\n",
                    answer.body.len()
                ));
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(answer.body.as_bytes());
            }
        });
        (base, seen)
    }

    fn client(base: &str, token: Option<&str>) -> Client {
        let repo = crate::github::RepoRef {
            owner: "me".into(),
            name: "proj".into(),
        };
        Client::at(base, repo, token.map(str::to_string))
    }

    fn has_authorization(request: &str) -> bool {
        request
            .to_ascii_lowercase()
            .contains("\r\nauthorization: bearer tok\r\n")
    }

    const PAGE_1: &str = r#"[{"id": 1, "path": "a.rs", "line": 3, "original_line": 3,
      "side": "RIGHT", "user": {"login": "rev"}, "body": "Why?",
      "created_at": "2026-10-14T09:00:00Z"}]"#;
    const PAGE_2: &str = r#"[{"id": 2, "in_reply_to_id": 1, "path": "a.rs", "line": 3,
      "original_line": 3, "side": "RIGHT", "user": {"login": "me"}, "body": "Because.",
      "created_at": "2026-10-14T10:00:00Z"},
      {"id": 3, "path": "b.rs", "line": 1, "original_line": 1, "side": "RIGHT",
      "user": {"login": "rev"}, "body": "Other", "created_at": "2026-10-14T11:00:00Z"}]"#;

    #[test]
    fn review_threads_follow_the_next_page() {
        let (base, seen) = serve(|base| {
            let link =
                format!("Link: <{base}/repos/me/proj/pulls/7/comments?page=2>; rel=\"next\"");
            vec![
                canned("200 OK", &[&link], PAGE_1),
                canned("200 OK", &[], PAGE_2),
            ]
        });
        let threads = client(&base, None).review_threads(7).unwrap();
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].comments.len(), 2);
        assert_eq!(threads[0].comments[1].body, "Because.");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[0].starts_with("GET /repos/me/proj/pulls/7/comments?per_page=100 "));
        assert!(seen[1].starts_with("GET /repos/me/proj/pulls/7/comments?page=2 "));
    }

    #[test]
    fn the_token_is_sent_only_when_there_is_one() {
        let (base, seen) =
            serve(|_| vec![canned("200 OK", &[], "[]"), canned("200 OK", &[], "[]")]);
        client(&base, Some("tok")).review_threads(7).unwrap();
        client(&base, None).review_threads(7).unwrap();
        let seen = seen.lock().unwrap();
        assert!(has_authorization(&seen[0]));
        assert!(!seen[1].to_ascii_lowercase().contains("authorization"));
    }

    #[test]
    fn a_next_page_on_another_origin_is_not_followed() {
        let (base, seen) = serve(|_| {
            vec![canned(
                "200 OK",
                &["Link: <http://127.0.0.2:1/x>; rel=\"next\""],
                PAGE_1,
            )]
        });
        // A followed link would fail this `unwrap()`: nothing listens there.
        let threads = client(&base, Some("tok")).review_threads(7).unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_repository_the_token_cannot_see_is_not_found() {
        let (base, _) = serve(|_| vec![canned("404 Not Found", &[], "{}")]);
        assert_eq!(
            client(&base, Some("tok")).review_threads(7).err(),
            Some(GithubError::NotFound)
        );
    }

    #[test]
    fn a_rejected_token_is_reported() {
        let (base, _) = serve(|_| vec![canned("401 Unauthorized", &[], "{}")]);
        assert_eq!(
            client(&base, Some("tok")).review_threads(7).err(),
            Some(GithubError::TokenRejected)
        );
    }

    #[test]
    fn an_exhausted_rate_limit_says_when_it_resets() {
        let reset = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 125;
        let (base, _) = serve(|_| {
            vec![canned(
                "403 Forbidden",
                &[
                    "x-ratelimit-remaining: 0",
                    &format!("x-ratelimit-reset: {reset}"),
                ],
                "{}",
            )]
        });
        let err = client(&base, None).review_threads(7).err();
        assert!(
            matches!(
                err,
                Some(GithubError::RateLimited {
                    minutes: Some(2 | 3)
                })
            ),
            "{err:?}"
        );
    }

    #[test]
    fn a_redirect_on_the_same_host_keeps_the_token() {
        let pull = r#"{"number": 7, "title": "t", "user": {"login": "u"},
          "head": {"ref": "h", "sha": "a", "repo": null},
          "base": {"ref": "b", "sha": "b", "repo": null},
          "body": null, "html_url": "https://github.com/me/proj/pull/7"}"#;
        let (base, seen) = serve(|base| {
            let location = format!("Location: {base}/repositories/1/pulls/7");
            vec![
                canned("301 Moved Permanently", &[&location], ""),
                canned("200 OK", &[], pull),
            ]
        });
        let detail = client(&base, Some("tok")).pull_detail(7).unwrap();
        assert_eq!(detail.summary.number, 7);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[1].starts_with("GET /repositories/1/pulls/7 "));
        assert!(has_authorization(&seen[1]));
    }

    #[test]
    fn next_page_comes_from_the_link_header() {
        let link = r#"<https://api.github.com/x?page=2>; rel="next", <https://api.github.com/x?page=5>; rel="last""#;
        assert_eq!(
            next_link(link).as_deref(),
            Some("https://api.github.com/x?page=2")
        );
        assert_eq!(
            next_link(r#"<https://api.github.com/x?page=1>; rel="prev""#),
            None
        );
        assert_eq!(next_link(""), None);
    }
}
