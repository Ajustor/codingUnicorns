//! Work done before a debug session starts, off the UI thread: download the
//! debug adapter when needed, run the `preLaunchTask` (e.g. a build), then
//! start the adapter and connect to it.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};

use super::adapters::DebugAdapter;
use super::transport::DapTransport;
use super::types::DapConfig;
use crate::process_ext::CommandExt as _;
use crate::runner::vscode_tasks::{TaskCommand, TaskStep};

/// Everything needed to start a debug session.
pub struct LaunchPlan {
    /// Adapter and launch arguments. With `adapter` set, `adapter_cmd` is
    /// filled in once the module's adapter is located (or downloaded).
    pub config: DapConfig,
    pub adapter: Option<DebugAdapter>,
    /// Tasks to run first, in order.
    pub tasks: Vec<TaskStep>,
    pub workspace: PathBuf,
}

pub enum LaunchEvent {
    /// A line for the debug output (progress, task output).
    Log(String),
    /// Preparation done: the adapter is started and connected.
    Ready(Box<(DapConfig, DapTransport)>),
    Failed(String),
}

/// A preparation running in the background.
pub struct Preparing {
    pub events: Receiver<LaunchEvent>,
    cancel: Arc<AtomicBool>,
}

impl Preparing {
    /// Stop the preparation; a running task is killed.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Start preparing `plan` on a background thread.
pub fn prepare(plan: LaunchPlan) -> Preparing {
    let (tx, rx) = unbounded();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    std::thread::Builder::new()
        .name("debug-launch".into())
        .spawn(move || {
            let event = match run(plan, &tx, &flag) {
                Ok(ready) => LaunchEvent::Ready(Box::new(ready)),
                Err(e) => LaunchEvent::Failed(e),
            };
            if !flag.load(Ordering::Relaxed) {
                let _ = tx.send(event);
            }
        })
        .expect("spawn debug-launch thread");
    Preparing { events: rx, cancel }
}

fn run(
    plan: LaunchPlan,
    tx: &Sender<LaunchEvent>,
    cancel: &AtomicBool,
) -> Result<(DapConfig, DapTransport), String> {
    let mut config = plan.config;
    if let Some(adapter) = &plan.adapter {
        if adapter.needs_download() {
            let _ = tx.send(LaunchEvent::Log(format!(
                "Downloading the {} debugger (first use)…",
                adapter.name()
            )));
            let p = adapter
                .download()
                .map_err(|e| format!("could not download {}: {e}", adapter.name()))?;
            let _ = tx.send(LaunchEvent::Log(format!("Installed {}", p.display())));
        }
        let program = adapter.locate().ok_or_else(|| adapter.missing_message())?;
        config.adapter_cmd = program.to_string_lossy().into_owned();
    }
    for task in &plan.tasks {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        run_task(task, &plan.workspace, tx, cancel)?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    let transport = DapTransport::open(&config, &plan.workspace)
        .map_err(|e| format!("could not start the debugger `{}`: {e}", config.adapter_cmd))?;
    Ok((config, transport))
}

fn task_command(task: &TaskStep) -> Command {
    match &task.command {
        TaskCommand::Process { program, args } => {
            let mut cmd = Command::new(program);
            cmd.args(args);
            cmd
        }
        TaskCommand::Shell(line) => shell_command(line),
    }
}

#[cfg(windows)]
fn shell_command(line: &str) -> Command {
    use std::os::windows::process::CommandExt as _;
    let mut cmd = Command::new("cmd");
    // Passed verbatim: cmd does its own quote parsing.
    cmd.raw_arg("/C").raw_arg(line);
    cmd
}

#[cfg(not(windows))]
fn shell_command(line: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(line);
    cmd
}

fn describe(task: &TaskStep) -> String {
    match &task.command {
        TaskCommand::Process { program, args } => std::iter::once(program.as_str())
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        TaskCommand::Shell(line) => line.clone(),
    }
}

/// Run one task, streaming its output; an error when it fails.
fn run_task(
    task: &TaskStep,
    workspace: &std::path::Path,
    tx: &Sender<LaunchEvent>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let _ = tx.send(LaunchEvent::Log(format!(
        "> Task `{}`: {}",
        task.label,
        describe(task)
    )));
    let mut cmd = task_command(task);
    cmd.no_window()
        .current_dir(
            task.cwd
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| workspace.to_path_buf()),
        )
        .envs(task.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("task `{}` could not start: {e}", task.label))?;
    let readers: Vec<_> = [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(|stream| {
        let tx = tx.clone();
        std::thread::spawn(move || forward_lines(stream, &tx))
    })
    .collect();
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            kill_tree(&mut child);
            return Err("cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("task `{}`: {e}", task.label)),
        }
    };
    for r in readers {
        let _ = r.join();
    }
    if status.success() {
        Ok(())
    } else {
        Err(format!("task `{}` failed ({status})", task.label))
    }
}

/// Kill a task and the processes it started (a shell task runs its command
/// as a child of `cmd`/`sh`).
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .no_window()
            .args(["/F", "/T", "/PID", &child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn forward_lines(stream: Box<dyn Read + Send>, tx: &Sender<LaunchEvent>) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    while matches!(reader.read_until(b'\n', &mut buf), Ok(n) if n > 0) {
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim_end();
        if !line.is_empty() {
            let _ = tx.send(LaunchEvent::Log(line.to_string()));
        }
        buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// An adapter that starts and exits right away.
    fn plan(tasks: Vec<TaskStep>, dir: &std::path::Path) -> LaunchPlan {
        #[cfg(windows)]
        let (cmd, args) = ("cmd", vec!["/C".to_string(), "exit 0".to_string()]);
        #[cfg(not(windows))]
        let (cmd, args) = ("sh", vec!["-c".to_string(), "exit 0".to_string()]);
        LaunchPlan {
            config: DapConfig {
                transport: Default::default(),
                adapter_cmd: cmd.into(),
                adapter_args: args,
                launch_config: json!({}),
            },
            adapter: None,
            tasks,
            workspace: dir.to_path_buf(),
        }
    }

    fn shell(label: &str, line: &str) -> TaskStep {
        TaskStep {
            label: label.into(),
            command: TaskCommand::Shell(line.into()),
            cwd: None,
            env: vec![("CU_TEST_VAR".into(), "hello".into())],
        }
    }

    /// Collect events until the preparation ends.
    fn finish(p: Preparing) -> (Vec<String>, Result<DapConfig, String>) {
        let mut logs = vec![];
        loop {
            match p.events.recv_timeout(Duration::from_secs(30)).unwrap() {
                LaunchEvent::Log(l) => logs.push(l),
                LaunchEvent::Ready(ready) => return (logs, Ok(ready.0)),
                LaunchEvent::Failed(e) => return (logs, Err(e)),
            }
        }
    }

    #[test]
    fn tasks_run_in_order_with_env_then_ready() {
        let dir = tempfile::tempdir().unwrap();
        let echo = if cfg!(windows) {
            "echo %CU_TEST_VAR%"
        } else {
            "echo $CU_TEST_VAR"
        };
        let (logs, res) = finish(prepare(plan(
            vec![shell("first", echo), shell("second", "echo two")],
            dir.path(),
        )));
        assert!(res.is_ok());
        let first = logs.iter().position(|l| l == "hello").expect("env applied");
        let second = logs.iter().position(|l| l == "two").unwrap();
        assert!(logs[0].starts_with("> Task `first`"));
        assert!(first < second);
    }

    #[test]
    fn failing_task_stops_the_launch() {
        let dir = tempfile::tempdir().unwrap();
        let (_logs, res) = finish(prepare(plan(
            vec![shell("build", "exit 3"), shell("never", "echo no")],
            dir.path(),
        )));
        let err = res.unwrap_err();
        assert!(err.contains("task `build` failed"), "{err}");
    }

    #[test]
    fn missing_adapter_without_download_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let module = super::super::adapters::tests::module(
            dir.path(),
            "rust",
            r#"["rs"]"#,
            "[debugger]
command = \"definitely-not-lldb-dap\"
install_hint = \"install LLVM\"",
        );
        let mut p = plan(vec![], dir.path());
        p.adapter = DebugAdapter::find(&[module], None, "rs");
        let (_logs, res) = finish(prepare(p));
        let err = res.unwrap_err();
        assert!(
            err.contains("`definitely-not-lldb-dap`") && err.contains("install LLVM"),
            "{err}"
        );
    }

    #[test]
    fn adapter_that_cannot_start_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = plan(vec![], dir.path());
        p.config.adapter_cmd = "definitely-not-a-real-adapter-xyz".into();
        let (_logs, res) = finish(prepare(p));
        let err = res.unwrap_err();
        assert!(err.contains("could not start the debugger"), "{err}");
    }

    #[test]
    fn cancel_kills_a_running_task() {
        let dir = tempfile::tempdir().unwrap();
        let sleep = if cfg!(windows) {
            "ping -n 30 127.0.0.1"
        } else {
            "sleep 30"
        };
        let p = prepare(plan(vec![shell("slow", sleep)], dir.path()));
        std::thread::sleep(Duration::from_millis(200));
        let start = std::time::Instant::now();
        p.cancel();
        // A cancelled preparation sends no final event; the channel closes.
        while let Ok(ev) = p.events.recv_timeout(Duration::from_secs(10)) {
            assert!(matches!(ev, LaunchEvent::Log(_)));
        }
        assert!(start.elapsed() < Duration::from_secs(10));
    }
}
