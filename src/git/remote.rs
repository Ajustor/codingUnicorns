//! Network operations against `origin`: authentication callbacks shared by
//! fetch/push, plus the `Fetch` action of the git panel.
//!
//! Credentials are tried in this order, depending on what the remote accepts:
//! SSH agent, the default key files in `~/.ssh` (unencrypted keys only:
//! passphrase-protected keys must be loaded into the agent), the git
//! credential helper for HTTP(S) remotes, then the platform default
//! (NTLM/Negotiate). libgit2 calls the credentials callback again after every
//! failed attempt, so each strategy is tried once and the callback then gives
//! up with a readable error instead of looping forever.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Hard cap on credential callback invocations for one operation.
const MAX_CREDENTIAL_ATTEMPTS: usize = 8;

/// Key files looked up in `~/.ssh`, in order of preference.
const DEFAULT_SSH_KEYS: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CredentialStep {
    SshAgent,
    SshKey(PathBuf),
    CredentialHelper,
    Default,
}

impl CredentialStep {
    fn describe(&self) -> String {
        match self {
            Self::SshAgent => "SSH agent".to_string(),
            Self::SshKey(path) => format!("key {}", path.display()),
            Self::CredentialHelper => "git credential helper".to_string(),
            Self::Default => "default credentials".to_string(),
        }
    }
}

fn is_http_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// Strategies applicable to `url` given the credential types the remote allows.
pub(crate) fn candidate_steps(
    url: &str,
    allowed: git2::CredentialType,
    ssh_dir: Option<&Path>,
) -> Vec<CredentialStep> {
    let mut steps = Vec::new();
    if allowed.contains(git2::CredentialType::SSH_KEY) {
        steps.push(CredentialStep::SshAgent);
        if let Some(dir) = ssh_dir {
            steps.extend(
                DEFAULT_SSH_KEYS
                    .iter()
                    .map(|name| dir.join(name))
                    .filter(|p| p.is_file())
                    .map(CredentialStep::SshKey),
            );
        }
    }
    if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) && is_http_url(url) {
        steps.push(CredentialStep::CredentialHelper);
    }
    if allowed.contains(git2::CredentialType::DEFAULT) {
        steps.push(CredentialStep::Default);
    }
    steps
}

/// Per-operation credential bookkeeping (what was tried, why we gave up).
#[derive(Debug, Default)]
pub(crate) struct AuthState {
    attempts: usize,
    tried: Vec<CredentialStep>,
    notes: Vec<String>,
    /// Set once the callback gave up: the message shown to the user.
    pub(crate) failure: Option<String>,
}

impl AuthState {
    fn give_up(&mut self, url: &str, reason: String) -> git2::Error {
        let mut msg = format!("Authentication failed for {url}: {reason}");
        if !self.notes.is_empty() {
            msg.push_str(&format!(" ({})", self.notes.join("; ")));
        }
        if self
            .tried
            .iter()
            .any(|s| matches!(s, CredentialStep::SshAgent | CredentialStep::SshKey(_)))
        {
            msg.push_str(". Passphrase-protected keys must be added to your SSH agent");
        }
        self.failure = Some(msg.clone());
        let mut err = git2::Error::from_str(&msg);
        err.set_code(git2::ErrorCode::Auth);
        err
    }

    /// Body of the credentials callback.
    pub(crate) fn credentials(
        &mut self,
        url: &str,
        username_from_url: Option<&str>,
        allowed: git2::CredentialType,
        ssh_dir: Option<&Path>,
    ) -> Result<git2::Cred, git2::Error> {
        self.attempts += 1;
        if self.attempts > MAX_CREDENTIAL_ATTEMPTS {
            return Err(self.give_up(url, "too many attempts".to_string()));
        }
        let user = username_from_url.unwrap_or("git");
        // SSH without a user in the URL: libgit2 first asks for a username.
        if allowed == git2::CredentialType::USERNAME {
            return git2::Cred::username(user);
        }
        loop {
            let next = candidate_steps(url, allowed, ssh_dir)
                .into_iter()
                .find(|s| !self.tried.contains(s));
            let Some(step) = next else {
                let reason = if self.tried.is_empty() {
                    format!("no supported credential type (remote accepts {allowed:?})")
                } else {
                    let tried: Vec<String> = self.tried.iter().map(|s| s.describe()).collect();
                    format!("tried {}", tried.join(", "))
                };
                return Err(self.give_up(url, reason));
            };
            self.tried.push(step.clone());
            let cred = match &step {
                CredentialStep::SshAgent => git2::Cred::ssh_key_from_agent(user),
                CredentialStep::SshKey(path) => {
                    let public = path.with_extension("pub");
                    let public = public.is_file().then_some(public);
                    git2::Cred::ssh_key(user, public.as_deref(), path, None)
                }
                CredentialStep::CredentialHelper => git2::Config::open_default()
                    .and_then(|cfg| git2::Cred::credential_helper(&cfg, url, username_from_url)),
                CredentialStep::Default => git2::Cred::default(),
            };
            match cred {
                Ok(cred) => return Ok(cred),
                // This strategy is unavailable (e.g. no helper configured):
                // move on to the next one within the same callback call.
                Err(e) => self
                    .notes
                    .push(format!("{}: {}", step.describe(), e.message())),
            }
        }
    }
}

/// Readable error for refs the remote refused during a push.
pub(crate) fn rejected_refs_error(rejected: &[String]) -> Option<String> {
    (!rejected.is_empty()).then(|| format!("Push rejected: {}", rejected.join(", ")))
}

/// Callbacks + shared state for one fetch/push against `url`.
pub(crate) struct RemoteSession {
    url: String,
    auth: Rc<RefCell<AuthState>>,
    rejected: Rc<RefCell<Vec<String>>>,
}

impl RemoteSession {
    pub(crate) fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            auth: Rc::default(),
            rejected: Rc::default(),
        }
    }

    pub(crate) fn callbacks(&self) -> git2::RemoteCallbacks<'static> {
        let mut cb = git2::RemoteCallbacks::new();
        let auth = Rc::clone(&self.auth);
        let ssh_dir = dirs_next::home_dir().map(|h| h.join(".ssh"));
        cb.credentials(move |url, username, allowed| {
            auth.borrow_mut()
                .credentials(url, username, allowed, ssh_dir.as_deref())
        });
        let rejected = Rc::clone(&self.rejected);
        cb.push_update_reference(move |refname, status| {
            if let Some(reason) = status {
                rejected.borrow_mut().push(format!("{refname} ({reason})"));
            }
            Ok(())
        });
        cb
    }

    /// Turn a libgit2 error from `context` ("Fetch"/"Push") into a UI message.
    pub(crate) fn describe_error(&self, context: &str, e: &git2::Error) -> String {
        if let Some(msg) = self.auth.borrow().failure.clone() {
            return msg;
        }
        match e.code() {
            git2::ErrorCode::Auth => {
                format!("Authentication failed for {}: {}", self.url, e.message())
            }
            git2::ErrorCode::NotFastForward => format!(
                "Push rejected (non-fast-forward): pull the remote changes first ({})",
                e.message()
            ),
            _ => format!("{context} error: {e}"),
        }
    }

    pub(crate) fn rejected_error(&self) -> Option<String> {
        rejected_refs_error(&self.rejected.borrow())
    }
}

fn proxy_options() -> git2::ProxyOptions<'static> {
    let mut proxy = git2::ProxyOptions::new();
    proxy.auto();
    proxy
}

/// Fetch `refspecs` (empty: the remote's configured ones) with authentication.
pub(crate) fn fetch_remote(remote: &mut git2::Remote, refspecs: &[&str]) -> Result<(), String> {
    let session = RemoteSession::new(remote.url().unwrap_or("origin"));
    let mut opts = git2::FetchOptions::new();
    opts.remote_callbacks(session.callbacks());
    opts.proxy_options(proxy_options());
    remote
        .fetch(refspecs, Some(&mut opts), None)
        .map_err(|e| session.describe_error("Fetch", &e))
}

/// Push `refspecs` with authentication; refs refused by the remote are errors.
pub(crate) fn push_remote(remote: &mut git2::Remote, refspecs: &[&str]) -> Result<(), String> {
    let session = RemoteSession::new(remote.url().unwrap_or("origin"));
    let mut opts = git2::PushOptions::new();
    opts.remote_callbacks(session.callbacks());
    opts.proxy_options(proxy_options());
    remote
        .push(refspecs, Some(&mut opts))
        .map_err(|e| session.describe_error("Push", &e))?;
    match session.rejected_error() {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::CredentialType as T;

    const SSH_URL: &str = "git@example.com:me/repo.git";
    const HTTPS_URL: &str = "https://example.com/me/repo.git";

    fn ssh_dir_with(keys: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for k in keys {
            std::fs::write(dir.path().join(k), "key").unwrap();
        }
        dir
    }

    #[test]
    fn ssh_candidates_are_agent_then_existing_default_keys() {
        let dir = ssh_dir_with(&["id_rsa", "id_ed25519"]);
        let steps = candidate_steps(SSH_URL, T::SSH_KEY, Some(dir.path()));
        assert_eq!(
            steps,
            [
                CredentialStep::SshAgent,
                CredentialStep::SshKey(dir.path().join("id_ed25519")),
                CredentialStep::SshKey(dir.path().join("id_rsa")),
            ],
            "missing id_ecdsa skipped, preference order kept"
        );
        assert_eq!(
            candidate_steps(SSH_URL, T::SSH_KEY, None),
            [CredentialStep::SshAgent]
        );
    }

    #[test]
    fn https_candidates_are_helper_then_default() {
        let steps = candidate_steps(HTTPS_URL, T::USER_PASS_PLAINTEXT | T::DEFAULT, None);
        assert_eq!(
            steps,
            [CredentialStep::CredentialHelper, CredentialStep::Default]
        );
        // The helper is only consulted for HTTP(S) URLs.
        assert!(candidate_steps(SSH_URL, T::USER_PASS_PLAINTEXT, None).is_empty());
        assert!(is_http_url("HTTP://host/x"));
    }

    #[test]
    fn username_request_uses_url_user_or_git() {
        let mut st = AuthState::default();
        let c = st
            .credentials(SSH_URL, Some("me"), T::USERNAME, None)
            .unwrap();
        assert_eq!(c.credtype() as u32, T::USERNAME.bits());
        assert!(st.credentials(SSH_URL, None, T::USERNAME, None).is_ok());
    }

    #[test]
    fn each_strategy_is_tried_once_then_gives_up_with_readable_error() {
        let dir = ssh_dir_with(&["id_ed25519"]);
        let mut st = AuthState::default();
        let mut call = || st.credentials(SSH_URL, Some("git"), T::SSH_KEY, Some(dir.path()));
        assert_eq!(
            call().unwrap().credtype() as u32,
            T::SSH_KEY.bits(),
            "agent"
        );
        assert_eq!(
            call().unwrap().credtype() as u32,
            T::SSH_KEY.bits(),
            "key file"
        );
        let err = call().err().unwrap();
        assert_eq!(err.code(), git2::ErrorCode::Auth);
        let msg = err.message();
        assert!(msg.starts_with(&format!("Authentication failed for {SSH_URL}")));
        assert!(
            msg.contains("SSH agent") && msg.contains("id_ed25519"),
            "{msg}"
        );
        assert!(msg.contains("Passphrase-protected"), "{msg}");
        assert_eq!(st.failure.as_deref(), Some(msg));
    }

    #[test]
    fn unsupported_credential_type_fails_immediately() {
        let mut st = AuthState::default();
        let err = st
            .credentials(SSH_URL, None, T::SSH_CUSTOM, None)
            .err()
            .unwrap();
        assert!(err.message().contains("no supported credential type"));
    }

    #[test]
    fn attempts_are_capped_even_when_libgit2_keeps_asking() {
        let mut st = AuthState::default();
        for _ in 0..MAX_CREDENTIAL_ATTEMPTS {
            let _ = st.credentials(SSH_URL, None, T::USERNAME, None);
        }
        let err = st
            .credentials(SSH_URL, None, T::USERNAME, None)
            .err()
            .unwrap();
        assert!(err.message().contains("too many attempts"));
    }

    #[test]
    fn rejected_refs_are_reported() {
        assert_eq!(rejected_refs_error(&[]), None);
        assert_eq!(
            rejected_refs_error(&["refs/heads/main (non-fast-forward)".into()]).unwrap(),
            "Push rejected: refs/heads/main (non-fast-forward)"
        );
    }

    #[test]
    fn session_error_prefers_auth_failure_and_explains_non_fast_forward() {
        let s = RemoteSession::new(HTTPS_URL);
        let generic = git2::Error::from_str("boom");
        assert_eq!(s.describe_error("Fetch", &generic), "Fetch error: boom");
        let mut nff = git2::Error::from_str("cannot push");
        nff.set_code(git2::ErrorCode::NotFastForward);
        assert!(s.describe_error("Push", &nff).contains("non-fast-forward"));
        let mut auth = git2::Error::from_str("denied");
        auth.set_code(git2::ErrorCode::Auth);
        assert_eq!(
            s.describe_error("Push", &auth),
            format!("Authentication failed for {HTTPS_URL}: denied")
        );
        s.auth.borrow_mut().failure = Some("Authentication failed for x: tried y".into());
        assert_eq!(
            s.describe_error("Push", &generic),
            "Authentication failed for x: tried y"
        );
    }
}
