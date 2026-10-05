//! Import of VS Code `.vscode/launch.json` debug configurations.
//!
//! Only used when the workspace has no `.coding-unicorns/launch.toml`. Each
//! configuration becomes a [`RunConfig`] whose terminal command is derived
//! from `type`/`program`/`args`, and whose [`DebugLaunch`] carries the whole
//! entry (unknown keys included) as the DAP `launch`/`attach` arguments.

use std::path::Path;

use serde_json::Value;

use super::{DebugLaunch, RunConfig};

/// Path of the VS Code launch file inside a workspace.
pub fn launch_json_path(workspace: &Path) -> std::path::PathBuf {
    workspace.join(".vscode").join("launch.json")
}

/// Read and map `.vscode/launch.json`. `None` when the file is missing or
/// unparsable.
pub fn load(workspace: &Path) -> Option<Vec<RunConfig>> {
    let content = std::fs::read_to_string(launch_json_path(workspace)).ok()?;
    parse_launch_json(&content).ok()
}

/// Parse a (JSONC) `launch.json` document into run configurations.
pub fn parse_launch_json(content: &str) -> Result<Vec<RunConfig>, serde_json::Error> {
    let root: Value = serde_json::from_str(&strip_jsonc(content))?;
    Ok(root["configurations"]
        .as_array()
        .map(|a| a.iter().filter_map(map_configuration).collect())
        .unwrap_or_default())
}

/// Map one `configurations[]` entry. Non-object entries are skipped.
pub fn map_configuration(entry: &Value) -> Option<RunConfig> {
    let obj = entry.as_object()?;
    let str_field = |k: &str| obj.get(k).and_then(|v| v.as_str());

    let adapter_type = str_field("type").unwrap_or("").to_string();
    let request = str_field("request").unwrap_or("launch").to_string();
    let name = str_field("name")
        .map(str::to_string)
        .unwrap_or_else(|| format!("{adapter_type} {request}").trim().to_string());
    let program = str_field("program");

    let args = match obj.get("args") {
        Some(Value::Array(a)) => a.iter().filter_map(scalar_to_string).collect(),
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        _ => vec![],
    };
    let env = obj
        .get("env")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| scalar_to_string(v).map(|v| (k.clone(), v)))
                .collect()
        })
        .unwrap_or_default();

    let mut launch_args = entry.clone();
    strip_nulls(&mut launch_args);

    Some(RunConfig {
        name,
        command: terminal_command(&adapter_type, &request, program),
        cwd: str_field("cwd").unwrap_or("${workspaceFolder}").to_string(),
        env,
        args,
        debug: Some(DebugLaunch {
            adapter_type,
            request,
            launch_args,
        }),
    })
}

/// Best-effort terminal equivalent of a debug configuration, used by "Run".
fn terminal_command(adapter_type: &str, request: &str, program: Option<&str>) -> String {
    if request == "attach" {
        // Attaching runs nothing; the process is started elsewhere.
        return String::new();
    }
    match adapter_type {
        "python" | "debugpy" => format!("python {}", program.unwrap_or("${file}")),
        "node" | "pwa-node" => format!("node {}", program.unwrap_or("${file}")),
        "go" => format!("go run {}", program.unwrap_or(".")),
        _ => program.unwrap_or("").to_string(),
    }
}

/// File extensions whose language plugin is likely to provide the adapter
/// for a VS Code debug `type` (tried in order).
pub fn extensions_for_type(adapter_type: &str) -> &'static [&'static str] {
    match adapter_type {
        "python" | "debugpy" => &["py"],
        "node" | "pwa-node" | "node-terminal" => &["js", "ts"],
        "go" => &["go"],
        "lldb" | "codelldb" => &["rs", "c", "cpp"],
        "cppdbg" | "cppvsdbg" => &["cpp", "c", "rs"],
        "coreclr" => &["cs"],
        _ => &[],
    }
}

fn scalar_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Remove `null` values recursively: they mean "unset" for DAP adapters and
/// cannot be represented in `launch.toml` when the configs are saved.
fn strip_nulls(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.retain(|_, v| !v.is_null());
            m.values_mut().for_each(strip_nulls);
        }
        Value::Array(a) => {
            a.retain(|v| !v.is_null());
            a.iter_mut().for_each(strip_nulls);
        }
        _ => {}
    }
}

/// Turn JSONC (as written by VS Code) into strict JSON: drops `//` and
/// `/* */` comments and trailing commas before `}`/`]`. String contents are
/// preserved verbatim (including `//` in URLs and escaped quotes).
pub fn strip_jsonc(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                // Copy the whole string literal, honouring escapes.
                out.push(c);
                i += 1;
                while i < chars.len() {
                    let s = chars[i];
                    out.push(s);
                    i += 1;
                    if s == '\\' {
                        if let Some(&n) = chars.get(i) {
                            out.push(n);
                            i += 1;
                        }
                    } else if s == '"' {
                        break;
                    }
                }
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i += 2;
                continue;
            }
            ',' => {
                // Trailing comma: drop it when the next significant char
                // (skipping whitespace and comments) closes the container.
                let mut j = i + 1;
                loop {
                    while j < chars.len() && chars[j].is_whitespace() {
                        j += 1;
                    }
                    if chars.get(j) == Some(&'/') && chars.get(j + 1) == Some(&'/') {
                        while j < chars.len() && chars[j] != '\n' {
                            j += 1;
                        }
                    } else if chars.get(j) == Some(&'/') && chars.get(j + 1) == Some(&'*') {
                        j += 2;
                        while j < chars.len()
                            && !(chars[j] == '*' && chars.get(j + 1) == Some(&'/'))
                        {
                            j += 1;
                        }
                        j += 2;
                    } else {
                        break;
                    }
                }
                if matches!(chars.get(j), Some('}') | Some(']')) {
                    i += 1;
                    continue;
                }
            }
            _ => {}
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strip_jsonc_removes_comments_and_trailing_commas() {
        let src = r#"{
            // line comment
            "version": "0.2.0", /* block */
            "url": "http://x//y", // keep the // inside strings
            "q": "a \" // not a comment",
            "list": [1, 2, /* c */ ],
            "obj": {"a": 1, // c
            },
        }"#;
        let v: Value = serde_json::from_str(&strip_jsonc(src)).unwrap();
        assert_eq!(v["version"], "0.2.0");
        assert_eq!(v["url"], "http://x//y");
        assert_eq!(v["q"], "a \" // not a comment");
        assert_eq!(v["list"], json!([1, 2]));
        assert_eq!(v["obj"], json!({"a": 1}));
    }

    #[test]
    fn strip_jsonc_keeps_commas_inside_strings_and_unterminated_input_is_safe() {
        assert_eq!(strip_jsonc(r#"["a,]", "b"]"#), r#"["a,]", "b"]"#);
        // Must not panic on truncated input.
        let _ = strip_jsonc("{\"a\": \"unterminated");
        let _ = strip_jsonc("/* never closed");
        let _ = strip_jsonc("[1,");
    }

    #[test]
    fn maps_a_python_launch_configuration() {
        let cfgs = parse_launch_json(
            r#"{
              "version": "0.2.0",
              "configurations": [
                {
                  "name": "Python: main",
                  "type": "debugpy",
                  "request": "launch",
                  "program": "${workspaceFolder}/main.py",
                  "args": ["--port", 8000, true],
                  "cwd": "${workspaceFolder}/app",
                  "env": {"DEBUG": "1", "N": 2, "UNSET": null},
                  "stopOnEntry": true,
                  "justMyCode": false, // unknown key: passed through
                  "console": null,
                },
              ],
            }"#,
        )
        .unwrap();
        assert_eq!(cfgs.len(), 1);
        let c = &cfgs[0];
        assert_eq!(c.name, "Python: main");
        assert_eq!(c.command, "python ${workspaceFolder}/main.py");
        assert_eq!(c.args, ["--port", "8000", "true"]);
        assert_eq!(c.cwd, "${workspaceFolder}/app");
        assert_eq!(
            c.env,
            vec![("DEBUG".into(), "1".into()), ("N".into(), "2".into())]
        );
        let d = c.debug.as_ref().unwrap();
        assert_eq!(d.adapter_type, "debugpy");
        assert_eq!(d.request, "launch");
        assert_eq!(d.launch_args["stopOnEntry"], true);
        assert_eq!(d.launch_args["justMyCode"], false);
        assert_eq!(d.launch_args["program"], "${workspaceFolder}/main.py");
        assert!(d.launch_args.get("console").is_none(), "nulls are dropped");
        assert!(d.launch_args["env"].get("UNSET").is_none());
    }

    #[test]
    fn substitutes_workspace_folder_when_resolved() {
        let c = map_configuration(&json!({
            "name": "n", "type": "node", "request": "launch",
            "program": "${workspaceFolder}/index.js"
        }))
        .unwrap();
        let r = c.resolve(Some(Path::new("ws")), None);
        assert_eq!(r.command, "node ws/index.js");
        assert_eq!(r.cwd, "ws", "cwd defaults to the workspace folder");
    }

    #[test]
    fn maps_defaults_attach_and_odd_entries() {
        let cfgs = parse_launch_json(
            r#"{"configurations": [
                {"type": "go", "request": "launch"},
                {"name": "Attach", "type": "debugpy", "request": "attach", "connect": {"port": 5678}},
                {"name": "Native", "type": "lldb", "program": "target/debug/app", "args": "--flag"},
                {"name": "Script", "type": "python"},
                42
            ]}"#,
        )
        .unwrap();
        assert_eq!(cfgs.len(), 4, "non-object entries are skipped");
        assert_eq!(cfgs[0].name, "go launch");
        assert_eq!(cfgs[0].command, "go run .");
        assert_eq!(cfgs[1].command, "", "attach runs nothing");
        let attach = cfgs[1].debug.as_ref().unwrap();
        assert_eq!(attach.request, "attach");
        assert_eq!(attach.launch_args["connect"]["port"], 5678);
        assert_eq!(cfgs[2].command, "target/debug/app");
        assert_eq!(cfgs[2].args, ["--flag"]);
        assert_eq!(cfgs[2].debug.as_ref().unwrap().request, "launch");
        assert_eq!(cfgs[3].command, "python ${file}");
    }

    #[test]
    fn missing_configurations_and_invalid_json() {
        assert!(parse_launch_json(r#"{"version": "0.2.0"}"#)
            .unwrap()
            .is_empty());
        assert!(parse_launch_json("{ nope").is_err());
    }

    #[test]
    fn extensions_for_known_types() {
        assert_eq!(extensions_for_type("debugpy"), ["py"]);
        assert_eq!(extensions_for_type("codelldb")[0], "rs");
        assert!(extensions_for_type("mystery").is_empty());
    }
}
