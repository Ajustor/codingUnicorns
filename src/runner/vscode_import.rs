//! Import de configurations de lancement depuis les fichiers `.vscode`
//! (`launch.json`, `tasks.json`). Fonctions pures, testables sans I/O.

use crate::runner::RunConfig;
use serde_json::Value;
use std::path::Path;

/// Nettoie le JSONC (commentaires ligne/bloc, virgules traînantes) pour que
/// `serde_json` puisse parser. Conscient des chaînes : le contenu entre `"`
/// (avec échappement `\"`) est préservé verbatim.
pub fn strip_jsonc(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push('"');
            }
            '/' if chars.peek() == Some(&'/') => {
                chars.next(); // consomme le second '/'
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next(); // consomme '*'
                let mut prev = '\0';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }

    strip_trailing_commas(&out)
}

/// Retire les virgules suivies (après espaces) d'un `}` ou `]`. String-aware.
fn strip_trailing_commas(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;

    for (idx, &c) in chars.iter().enumerate() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push(c);
            continue;
        }
        if c == ',' {
            let mut j = idx + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                continue; // virgule traînante : on la saute
            }
        }
        out.push(c);
    }
    out
}

/// Traduit les variables VSCode vers celles du runner.
fn translate_vars(s: &str) -> String {
    s.replace("${workspaceFolder}", "${workspaceRoot}")
        .replace("${fileDirname}", "${fileDir}")
        .replace("${fileBasenameNoExtension}", "${fileName}")
    // ${file} est identique dans les deux conventions.
}

fn string_array(v: Option<&Value>) -> Vec<String> {
    v.and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(translate_vars)
                .collect()
        })
        .unwrap_or_default()
}

fn env_pairs(v: Option<&Value>) -> Vec<(String, String)> {
    v.and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .map(|(k, val)| (k.clone(), val.as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Parse la racine d'un `tasks.json` en `RunConfig`.
pub fn parse_tasks(json: &Value) -> Vec<RunConfig> {
    let Some(tasks) = json.get("tasks").and_then(|t| t.as_array()) else {
        return Vec::new();
    };
    let mut configs = Vec::new();
    for task in tasks {
        let Some(label) = task.get("label").and_then(|v| v.as_str()) else {
            continue;
        };
        let command = task.get("command").and_then(|v| v.as_str()).unwrap_or("");
        if command.is_empty() {
            continue;
        }
        let options = task.get("options");
        configs.push(RunConfig {
            name: label.to_string(),
            command: translate_vars(command),
            cwd: options
                .and_then(|o| o.get("cwd"))
                .and_then(|v| v.as_str())
                .map(translate_vars)
                .unwrap_or_else(|| "${workspaceRoot}".to_string()),
            env: env_pairs(options.and_then(|o| o.get("env"))),
            args: string_array(task.get("args")),
        });
    }
    configs
}

/// Spec de débogage dérivée d'une entrée `launch.json` (type débogable connu).
#[derive(Debug, Clone, PartialEq)]
pub struct DebugSpec {
    pub adapter_cmd: String,
    pub adapter_args: Vec<String>,
    /// Raw `launch.json` entry passed verbatim to the DAP adapter.
    /// VSCode variables inside it (e.g. `${workspaceFolder}`) are intentionally
    /// NOT translated here — the adapter / DapClient resolves them itself.
    pub launch_config: Value,
}

/// Commande terminal synthétisée selon le `type` VSCode.
/// `None` => entrée non lançable (ignorée).
fn synth_command(typ: &str, program: Option<&str>) -> Option<String> {
    match typ {
        "python" | "debugpy" => Some(format!("python {}", program.unwrap_or("${file}"))),
        "node" | "pwa-node" => Some(format!("node {}", program.unwrap_or("${file}"))),
        "go" => Some(format!("go run {}", program.unwrap_or("."))),
        // Types compilés / divers : on exécute directement le programme s'il existe.
        _ => program.map(|p| p.to_string()),
    }
}

/// Adaptateur DAP pour un `type` débogable connu, sinon `None`.
/// Le `type` `node` est volontairement omis (adaptateur js-debug rarement
/// disponible en standalone ; l'entrée reste lançable en terminal).
fn adapter_for_type(typ: &str) -> Option<(&'static str, Vec<&'static str>)> {
    match typ {
        "python" | "debugpy" => Some(("python", vec!["-m", "debugpy.adapter"])),
        "go" => Some(("dlv", vec!["dap"])),
        "lldb" | "cppdbg" => Some(("codelldb", vec![])),
        "coreclr" => Some(("netcoredbg", vec!["--interpreter=vscode"])),
        _ => None,
    }
}

/// Parse la racine d'un `launch.json`. Retourne (RunConfigs terminal,
/// (nom, DebugSpec) pour les types débogables).
pub fn parse_launch(json: &Value) -> (Vec<RunConfig>, Vec<(String, DebugSpec)>) {
    let Some(arr) = json.get("configurations").and_then(|c| c.as_array()) else {
        return (Vec::new(), Vec::new());
    };
    let mut configs = Vec::new();
    let mut specs = Vec::new();
    for cfg in arr {
        let name = cfg.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let typ = cfg.get("type").and_then(|v| v.as_str()).unwrap_or("");
        // `program` is pre-translated so `synth_command` receives an already-translated path.
        let program = cfg.get("program").and_then(|v| v.as_str()).map(translate_vars);

        let Some(command) = synth_command(typ, program.as_deref()) else {
            continue;
        };
        configs.push(RunConfig {
            name: name.to_string(),
            command,
            cwd: cfg
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(translate_vars)
                .unwrap_or_else(|| "${workspaceRoot}".to_string()),
            env: env_pairs(cfg.get("env")),
            args: string_array(cfg.get("args")),
        });

        if let Some((adapter_cmd, adapter_args)) = adapter_for_type(typ) {
            specs.push((
                name.to_string(),
                DebugSpec {
                    adapter_cmd: adapter_cmd.to_string(),
                    adapter_args: adapter_args.iter().map(|s| s.to_string()).collect(),
                    launch_config: cfg.clone(),
                },
            ));
        }
    }
    (configs, specs)
}

/// Résultat de l'import du répertoire `.vscode` d'un workspace.
#[derive(Debug, Default)]
pub struct VscodeImport {
    pub configs: Vec<RunConfig>,
    /// (nom de config, spec) — clé = `RunConfig.name`.
    pub debug_specs: Vec<(String, DebugSpec)>,
}

/// Lit + nettoie un fichier JSONC. `None` si absent ou illisible/invalide.
fn read_jsonc(path: &Path) -> Option<Value> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&strip_jsonc(&content)).ok()
}

/// Importe `.vscode/tasks.json` et `.vscode/launch.json` d'un workspace.
/// Tolérant : un fichier absent ou invalide est simplement ignoré.
pub fn import_vscode(workspace: &Path) -> VscodeImport {
    let dir = workspace.join(".vscode");
    let mut result = VscodeImport::default();

    if let Some(json) = read_jsonc(&dir.join("tasks.json")) {
        result.configs.extend(parse_tasks(&json));
    }
    if let Some(json) = read_jsonc(&dir.join("launch.json")) {
        let (configs, specs) = parse_launch(&json);
        result.configs.extend(configs);
        result.debug_specs.extend(specs);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_line_and_block_comments() {
        let src = "{\n  // ligne\n  \"a\": 1, /* bloc */ \"b\": 2\n}";
        let cleaned = strip_jsonc(src);
        let v: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
        assert_eq!(v["a"], 1);
        assert_eq!(v["b"], 2);
    }

    #[test]
    fn strips_trailing_commas() {
        let src = "{ \"a\": [1, 2,], \"b\": 3, }";
        let cleaned = strip_jsonc(src);
        let v: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
        assert_eq!(v["a"], serde_json::json!([1, 2]));
        assert_eq!(v["b"], 3);
    }

    #[test]
    fn preserves_slashes_and_commas_inside_strings() {
        let src = "{ \"url\": \"http://x//y\", \"csv\": \"a,b,\" }";
        let cleaned = strip_jsonc(src);
        let v: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
        assert_eq!(v["url"], "http://x//y");
        assert_eq!(v["csv"], "a,b,");
    }

    #[test]
    fn parses_tasks_with_var_translation() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{ "tasks": [
                { "label": "Build", "type": "shell", "command": "make",
                  "args": ["-C", "${workspaceFolder}/sub"],
                  "options": { "cwd": "${workspaceFolder}", "env": { "K": "v" } } },
                { "label": "no command" }
            ] }"#,
        ).unwrap();
        let configs = parse_tasks(&json);
        assert_eq!(configs.len(), 1);
        let c = &configs[0];
        assert_eq!(c.name, "Build");
        assert_eq!(c.command, "make");
        assert_eq!(c.args, vec!["-C", "${workspaceRoot}/sub"]);
        assert_eq!(c.cwd, "${workspaceRoot}");
        assert_eq!(c.env, vec![("K".to_string(), "v".to_string())]);
    }

    #[test]
    fn launch_python_yields_runconfig_and_debugspec() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{ "configurations": [
                { "name": "Dbg", "type": "python", "request": "launch",
                  "program": "${workspaceFolder}/app.py", "args": ["--x"] }
            ] }"#,
        ).unwrap();
        let (configs, specs) = parse_launch(&json);
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "Dbg");
        assert_eq!(configs[0].command, "python ${workspaceRoot}/app.py");
        assert_eq!(configs[0].args, vec!["--x"]);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].0, "Dbg");
        assert_eq!(specs[0].1.adapter_cmd, "python");
        assert_eq!(specs[0].1.adapter_args, vec!["-m", "debugpy.adapter"]);
    }

    #[test]
    fn launch_unknown_type_with_program_runs_but_no_debugspec() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{ "configurations": [
                { "name": "Exotic", "type": "ruby", "program": "main.rb" }
            ] }"#,
        ).unwrap();
        let (configs, specs) = parse_launch(&json);
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].command, "main.rb");
        assert!(specs.is_empty());
    }

    #[test]
    fn launch_unknown_type_without_program_is_skipped() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{ "configurations": [ { "name": "X", "type": "ruby" } ] }"#)
                .unwrap();
        let (configs, specs) = parse_launch(&json);
        assert!(configs.is_empty());
        assert!(specs.is_empty());
    }
}
