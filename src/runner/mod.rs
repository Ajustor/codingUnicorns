use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod vscode_launch;
pub mod vscode_tasks;

/// A single run configuration (like VSCode's launch.json entry)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunConfig {
    pub name: String,
    pub command: String,
    pub cwd: String,
    #[serde(default)]
    pub env: Vec<(String, String)>,
    #[serde(default)]
    pub args: Vec<String>,
    /// Debug-adapter settings, set for configurations imported from
    /// `.vscode/launch.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<DebugLaunch>,
}

/// How to start a DAP session for a run configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DebugLaunch {
    /// VS Code debug `type` (e.g. "debugpy", "lldb"); picks the adapter.
    #[serde(rename = "type")]
    pub adapter_type: String,
    /// "launch" or "attach".
    pub request: String,
    /// Arguments of the DAP `launch`/`attach` request: the original
    /// launch.json entry, unknown keys included. `${workspaceFolder}` and
    /// `${file}` are substituted by the DAP client at session start.
    pub launch_args: serde_json::Value,
}

impl RunConfig {
    /// Resolve variables in command/args/cwd (see [`variable_value`] for the
    /// list) into a command line for `shell`: substituted paths and arguments
    /// are quoted when they need it, and `env` is set for the command.
    pub fn resolve(
        &self,
        workspace: Option<&Path>,
        current_file: Option<&Path>,
        shell: ShellKind,
    ) -> ResolvedRun {
        let mut full_cmd = expand_with(&self.command, workspace, current_file, |v, quoted| {
            // An empty value (no file open) stays empty rather than becoming `''`.
            if quoted || v.is_empty() {
                v.to_string()
            } else {
                shell.quote(v)
            }
        });
        for arg in &self.args {
            full_cmd.push(' ');
            full_cmd.push_str(&shell.quote(&expand_variables(arg, workspace, current_file)));
        }
        // PowerShell runs a quoted program path only through the call operator.
        if shell == ShellKind::PowerShell && full_cmd.starts_with(['\'', '"']) {
            full_cmd.insert_str(0, "& ");
        }
        let command = shell.with_env(&self.env, full_cmd);

        ResolvedRun {
            command,
            cwd: expand_variables(&self.cwd, workspace, current_file),
            env: self.env.clone(),
        }
    }
}

/// The shell a run command is typed into: quoting and environment syntax
/// differ between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    PowerShell,
    Cmd,
    Fish,
    /// sh, bash, zsh and the like.
    Posix,
}

impl ShellKind {
    /// From the terminal's shell executable name (`pwsh.exe`, `bash`…).
    pub fn from_shell_name(name: &str) -> Self {
        let name = name.to_lowercase();
        if name.contains("pwsh") || name.contains("powershell") {
            Self::PowerShell
        } else if name.starts_with("cmd") {
            Self::Cmd
        } else if name.starts_with("fish") {
            Self::Fish
        } else {
            Self::Posix
        }
    }

    /// `s` as a single word of this shell, quoted only when it has to be.
    pub fn quote(self, s: &str) -> String {
        let special = |c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '"' | '\''
                        | '`'
                        | '$'
                        | '&'
                        | '|'
                        | ';'
                        | '<'
                        | '>'
                        | '('
                        | ')'
                        | '*'
                        | '?'
                        | '#'
                        | '{'
                        | '}'
                        | '%'
                        | '^'
                        | '!'
                )
        };
        if !s.is_empty() && !s.chars().any(special) {
            return s.to_string();
        }
        match self {
            Self::PowerShell => format!("'{}'", s.replace('\'', "''")),
            Self::Cmd => format!("\"{}\"", s.replace('"', "\"\"")),
            Self::Fish => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")),
            Self::Posix => format!("'{}'", s.replace('\'', "'\\''")),
        }
    }

    /// `command` preceded by the assignments of `env`, in this shell's syntax.
    fn with_env(self, env: &[(String, String)], command: String) -> String {
        if env.is_empty() {
            return command;
        }
        let assigns: Vec<String> = env
            .iter()
            .map(|(k, v)| match self {
                Self::PowerShell => format!("$env:{k}={};", self.quote_always(v)),
                Self::Cmd => format!("set \"{k}={v}\" &&"),
                Self::Fish | Self::Posix => format!("{k}={}", self.quote(v)),
            })
            .collect();
        let prefix = assigns.join(" ");
        match self {
            // fish only accepts `VAR=value cmd` since 3.1; `env` works everywhere.
            Self::Fish => format!("env {prefix} {command}"),
            _ => format!("{prefix} {command}"),
        }
    }

    /// `s` quoted even when it needs no quoting (PowerShell string values).
    fn quote_always(self, s: &str) -> String {
        let q = self.quote(s);
        if q == s {
            format!("'{s}'")
        } else {
            q
        }
    }

    /// The line changing directory to `dir`.
    fn cd(self, dir: &str) -> String {
        match self {
            // `/d` also switches drive (`cd C:\x` from D: doesn't).
            Self::Cmd => format!("cd /d {}", self.quote(dir)),
            _ => format!("cd {}", self.quote(dir)),
        }
    }
}

/// The value of a VS Code-style variable, if it is one:
/// `workspaceRoot` / `workspaceFolder` → workspace path,
/// `file` → current file path,
/// `fileDir` / `fileDirname` → current file's directory,
/// `fileBasename` → current file name, `fileExtname` → its extension (`.rs`),
/// `fileName` / `fileBasenameNoExtension` → current file name without extension.
fn variable_value(
    name: &str,
    workspace: Option<&Path>,
    current_file: Option<&Path>,
) -> Option<String> {
    let lossy = |p: &Path| p.to_string_lossy().to_string();
    let os = |s: Option<&std::ffi::OsStr>| {
        s.map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    };
    let ws = || workspace.map(lossy).unwrap_or_default();
    Some(match name {
        "workspaceRoot" | "workspaceFolder" => ws(),
        "file" => current_file.map(lossy).unwrap_or_default(),
        "fileDir" | "fileDirname" => current_file
            .and_then(|p| p.parent())
            .map(lossy)
            .unwrap_or_else(ws),
        "fileBasename" => os(current_file.and_then(|p| p.file_name())),
        "fileExtname" => current_file
            .and_then(|p| p.extension())
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default(),
        "fileName" | "fileBasenameNoExtension" => os(current_file.and_then(|p| p.file_stem())),
        _ => return None,
    })
}

/// Replace every known `${variable}` of `s` with `render(value, quoted)`,
/// where `quoted` tells whether the template already wraps it in quotes.
/// Unknown variables are left as they are.
fn expand_with(
    s: &str,
    workspace: Option<&Path>,
    current_file: Option<&Path>,
    render: impl Fn(&str, bool) -> String,
) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        let Some(len) = rest[start..].find('}') else {
            break;
        };
        let name = &rest[start + 2..start + len];
        out.push_str(&rest[..start]);
        match variable_value(name, workspace, current_file) {
            Some(value) => {
                let quoted = out.ends_with(['"', '\'']);
                out.push_str(&render(&value, quoted));
            }
            None => out.push_str(&rest[start..=start + len]),
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    out
}

/// Expand the VS Code-style variables of run configurations, tasks and
/// debug launch arguments (see [`variable_value`] for the list), unquoted.
pub fn expand_variables(s: &str, workspace: Option<&Path>, current_file: Option<&Path>) -> String {
    if !s.contains("${") {
        return s.to_string();
    }
    expand_with(s, workspace, current_file, |v, _| v.to_string())
}

#[derive(Debug, Clone)]
pub struct ResolvedRun {
    /// The command line, `env` assignments included.
    pub command: String,
    pub cwd: String,
    pub env: Vec<(String, String)>,
}

/// Top-level structure for `.coding-unicorns/launch.toml`
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LaunchFile {
    #[serde(default)]
    pub configurations: Vec<RunConfig>,
}

pub struct RunManager {
    pub configs: Vec<RunConfig>,
    pub active_config: usize,
    pub is_running: bool,
    workspace: Option<PathBuf>,
}

impl RunManager {
    pub fn new() -> Self {
        Self {
            configs: Vec::new(),
            active_config: 0,
            is_running: false,
            workspace: None,
        }
    }

    /// Call when the workspace changes. Loads `launch.toml`; without one,
    /// imports `.vscode/launch.json` (if any) followed by auto-detected configs.
    pub fn load_for_workspace(&mut self, workspace: &Path) {
        self.workspace = Some(workspace.to_path_buf());
        self.active_config = 0;

        let launch_file = workspace.join(".coding-unicorns").join("launch.toml");
        if launch_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&launch_file) {
                if let Ok(lf) = toml::from_str::<LaunchFile>(&content) {
                    self.configs = lf.configurations;
                    return;
                }
            }
        }

        let mut configs = if launch_file.exists() {
            Vec::new() // launch.toml present but invalid: don't import over it
        } else {
            vscode_launch::load(workspace).unwrap_or_default()
        };
        for c in auto_detect_configs(workspace) {
            if !configs.iter().any(|i| i.name == c.name) {
                configs.push(c);
            }
        }
        self.configs = configs;
    }

    /// Save current configs to `.coding-unicorns/launch.toml`.
    pub fn save(&self) {
        if let Some(ws) = &self.workspace {
            let dir = ws.join(".coding-unicorns");
            let _ = std::fs::create_dir_all(&dir);
            let lf = LaunchFile {
                configurations: self.configs.clone(),
            };
            if let Ok(content) = toml::to_string_pretty(&lf) {
                let _ = std::fs::write(dir.join("launch.toml"), content);
            }
        }
    }

    pub fn active_config(&self) -> Option<&RunConfig> {
        self.configs.get(self.active_config)
    }

    /// Build the shell command string to send to the terminal.
    pub fn build_command(
        &self,
        workspace: Option<&Path>,
        current_file: Option<&Path>,
        shell: ShellKind,
    ) -> Option<String> {
        let config = self.active_config()?;
        let resolved = config.resolve(workspace, current_file, shell);

        // Terminate with `\r` (carriage return) — that's what the interactive
        // terminal sends for Enter; a bare `\n` does NOT submit in PowerShell.
        // Send `cd` and the command as two separate lines instead of `cd && cmd`,
        // since Windows PowerShell 5.1 (the default shell) doesn't support `&&`.
        if resolved.cwd.is_empty() {
            Some(format!("{}\r", resolved.command))
        } else {
            Some(format!(
                "{}\r{}\r",
                shell.cd(&resolved.cwd),
                resolved.command
            ))
        }
    }

    pub fn add_config(&mut self, config: RunConfig) {
        self.configs.push(config);
    }

    pub fn remove_config(&mut self, idx: usize) {
        if idx < self.configs.len() {
            self.configs.remove(idx);
            if self.active_config >= self.configs.len() && !self.configs.is_empty() {
                self.active_config = self.configs.len() - 1;
            }
        }
    }
}

impl Default for RunManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Auto-detect run configurations from workspace contents.
pub fn auto_detect_configs(workspace: &Path) -> Vec<RunConfig> {
    let mut configs = Vec::new();

    if workspace.join("Cargo.toml").exists() {
        configs.push(RunConfig {
            name: "Cargo Run".to_string(),
            command: "cargo run".to_string(),
            cwd: "${workspaceRoot}".to_string(),
            env: vec![],
            args: vec![],
            debug: None,
        });
        configs.push(RunConfig {
            name: "Cargo Test".to_string(),
            command: "cargo test".to_string(),
            cwd: "${workspaceRoot}".to_string(),
            env: vec![],
            args: vec![],
            debug: None,
        });
        configs.push(RunConfig {
            name: "Cargo Build".to_string(),
            command: "cargo build".to_string(),
            cwd: "${workspaceRoot}".to_string(),
            env: vec![],
            args: vec![],
            debug: None,
        });
    }

    if workspace.join("package.json").exists() {
        if let Ok(content) = std::fs::read_to_string(workspace.join("package.json")) {
            if content.contains("\"start\"") {
                configs.push(RunConfig {
                    name: "npm start".to_string(),
                    command: "npm start".to_string(),
                    cwd: "${workspaceRoot}".to_string(),
                    env: vec![],
                    args: vec![],
                    debug: None,
                });
            }
            if content.contains("\"dev\"") {
                configs.push(RunConfig {
                    name: "npm run dev".to_string(),
                    command: "npm run dev".to_string(),
                    cwd: "${workspaceRoot}".to_string(),
                    env: vec![],
                    args: vec![],
                    debug: None,
                });
            }
            if content.contains("\"test\"") {
                configs.push(RunConfig {
                    name: "npm test".to_string(),
                    command: "npm test".to_string(),
                    cwd: "${workspaceRoot}".to_string(),
                    env: vec![],
                    args: vec![],
                    debug: None,
                });
            }
            if content.contains("\"build\"") {
                configs.push(RunConfig {
                    name: "npm run build".to_string(),
                    command: "npm run build".to_string(),
                    cwd: "${workspaceRoot}".to_string(),
                    env: vec![],
                    args: vec![],
                    debug: None,
                });
            }
        }
    }

    if workspace.join("manage.py").exists() {
        configs.push(RunConfig {
            name: "Django run".to_string(),
            command: "python3 manage.py runserver".to_string(),
            cwd: "${workspaceRoot}".to_string(),
            env: vec![],
            args: vec![],
            debug: None,
        });
    }
    if workspace.join("main.py").exists() {
        configs.push(RunConfig {
            name: "Run main.py".to_string(),
            command: "python3 main.py".to_string(),
            cwd: "${workspaceRoot}".to_string(),
            env: vec![],
            args: vec![],
            debug: None,
        });
    }

    if workspace.join("go.mod").exists() {
        configs.push(RunConfig {
            name: "Go run".to_string(),
            command: "go run .".to_string(),
            cwd: "${workspaceRoot}".to_string(),
            env: vec![],
            args: vec![],
            debug: None,
        });
        configs.push(RunConfig {
            name: "Go test".to_string(),
            command: "go test ./...".to_string(),
            cwd: "${workspaceRoot}".to_string(),
            env: vec![],
            args: vec![],
            debug: None,
        });
    }

    if workspace.join("Makefile").exists() || workspace.join("makefile").exists() {
        configs.push(RunConfig {
            name: "make".to_string(),
            command: "make".to_string(),
            cwd: "${workspaceRoot}".to_string(),
            env: vec![],
            args: vec![],
            debug: None,
        });
    }

    if workspace.join("docker-compose.yml").exists()
        || workspace.join("docker-compose.yaml").exists()
    {
        configs.push(RunConfig {
            name: "Docker Compose Up".to_string(),
            command: "docker-compose up".to_string(),
            cwd: "${workspaceRoot}".to_string(),
            env: vec![],
            args: vec![],
            debug: None,
        });
    }

    // Always available as a fallback — runs the currently open file directly.
    configs.push(RunConfig {
        name: "Run current file".to_string(),
        command: "${file}".to_string(),
        cwd: "${fileDir}".to_string(),
        env: vec![],
        args: vec![],
        debug: None,
    });

    configs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(name: &str, command: &str, cwd: &str) -> RunConfig {
        RunConfig {
            name: name.to_string(),
            command: command.to_string(),
            cwd: cwd.to_string(),
            env: vec![],
            args: vec![],
            debug: None,
        }
    }

    fn names(configs: &[RunConfig]) -> Vec<&str> {
        configs.iter().map(|c| c.name.as_str()).collect()
    }

    fn touch(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    // ── RunConfig::resolve ─────────────────────────────────────────────────

    #[test]
    fn resolve_substitutes_all_variables_in_command_args_and_cwd() {
        let ws = PathBuf::from("ws");
        let file = PathBuf::from("ws").join("src").join("app.test.py");
        let mut c = cfg(
            "x",
            "run ${file} in ${fileDir} named ${fileName}",
            "${workspaceRoot}",
        );
        c.args = vec!["--root=${workspaceRoot}".into(), "${fileName}".into()];
        c.env = vec![("K".into(), "V".into())];
        let r = c.resolve(Some(&ws), Some(&file), ShellKind::Posix);
        let file_s = file.to_string_lossy();
        let dir_s = file.parent().unwrap().to_string_lossy();
        // `env` is set for the command itself.
        assert_eq!(
            r.command,
            format!("K=V run {file_s} in {dir_s} named app.test --root=ws app.test")
        );
        assert_eq!(r.cwd, "ws");
        assert_eq!(r.env, vec![("K".to_string(), "V".to_string())]);
    }

    #[test]
    fn resolve_without_file_falls_back_to_workspace_for_file_dir() {
        let ws = PathBuf::from("root");
        let c = cfg("x", "[${file}][${fileName}]", "${fileDir}");
        let r = c.resolve(Some(&ws), None, ShellKind::Posix);
        assert_eq!(r.command, "[][]");
        assert_eq!(r.cwd, "root");
    }

    #[test]
    fn resolve_without_anything_yields_empty_substitutions() {
        let c = cfg("x", "echo ${workspaceRoot}!", "${fileDir}");
        let r = c.resolve(None, None, ShellKind::Posix);
        assert_eq!(r.command, "echo !");
        assert_eq!(r.cwd, "");
    }

    #[test]
    fn resolve_leaves_unknown_variables_and_plain_text_untouched() {
        let c = cfg("x", "echo ${HOME} $PATH", "/abs");
        let r = c.resolve(
            Some(Path::new("w")),
            Some(Path::new("f.rs")),
            ShellKind::Posix,
        );
        assert_eq!(r.command, "echo ${HOME} $PATH");
        assert_eq!(r.cwd, "/abs");
    }

    // ── build_command ──────────────────────────────────────────────────────

    #[test]
    fn build_command_none_without_configs() {
        let rm = RunManager::new();
        assert!(rm.active_config().is_none());
        assert!(rm.build_command(None, None, ShellKind::Posix).is_none());
    }

    #[test]
    fn build_command_with_empty_cwd_sends_only_command() {
        let mut rm = RunManager::default();
        rm.add_config(cfg("x", "cargo run", ""));
        assert_eq!(
            rm.build_command(None, None, ShellKind::Posix).unwrap(),
            "cargo run\r"
        );
    }

    #[test]
    fn build_command_changes_directory_on_separate_line() {
        let mut rm = RunManager::new();
        rm.add_config(cfg("x", "make", "${workspaceRoot}"));
        assert_eq!(
            rm.build_command(Some(Path::new("proj")), None, ShellKind::Posix)
                .unwrap(),
            "cd proj\rmake\r"
        );
    }

    #[test]
    fn build_command_quotes_cwd_with_spaces_or_parens() {
        let mut rm = RunManager::new();
        rm.add_config(cfg("x", "make", "${workspaceRoot}"));
        assert_eq!(
            rm.build_command(Some(Path::new("my proj")), None, ShellKind::Posix)
                .unwrap(),
            "cd 'my proj'\rmake\r"
        );
        assert_eq!(
            rm.build_command(Some(Path::new("my proj")), None, ShellKind::PowerShell)
                .unwrap(),
            "cd 'my proj'\rmake\r"
        );
        assert_eq!(
            rm.build_command(Some(Path::new("my proj")), None, ShellKind::Cmd)
                .unwrap(),
            "cd /d \"my proj\"\rmake\r"
        );
        assert_eq!(
            rm.build_command(Some(Path::new("x(86)")), None, ShellKind::Posix)
                .unwrap(),
            "cd 'x(86)'\rmake\r"
        );
    }

    #[test]
    fn quote_follows_each_shell_and_leaves_plain_words_alone() {
        use ShellKind::*;
        assert_eq!(Posix.quote("plain-word.rs"), "plain-word.rs");
        assert_eq!(Posix.quote("it's here"), "'it'\\''s here'");
        assert_eq!(Fish.quote("it's here"), "'it\\'s here'");
        assert_eq!(PowerShell.quote("it's here"), "'it''s here'");
        assert_eq!(Cmd.quote("a \"b\""), "\"a \"\"b\"\"\"");
        assert_eq!(Posix.quote(""), "''", "an empty argument stays an argument");
        assert_eq!(ShellKind::from_shell_name("pwsh.exe"), PowerShell);
        assert_eq!(ShellKind::from_shell_name("powershell.exe"), PowerShell);
        assert_eq!(ShellKind::from_shell_name("cmd.exe"), Cmd);
        assert_eq!(ShellKind::from_shell_name("fish"), Fish);
        assert_eq!(ShellKind::from_shell_name("zsh"), Posix);
    }

    #[test]
    fn paths_with_spaces_are_quoted_for_the_shell() {
        let file = Path::new("C:/My Projects/a.py");
        let c = cfg("x", "python ${file}", "");
        assert_eq!(
            c.resolve(None, Some(file), ShellKind::Posix).command,
            "python 'C:/My Projects/a.py'"
        );
        assert_eq!(
            c.resolve(None, Some(file), ShellKind::Cmd).command,
            "python \"C:/My Projects/a.py\""
        );
        // Already quoted in the template: not quoted twice.
        let c = cfg("x", "python \"${file}\"", "");
        assert_eq!(
            c.resolve(None, Some(file), ShellKind::Posix).command,
            "python \"C:/My Projects/a.py\""
        );
        // A quoted program path needs PowerShell's call operator.
        let c = cfg("x", "${file}", "");
        assert_eq!(
            c.resolve(None, Some(file), ShellKind::PowerShell).command,
            "& 'C:/My Projects/a.py'"
        );
        // An argument with a space stays one argument.
        let mut c = cfg("x", "run", "");
        c.args = vec!["two words".into()];
        assert_eq!(
            c.resolve(None, None, ShellKind::Posix).command,
            "run 'two words'"
        );
    }

    #[test]
    fn env_is_set_in_each_shells_syntax() {
        let mut c = cfg("x", "app", "");
        c.env = vec![("A".into(), "1".into()), ("B".into(), "x y".into())];
        let r = |s| c.resolve(None, None, s).command;
        assert_eq!(r(ShellKind::Posix), "A=1 B='x y' app");
        assert_eq!(r(ShellKind::Fish), "env A=1 B='x y' app");
        assert_eq!(r(ShellKind::PowerShell), "$env:A='1'; $env:B='x y'; app");
        assert_eq!(r(ShellKind::Cmd), "set \"A=1\" && set \"B=x y\" && app");
    }

    #[test]
    fn file_basename_and_extname_follow_vs_code() {
        let file = Path::new("src/main.rs");
        let c = cfg("x", "${fileBasename} ${fileExtname} ${fileName}", "");
        assert_eq!(
            c.resolve(None, Some(file), ShellKind::Posix).command,
            "main.rs .rs main"
        );
    }

    #[test]
    fn build_command_uses_active_config() {
        let mut rm = RunManager::new();
        rm.add_config(cfg("a", "first", ""));
        rm.add_config(cfg("b", "second", ""));
        rm.active_config = 1;
        assert_eq!(rm.active_config().unwrap().name, "b");
        assert_eq!(
            rm.build_command(None, None, ShellKind::Posix).unwrap(),
            "second\r"
        );
        rm.active_config = 5;
        assert!(rm.build_command(None, None, ShellKind::Posix).is_none());
    }

    // ── add / remove ───────────────────────────────────────────────────────

    #[test]
    fn remove_config_clamps_active_index() {
        let mut rm = RunManager::new();
        for n in ["a", "b", "c"] {
            rm.add_config(cfg(n, n, ""));
        }
        rm.active_config = 2;
        rm.remove_config(2);
        assert_eq!(names(&rm.configs), ["a", "b"]);
        assert_eq!(rm.active_config, 1);

        // Removing before the active one keeps the index if still valid.
        rm.active_config = 0;
        rm.remove_config(1);
        assert_eq!(names(&rm.configs), ["a"]);
        assert_eq!(rm.active_config, 0);

        // Out-of-range is a no-op.
        rm.remove_config(7);
        assert_eq!(rm.configs.len(), 1);

        // Removing the last one leaves the index alone (nothing to clamp to).
        rm.remove_config(0);
        assert!(rm.configs.is_empty());
        assert_eq!(rm.active_config, 0);
        assert!(rm.active_config().is_none());
    }

    // ── auto detection ─────────────────────────────────────────────────────

    #[test]
    fn auto_detect_empty_workspace_has_only_run_current_file() {
        let dir = tempfile::tempdir().unwrap();
        let c = auto_detect_configs(dir.path());
        assert_eq!(names(&c), ["Run current file"]);
        assert_eq!(c[0].command, "${file}");
        assert_eq!(c[0].cwd, "${fileDir}");
    }

    #[test]
    fn auto_detect_cargo_project() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Cargo.toml", "[package]");
        let c = auto_detect_configs(dir.path());
        assert_eq!(
            names(&c),
            ["Cargo Run", "Cargo Test", "Cargo Build", "Run current file"]
        );
        assert_eq!(c[1].command, "cargo test");
        assert!(c[..3].iter().all(|c| c.cwd == "${workspaceRoot}"));
    }

    #[test]
    fn auto_detect_npm_scripts_only_present_ones() {
        let dir = tempfile::tempdir().unwrap();
        touch(
            dir.path(),
            "package.json",
            r#"{"scripts": {"dev": "vite", "build": "vite build"}}"#,
        );
        let c = auto_detect_configs(dir.path());
        assert_eq!(
            names(&c),
            ["npm run dev", "npm run build", "Run current file"]
        );

        touch(
            dir.path(),
            "package.json",
            r#"{"scripts": {"start": "a", "dev": "b", "test": "c", "build": "d"}}"#,
        );
        let c = auto_detect_configs(dir.path());
        assert_eq!(
            names(&c),
            [
                "npm start",
                "npm run dev",
                "npm test",
                "npm run build",
                "Run current file"
            ]
        );
        assert_eq!(c[2].command, "npm test");
    }

    #[test]
    fn auto_detect_python_go_make_docker() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "manage.py", "");
        touch(dir.path(), "main.py", "");
        touch(dir.path(), "go.mod", "module x");
        touch(dir.path(), "makefile", "all:");
        touch(dir.path(), "docker-compose.yaml", "");
        let c = auto_detect_configs(dir.path());
        assert_eq!(
            names(&c),
            [
                "Django run",
                "Run main.py",
                "Go run",
                "Go test",
                "make",
                "Docker Compose Up",
                "Run current file"
            ]
        );
        assert_eq!(c[0].command, "python3 manage.py runserver");
        assert_eq!(c[3].command, "go test ./...");
        assert_eq!(c[5].command, "docker-compose up");
    }

    #[test]
    fn auto_detect_alternate_file_names() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Makefile", "all:");
        touch(dir.path(), "docker-compose.yml", "");
        let c = auto_detect_configs(dir.path());
        assert_eq!(names(&c), ["make", "Docker Compose Up", "Run current file"]);
    }

    // ── workspace loading / saving ─────────────────────────────────────────

    #[test]
    fn load_for_workspace_without_launch_file_auto_detects() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Cargo.toml", "");
        let mut rm = RunManager::new();
        rm.active_config = 3;
        rm.load_for_workspace(dir.path());
        assert_eq!(rm.configs[0].name, "Cargo Run");
        assert_eq!(rm.active_config, 0);
    }

    #[test]
    fn load_for_workspace_prefers_launch_toml() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Cargo.toml", "");
        let cu = dir.path().join(".coding-unicorns");
        std::fs::create_dir_all(&cu).unwrap();
        std::fs::write(
            cu.join("launch.toml"),
            r#"
            [[configurations]]
            name = "Serve"
            command = "python -m http.server"
            cwd = "${workspaceRoot}/public"
            env = [["PORT", "8000"]]
            args = ["8000"]

            [[configurations]]
            name = "Minimal"
            command = "true"
            cwd = ""
            "#,
        )
        .unwrap();
        let mut rm = RunManager::new();
        rm.active_config = 1;
        rm.load_for_workspace(dir.path());
        assert_eq!(names(&rm.configs), ["Serve", "Minimal"]);
        assert_eq!(rm.active_config, 0);
        let serve = &rm.configs[0];
        assert_eq!(serve.env, vec![("PORT".to_string(), "8000".to_string())]);
        assert_eq!(serve.args, vec!["8000".to_string()]);
        // env/args default to empty when omitted.
        assert!(rm.configs[1].env.is_empty() && rm.configs[1].args.is_empty());
    }

    #[test]
    fn load_for_workspace_with_invalid_launch_toml_falls_back_to_detection() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "go.mod", "");
        let cu = dir.path().join(".coding-unicorns");
        std::fs::create_dir_all(&cu).unwrap();
        std::fs::write(cu.join("launch.toml"), "[[configurations]]\nname = 1").unwrap();
        let mut rm = RunManager::new();
        rm.load_for_workspace(dir.path());
        assert_eq!(
            names(&rm.configs),
            ["Go run", "Go test", "Run current file"]
        );
    }

    #[test]
    fn empty_launch_toml_yields_no_configs() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Cargo.toml", "");
        let cu = dir.path().join(".coding-unicorns");
        std::fs::create_dir_all(&cu).unwrap();
        std::fs::write(cu.join("launch.toml"), "").unwrap();
        let mut rm = RunManager::new();
        rm.load_for_workspace(dir.path());
        assert!(rm.configs.is_empty());
    }

    #[test]
    fn save_then_reload_round_trips_configs() {
        let dir = tempfile::tempdir().unwrap();
        let mut rm = RunManager::new();
        rm.load_for_workspace(dir.path());
        let mut c = cfg("Custom", "node index.js", "${workspaceRoot}");
        c.env = vec![("NODE_ENV".into(), "dev".into())];
        c.args = vec!["--inspect".into()];
        rm.configs = vec![c];
        rm.save();
        assert!(dir.path().join(".coding-unicorns/launch.toml").exists());

        let mut rm2 = RunManager::new();
        rm2.load_for_workspace(dir.path());
        assert_eq!(names(&rm2.configs), ["Custom"]);
        assert_eq!(rm2.configs[0].env[0].1, "dev");
        assert_eq!(rm2.configs[0].args, vec!["--inspect".to_string()]);
    }

    fn write_launch_json(dir: &Path, content: &str) {
        let vs = dir.join(".vscode");
        std::fs::create_dir_all(&vs).unwrap();
        std::fs::write(vs.join("launch.json"), content).unwrap();
    }

    const LAUNCH_JSON: &str = r#"{
        // VS Code launch file
        "version": "0.2.0",
        "configurations": [
            {
                "name": "Debug app",
                "type": "lldb",
                "request": "launch",
                "program": "${workspaceFolder}/target/debug/app",
                "args": ["-v"],
                "env": {"RUST_LOG": "debug"},
                "stopOnEntry": false,
                "sourceLanguages": ["rust"],
            },
        ]
    }"#;

    #[test]
    fn load_for_workspace_imports_vscode_launch_json_without_launch_toml() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Cargo.toml", "");
        write_launch_json(dir.path(), LAUNCH_JSON);
        let mut rm = RunManager::new();
        rm.load_for_workspace(dir.path());
        assert_eq!(
            names(&rm.configs),
            [
                "Debug app",
                "Cargo Run",
                "Cargo Test",
                "Cargo Build",
                "Run current file"
            ]
        );
        let c = &rm.configs[0];
        let d = c.debug.as_ref().unwrap();
        assert_eq!(d.adapter_type, "lldb");
        assert_eq!(
            d.launch_args["sourceLanguages"],
            serde_json::json!(["rust"])
        );
        let r = c.resolve(Some(Path::new("ws")), None, ShellKind::Posix);
        assert_eq!(r.command, "RUST_LOG=debug ws/target/debug/app -v");
        assert_eq!(r.cwd, "ws");
        assert_eq!(r.env, vec![("RUST_LOG".to_string(), "debug".to_string())]);
    }

    #[test]
    fn launch_toml_takes_precedence_over_launch_json() {
        let dir = tempfile::tempdir().unwrap();
        write_launch_json(dir.path(), LAUNCH_JSON);
        let cu = dir.path().join(".coding-unicorns");
        std::fs::create_dir_all(&cu).unwrap();
        std::fs::write(
            cu.join("launch.toml"),
            "[[configurations]]\nname = \"Mine\"\ncommand = \"x\"\ncwd = \"\"",
        )
        .unwrap();
        let mut rm = RunManager::new();
        rm.load_for_workspace(dir.path());
        assert_eq!(names(&rm.configs), ["Mine"]);
        assert!(rm.configs[0].debug.is_none());

        // An invalid launch.toml is not silently replaced by the import.
        std::fs::write(cu.join("launch.toml"), "[[configurations]]\nname = 1").unwrap();
        rm.load_for_workspace(dir.path());
        assert_eq!(names(&rm.configs), ["Run current file"]);
    }

    #[test]
    fn invalid_launch_json_falls_back_to_detection() {
        let dir = tempfile::tempdir().unwrap();
        write_launch_json(dir.path(), "{ broken");
        let mut rm = RunManager::new();
        rm.load_for_workspace(dir.path());
        assert_eq!(names(&rm.configs), ["Run current file"]);
    }

    #[test]
    fn imported_debug_settings_survive_save_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        write_launch_json(dir.path(), LAUNCH_JSON);
        let mut rm = RunManager::new();
        rm.load_for_workspace(dir.path());
        let before = rm.configs[0].debug.clone();
        rm.save();

        // launch.toml now exists and wins; the debug settings round-trip.
        std::fs::remove_file(dir.path().join(".vscode/launch.json")).unwrap();
        let mut rm2 = RunManager::new();
        rm2.load_for_workspace(dir.path());
        assert_eq!(rm2.configs[0].name, "Debug app");
        assert_eq!(rm2.configs[0].debug, before);
        // Plain configs keep no `debug` table.
        let toml =
            std::fs::read_to_string(dir.path().join(".coding-unicorns/launch.toml")).unwrap();
        assert_eq!(toml.matches("[configurations.debug]").count(), 1);
    }

    #[test]
    fn resolve_supports_vscode_variable_aliases() {
        let file = PathBuf::from("ws").join("src").join("app.py");
        let c = cfg(
            "x",
            "${workspaceFolder}|${fileDirname}|${fileBasenameNoExtension}",
            "",
        );
        let r = c.resolve(Some(Path::new("ws")), Some(&file), ShellKind::Posix);
        let dir_s = file.parent().unwrap().to_string_lossy();
        assert_eq!(r.command, format!("ws|{dir_s}|app"));
    }

    #[test]
    fn save_without_workspace_writes_nothing() {
        let rm = RunManager::new();
        let cwd_launch = Path::new(".coding-unicorns").join("launch.toml");
        let existed = cwd_launch.exists();
        rm.save(); // must be a silent no-op, never writing relative to the CWD
        assert_eq!(cwd_launch.exists(), existed);
    }
}
