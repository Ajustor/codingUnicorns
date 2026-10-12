//! Debug sessions started by a language server (`transport = "lsp"`), as
//! Java's: the launcher runs the module's `lsp_resolve` commands to complete
//! the launch arguments (main class, classpath…), then the debugger
//! `command`, which starts the session and returns its port, and connects.
//!
//! The language server clients live on the UI thread, so each command is
//! sent there as a [`LaunchEvent::LspCommand`] and its answer awaited.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, RecvTimeoutError, Sender};
use serde_json::{json, Value};

use super::adapters::DebugAdapter;
use super::launcher::LaunchEvent;
use super::transport::DapTransport;
use crate::extension::manifest::LspResolveStep;

/// How long a command may wait for its answer (the server may be starting
/// and importing the project first).
const ANSWER_TIMEOUT: Duration = Duration::from_secs(300);
/// How long an empty `pick` result is retried while the project loads.
const PROJECT_WAIT: Duration = Duration::from_secs(120);

/// A language server command the launcher needs run.
pub struct LspCommand {
    /// Language (file extension) whose server runs it.
    pub language: String,
    pub command: String,
    pub args: Vec<Value>,
    /// The command's `result`, or why it failed.
    pub reply: Sender<Result<Value, String>>,
}

/// What the launcher knows when it resolves the launch arguments.
pub struct Context<'a> {
    pub language: &'a str,
    pub file: Option<&'a Path>,
    pub workspace: &'a Path,
}

/// Resolve the launch arguments, start the session and connect to it.
pub fn start(
    adapter: &DebugAdapter,
    ctx: &Context<'_>,
    launch: &mut Value,
    tx: &Sender<LaunchEvent>,
    cancel: &AtomicBool,
) -> Result<DapTransport, String> {
    if !launch.is_object() {
        *launch = json!({});
    }
    for step in &adapter.spec.lsp_resolve {
        if step
            .unless
            .as_ref()
            .is_some_and(|k| launch.get(k).is_some_and(|v| !v.is_null()))
        {
            continue;
        }
        let args = expand_args(&step.args, ctx, launch);
        let started = Instant::now();
        let mut waiting_said = false;
        loop {
            let result = ask(tx, cancel, ctx.language, &step.command, args.clone())?;
            if apply(step, &result, launch, ctx.file) {
                break;
            }
            if started.elapsed() > PROJECT_WAIT {
                return Err(format!("`{}` found nothing to debug", step.command));
            }
            if !waiting_said {
                waiting_said = true;
                let _ = tx.send(LaunchEvent::Log(
                    "Waiting for the language server to load the project…".into(),
                ));
            }
            wait(cancel, Duration::from_secs(2))?;
        }
    }
    let command = adapter
        .spec
        .command
        .as_slice()
        .first()
        .ok_or("the module's debugger has no command")?;
    let result = ask(tx, cancel, ctx.language, command, vec![])?;
    let port = result
        .as_u64()
        .or_else(|| result.as_str().and_then(|s| s.parse().ok()))
        .and_then(|p| u16::try_from(p).ok())
        .ok_or_else(|| format!("`{command}` returned no port: {result}"))?;
    DapTransport::connect(port).map_err(|e| format!("could not connect to the debugger: {e}"))
}

/// Send `command` to the UI thread and wait for its answer.
fn ask(
    tx: &Sender<LaunchEvent>,
    cancel: &AtomicBool,
    language: &str,
    command: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    let (reply, answer) = bounded(1);
    tx.send(LaunchEvent::LspCommand(LspCommand {
        language: language.to_string(),
        command: command.to_string(),
        args,
        reply,
    }))
    .map_err(|_| "cancelled".to_string())?;
    let asked = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        match answer.recv_timeout(Duration::from_millis(200)) {
            Ok(result) => return result.map_err(|e| format!("`{command}`: {e}")),
            Err(RecvTimeoutError::Timeout) if asked.elapsed() < ANSWER_TIMEOUT => {}
            Err(RecvTimeoutError::Timeout) => {
                return Err(format!("no answer from the language server to `{command}`"))
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(format!("`{command}` was not sent to a language server"))
            }
        }
    }
}

/// Sleep `d`, unless cancelled.
fn wait(cancel: &AtomicBool, d: Duration) -> Result<(), String> {
    let end = Instant::now() + d;
    while Instant::now() < end {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// The step's arguments as JSON, variables expanded.
fn expand_args(args: &[toml::Value], ctx: &Context<'_>, launch: &Value) -> Vec<Value> {
    args.iter()
        .map(|a| {
            let mut v = serde_json::to_value(a).unwrap_or(Value::Null);
            expand(&mut v, ctx, launch);
            v
        })
        .collect()
}

fn expand(v: &mut Value, ctx: &Context<'_>, launch: &Value) {
    match v {
        Value::String(s) => {
            // A whole `${launch:key}` keeps the value's type (a list…).
            if let Some(key) = s
                .strip_prefix("${launch:")
                .and_then(|k| k.strip_suffix('}'))
            {
                if !key.contains('}') {
                    *v = launch.get(key).cloned().unwrap_or(Value::Null);
                    return;
                }
            }
            let file = ctx.file.map(|f| f.to_string_lossy().into_owned());
            let ws = ctx.workspace.to_string_lossy().into_owned();
            let mut out = s
                .replace(
                    "${fileUri}",
                    &ctx.file
                        .map(crate::lsp::client::path_to_uri)
                        .unwrap_or_default(),
                )
                .replace("${file}", file.as_deref().unwrap_or(""))
                .replace(
                    "${workspaceFolderUri}",
                    &crate::lsp::client::path_to_uri(ctx.workspace),
                )
                .replace("${workspaceFolder}", &ws);
            while let Some(start) = out.find("${launch:") {
                let Some(len) = out[start..].find('}') else {
                    break;
                };
                let key = &out[start + 9..start + len];
                let value = match launch.get(key) {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(other) => other.to_string(),
                };
                out.replace_range(start..start + len + 1, &value);
            }
            *s = out;
        }
        Value::Array(items) => items.iter_mut().for_each(|i| expand(i, ctx, launch)),
        Value::Object(map) => map.values_mut().for_each(|i| expand(i, ctx, launch)),
        _ => {}
    }
}

/// Fill `launch` from a step's result. False when a `pick` step found
/// nothing (the server may still be loading the project).
fn apply(step: &LspResolveStep, result: &Value, launch: &mut Value, file: Option<&Path>) -> bool {
    let value = match &step.pick {
        Some(field) => {
            let Some(items) = result.as_array().filter(|a| !a.is_empty()) else {
                return false;
            };
            items
                .iter()
                .find(|it| it[field].as_str().is_some_and(|p| same_file(p, file)))
                .unwrap_or(&items[0])
                .clone()
        }
        None => result.clone(),
    };
    for key in &step.merge {
        if let Some(v) = value.get(key).filter(|v| !v.is_null()) {
            launch[key] = v.clone();
        }
    }
    match step.set.as_slice() {
        [] => {}
        [key] => launch[key] = value,
        keys => {
            for (i, key) in keys.iter().enumerate() {
                launch[key] = value.get(i).cloned().unwrap_or(Value::Null);
            }
        }
    }
    true
}

/// Whether `path` (from the server) is `file`.
fn same_file(path: &str, file: Option<&Path>) -> bool {
    let Some(file) = file else {
        return false;
    };
    let path = Path::new(path);
    if path == file {
        return true;
    }
    match (std::fs::canonicalize(path), std::fs::canonicalize(file)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(toml_src: &str) -> LspResolveStep {
        toml::from_str(toml_src).unwrap()
    }

    fn ctx<'a>(file: Option<&'a Path>, ws: &'a Path) -> Context<'a> {
        Context {
            language: "java",
            file,
            workspace: ws,
        }
    }

    #[test]
    fn args_expand_files_workspace_and_launch_values() {
        let ws = Path::new("/ws");
        let file = Path::new("/ws/A.java");
        let launch = json!({"mainClass": "a.A", "paths": ["x", "y"], "n": 3});
        let args: Vec<toml::Value> = toml::from_str::<toml::Table>(
            r#"a = ["${file}", "${workspaceFolder}", "${launch:mainClass}", "${launch:paths}", "m=${launch:mainClass};n=${launch:n};${launch:missing}", { k = "${launch:mainClass}" }]"#,
        )
        .unwrap()["a"]
            .as_array()
            .unwrap()
            .clone();
        let out = expand_args(&args, &ctx(Some(file), ws), &launch);
        assert_eq!(out[0], file.to_string_lossy().as_ref());
        assert_eq!(out[1], ws.to_string_lossy().as_ref());
        assert_eq!(out[2], "a.A");
        assert_eq!(out[3], json!(["x", "y"]), "whole value keeps its type");
        assert_eq!(out[4], "m=a.A;n=3;");
        assert_eq!(out[5], json!({"k": "a.A"}));

        let uris: Vec<toml::Value> = vec!["${fileUri}".into(), "${workspaceFolderUri}".into()];
        let out = expand_args(&uris, &ctx(Some(file), ws), &launch);
        assert!(out[0].as_str().unwrap().starts_with("file://"));
        assert!(out[1].as_str().unwrap().starts_with("file://"));
    }

    #[test]
    fn pick_takes_the_current_file_else_the_first() {
        let s = step(
            r#"command = "c"
pick = "filePath"
merge = ["mainClass", "projectName"]"#,
        );
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("A.java");
        let b = dir.path().join("B.java");
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        let result = json!([
            {"mainClass": "A", "projectName": "p", "filePath": a.to_string_lossy()},
            {"mainClass": "B", "projectName": "p", "filePath": b.to_string_lossy()},
        ]);
        let mut launch = json!({"cwd": "x"});
        assert!(apply(&s, &result, &mut launch, Some(&b)));
        assert_eq!(
            launch,
            json!({"cwd": "x", "mainClass": "B", "projectName": "p"})
        );

        let mut launch = json!({});
        assert!(apply(&s, &result, &mut launch, Some(Path::new("/other"))));
        assert_eq!(launch["mainClass"], "A");

        assert!(!apply(&s, &json!([]), &mut launch, None), "empty: retry");
        assert!(!apply(&s, &Value::Null, &mut launch, None));
    }

    #[test]
    fn set_takes_the_whole_result_or_list_items() {
        let mut launch = json!({});
        let one = step("command = \"c\"\nset = [\"javaExec\"]");
        assert!(apply(&one, &json!("/jdk/bin/java"), &mut launch, None));
        let many = step("command = \"c\"\nset = [\"modulePaths\", \"classPaths\", \"extra\"]");
        assert!(apply(
            &many,
            &json!([["m"], ["c1", "c2"]]),
            &mut launch,
            None
        ));
        assert_eq!(
            launch,
            json!({
                "javaExec": "/jdk/bin/java",
                "modulePaths": ["m"],
                "classPaths": ["c1", "c2"],
                "extra": null
            })
        );
    }

    /// The launcher side of `start`, with a fake UI thread answering.
    #[test]
    fn start_runs_the_steps_then_connects_to_the_returned_port() {
        use crate::extension::manifest::ExtensionManifest;
        let manifest = ExtensionManifest::parse(
            r#"[extension]
id = "acme.java"
name = "Java"
version = "1.0.0"
description = ""
author = ""

[capabilities]
languages = ["java"]

[debugger]
types = ["java"]
command = "start"
transport = "lsp"

[[debugger.lsp_resolve]]
command = "main"
pick = "filePath"
merge = ["mainClass"]
unless = "mainClass"

[[debugger.lsp_resolve]]
command = "cp"
args = ["${launch:mainClass}"]
set = ["classPaths"]
"#,
        )
        .unwrap();
        let adapter = DebugAdapter {
            module: "Java".into(),
            spec: manifest.debugger.unwrap(),
            dir: std::path::PathBuf::new(),
            languages: vec!["java".into()],
        };
        // A DAP "adapter" accepting the connection.
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let _conn = listener.accept();
            std::thread::sleep(Duration::from_secs(2));
        });

        let (tx, rx) = crossbeam_channel::unbounded();
        let ui = std::thread::spawn(move || {
            let mut seen = vec![];
            let mut main_calls = 0;
            while let Ok(event) = rx.recv_timeout(Duration::from_secs(20)) {
                let LaunchEvent::LspCommand(c) = event else {
                    continue;
                };
                seen.push((c.command.clone(), c.args.clone()));
                let answer = match c.command.as_str() {
                    // Still loading: an empty list is retried.
                    "main" if main_calls == 0 => {
                        main_calls += 1;
                        json!([])
                    }
                    "main" => json!([{"mainClass": "p.Main", "filePath": "/x"}]),
                    "cp" => json!(["/out"]),
                    "start" => json!(port),
                    _ => Value::Null,
                };
                let _ = c.reply.send(Ok(answer));
                if c.command == "start" {
                    break;
                }
            }
            seen
        });
        let cancel = AtomicBool::new(false);
        let ws = std::env::temp_dir();
        let mut launch = json!({"request": "launch"});
        let transport = start(&adapter, &ctx(None, &ws), &mut launch, &tx, &cancel);
        drop(tx);
        assert!(transport.is_ok(), "{:?}", transport.err());
        assert_eq!(launch["mainClass"], "p.Main");
        assert_eq!(launch["classPaths"], json!(["/out"]));
        let seen = ui.join().unwrap();
        let names: Vec<&str> = seen.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(names, ["main", "main", "cp", "start"]);
        assert_eq!(seen[2].1, vec![json!("p.Main")]);
    }

    #[test]
    fn errors_and_cancellation_are_reported() {
        let (tx, rx) = crossbeam_channel::unbounded();
        std::thread::spawn(move || {
            while let Ok(e) = rx.recv() {
                if let LaunchEvent::LspCommand(c) = e {
                    let _ = c.reply.send(Err("no server".into()));
                }
            }
        });
        let cancel = AtomicBool::new(false);
        let err = ask(&tx, &cancel, "java", "x", vec![]).unwrap_err();
        assert_eq!(err, "`x`: no server");

        cancel.store(true, Ordering::Relaxed);
        let (tx2, _rx2) = crossbeam_channel::unbounded();
        assert_eq!(
            ask(&tx2, &cancel, "java", "x", vec![]).unwrap_err(),
            "cancelled"
        );
    }
}
