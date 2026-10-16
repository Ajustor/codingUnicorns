//! The token GitHub requests are made with: the one set in Settings, else
//! the GitHub CLI's (`gh auth token`), else none (public repositories only).

use crate::process_ext::CommandExt;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A stuck `gh` must not hold a request back for long.
const GH_TIMEOUT: Duration = Duration::from_secs(5);

/// The GitHub CLI's token, once found (`None`: not found yet).
static CLI_TOKEN: Mutex<Option<String>> = Mutex::new(None);

/// The token for GitHub requests. While Settings has none and the CLI's token
/// isn't known yet, this runs `gh`: call it off the UI thread.
pub fn resolve(config_token: &str) -> Option<String> {
    resolve_with(config_token, || {
        // The lock is held while `gh` runs: a thread waiting on it reuses the
        // token found (after a failure, it runs `gh` again).
        let mut cached = CLI_TOKEN.lock().unwrap_or_else(|e| e.into_inner());
        if cached.is_none() {
            *cached = gh_auth_token();
        }
        cached.clone()
    })
}

/// [`resolve`] with the GitHub CLI lookup injected.
fn resolve_with(config_token: &str, gh: impl FnOnce() -> Option<String>) -> Option<String> {
    let configured = config_token.trim();
    if !configured.is_empty() {
        return Some(configured.to_string());
    }
    gh().filter(|t| !t.is_empty())
}

/// Forget the CLI's token (GitHub refused it, or it can't see the
/// repository): the next request asks `gh` again. Waits for a `gh` call in progress, so call it off the UI thread.
pub fn forget() {
    *CLI_TOKEN.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// `gh auth token` for github.com, when the GitHub CLI is installed and
/// logged in. The host is explicit: `GH_HOST` or an Enterprise default host
/// must not send its token to api.github.com.
fn gh_auth_token() -> Option<String> {
    let mut command = Command::new("gh");
    command.args(["auth", "token", "--hostname", "github.com"]);
    token_from(command, GH_TIMEOUT)
}

/// The trimmed stdout of `command` when it succeeds within `timeout`.
fn token_from(mut command: Command, timeout: Duration) -> Option<String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .no_window()
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    // The timeout only covers the wait above. Reading afterwards is fine: the
    // token is tiny (it fits the pipe, so the child never blocks on it) and
    // `gh` leaves no child process holding the pipe open.
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let token = out.trim();
    (!token.is_empty()).then(|| token.to_string())
}

#[cfg(test)]
mod tests {
    use super::{resolve_with, token_from};
    use std::process::Command;
    use std::time::{Duration, Instant};

    fn shell(script: &str) -> Command {
        if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", script]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", script]);
            c
        }
    }

    #[test]
    fn the_settings_token_comes_first() {
        assert_eq!(
            resolve_with(" ghp_settings ", || panic!("gh must not run")),
            Some("ghp_settings".to_string())
        );
    }

    #[test]
    fn falls_back_to_the_cli_then_to_none() {
        assert_eq!(
            resolve_with("", || Some("gho_cli".into())),
            Some("gho_cli".into())
        );
        assert_eq!(resolve_with("  ", || None), None);
        assert_eq!(resolve_with("", || Some(String::new())), None);
    }

    #[test]
    fn the_output_is_trimmed() {
        let out = token_from(shell("echo  tok "), Duration::from_secs(5));
        assert_eq!(out, Some("tok".to_string()));
    }

    #[test]
    fn an_empty_output_is_no_token() {
        let script = if cfg!(windows) { "rem" } else { "true" };
        assert_eq!(token_from(shell(script), Duration::from_secs(5)), None);
    }

    #[test]
    fn a_failing_command_is_no_token() {
        let script = if cfg!(windows) {
            "echo tok & exit 1"
        } else {
            "echo tok; exit 1"
        };
        assert_eq!(token_from(shell(script), Duration::from_secs(5)), None);
    }

    #[test]
    fn a_stuck_command_is_given_up_on() {
        let c = if cfg!(windows) {
            let mut c = Command::new("ping");
            c.args(["-n", "30", "127.0.0.1"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("30");
            c
        };
        let start = Instant::now();
        assert_eq!(token_from(c, Duration::from_millis(200)), None);
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
