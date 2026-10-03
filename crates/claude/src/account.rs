//! Resolve which Claude account the spawned `claude` CLI is authenticated as,
//! via `claude auth status --json`. Used only to display an indicator in the
//! panel header — the editor never handles credentials itself.

/// Run `<binary> auth status --json` and return a short display string such as
/// `"user@example.com · team"`, `"not logged in"`, or `None` if the CLI could
/// not be run / parsed. Blocking — call from a background thread.
pub fn fetch_account(binary: &str) -> Option<String> {
    let output = std::process::Command::new(binary)
        .args(["auth", "status", "--json"])
        .output()
        .ok()?;
    parse_auth_status(&output.stdout)
}

/// Turn the JSON printed by `claude auth status --json` into the display string
/// described on [`fetch_account`]. `None` if the output is not valid JSON.
fn parse_auth_status(stdout: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(stdout).ok()?;

    if v.get("loggedIn").and_then(|b| b.as_bool()) != Some(true) {
        return Some("not logged in".to_string());
    }
    let email = v
        .get("email")
        .and_then(|e| e.as_str())
        .unwrap_or("signed in");
    match v.get("subscriptionType").and_then(|s| s.as_str()) {
        Some(sub) => Some(format!("{email} · {sub}")),
        None => Some(email.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logged_in_with_subscription() {
        let out = br#"{"loggedIn":true,"email":"a@b.c","subscriptionType":"max"}"#;
        assert_eq!(parse_auth_status(out).as_deref(), Some("a@b.c · max"));
    }

    #[test]
    fn logged_in_without_subscription() {
        let out = br#"{"loggedIn":true,"email":"a@b.c"}"#;
        assert_eq!(parse_auth_status(out).as_deref(), Some("a@b.c"));
    }

    #[test]
    fn logged_in_without_email_uses_placeholder() {
        assert_eq!(
            parse_auth_status(br#"{"loggedIn":true}"#).as_deref(),
            Some("signed in")
        );
        assert_eq!(
            parse_auth_status(br#"{"loggedIn":true,"subscriptionType":"pro"}"#).as_deref(),
            Some("signed in · pro")
        );
    }

    #[test]
    fn not_logged_in_variants() {
        for out in [
            &br#"{"loggedIn":false,"email":"a@b.c"}"#[..],
            br#"{}"#,
            br#"{"loggedIn":"true"}"#,
            br#"[]"#,
        ] {
            assert_eq!(parse_auth_status(out).as_deref(), Some("not logged in"));
        }
    }

    #[test]
    fn invalid_output_is_none() {
        assert_eq!(parse_auth_status(b""), None);
        assert_eq!(parse_auth_status(b"Error: not found"), None);
    }

    #[test]
    fn missing_binary_is_none() {
        assert_eq!(
            fetch_account("cu-definitely-not-an-installed-binary-xyz"),
            None
        );
    }
}
