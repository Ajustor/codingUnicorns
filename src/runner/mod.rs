use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
}

impl RunConfig {
    /// Resolve variables in command/cwd:
    /// `${workspaceRoot}` → workspace path
    /// `${file}` → current file path
    /// `${fileDir}` → current file's directory
    /// `${fileName}` → current file name without extension
    pub fn resolve(&self, workspace: Option<&Path>, current_file: Option<&Path>) -> ResolvedRun {
        let ws = workspace
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let file = current_file
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let file_dir = current_file
            .and_then(|p| p.parent())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ws.clone());
        let file_name = current_file
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let resolve_str = |s: &str| -> String {
            s.replace("${workspaceRoot}", &ws)
                .replace("${file}", &file)
                .replace("${fileDir}", &file_dir)
                .replace("${fileName}", &file_name)
        };

        let mut full_cmd = resolve_str(&self.command);
        for arg in &self.args {
            full_cmd.push(' ');
            full_cmd.push_str(&resolve_str(arg));
        }

        ResolvedRun {
            command: full_cmd,
            cwd: resolve_str(&self.cwd),
            env: self.env.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedRun {
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

    /// Call when the workspace changes. Loads `launch.toml` or auto-detects configs.
    pub fn load_for_workspace(&mut self, workspace: &Path) {
        self.workspace = Some(workspace.to_path_buf());

        let launch_file = workspace.join(".coding-unicorns").join("launch.toml");
        if launch_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&launch_file) {
                if let Ok(lf) = toml::from_str::<LaunchFile>(&content) {
                    self.configs = lf.configurations;
                    self.active_config = 0;
                    return;
                }
            }
        }

        self.configs = auto_detect_configs(workspace);
        self.active_config = 0;
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
    ) -> Option<String> {
        let config = self.active_config()?;
        let resolved = config.resolve(workspace, current_file);

        // Terminate with `\r` (carriage return) — that's what the interactive
        // terminal sends for Enter; a bare `\n` does NOT submit in PowerShell.
        // Send `cd` and the command as two separate lines instead of `cd && cmd`,
        // since Windows PowerShell 5.1 (the default shell) doesn't support `&&`.
        if resolved.cwd.is_empty() {
            Some(format!("{}\r", resolved.command))
        } else {
            Some(format!(
                "cd {}\r{}\r",
                shell_escape(&resolved.cwd),
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

fn shell_escape(s: &str) -> String {
    if s.contains(' ') || s.contains('(') || s.contains(')') {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
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
        });
        configs.push(RunConfig {
            name: "Cargo Test".to_string(),
            command: "cargo test".to_string(),
            cwd: "${workspaceRoot}".to_string(),
            env: vec![],
            args: vec![],
        });
        configs.push(RunConfig {
            name: "Cargo Build".to_string(),
            command: "cargo build".to_string(),
            cwd: "${workspaceRoot}".to_string(),
            env: vec![],
            args: vec![],
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
                });
            }
            if content.contains("\"dev\"") {
                configs.push(RunConfig {
                    name: "npm run dev".to_string(),
                    command: "npm run dev".to_string(),
                    cwd: "${workspaceRoot}".to_string(),
                    env: vec![],
                    args: vec![],
                });
            }
            if content.contains("\"test\"") {
                configs.push(RunConfig {
                    name: "npm test".to_string(),
                    command: "npm test".to_string(),
                    cwd: "${workspaceRoot}".to_string(),
                    env: vec![],
                    args: vec![],
                });
            }
            if content.contains("\"build\"") {
                configs.push(RunConfig {
                    name: "npm run build".to_string(),
                    command: "npm run build".to_string(),
                    cwd: "${workspaceRoot}".to_string(),
                    env: vec![],
                    args: vec![],
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
        });
    }
    if workspace.join("main.py").exists() {
        configs.push(RunConfig {
            name: "Run main.py".to_string(),
            command: "python3 main.py".to_string(),
            cwd: "${workspaceRoot}".to_string(),
            env: vec![],
            args: vec![],
        });
    }

    if workspace.join("go.mod").exists() {
        configs.push(RunConfig {
            name: "Go run".to_string(),
            command: "go run .".to_string(),
            cwd: "${workspaceRoot}".to_string(),
            env: vec![],
            args: vec![],
        });
        configs.push(RunConfig {
            name: "Go test".to_string(),
            command: "go test ./...".to_string(),
            cwd: "${workspaceRoot}".to_string(),
            env: vec![],
            args: vec![],
        });
    }

    if workspace.join("Makefile").exists() || workspace.join("makefile").exists() {
        configs.push(RunConfig {
            name: "make".to_string(),
            command: "make".to_string(),
            cwd: "${workspaceRoot}".to_string(),
            env: vec![],
            args: vec![],
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
        });
    }

    // Always available as a fallback — runs the currently open file directly.
    configs.push(RunConfig {
        name: "Run current file".to_string(),
        command: "${file}".to_string(),
        cwd: "${fileDir}".to_string(),
        env: vec![],
        args: vec![],
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
        let r = c.resolve(Some(&ws), Some(&file));
        let file_s = file.to_string_lossy();
        let dir_s = file.parent().unwrap().to_string_lossy();
        assert_eq!(
            r.command,
            format!("run {file_s} in {dir_s} named app.test --root=ws app.test")
        );
        assert_eq!(r.cwd, "ws");
        assert_eq!(r.env, vec![("K".to_string(), "V".to_string())]);
    }

    #[test]
    fn resolve_without_file_falls_back_to_workspace_for_file_dir() {
        let ws = PathBuf::from("root");
        let c = cfg("x", "[${file}][${fileName}]", "${fileDir}");
        let r = c.resolve(Some(&ws), None);
        assert_eq!(r.command, "[][]");
        assert_eq!(r.cwd, "root");
    }

    #[test]
    fn resolve_without_anything_yields_empty_substitutions() {
        let c = cfg("x", "echo ${workspaceRoot}!", "${fileDir}");
        let r = c.resolve(None, None);
        assert_eq!(r.command, "echo !");
        assert_eq!(r.cwd, "");
    }

    #[test]
    fn resolve_leaves_unknown_variables_and_plain_text_untouched() {
        let c = cfg("x", "echo ${HOME} $PATH", "/abs");
        let r = c.resolve(Some(Path::new("w")), Some(Path::new("f.rs")));
        assert_eq!(r.command, "echo ${HOME} $PATH");
        assert_eq!(r.cwd, "/abs");
    }

    // ── build_command ──────────────────────────────────────────────────────

    #[test]
    fn build_command_none_without_configs() {
        let rm = RunManager::new();
        assert!(rm.active_config().is_none());
        assert!(rm.build_command(None, None).is_none());
    }

    #[test]
    fn build_command_with_empty_cwd_sends_only_command() {
        let mut rm = RunManager::default();
        rm.add_config(cfg("x", "cargo run", ""));
        assert_eq!(rm.build_command(None, None).unwrap(), "cargo run\r");
    }

    #[test]
    fn build_command_changes_directory_on_separate_line() {
        let mut rm = RunManager::new();
        rm.add_config(cfg("x", "make", "${workspaceRoot}"));
        assert_eq!(
            rm.build_command(Some(Path::new("proj")), None).unwrap(),
            "cd proj\rmake\r"
        );
    }

    #[test]
    fn build_command_quotes_cwd_with_spaces_or_parens() {
        let mut rm = RunManager::new();
        rm.add_config(cfg("x", "make", "${workspaceRoot}"));
        assert_eq!(
            rm.build_command(Some(Path::new("my proj")), None).unwrap(),
            "cd \"my proj\"\rmake\r"
        );
        assert_eq!(
            rm.build_command(Some(Path::new("x(86)")), None).unwrap(),
            "cd \"x(86)\"\rmake\r"
        );
    }

    #[test]
    fn shell_escape_escapes_embedded_quotes_only_when_quoting() {
        assert_eq!(shell_escape("plain"), "plain");
        assert_eq!(shell_escape("a\"b"), "a\"b");
        assert_eq!(shell_escape("a \"b\""), "\"a \\\"b\\\"\"");
    }

    #[test]
    fn build_command_uses_active_config() {
        let mut rm = RunManager::new();
        rm.add_config(cfg("a", "first", ""));
        rm.add_config(cfg("b", "second", ""));
        rm.active_config = 1;
        assert_eq!(rm.active_config().unwrap().name, "b");
        assert_eq!(rm.build_command(None, None).unwrap(), "second\r");
        rm.active_config = 5;
        assert!(rm.build_command(None, None).is_none());
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

    #[test]
    fn save_without_workspace_writes_nothing() {
        let rm = RunManager::new();
        let cwd_launch = Path::new(".coding-unicorns").join("launch.toml");
        let existed = cwd_launch.exists();
        rm.save(); // must be a silent no-op, never writing relative to the CWD
        assert_eq!(cwd_launch.exists(), existed);
    }
}
