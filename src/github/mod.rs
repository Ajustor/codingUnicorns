//! Read-only access to the GitHub repository of the workspace: it reads the
//! `origin` remote from the git config, and talks to the GitHub API for the
//! open pull requests and their review comments. Diffs are computed locally by
//! git (`crate::git::compare`).

pub mod api;
pub mod auth;
pub mod error;

pub use api::{Client, PullDetail, PullSummary, ReviewThread, Side};
pub use error::GithubError;

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
/// credentials. `None` for another host, a local path, or an invalid owner or
/// name. SSH host aliases from `~/.ssh/config` (e.g. `git@github-work:o/r`) are
/// not recognized.
pub fn repo_from_remote_url(url: &str) -> Option<RepoRef> {
    let url = url.trim();
    let (authority, path) = match url.split_once("://") {
        Some((_scheme, rest)) => rest.split_once('/')?,
        // scp-like syntax: `[user@]host:path`.
        None => {
            let (host, path) = url.split_once(':')?;
            (host, path.strip_prefix('/').unwrap_or(path))
        }
    };
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host.split_once(':').map_or(host, |(h, _)| h);
    let is_github = ["github.com", "www.github.com", "ssh.github.com"]
        .iter()
        .any(|h| host.eq_ignore_ascii_case(h));
    if !is_github {
        return None;
    }
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    if !valid_segment(owner) || !valid_segment(name) || parts.next().is_some() {
        return None;
    }
    Some(RepoRef {
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

/// Owner or repository name: it ends up in an API URL, so only the characters
/// GitHub allows, and never `.` or `..`.
fn valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The GitHub repository of the `origin` remote of the repository holding `path`.
pub fn origin_repo(path: &Path) -> Result<RepoRef, GithubError> {
    let repo = git2::Repository::discover(path).map_err(|_| GithubError::NotGitHub)?;
    let remote = repo
        .find_remote("origin")
        .map_err(|_| GithubError::NotGitHub)?;
    remote
        .url()
        .and_then(repo_from_remote_url)
        .ok_or(GithubError::NotGitHub)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(owner: &str, name: &str) -> RepoRef {
        RepoRef {
            owner: owner.into(),
            name: name.into(),
        }
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
            "ssh://git@ssh.github.com:443/Ajustor/writingUnicorns.git",
            "git@github.com:/Ajustor/writingUnicorns.git",
            "https://www.github.com/Ajustor/writingUnicorns",
        ] {
            assert_eq!(
                repo_from_remote_url(url),
                Some(repo("Ajustor", "writingUnicorns")),
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
            "https://github.com@evil.example/o/r",
            "git@github.com:o/r?x=1",
            "git@github.com:../user",
            "https://github.com/o/my repo",
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
        assert_eq!(origin_repo(r.path()), Ok(repo("me", "proj")));
        assert_eq!(origin_repo(r.path()).unwrap().full_name(), "me/proj");
        let sub = r.path().join("sub/dir");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(origin_repo(&sub), Ok(repo("me", "proj")));
        r.repo
            .remote_set_url("origin", "https://gitlab.com/me/proj.git")
            .unwrap();
        assert_eq!(origin_repo(r.path()), Err(GithubError::NotGitHub));
    }
}
