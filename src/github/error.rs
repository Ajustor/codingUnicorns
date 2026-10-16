//! Errors of GitHub requests, worded for the user.

/// Why a GitHub request failed. `Display` is the message the Git panel and
/// the pull request tab show as is; it never contains the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubError {
    /// `origin` is missing or isn't on GitHub: nothing is requested.
    NotGitHub,
    /// 401 with a token: it expired or was revoked.
    TokenRejected,
    /// 401 without a token, or 404 without a token (GitHub hides private repositories).
    NeedsToken,
    /// 404 with a token.
    NotFound,
    /// The rate limit is reached; minutes until it resets, when known.
    RateLimited { minutes: Option<u64> },
    /// No response (DNS, TLS, timeout…). Transport error text only, never
    /// request headers or URLs carrying credentials.
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
            401 if has_token => Self::TokenRejected,
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
            Self::TokenRejected => write!(
                f,
                "GitHub refused the token: check it in Settings → GitHub token, or run `gh auth login` again."
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
            Self::Network(reason) => {
                write!(f, "Cannot reach GitHub: {}.", reason.trim_end_matches('.'))
            }
            Self::Status(code) => write!(f, "GitHub answered with status {code}."),
            Self::BadResponse => write!(f, "Unexpected response from GitHub."),
        }
    }
}

impl std::error::Error for GithubError {}

#[cfg(test)]
mod tests {
    use super::GithubError;

    #[test]
    fn statuses_map_to_actionable_errors() {
        assert_eq!(
            GithubError::from_response(401, true, None, None, 0),
            GithubError::TokenRejected
        );
        assert_eq!(
            GithubError::from_response(401, false, None, None, 0),
            GithubError::NeedsToken
        );
        assert_eq!(
            GithubError::from_response(404, false, None, None, 0),
            GithubError::NeedsToken
        );
        assert_eq!(
            GithubError::from_response(404, true, None, None, 0),
            GithubError::NotFound
        );
        assert_eq!(
            GithubError::from_response(403, true, Some("12"), None, 0),
            GithubError::Status(403)
        );
        assert_eq!(
            GithubError::from_response(500, true, None, None, 0),
            GithubError::Status(500)
        );
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
        assert!(GithubError::NeedsToken
            .to_string()
            .contains("gh auth login"));
        assert_eq!(
            GithubError::RateLimited { minutes: Some(3) }.to_string(),
            "GitHub rate limit reached, try again in 3 min."
        );
        assert!(GithubError::TokenRejected
            .to_string()
            .contains("GitHub refused the token"));
        assert_eq!(
            GithubError::Network("timed out.".into()).to_string(),
            "Cannot reach GitHub: timed out."
        );
        assert_eq!(
            GithubError::NotGitHub.to_string(),
            "The origin remote is not a GitHub repository."
        );
    }
}
