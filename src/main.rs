#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

mod app;
mod cli_link;
mod config;
pub mod dap;
mod editor;
pub mod extension;
mod filetree;
mod git;
mod keybinds;
mod language;
mod launch_log;
mod login_path;
mod lsp;
mod nav_history;
pub mod plugin;
mod process_ext;
pub mod runner;
mod single_instance;
mod tabs;
mod terminal;
mod ui;
mod updater;

use app::CodingUnicorns;

// On Windows, stop a child process (notably the integrated terminal's shell) from popping a
// blocking OS "Application Error" modal when its loader hits a critical error. The error mode is
// inherited by spawned children, so the terminal can surface failures in-panel instead.
#[cfg(windows)]
extern "system" {
    fn SetErrorMode(u_mode: u32) -> u32;
}

/// Path of the crash log: `<config_dir>/coding-unicorns/crash.log`.
fn crash_log_path() -> std::path::PathBuf {
    let mut path = dirs_next::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    path.push("coding-unicorns");
    path.push("crash.log");
    path
}

/// Install a panic hook that appends panic message + source location to a log file.
///
/// In release builds the window has no console (`windows_subsystem = "windows"`),
/// so panics would otherwise close the app silently. `info.location()` reports the
/// Rust source file/line of the panic and does NOT depend on debug symbols, so it
/// survives `strip = true`. A backtrace is also captured (set `RUST_BACKTRACE=1`),
/// though it is only symbolicated in non-stripped builds.
fn install_panic_logger() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());
        let message = match info.payload().downcast_ref::<&str>() {
            Some(s) => (*s).to_string(),
            None => match info.payload().downcast_ref::<String>() {
                Some(s) => s.clone(),
                None => "<non-string panic payload>".to_string(),
            },
        };
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backtrace = std::backtrace::Backtrace::force_capture();
        let entry = format!(
            "\n=== PANIC @ unix:{elapsed} ===\nlocation: {location}\nmessage: {message}\nbacktrace:\n{backtrace}\n"
        );

        let path = crash_log_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            use std::io::Write;
            let _ = f.write_all(entry.as_bytes());
        }

        // Preserve default behavior (stderr) for debug builds / when a console exists.
        default_hook(info);

        // Before the window: the app would just vanish. Say why (dialogs need the main
        // thread on macOS).
        if launch_log::is_started()
            && !launch_log::is_window_open()
            && std::thread::current().name() == Some("main")
        {
            launch_log::fatal(&format!("It crashed: {message} ({location})"));
        }
    }));
}

fn load_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../assets/icon.png");
    let rgba = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    let (width, height) = rgba.dimensions();
    Some(egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    })
}

/// Keeps the IDE in the foreground of the terminal that started it.
#[cfg(unix)]
const WAIT_FLAG: &str = "--wait";
/// Set on the background copy so it doesn't detach again.
#[cfg(unix)]
const DETACHED_ENV: &str = "CODING_UNICORNS_DETACHED";

/// Started from a terminal (`cu .`), relaunch in the background and let the
/// shell have its prompt back, like `code .`. Returns true when the caller
/// should exit. Windows needs nothing here: the app has no console, and the
/// `cu.cmd` wrapper goes through `start`.
#[cfg(unix)]
fn detach_from_terminal(args: &[String]) -> bool {
    use std::io::IsTerminal;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    // `cargo run` stays attached, and so does an explicit --wait.
    if cfg!(debug_assertions)
        || args.iter().any(|a| a == WAIT_FLAG)
        || std::env::var_os(DETACHED_ENV).is_some()
        || !std::io::stdin().is_terminal()
    {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let spawned = Command::new(exe)
        .args(args)
        .env(DETACHED_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Its own process group: closing the terminal or Ctrl+C there no
        // longer reaches the IDE.
        .process_group(0)
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            launch_log::step(&format!(
                "could not start in the background ({e}): staying here"
            ));
            return false;
        }
    };
    launch_log::step(&format!(
        "continues in the background as pid {}",
        child.id()
    ));
    // The background copy has no terminal: if it dies right away, say it here rather
    // than leave the user with a prompt and no window.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
    while std::time::Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            eprintln!(
                "Coding Unicorns stopped during startup ({status}):\n{}",
                launch_log::tail(15)
            );
            if let Some(log) = launch_log::path() {
                eprintln!("Full log: {}", log.display());
            }
            std::process::exit(status.code().unwrap_or(1));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    true
}

#[cfg(not(unix))]
fn detach_from_terminal(_args: &[String]) -> bool {
    false
}

fn main() -> eframe::Result<()> {
    // Re-invoked by `claude` as the permission MCP server — no GUI.
    let raw_args: Vec<String> = std::env::args().collect();
    if raw_args.iter().any(|a| a == "--claude-permission-server") {
        let port = raw_args
            .iter()
            .position(|a| a == "--port")
            .and_then(|i| raw_args.get(i + 1))
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(0);
        let token = std::env::var("CU_PERM_TOKEN").unwrap_or_default();
        if port != 0 {
            claude::permission::run_permission_mcp_server(port, token);
        }
        return Ok(());
    }

    install_panic_logger();
    env_logger::init();
    // The copy started in the background by `cu` continues its parent's log.
    launch_log::start(std::env::var_os("CODING_UNICORNS_DETACHED").is_some());
    // Before any thread: it sets PATH.
    let started = std::time::Instant::now();
    login_path::import();
    launch_log::step(&format!(
        "PATH ({} ms): {}",
        started.elapsed().as_millis(),
        std::env::var("PATH").unwrap_or_default()
    ));

    // SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX
    #[cfg(windows)]
    unsafe {
        SetErrorMode(0x0001 | 0x0002 | 0x8000);
    }

    // `-psn_0_…`: the process serial number macOS may pass to an app started from the
    // Finder (first launch after download); not a path to open.
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with("-psn_"))
        .collect();
    let new_window = args.iter().any(|a| a == single_instance::NEW_WINDOW_FLAG);
    let initial_path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(std::path::PathBuf::from);
    // Opening a file/folder while the IDE already runs: hand it over and quit.
    if let Some(path) = initial_path.as_deref() {
        if !new_window && single_instance::forward(path) {
            launch_log::step(&format!(
                "{} handed to the running instance",
                path.display()
            ));
            return Ok(());
        }
    }

    if detach_from_terminal(&args) {
        return Ok(());
    }
    // The .app / AppImage puts `cu` on the PATH (after login_path::import).
    cli_link::install_in_background();

    let icon = load_icon();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Coding Unicorns")
        // Wayland app id / X11 WM_CLASS: matches the AppImage's coding-unicorns.desktop,
        // so the desktop shows its icon and name for the window.
        .with_app_id("coding-unicorns")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([600.0, 400.0]);
    if let Some(icon_data) = icon {
        viewport = viewport.with_icon(std::sync::Arc::new(icon_data));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    launch_log::step("opening the window");
    let result = eframe::run_native(
        "Coding Unicorns",
        options,
        Box::new(|cc| {
            launch_log::step("window created, loading the app");
            cc.egui_ctx.set_fonts(ui::theme::app_fonts());
            // Before the app creates its first terminal.
            terminal::set_output_waker(cc.egui_ctx.clone());

            let app = CodingUnicorns::new(cc, initial_path);
            // Only now: a panic while loading the config or the workspace must still
            // show the dialog.
            launch_log::window_opened();
            Ok(Box::new(app))
        }),
    );
    match &result {
        Ok(()) => launch_log::step("closed"),
        Err(e) if !launch_log::is_window_open() => {
            launch_log::fatal(&format!("Its window could not be opened: {e}"));
        }
        Err(e) => launch_log::step(&format!("stopped: {e}")),
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_icon_decodes_to_square_rgba() {
        let icon = super::load_icon().expect("assets/icon.png must decode");
        assert_eq!(icon.width, icon.height);
        assert!(
            icon.width >= 256,
            "the MSI's ICO export needs a 256 px source"
        );
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
    }
}
