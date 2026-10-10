//! A step-by-step record of the last launch. An app opened from the Finder or the Dock has
//! no console: when it does not show up, this file (and the dialog [`fatal`] shows) is what
//! tells why.
//!
//! macOS: `~/Library/Logs/Coding Unicorns/launch.log` (Console.app lists it); elsewhere
//! `<config dir>/coding-unicorns/launch.log`.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();
static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);
/// Set once the window exists: from then on errors show in the app, not in a dialog.
static WINDOW_OPEN: AtomicBool = AtomicBool::new(false);

/// Where the log is written.
pub fn path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let dir = dirs_next::home_dir().map(|h| h.join("Library/Logs/Coding Unicorns"));
    #[cfg(not(target_os = "macos"))]
    let dir = dirs_next::config_dir().map(|c| c.join("coding-unicorns"));
    dir.map(|d| d.join("launch.log"))
}

/// Start the log of this launch. `append` keeps the previous content (the background copy
/// started from a terminal continues the log of the process that started it).
pub fn start(append: bool) {
    START.get_or_init(Instant::now);
    let Some(path) = path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&path);
    if let Ok(file) = file {
        *FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(file);
    }
    use std::io::IsTerminal as _;
    step(&format!(
        "Coding Unicorns {} (pid {}) {:?}, args {:?}, stdin is a terminal: {}",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        std::env::current_exe().unwrap_or_default(),
        std::env::args().skip(1).collect::<Vec<_>>(),
        std::io::stdin().is_terminal(),
    ));
}

/// Append one timestamped line (seconds since launch).
pub fn step(msg: &str) {
    let elapsed = START
        .get()
        .map(|s| s.elapsed().as_secs_f64())
        .unwrap_or(0.0);
    if let Some(file) = FILE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = writeln!(file, "[{elapsed:7.3}s] {msg}");
        let _ = file.flush();
    }
}

/// The app is up in its window: later failures are its to report.
pub fn window_opened() {
    WINDOW_OPEN.store(true, Ordering::Relaxed);
    step("app ready");
}

/// Whether [`start`] ran (not in the `--claude-permission-server` helper).
pub fn is_started() -> bool {
    START.get().is_some()
}

/// Whether the window has been created yet.
pub fn is_window_open() -> bool {
    WINDOW_OPEN.load(Ordering::Relaxed)
}

/// The launch failed before the window opened: log it and tell the user in a native
/// dialog, so the app does not just vanish. Call on the main thread.
pub fn fatal(msg: &str) {
    step(&format!("FATAL: {msg}"));
    let log = path().map(|p| p.display().to_string()).unwrap_or_default();
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Coding Unicorns could not start")
        .set_description(format!("{msg}\n\nDetails: {log}"))
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// The last `lines` lines of the log, to print in a terminal.
pub fn tail(lines: usize) -> String {
    let text = path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}
