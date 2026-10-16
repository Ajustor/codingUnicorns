//! Network operations against `origin`: authentication callbacks shared by
//! fetch/push, plus the `Fetch` action of the git panel.
//!
//! Credentials are tried in this order, depending on what the remote accepts:
//! SSH agent, then the private keys found in `~/.ssh` (`id_ed25519`,
//! `id_ecdsa`, `id_rsa` first, then any other private key file by name;
//! unencrypted keys only: passphrase-protected keys must be loaded into the
//! agent), the git credential helper for HTTP(S) remotes, then the platform
//! default (NTLM/Negotiate). libgit2 calls the credentials callback again
//! after every failed attempt, so each strategy is tried once and the callback
//! then gives up with a readable error instead of looping forever.
//!
//! libgit2's built-in SSH client doesn't support every key type on every
//! platform (e.g. ECDSA keys with the Windows crypto backend). When an SSH
//! remote fails authentication that way, the operation is retried once with
//! the `git` command line (OpenSSH, so the user's agent and `ssh_config`
//! apply), without ever prompting. If that fails too, both errors are shown.

use super::GitStatus;
use crate::process_ext::CommandExt;
use std::cell::RefCell;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Hard cap on credential callback invocations for one operation: the
/// username request, the agent, a generous number of keys, the helper and the
/// default, with some margin.
const MAX_CREDENTIAL_ATTEMPTS: usize = 32;

/// Key files looked up first in `~/.ssh`, in order of preference.
const DEFAULT_SSH_KEYS: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

/// A `git` command line that doesn't answer within this is killed.
const CLI_TIMEOUT: Duration = Duration::from_secs(120);

/// First line of `path` (at most a few hundred bytes).
fn first_line(path: &Path) -> Option<String> {
    let mut buf = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(512)
        .read_to_end(&mut buf)
        .ok()?;
    let text = String::from_utf8_lossy(&buf);
    Some(text.lines().next().unwrap_or("").trim().to_string())
}

/// Private keys in `dir`: the default names first (in their usual order),
/// then every other regular file whose first line mentions `PRIVATE KEY`,
/// sorted by name. Public keys and `known_hosts`/`config`/`authorized_keys`
/// are never read.
pub(crate) fn scan_ssh_keys(dir: &Path) -> Vec<PathBuf> {
    let is_key = |p: &Path| first_line(p).is_some_and(|l| l.contains("PRIVATE KEY"));
    let mut keys: Vec<PathBuf> = DEFAULT_SSH_KEYS
        .iter()
        .map(|name| dir.join(name))
        .filter(|p| p.is_file() && is_key(p))
        .collect();
    let mut others: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            !DEFAULT_SSH_KEYS.contains(&name.as_str())
                && !name.ends_with(".pub")
                && !name.starts_with("known_hosts")
                && name != "config"
                && name != "authorized_keys"
        })
        .map(|e| e.path())
        .filter(|p| is_key(p))
        .collect();
    others.sort_by_key(|p| p.file_name().map(|n| n.to_os_string()));
    keys.extend(others);
    keys
}

/// Decode the beginning of standard base64 text (stops at the first byte
/// that isn't part of the alphabet).
fn decode_base64_prefix(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => break,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

/// Whether the private key file needs a passphrase. OpenSSH format: the
/// cipher name after the `openssh-key-v1\0` magic isn't `none`. Legacy PEM:
/// a `Proc-Type: 4,ENCRYPTED` header. Unreadable files count as unencrypted.
pub(crate) fn key_is_encrypted(path: &Path) -> bool {
    let mut buf = Vec::new();
    if std::fs::File::open(path)
        .and_then(|f| f.take(8192).read_to_end(&mut buf))
        .is_err()
    {
        return false;
    }
    let text = String::from_utf8_lossy(&buf);
    if text.contains("BEGIN OPENSSH PRIVATE KEY") {
        let body: String = text
            .lines()
            .skip(1)
            .take_while(|l| !l.starts_with("-----"))
            .flat_map(|l| l.trim().chars())
            .take(128)
            .collect();
        let bytes = decode_base64_prefix(&body);
        const MAGIC: &[u8] = b"openssh-key-v1\0";
        let Some(rest) = bytes.strip_prefix(MAGIC) else {
            return false;
        };
        let Some(len) = rest.get(..4) else {
            return false;
        };
        let len = u32::from_be_bytes([len[0], len[1], len[2], len[3]]) as usize;
        // A cipher name longer than what was decoded is not "none".
        return rest.get(4..4 + len).is_none_or(|name| name != b"none");
    }
    text.lines().any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("proc-type:") && l.contains("encrypted")
    })
}

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
            steps.extend(scan_ssh_keys(dir).into_iter().map(CredentialStep::SshKey));
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
        let keys: Vec<&PathBuf> = self
            .tried
            .iter()
            .filter_map(|s| match s {
                CredentialStep::SshKey(p) => Some(p),
                _ => None,
            })
            .collect();
        if keys.iter().any(|p| key_is_encrypted(p)) {
            msg.push_str(". Passphrase-protected keys must be added to your SSH agent");
        } else if !keys.is_empty() {
            msg.push_str(". The key type may not be supported by the built-in SSH client");
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

/// Whether `url` is an SSH remote: `ssh://`, `git+ssh://` or scp-like
/// (`user@host:path`). Not HTTP(S), `file://` nor a local path (including a
/// Windows `C:\` path).
fn is_ssh_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("ssh://") || lower.starts_with("git+ssh://") {
        return true;
    }
    if lower.contains("://") {
        return false;
    }
    match url.find(':') {
        // A single letter before ':' is a drive, not a host.
        Some(i) if i > 1 => !url[..i].contains(['/', '\\']),
        _ => false,
    }
}

/// Combine libgit2's failure with the `git` CLI's.
pub(crate) fn combine_errors(libgit2: &str, cli: &str) -> String {
    format!("{libgit2}; git CLI: {cli}")
}

/// The line of git's `stderr` that best says what went wrong.
fn first_meaningful_line(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("hint:"))
        .collect();
    let best = lines
        .iter()
        .find(|l| {
            l.starts_with("fatal:")
                || l.starts_with("error:")
                || l.starts_with('!')
                || l.contains("Permission denied")
        })
        .or_else(|| lines.first());
    match best {
        Some(l) => l.strip_prefix("fatal: ").unwrap_or(l).to_string(),
        None => "git exited with an error".to_string(),
    }
}

/// Whether the user configured how ssh is launched (then it is left alone).
fn ssh_command_configured(repo_dir: &Path) -> bool {
    let env = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    if env("GIT_SSH_COMMAND") || env("GIT_SSH") {
        return true;
    }
    let config = git2::Repository::open(repo_dir)
        .and_then(|r| r.config())
        .or_else(|_| git2::Config::open_default());
    config.is_ok_and(|c| c.get_string("core.sshCommand").is_ok())
}

/// Run `git -C repo_dir <args>` without ever prompting, killing it after
/// `timeout`. `Err` carries the first meaningful stderr line.
fn run_git(repo_dir: &Path, args: &[String], timeout: Duration) -> Result<(), String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo_dir);
    if !ssh_command_configured(repo_dir) {
        cmd.args(["-c", "core.sshCommand=ssh -o BatchMode=yes"]);
    }
    let mut child = cmd
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .no_window()
        .spawn()
        .map_err(|e| format!("cannot run git: {e}"))?;
    // Drain stderr on a thread so a chatty child never blocks on a full pipe.
    let mut stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        if let Some(s) = stderr.as_mut() {
            let mut bytes = Vec::new();
            let _ = s.read_to_end(&mut bytes);
            out = String::from_utf8_lossy(&bytes).to_string();
        }
        out
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let stderr = reader.join().unwrap_or_default();
    match status {
        Some(s) if s.success() => Ok(()),
        Some(_) => Err(first_meaningful_line(&stderr)),
        None => Err(format!("timed out after {}s", timeout.as_secs())),
    }
}

/// Fetch with the `git` CLI (`origin`, `refspecs`; empty: the configured ones).
pub(crate) fn cli_fetch(
    repo_dir: &Path,
    refspecs: &[&str],
    update_fetchhead: bool,
    timeout: Duration,
) -> Result<(), String> {
    let mut args = vec!["fetch".to_string()];
    if !update_fetchhead {
        args.push("--no-write-fetch-head".to_string());
    }
    args.push("origin".to_string());
    args.extend(refspecs.iter().map(|s| s.to_string()));
    run_git(repo_dir, &args, timeout)
}

/// Push with the `git` CLI (`origin`, `refspecs`).
pub(crate) fn cli_push(
    repo_dir: &Path,
    refspecs: &[&str],
    timeout: Duration,
) -> Result<(), String> {
    let mut args = vec!["push".to_string(), "origin".to_string()];
    args.extend(refspecs.iter().map(|s| s.to_string()));
    run_git(repo_dir, &args, timeout)
}

/// The repository's directory for `git -C` (its working tree, else its git dir).
fn repo_dir(repo: &git2::Repository) -> &Path {
    repo.workdir().unwrap_or_else(|| repo.path())
}

/// Whether libgit2's failure is one the `git` CLI may fix: SSH remote and an
/// authentication failure.
fn should_fall_back(url: &str, session: &RemoteSession, e: &git2::Error) -> bool {
    is_ssh_url(url)
        && (session.auth.borrow().failure.is_some() || e.code() == git2::ErrorCode::Auth)
}

/// Fetch `refspecs` (empty: the remote's configured ones) from `origin`.
pub(crate) fn fetch_remote(repo: &git2::Repository, refspecs: &[&str]) -> Result<(), String> {
    fetch_remote_with(repo, refspecs, true)
}

/// [`fetch_remote`], choosing whether `FETCH_HEAD` is rewritten. Background
/// fetches the user didn't ask for leave it alone: it is theirs.
pub(crate) fn fetch_remote_with(
    repo: &git2::Repository,
    refspecs: &[&str],
    update_fetchhead: bool,
) -> Result<(), String> {
    let mut remote = repo
        .find_remote("origin")
        .map_err(|e| format!("Remote error: {e}"))?;
    let url = remote.url().unwrap_or("origin").to_string();
    let session = RemoteSession::new(&url);
    let mut opts = git2::FetchOptions::new();
    opts.update_fetchhead(update_fetchhead);
    opts.remote_callbacks(session.callbacks());
    opts.proxy_options(proxy_options());
    match remote.fetch(refspecs, Some(&mut opts), None) {
        Ok(()) => Ok(()),
        Err(e) => {
            let msg = session.describe_error("Fetch", &e);
            if !should_fall_back(&url, &session, &e) {
                return Err(msg);
            }
            cli_fetch(repo_dir(repo), refspecs, update_fetchhead, CLI_TIMEOUT)
                .map_err(|cli| combine_errors(&msg, &cli))
        }
    }
}

/// Push `refspecs` to `origin`; refs refused by the remote are errors.
pub(crate) fn push_remote(repo: &git2::Repository, refspecs: &[&str]) -> Result<(), String> {
    let mut remote = repo
        .find_remote("origin")
        .map_err(|e| format!("Remote error: {e}"))?;
    let url = remote.url().unwrap_or("origin").to_string();
    let session = RemoteSession::new(&url);
    let mut opts = git2::PushOptions::new();
    opts.remote_callbacks(session.callbacks());
    opts.proxy_options(proxy_options());
    match remote.push(refspecs, Some(&mut opts)) {
        Ok(()) => match session.rejected_error() {
            Some(err) => Err(err),
            None => Ok(()),
        },
        Err(e) => {
            let msg = session.describe_error("Push", &e);
            if !should_fall_back(&url, &session, &e) {
                return Err(msg);
            }
            cli_push(repo_dir(repo), refspecs, CLI_TIMEOUT)
                .map_err(|cli| combine_errors(&msg, &cli))
        }
    }
}

impl GitStatus {
    /// Fetch every configured refspec of `origin`, then refresh (ahead/behind).
    pub fn fetch(&mut self) -> Result<(), String> {
        let repo = self.open_repo()?;
        fetch_remote(&repo, &[])?;
        self.refresh();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::{bare_origin, TestRepo};
    use git2::CredentialType as T;

    const SSH_URL: &str = "git@example.com:me/repo.git";
    const HTTPS_URL: &str = "https://example.com/me/repo.git";

    const KEY_BODY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n";

    fn ssh_dir_with(keys: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for k in keys {
            std::fs::write(dir.path().join(k), KEY_BODY).unwrap();
        }
        dir
    }

    fn b64(bytes: &[u8]) -> String {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
            for i in 0..=chunk.len() {
                out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
            for _ in chunk.len()..3 {
                out.push('=');
            }
        }
        out
    }

    /// An OpenSSH-format key file whose header names `cipher` (rest is fake).
    fn openssh_key(cipher: &str) -> String {
        let mut bytes = b"openssh-key-v1\0".to_vec();
        bytes.extend((cipher.len() as u32).to_be_bytes());
        bytes.extend(cipher.as_bytes());
        bytes.extend([0u8; 40]);
        format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n",
            b64(&bytes)
        )
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
    fn key_scan_finds_custom_keys_after_defaults_and_skips_other_files() {
        let dir = ssh_dir_with(&["id_rsa", "id_ecdsa", "work_key", "a_key"]);
        let p = dir.path();
        std::fs::write(p.join("id_rsa.pub"), "ssh-rsa AAAA me").unwrap();
        std::fs::write(p.join("known_hosts"), "github.com ssh-rsa AAAA").unwrap();
        std::fs::write(p.join("known_hosts.old"), KEY_BODY).unwrap();
        std::fs::write(p.join("config"), KEY_BODY).unwrap();
        std::fs::write(p.join("authorized_keys"), KEY_BODY).unwrap();
        std::fs::write(p.join("notes.txt"), "just some text").unwrap();
        std::fs::write(p.join("legacy"), "-----BEGIN RSA PRIVATE KEY-----\nxx\n").unwrap();
        std::fs::create_dir(p.join("a_dir")).unwrap();
        assert_eq!(
            scan_ssh_keys(p),
            [
                p.join("id_ecdsa"),
                p.join("id_rsa"),
                p.join("a_key"),
                p.join("legacy"),
                p.join("work_key"),
            ]
        );
        let steps = candidate_steps(SSH_URL, T::SSH_KEY, Some(p));
        assert_eq!(steps.len(), 6, "agent + 5 keys");
        assert_eq!(steps[0], CredentialStep::SshAgent);
        assert_eq!(steps[3], CredentialStep::SshKey(p.join("a_key")));
        assert!(scan_ssh_keys(&p.join("missing")).is_empty());
    }

    #[test]
    fn key_encryption_is_detected_from_the_file_header() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, body: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, body).unwrap();
            path
        };
        assert!(!key_is_encrypted(&write("plain", &openssh_key("none"))));
        assert!(key_is_encrypted(&write("aes", &openssh_key("aes256-ctr"))));
        assert!(key_is_encrypted(&write(
            "gcm",
            &openssh_key("aes256-gcm@openssh.com")
        )));
        assert!(key_is_encrypted(&write(
            "pem",
            "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-128-CBC,AB\n\nxx\n"
        )));
        assert!(!key_is_encrypted(&write(
            "pem2",
            "-----BEGIN RSA PRIVATE KEY-----\nMIIE\n"
        )));
        assert!(!key_is_encrypted(&dir.path().join("missing")));
    }

    #[test]
    fn give_up_hint_depends_on_whether_a_tried_key_is_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("id_ecdsa"), openssh_key("none")).unwrap();
        let mut st = AuthState::default();
        let _ = st.credentials(SSH_URL, Some("git"), T::SSH_KEY, Some(dir.path()));
        let _ = st.credentials(SSH_URL, Some("git"), T::SSH_KEY, Some(dir.path()));
        let msg = st
            .credentials(SSH_URL, Some("git"), T::SSH_KEY, Some(dir.path()))
            .err()
            .unwrap()
            .message()
            .to_string();
        assert!(!msg.contains("Passphrase"), "{msg}");
        assert!(msg.contains("may not be supported"), "{msg}");

        std::fs::write(dir.path().join("id_ecdsa"), openssh_key("aes256-ctr")).unwrap();
        let mut st = AuthState::default();
        let _ = st.credentials(SSH_URL, Some("git"), T::SSH_KEY, Some(dir.path()));
        let _ = st.credentials(SSH_URL, Some("git"), T::SSH_KEY, Some(dir.path()));
        let msg = st
            .credentials(SSH_URL, Some("git"), T::SSH_KEY, Some(dir.path()))
            .err()
            .unwrap()
            .message()
            .to_string();
        assert!(msg.contains("Passphrase-protected"), "{msg}");
    }

    #[test]
    fn ssh_urls_are_told_apart_from_http_and_local_paths() {
        for url in [
            "git@github.com:me/repo.git",
            "ssh://git@host/me/repo.git",
            "host:repo.git",
        ] {
            assert!(is_ssh_url(url), "{url}");
        }
        for url in [
            "https://github.com/me/repo.git",
            "file:///tmp/repo",
            "C:\\repos\\x",
            "C:/repos/x",
            "/tmp/repo",
            "../repo",
        ] {
            assert!(!is_ssh_url(url), "{url}");
        }
    }

    #[test]
    fn errors_are_combined_and_stderr_summarised() {
        assert_eq!(
            combine_errors("Authentication failed", "Permission denied"),
            "Authentication failed; git CLI: Permission denied"
        );
        let stderr = "Warning: Permanently added 'x' to known hosts.\nhint: nope\nfatal: Could not read from remote repository.\n";
        assert_eq!(
            first_meaningful_line(stderr),
            "Could not read from remote repository."
        );
        assert_eq!(
            first_meaningful_line("git@x: Permission denied (publickey)."),
            "git@x: Permission denied (publickey)."
        );
        assert_eq!(first_meaningful_line(""), "git exited with an error");
    }

    const T10: Duration = Duration::from_secs(60);

    fn fetch_head(r: &TestRepo) -> Option<String> {
        std::fs::read_to_string(r.path().join(".git").join("FETCH_HEAD")).ok()
    }

    /// `r` (with one commit pushed to a fresh bare origin) and a clone of that
    /// origin that has since pushed a newer commit: returns (r, tip, keep-alive).
    fn origin_with_newer_commit() -> (TestRepo, git2::Oid, Vec<Box<dyn std::any::Any>>) {
        let r = TestRepo::new();
        r.write("a.txt", "1");
        r.commit_all("init");
        let (bare, url) = bare_origin();
        r.repo.remote("origin", &url).unwrap();
        r.status().push().unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let other = git2::Repository::clone(&url, other_dir.path()).unwrap();
        std::fs::write(other_dir.path().join("b.txt"), "2").unwrap();
        let mut idx = other.index().unwrap();
        idx.add_path(Path::new("b.txt")).unwrap();
        idx.write().unwrap();
        let tree = other.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = crate::git::tests::sig("Other");
        let parent = other.head().unwrap().peel_to_commit().unwrap();
        let tip = other
            .commit(Some("HEAD"), &sig, &sig, "remote", &tree, &[&parent])
            .unwrap();
        push_remote(&other, &["refs/heads/main:refs/heads/main"]).unwrap();
        (r, tip, vec![Box::new(bare), Box::new(other_dir)])
    }

    #[test]
    fn cli_fetch_brings_refs_and_can_leave_fetch_head_alone() {
        let (r, tip, _keep) = origin_with_newer_commit();
        let spec = "+refs/heads/main:refs/remotes/origin/main";
        let before = fetch_head(&r);
        cli_fetch(r.path(), &[spec], false, T10).unwrap();
        let tracking = r.repo.find_reference("refs/remotes/origin/main").unwrap();
        assert_eq!(tracking.target(), Some(tip));
        assert_eq!(fetch_head(&r), before, "FETCH_HEAD untouched");

        cli_fetch(r.path(), &[spec], true, T10).unwrap();
        assert!(fetch_head(&r).is_some_and(|f| f.contains(&tip.to_string())));
    }

    #[test]
    fn cli_fetch_reports_git_message_for_a_bad_remote() {
        let r = TestRepo::new();
        let missing = r.path().join("no-such-remote-dir");
        r.repo.remote("origin", &missing.to_string_lossy()).unwrap();
        let err = cli_fetch(r.path(), &[], true, T10).unwrap_err();
        assert!(err.contains("no-such-remote-dir"), "{err}");
        assert!(!err.starts_with("fatal:"), "{err}");
    }

    #[test]
    fn cli_push_pushes_a_branch() {
        let r = TestRepo::new();
        r.write("a.txt", "1");
        r.commit_all("init");
        let (bare, url) = bare_origin();
        r.repo.remote("origin", &url).unwrap();
        cli_push(r.path(), &["refs/heads/main:refs/heads/main"], T10).unwrap();
        let origin = git2::Repository::open(bare.path()).unwrap();
        let pushed = origin.find_reference("refs/heads/main").unwrap().target();
        assert_eq!(pushed, r.repo.head().unwrap().target());
        // A refused ref is an error carrying git's message.
        let err = cli_push(r.path(), &["refs/heads/nope:refs/heads/nope"], T10).unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn cli_commands_are_killed_after_the_timeout() {
        let r = TestRepo::new();
        let err = run_git(
            r.path(),
            &["-c".into(), "alias.slow=!sleep 30".into(), "slow".into()],
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(err.contains("timed out"), "{err}");
    }

    /// Manual check against the real repository's SSH origin:
    /// `cargo test real_ssh -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs network and an SSH key"]
    fn real_ssh_fetch_goes_through_the_cli_fallback() {
        let repo = git2::Repository::discover(env!("CARGO_MANIFEST_DIR")).unwrap();
        let url = repo
            .find_remote("origin")
            .unwrap()
            .url()
            .unwrap()
            .to_string();
        assert!(is_ssh_url(&url), "{url}");
        let spec = "+refs/heads/master:refs/remotes/origin/master";
        let res = fetch_remote_with(&repo, &[spec], false);
        println!("fetch_remote_with: {res:?}");
        res.unwrap();
    }

    #[test]
    fn non_ssh_auth_failures_do_not_fall_back() {
        let s = RemoteSession::new(HTTPS_URL);
        let mut auth = git2::Error::from_str("denied");
        auth.set_code(git2::ErrorCode::Auth);
        assert!(!should_fall_back(HTTPS_URL, &s, &auth));
        assert!(should_fall_back(SSH_URL, &s, &auth));
        let generic = git2::Error::from_str("boom");
        assert!(!should_fall_back(SSH_URL, &s, &generic));
        s.auth.borrow_mut().failure = Some("x".into());
        assert!(should_fall_back(SSH_URL, &s, &generic));
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

    /// libgit2's credential type is `c_int` on Windows and `c_uint` elsewhere, so
    /// the cast is needed on one platform and a no-op on the other.
    #[allow(clippy::unnecessary_cast)]
    fn credtype(c: &git2::Cred) -> u32 {
        c.credtype() as u32
    }

    #[test]
    fn username_request_uses_url_user_or_git() {
        let mut st = AuthState::default();
        let c = st
            .credentials(SSH_URL, Some("me"), T::USERNAME, None)
            .unwrap();
        assert_eq!(credtype(&c), T::USERNAME.bits());
        assert!(st.credentials(SSH_URL, None, T::USERNAME, None).is_ok());
    }

    #[test]
    fn each_strategy_is_tried_once_then_gives_up_with_readable_error() {
        let dir = ssh_dir_with(&["id_ed25519"]);
        let mut st = AuthState::default();
        let mut call = || st.credentials(SSH_URL, Some("git"), T::SSH_KEY, Some(dir.path()));
        assert_eq!(credtype(&call().unwrap()), T::SSH_KEY.bits(), "agent");
        assert_eq!(credtype(&call().unwrap()), T::SSH_KEY.bits(), "key file");
        let err = call().err().unwrap();
        assert_eq!(err.code(), git2::ErrorCode::Auth);
        let msg = err.message();
        assert!(msg.starts_with(&format!("Authentication failed for {SSH_URL}")));
        assert!(
            msg.contains("SSH agent") && msg.contains("id_ed25519"),
            "{msg}"
        );
        assert!(msg.contains("may not be supported"), "{msg}");
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

    #[test]
    fn fetch_updates_tracking_refs_and_behind_count() {
        let r = TestRepo::new();
        r.write("a.txt", "1");
        r.commit_all("init");
        let (_bare, url) = bare_origin();
        r.repo.remote("origin", &url).unwrap();
        let mut s = r.status();
        s.push().unwrap();

        // Someone else pushes two commits.
        let other_dir = tempfile::tempdir().unwrap();
        let other = git2::Repository::clone(&url, other_dir.path()).unwrap();
        for i in 0..2 {
            std::fs::write(other_dir.path().join("b.txt"), i.to_string()).unwrap();
            let mut idx = other.index().unwrap();
            idx.add_path(Path::new("b.txt")).unwrap();
            idx.write().unwrap();
            let tree = other.find_tree(idx.write_tree().unwrap()).unwrap();
            let sig = crate::git::tests::sig("Other");
            let parent = other.head().unwrap().peel_to_commit().unwrap();
            other
                .commit(Some("HEAD"), &sig, &sig, "remote", &tree, &[&parent])
                .unwrap();
        }
        let tip = other.head().unwrap().target().unwrap();
        push_remote(&other, &["refs/heads/main:refs/heads/main"]).unwrap();

        assert_eq!((s.ahead, s.behind), (0, 0), "not fetched yet");
        s.fetch().unwrap();
        assert_eq!((s.ahead, s.behind), (0, 2));
        let tracking = r.repo.find_reference("refs/remotes/origin/main").unwrap();
        assert_eq!(tracking.target(), Some(tip));
    }

    #[test]
    fn fetch_errors() {
        let mut s = GitStatus::new();
        assert!(s.fetch().is_err());
        let r = TestRepo::new();
        let mut s = r.status();
        assert!(s.fetch().unwrap_err().starts_with("Remote error"));
        let missing = r.path().join("no-such-remote-dir");
        r.repo.remote("origin", &missing.to_string_lossy()).unwrap();
        assert!(s.fetch().unwrap_err().starts_with("Fetch error"));
    }
}
