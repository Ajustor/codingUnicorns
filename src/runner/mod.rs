use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use vscode_import::{import_vscode, DebugSpec};

pub mod vscode_import;

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
    /// Specs de debug importées de `.vscode/launch.json`, clé = nom de config.
    /// Non sérialisé : re-dérivé à chaque `load_for_workspace`.
    debug_specs: HashMap<String, DebugSpec>,
}

impl RunManager {
    pub fn new() -> Self {
        Self {
            configs: Vec::new(),
            active_config: 0,
            is_running: false,
            workspace: None,
            debug_specs: HashMap::new(),
        }
    }

    /// Call when the workspace changes. Fusionne `launch.toml`, l'import
    /// `.vscode` et l'auto-détection, dédupliqués.
    pub fn load_for_workspace(&mut self, workspace: &Path) {
        self.workspace = Some(workspace.to_path_buf());
        self.debug_specs.clear();

        let mut configs: Vec<RunConfig> = Vec::new();

        // 1. launch.toml (précédence la plus haute).
        let launch_file = workspace.join(".coding-unicorns").join("launch.toml");
        if launch_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&launch_file) {
                if let Ok(lf) = toml::from_str::<LaunchFile>(&content) {
                    configs.extend(lf.configurations);
                }
            }
        }

        // 2. Import .vscode (tasks.json + launch.json).
        let imported = import_vscode(workspace);
        configs.extend(imported.configs);
        // Stash raw specs for post-dedup re-keying (consumed below).
        let imported_debug_specs = imported.debug_specs;

        // 3. Auto-détection (précédence la plus basse).
        configs.extend(auto_detect_configs(workspace));

        self.configs = merge_dedup(configs);
        self.active_config = 0;

        // Re-key debug specs to the surviving config names (post-dedup), so every
        // debug_specs key matches a config in self.configs. A .vscode config that
        // was deduped away must not leave an orphan spec.
        let raw_specs: HashMap<String, DebugSpec> = imported_debug_specs
            .into_iter()
            .map(|(name, spec)| (name.to_lowercase(), spec))
            .collect();
        for cfg in &self.configs {
            if let Some(spec) = raw_specs.get(&cfg.name.to_lowercase()) {
                self.debug_specs.insert(cfg.name.clone(), spec.clone());
            }
        }
    }

    /// Spec de debug pour une config nommée, si importée depuis `.vscode`.
    pub fn debug_spec_for(&self, name: &str) -> Option<&DebugSpec> {
        self.debug_specs.get(name)
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

/// Fusionne en supprimant les doublons : par nom (insensible à la casse),
/// puis par couple (command, cwd). Conserve la première occurrence.
fn merge_dedup(configs: Vec<RunConfig>) -> Vec<RunConfig> {
    let mut seen_names: HashSet<String> = HashSet::new();
    let mut seen_cmds: HashSet<(String, String)> = HashSet::new();
    let mut out = Vec::new();
    for c in configs {
        let name_key = c.name.to_lowercase();
        let cmd_key = (c.command.clone(), c.cwd.clone());
        // Name-dedup (case-insensitive) takes precedence; a config is dropped if EITHER name OR (command, cwd) matches.
        if seen_names.contains(&name_key) || seen_cmds.contains(&cmd_key) {
            continue;
        }
        seen_names.insert(name_key);
        seen_cmds.insert(cmd_key);
        out.push(c);
    }
    out
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

    #[test]
    fn merge_dedup_drops_duplicate_names_and_commands() {
        let configs = vec![
            RunConfig { name: "Run".into(), command: "a".into(), cwd: "w".into(), env: vec![], args: vec![] },
            RunConfig { name: "run".into(), command: "b".into(), cwd: "w".into(), env: vec![], args: vec![] }, // même nom (casse)
            RunConfig { name: "Other".into(), command: "a".into(), cwd: "w".into(), env: vec![], args: vec![] }, // même (command, cwd)
            RunConfig { name: "Keep".into(), command: "c".into(), cwd: "w".into(), env: vec![], args: vec![] },
        ];
        let merged = merge_dedup(configs);
        let names: Vec<&str> = merged.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["Run", "Keep"]);
    }
}
