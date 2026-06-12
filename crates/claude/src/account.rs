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
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;

    if v.get("loggedIn").and_then(|b| b.as_bool()) != Some(true) {
        return Some("not logged in".to_string());
    }
    let email = v.get("email").and_then(|e| e.as_str()).unwrap_or("signed in");
    match v.get("subscriptionType").and_then(|s| s.as_str()) {
        Some(sub) => Some(format!("{email} · {sub}")),
        None => Some(email.to_string()),
    }
}
