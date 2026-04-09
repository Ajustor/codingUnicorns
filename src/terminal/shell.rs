/// Returns `(shell_path, extra_args)` for the current OS.
pub(super) fn resolve_shell() -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        // Prefer pwsh (PowerShell 7+), then Windows PowerShell 5.1, then cmd.exe
        if which_exists("pwsh") {
            return (
                "pwsh.exe".to_string(),
                vec!["-NoLogo".to_string()],
            );
        }
        let ps5 =
            std::path::Path::new("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe");
        if ps5.exists() {
            return (
                ps5.to_string_lossy().to_string(),
                vec!["-NoLogo".to_string()],
            );
        }
        (
            "cmd.exe".to_string(),
            vec![],
        )
    }

    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                if std::path::Path::new("/bin/bash").exists() {
                    Some("/bin/bash".to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "/bin/sh".to_string());
        let args = vec!["-l".to_string()];
        (shell, args)
    }
}

/// Check if a program exists on PATH (Windows).
#[cfg(windows)]
fn which_exists(name: &str) -> bool {
    std::process::Command::new("where")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
