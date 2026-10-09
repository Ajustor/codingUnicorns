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
            std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string())
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
            std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string())
                + r"\Git\bin\bash.exe",
            std::env::var("LOCALAPPDATA").unwrap_or_default() + r"\Programs\Git\bin\bash.exe",
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
    // Relative name (e.g. "pwsh.exe", "bash.exe"): look it up in PATH here.
    // Spawning `where` / `which` took ~100 ms per shell, which froze the
    // Settings page the first time it opened.
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    find_in_dirs(std::env::split_paths(&path_var), path)
}

/// Whether `name` is a file in one of `dirs` (executable on Unix).
fn find_in_dirs(dirs: impl IntoIterator<Item = std::path::PathBuf>, name: &str) -> bool {
    dirs.into_iter().any(|dir| {
        // `symlink_metadata`: Windows app execution aliases (e.g. `pwsh.exe`
        // from the Store / winget in WindowsApps) can't be opened by
        // `metadata`, but `where` finds them and they do run.
        let Ok(meta) = std::fs::symlink_metadata(dir.join(name)) else {
            return false;
        };
        if meta.is_dir() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            // Follow symlinks for the mode (`/bin/sh` is often a link).
            let mode = std::fs::metadata(dir.join(name))
                .map(|m| m.permissions().mode())
                .unwrap_or(0);
            mode & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_in_dirs_looks_for_the_file_in_each_directory() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let shell = b.path().join("myshell.exe");
        std::fs::write(&shell, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::create_dir(a.path().join("dir.exe")).unwrap();
        let dirs = || vec![a.path().to_path_buf(), b.path().to_path_buf()];
        assert!(find_in_dirs(dirs(), "myshell.exe"));
        assert!(!find_in_dirs(dirs(), "missing.exe"));
        assert!(
            !find_in_dirs(dirs(), "dir.exe"),
            "a directory is not a shell"
        );
        assert!(!find_in_dirs(Vec::new(), "myshell.exe"));
    }

    #[test]
    #[ignore = "timing, run by hand"]
    fn list_available_shells_timing() {
        let t = std::time::Instant::now();
        let shells = list_available_shells();
        println!("{shells:?} in {:?}", t.elapsed());
    }

    #[cfg(unix)]
    #[test]
    fn find_in_dirs_needs_the_executable_bit_on_unix() {
        use std::os::unix::fs::PermissionsExt as _;
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("sh");
        std::fs::write(&f, "").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!find_in_dirs(vec![d.path().to_path_buf()], "sh"));
    }

    #[test]
    fn user_powershell_gets_standard_args() {
        for sh in ["pwsh", "C:\\Tools\\PowerShell.exe", "PWSH.EXE"] {
            let (path, args) = resolve_shell(sh);
            assert_eq!(path, sh);
            assert_eq!(args, ps_args());
        }
    }

    #[test]
    fn user_non_powershell_gets_no_args() {
        let (path, args) = resolve_shell("/usr/bin/fish");
        assert_eq!(path, "/usr/bin/fish");
        assert!(args.is_empty());
        let (path, args) = resolve_shell("cmd.exe");
        assert_eq!(path, "cmd.exe");
        assert!(args.is_empty());
    }

    #[test]
    fn ps_args_match_constant() {
        assert_eq!(
            ps_args(),
            vec!["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass"]
        );
    }

    #[test]
    fn discover_shells_has_platform_candidates() {
        let shells = discover_shells();
        assert!(!shells.is_empty());
        #[cfg(windows)]
        {
            assert!(shells[0].path.ends_with("powershell.exe"));
            assert_eq!(shells[0].args, ps_args());
            assert_eq!(shells.last().unwrap().path, "cmd.exe");
            assert!(shells
                .iter()
                .any(|s| s.path.ends_with("bash.exe") && s.args == vec!["--login"]));
            assert!(shells.iter().any(|s| s.path == "wsl.exe"));
        }
        #[cfg(not(windows))]
        {
            assert_eq!(shells.last().unwrap().path, "/bin/sh");
            assert!(shells.iter().any(|s| s.path == "/bin/bash"));
        }
    }

    #[test]
    fn shell_exists_checks_absolute_paths_on_disk() {
        let exe = std::env::current_exe().unwrap();
        assert!(shell_exists(&exe.to_string_lossy()));
        let missing = std::env::temp_dir().join("definitely-missing-shell-xyz.exe");
        assert!(!shell_exists(&missing.to_string_lossy()));
    }

    #[test]
    fn shell_exists_searches_path_for_relative_names() {
        assert!(!shell_exists("definitely-not-a-shell-xyz"));
        #[cfg(windows)]
        assert!(shell_exists("cmd.exe"));
        #[cfg(not(windows))]
        assert!(shell_exists("sh"));
    }

    #[test]
    fn auto_resolution_returns_an_existing_shell() {
        let (path, _args) = resolve_shell("");
        assert!(!path.is_empty());
        assert!(shell_exists(&path));
    }

    #[test]
    fn available_shells_are_named_by_file_name() {
        let shells = list_available_shells();
        assert!(!shells.is_empty(), "at least the fallback shell exists");
        for (name, path) in &shells {
            assert!(path.ends_with(name.as_str()), "{name} vs {path}");
            assert!(!name.contains('\\') && !name.contains('/'));
        }
    }
}
