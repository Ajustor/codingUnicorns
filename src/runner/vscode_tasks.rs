//! Import of VS Code `.vscode/tasks.json`, used to run the `preLaunchTask`
//! of a debug configuration (typically a build) before the session starts.
//!
//! Supported task types: `process`, `shell` (the default when `type` is
//! missing) and `npm`. `dependsOn` tasks run first, in order. Per-OS
//! overrides (`windows` / `linux` / `osx`) are applied.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::expand_variables;
use super::vscode_launch::strip_jsonc;

/// Path of the VS Code tasks file inside a workspace.
pub fn tasks_json_path(workspace: &Path) -> PathBuf {
    workspace.join(".vscode").join("tasks.json")
}

/// How to run one task.
#[derive(Debug, Clone, PartialEq)]
pub enum TaskCommand {
    /// Run `program` directly with `args`.
    Process { program: String, args: Vec<String> },
    /// Run a command line through the system shell (`cmd /C`, `sh -c`).
    Shell(String),
}

/// One task ready to run, variables already expanded.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskStep {
    pub label: String,
    pub command: TaskCommand,
    /// Working directory, `None` for the workspace root.
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
}

/// Commands to run, in order, for the task `label` of the workspace's
/// `tasks.json` (its `dependsOn` tasks first).
pub fn plan_for_workspace(
    workspace: &Path,
    label: &str,
    current_file: Option<&Path>,
) -> Result<Vec<TaskStep>, String> {
    let path = tasks_json_path(workspace);
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    plan(&content, label, Some(workspace), current_file)
}

/// Commands to run, in order, for the task `label` of a `tasks.json`
/// document.
pub fn plan(
    content: &str,
    label: &str,
    workspace: Option<&Path>,
    current_file: Option<&Path>,
) -> Result<Vec<TaskStep>, String> {
    let root: Value = serde_json::from_str(&strip_jsonc(content))
        .map_err(|e| format!("invalid tasks.json: {e}"))?;
    let tasks: Vec<Map<String, Value>> = root["tasks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_object())
                .map(with_os_overrides)
                .collect()
        })
        .unwrap_or_default();
    let mut steps = Vec::new();
    let mut visiting = HashSet::new();
    let expand = |s: &str| expand_variables(s, workspace, current_file);
    collect(&tasks, label, &expand, &mut visiting, &mut steps)?;
    Ok(steps)
}

fn task_label(task: &Map<String, Value>) -> Option<&str> {
    task.get("label")
        .or_else(|| task.get("taskName"))
        .and_then(|v| v.as_str())
}

fn collect(
    tasks: &[Map<String, Value>],
    label: &str,
    expand: &dyn Fn(&str) -> String,
    visiting: &mut HashSet<String>,
    steps: &mut Vec<TaskStep>,
) -> Result<(), String> {
    let task = tasks
        .iter()
        .find(|t| task_label(t) == Some(label))
        .ok_or_else(|| format!("task `{label}` not found in .vscode/tasks.json"))?;
    if !visiting.insert(label.to_string()) {
        return Err(format!("task `{label}` depends on itself"));
    }
    let deps: Vec<&str> = match task.get("dependsOn") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str()).collect(),
        _ => vec![],
    };
    for dep in deps {
        collect(tasks, dep, expand, visiting, steps)?;
    }
    visiting.remove(label);
    if let Some(command) = task_command(task, expand)? {
        let options = task.get("options");
        let cwd = options.and_then(|o| o["cwd"].as_str()).map(expand);
        let env = options
            .and_then(|o| o["env"].as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), expand(v))))
                    .collect()
            })
            .unwrap_or_default();
        steps.push(TaskStep {
            label: label.to_string(),
            command,
            cwd,
            env,
        });
    }
    Ok(())
}

/// The command of a task, `None` for a task that only groups `dependsOn`.
fn task_command(
    task: &Map<String, Value>,
    expand: &dyn Fn(&str) -> String,
) -> Result<Option<TaskCommand>, String> {
    let kind = task.get("type").and_then(|v| v.as_str()).unwrap_or("shell");
    if kind == "npm" {
        let script = task
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or("npm task without `script`")?;
        return Ok(Some(TaskCommand::Shell(format!("npm run {script}"))));
    }
    if !matches!(kind, "process" | "shell") {
        return Err(format!(
            "task type `{kind}` is not supported (only process, shell and npm tasks can run)"
        ));
    }
    let Some(command) = task.get("command").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    let command = expand(command);
    let args: Vec<String> = task
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(arg_value).map(|s| expand(&s)).collect())
        .unwrap_or_default();
    if kind == "process" {
        return Ok(Some(TaskCommand::Process {
            program: command,
            args,
        }));
    }
    let mut line = command;
    for arg in &args {
        line.push(' ');
        line.push_str(&quote_shell_arg(arg));
    }
    Ok(Some(TaskCommand::Shell(line)))
}

/// An `args` entry: a string, or `{ "value": …, "quoting": … }`.
fn arg_value(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o.get("value").and_then(|v| v.as_str()).map(String::from),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn quote_shell_arg(arg: &str) -> String {
    if arg.is_empty() || arg.contains([' ', '\t']) {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_string()
    }
}

/// Merge the current OS's `windows` / `linux` / `osx` section over the task.
fn with_os_overrides(task: &Map<String, Value>) -> Map<String, Value> {
    let os_key = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    };
    let mut merged = task.clone();
    if let Some(Value::Object(o)) = task.get(os_key) {
        for (k, v) in o {
            merged.insert(k.clone(), v.clone());
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASKS: &str = r#"{
        // VS Code writes JSONC
        "version": "2.0.0",
        "tasks": [
            {
                "label": "build",
                "command": "dotnet",
                "type": "process",
                "args": ["build", "${workspaceFolder}/App.sln", {"value": "-v q", "quoting": "strong"}],
                "problemMatcher": "$msCompile",
            },
            {"label": "restore", "type": "shell", "command": "dotnet restore", "args": ["my dir"]},
            {"label": "all", "dependsOn": ["restore", "build"]},
            {"label": "web", "type": "npm", "script": "build", "options": {"cwd": "${workspaceFolder}/web", "env": {"A": "1"}}},
            {"label": "docker", "type": "docker-build", "dockerBuild": {}},
            {"label": "loop", "command": "x", "dependsOn": "loop"},
            {"label": "os", "command": "unix", "windows": {"command": "win"}}
        ]
    }"#;

    fn ws() -> Option<&'static Path> {
        Some(Path::new("/ws"))
    }

    #[test]
    fn process_task_expands_variables_and_object_args() {
        let steps = plan(TASKS, "build", ws(), None).unwrap();
        assert_eq!(
            steps,
            [TaskStep {
                label: "build".into(),
                command: TaskCommand::Process {
                    program: "dotnet".into(),
                    args: vec!["build".into(), "/ws/App.sln".into(), "-v q".into()],
                },
                cwd: None,
                env: vec![],
            }]
        );
    }

    #[test]
    fn depends_on_runs_first_and_shell_args_are_quoted() {
        let steps = plan(TASKS, "all", ws(), None).unwrap();
        let labels: Vec<&str> = steps.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["restore", "build"], "grouping task has no command");
        assert_eq!(
            steps[0].command,
            TaskCommand::Shell("dotnet restore \"my dir\"".into())
        );
    }

    #[test]
    fn npm_task_runs_its_script_with_options() {
        let steps = plan(TASKS, "web", ws(), None).unwrap();
        assert_eq!(steps[0].command, TaskCommand::Shell("npm run build".into()));
        assert_eq!(steps[0].cwd.as_deref(), Some("/ws/web"));
        assert_eq!(steps[0].env, [("A".to_string(), "1".to_string())]);
    }

    #[test]
    fn unsupported_missing_and_cyclic_tasks_are_errors() {
        let e = plan(TASKS, "docker", ws(), None).unwrap_err();
        assert!(e.contains("docker-build"), "{e}");
        let e = plan(TASKS, "nope", ws(), None).unwrap_err();
        assert!(e.contains("`nope` not found"), "{e}");
        let e = plan(TASKS, "loop", ws(), None).unwrap_err();
        assert!(e.contains("depends on itself"), "{e}");
        assert!(plan("{", "x", ws(), None).is_err());
    }

    #[test]
    fn os_override_replaces_command() {
        let steps = plan(TASKS, "os", ws(), None).unwrap();
        let expected = if cfg!(windows) { "win" } else { "unix" };
        assert_eq!(steps[0].command, TaskCommand::Shell(expected.into()));
    }

    #[test]
    fn plan_for_workspace_reads_vscode_tasks() {
        let dir = tempfile::tempdir().unwrap();
        assert!(plan_for_workspace(dir.path(), "build", None)
            .unwrap_err()
            .contains("cannot read"));
        std::fs::create_dir_all(dir.path().join(".vscode")).unwrap();
        std::fs::write(tasks_json_path(dir.path()), TASKS).unwrap();
        let steps = plan_for_workspace(dir.path(), "build", None).unwrap();
        assert_eq!(steps.len(), 1);
    }
}
