/// Standard args for PowerShell in an embedded terminal.
const PS_ARGS: &[&str] = &["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass"];

/// A shell candidate with its path, arguments, and a human-readable name.
struct ShellCandidate {
    path: String,
    args: Vec<String>,
}

/// Returns `(shell_path, extra_args)` for the current OS.
/// When `user_shell` is non-empty, use it directly.
/// Otherwise, dynamically discovers installed shells and picks the first that exists.
pub(super) fn resolve_shell(user_shell: &str) -> (String, Vec<String>) {
    if !user_shell.is_empty() {
        let lower = user_shell.to_lowercase();
        let args = if lower.contains("pwsh") || lower.contains("powershell") {
            PS_ARGS.iter().map(|s| s.to_string()).collect()
        } else {
            vec![]
        };
        return (user_shell.to_string(), args);
    }

    let candidates = discover_shells();
    for candidate in candidates {
        if shell_exists(&candidate.path) {
            return (candidate.path, candidate.args);
        }
    }

    // Last resort
    #[cfg(windows)]
    return ("cmd.exe".to_string(), vec![]);
    #[cfg(not(windows))]
    return ("/bin/sh".to_string(), vec![]);
}

/// Returns the list of shells available on the system (for display in settings).
pub fn list_available_shells() -> Vec<(String, String)> {
    let candidates = discover_shells();
    candidates
        .into_iter()
        .filter(|c| shell_exists(&c.path))
        .map(|c| {
            let name = std::path::Path::new(&c.path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&c.path)
                .to_string();
            (name, c.path)
        })
        .collect()
}

fn ps_args() -> Vec<String> {
    PS_ARGS.iter().map(|s| s.to_string()).collect()
}

/// Build the ordered list of shell candidates for the current platform.
fn discover_shells() -> Vec<ShellCandidate> {
    let mut shells = Vec::new();

    #[cfg(windows)]
    {
        // Windows PowerShell 5.1 (built-in, always present on modern Windows)
        let ps5 = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
        shells.push(ShellCandidate {
            path: ps5.to_string(),
            args: ps_args(),
        });

        // PowerShell 7+ (pwsh) — check common install locations
        let pwsh_locations = [
            std::env::var("ProgramFiles")
                .unwrap_or_else(|_| r"C:\Program Files".to_string())
                + r"\PowerShell\7\pwsh.exe",
            // winget / scoop / chocolatey may put it on PATH
            "pwsh.exe".to_string(),
        ];
        for loc in pwsh_locations {
            shells.push(ShellCandidate {
                path: loc,
                args: ps_args(),
            });
        }

        // Git Bash
        let git_bash_locations = [
            std::env::var("ProgramFiles")
                .unwrap_or_else(|_| r"C:\Program Files".to_string())
                + r"\Git\bin\bash.exe",
            std::env::var("LOCALAPPDATA")
                .unwrap_or_default()
                + r"\Programs\Git\bin\bash.exe",
            "bash.exe".to_string(),
        ];
        for loc in git_bash_locations {
            shells.push(ShellCandidate {
                path: loc,
                args: vec!["--login".to_string()],
            });
        }

        // WSL default shell
        shells.push(ShellCandidate {
            path: "wsl.exe".to_string(),
            args: vec![],
        });

        // cmd.exe (always available, last resort)
        shells.push(ShellCandidate {
            path: "cmd.exe".to_string(),
            args: vec![],
        });
    }

    #[cfg(not(windows))]
    {
        // Prefer $SHELL env var (user's configured login shell)
        if let Ok(shell) = std::env::var("SHELL") {
            if !shell.is_empty() {
                shells.push(ShellCandidate {
                    path: shell,
                    args: vec!["-l".to_string()],
                });
            }
        }

        // Common Unix shells in preference order
        for (path, args) in [
            ("/bin/zsh", vec!["-l"]),
            ("/bin/bash", vec!["-l"]),
            ("/bin/fish", vec!["-l"]),
            ("/bin/sh", vec![]),
        ] {
            shells.push(ShellCandidate {
                path: path.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
            });
        }
    }

    shells
}

/// Check whether a shell binary exists and is executable.
fn shell_exists(path: &str) -> bool {
    let p = std::path::Path::new(path);
    // Absolute path: check filesystem directly
    if p.is_absolute() {
        return p.exists();
    }
    // Relative name (e.g. "pwsh.exe", "bash.exe"): check PATH
    #[cfg(windows)]
    {
        std::process::Command::new("where")
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("which")
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}
