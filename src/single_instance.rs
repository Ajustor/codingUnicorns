//! Hand files and folders opened from Explorer (or `cu <path>`) to the IDE window that
//! already shows them, instead of starting another window.
//!
//! Each running window listens on a loopback TCP port and records `port` + a random token
//! in its own file of the `instances` folder in the config dir. A new process launched
//! with a path asks them in turn, sending `token\nmode\npath\n`; a window answers `ok`
//! when it takes the path, `no` otherwise:
//! - a folder goes only to the window that has exactly that folder open: any other
//!   folder opens a new window (`cu .` never replaces another project);
//! - a file goes to the window whose folder holds it, else to the newest window.
//!
//! Anything that goes wrong (no window, stale port, wrong token, timeout) falls back to
//! a normal start.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const INSTANCES_DIR: &str = "instances";
const TIMEOUT: Duration = Duration::from_millis(800);
/// Paths longer than this are rejected so a bogus client can't make us buffer forever.
const MAX_LINE: u64 = 64 * 1024;

/// Command-line flag that skips forwarding and always opens a new window.
pub const NEW_WINDOW_FLAG: &str = "--new-window";

fn default_registry() -> Option<PathBuf> {
    Some(
        dirs_next::config_dir()?
            .join("coding-unicorns")
            .join(INSTANCES_DIR),
    )
}

/// Contents of an instance file: where the window listens and the token it expects.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LockInfo {
    port: u16,
    token: String,
}

impl LockInfo {
    fn parse(s: &str) -> Option<Self> {
        let mut lines = s.lines();
        let port = lines.next()?.trim().parse().ok()?;
        let token = lines.next()?.trim().to_string();
        (!token.is_empty()).then_some(Self { port, token })
    }

    fn render(&self) -> String {
        format!("{}\n{}\n", self.port, self.token)
    }
}

/// Which windows may take a forwarded path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Only the window showing it: its folder, or a file inside that folder.
    Match,
    /// Any window (for a file no window's folder holds).
    Any,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Match => "match",
            Self::Any => "any",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "match" => Some(Self::Match),
            "any" => Some(Self::Any),
            _ => None,
        }
    }
}

/// Send `path` to the window that shows it. Returns true when one took it, in which
/// case this process should exit.
pub fn forward(path: &Path) -> bool {
    default_registry().is_some_and(|dir| forward_with_registry(&dir, path))
}

fn forward_with_registry(registry: &Path, path: &Path) -> bool {
    // The running windows have other working directories: always send an absolute path.
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut live = Vec::new();
    for (file, info) in instances(registry) {
        match send(&info, Mode::Match, &path) {
            Some(true) => return true,
            Some(false) => live.push(info),
            // Nothing listens there any more: a window that crashed.
            None => {
                let _ = std::fs::remove_file(file);
            }
        }
    }
    if path.is_dir() {
        return false;
    }
    live.iter()
        .any(|info| send(info, Mode::Any, &path) == Some(true))
}

/// The registered windows, newest first.
fn instances(registry: &Path) -> Vec<(PathBuf, LockInfo)> {
    let Ok(entries) = std::fs::read_dir(registry) else {
        return Vec::new();
    };
    let mut found: Vec<_> = entries
        .flatten()
        .filter_map(|e| {
            let info = LockInfo::parse(&std::fs::read_to_string(e.path()).ok()?)?;
            let modified = e.metadata().and_then(|m| m.modified()).ok();
            Some((modified, e.path(), info))
        })
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found.into_iter().map(|(_, p, i)| (p, i)).collect()
}

/// Offer `path` to one window: `Some(true)` taken, `Some(false)` declined (or no
/// answer), `None` when nothing listens on its port.
fn send(info: &LockInfo, mode: Mode, path: &Path) -> Option<bool> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, info.port));
    let stream = TcpStream::connect_timeout(&addr, TIMEOUT).ok()?;
    let exchange = |mut stream: TcpStream| -> std::io::Result<bool> {
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        writeln!(stream, "{}", info.token)?;
        writeln!(stream, "{}", mode.as_str())?;
        writeln!(stream, "{}", path.display())?;
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply)?;
        Ok(reply.trim() == "ok")
    };
    Some(exchange(stream).unwrap_or(false))
}

/// Listener owned by a running window. Dropping it does not stop the accept thread,
/// but [`Server::release`] removes its instance file so no new client will find it.
pub struct Server {
    lock: PathBuf,
    /// The folder this window shows, read by the accept thread.
    workspace: Arc<Mutex<Option<PathBuf>>>,
    /// Paths sent by other processes, to be opened by the UI.
    pub rx: Receiver<PathBuf>,
}

impl Server {
    /// Start listening and register this window. `on_path` runs on the accept thread
    /// after each received path (used to wake the UI). Returns `None` if anything fails;
    /// the app then simply works without forwarding.
    pub fn start(on_path: impl Fn() + Send + 'static) -> Option<Self> {
        Self::start_in(default_registry()?, on_path)
    }

    fn start_in(registry: PathBuf, on_path: impl Fn() + Send + 'static) -> Option<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
        let info = LockInfo {
            port: listener.local_addr().ok()?.port(),
            token: uuid::Uuid::new_v4().simple().to_string(),
        };
        std::fs::create_dir_all(&registry).ok()?;
        let lock = registry.join(format!("{}.lock", info.token));
        std::fs::write(&lock, info.render()).ok()?;

        let (tx, rx) = mpsc::channel();
        let workspace: Arc<Mutex<Option<PathBuf>>> = Arc::default();
        let shown = workspace.clone();
        let token = info.token;
        std::thread::Builder::new()
            .name("single-instance".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let takes = |mode, path: &Path| {
                        let ws = shown.lock().map(|ws| ws.clone()).unwrap_or_default();
                        takes_path(ws.as_deref(), mode, path)
                    };
                    if let Some(path) = handle_client(stream, &token, takes) {
                        if tx.send(path).is_err() {
                            break; // UI gone
                        }
                        on_path();
                    }
                }
            })
            .ok()?;
        Some(Self {
            lock,
            workspace,
            rx,
        })
    }

    /// Record the folder this window shows (cheap when it did not change).
    pub fn set_workspace(&self, path: Option<&Path>) {
        if let Ok(mut ws) = self.workspace.lock() {
            if ws.as_deref() != path {
                *ws = path.map(Path::to_path_buf);
            }
        }
    }

    /// Stop advertising this window (called on exit, before any relaunch, so the new
    /// process starts its own window instead of forwarding to us).
    pub fn release(&self) {
        let _ = std::fs::remove_file(&self.lock);
    }
}

/// Whether a window showing `workspace` takes `path` in `mode`.
fn takes_path(workspace: Option<&Path>, mode: Mode, path: &Path) -> bool {
    let is_dir = path.is_dir();
    let shows = workspace.is_some_and(|ws| {
        let (ws, path) = (normalize(ws), normalize(path));
        if is_dir {
            path == ws
        } else {
            path.starts_with(&ws)
        }
    });
    shows || (mode == Mode::Any && !is_dir)
}

/// One spelling per existing path (case on Windows, `\\?\`, symlinks).
fn normalize(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Read one request and answer it; returns the path when the token matches and `takes`
/// accepts it.
fn handle_client(
    stream: TcpStream,
    token: &str,
    takes: impl Fn(Mode, &Path) -> bool,
) -> Option<PathBuf> {
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?).take(MAX_LINE);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    if line.trim_end() != token {
        return None;
    }
    line.clear();
    reader.read_line(&mut line).ok()?;
    let mode = Mode::parse(line.trim_end());
    line.clear();
    reader.read_line(&mut line).ok()?;
    let path = PathBuf::from(line.trim_end_matches(['\r', '\n']));
    let taken = !path.as_os_str().is_empty() && mode.is_some_and(|mode| takes(mode, &path));
    let mut stream = stream;
    let _ = stream.write_all(if taken { b"ok\n" } else { b"no\n" });
    taken.then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn registry_in(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("nested").join(INSTANCES_DIR)
    }

    /// A window showing `workspace`.
    fn window(registry: &Path, workspace: Option<&Path>) -> Server {
        let server = Server::start_in(registry.to_path_buf(), || {}).unwrap();
        server.set_workspace(workspace);
        server
    }

    fn received(server: &Server) -> Option<PathBuf> {
        server.rx.recv_timeout(Duration::from_millis(500)).ok()
    }

    #[test]
    fn lock_info_round_trips_and_rejects_garbage() {
        let info = LockInfo {
            port: 4321,
            token: "abc".into(),
        };
        assert_eq!(LockInfo::parse(&info.render()), Some(info));
        assert_eq!(LockInfo::parse(""), None);
        assert_eq!(LockInfo::parse("notaport\nabc"), None);
        assert_eq!(LockInfo::parse("80\n"), None, "token is required");
    }

    #[test]
    fn a_file_in_the_workspace_reaches_its_window_and_wakes_it() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_in(&dir);
        let woken = Arc::new(AtomicUsize::new(0));
        let w = woken.clone();
        let server = Server::start_in(registry.clone(), move || {
            w.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        server.set_workspace(Some(dir.path()));

        let file = dir.path().join("notes with spaces.txt");
        std::fs::write(&file, "").unwrap();
        assert!(forward_with_registry(&registry, &file));
        assert_eq!(received(&server), Some(file));
        // The server wakes the UI right after sending the path: wait for it.
        let start = std::time::Instant::now();
        while woken.load(Ordering::SeqCst) == 0 && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(woken.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn another_folder_starts_a_new_window() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_in(&dir);
        let (open, other) = (dir.path().join("open"), dir.path().join("other"));
        std::fs::create_dir_all(&open).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let server = window(&registry, Some(&open));

        assert!(!forward_with_registry(&registry, &other), "not its folder");
        assert!(
            !forward_with_registry(&registry, &open.join("..").join("other")),
            "however it is spelled"
        );
        let sub = open.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        assert!(
            !forward_with_registry(&registry, &sub),
            "a subfolder is another project"
        );
        assert_eq!(received(&server), None);

        let empty = window(&registry, None);
        assert!(
            !forward_with_registry(&registry, &other),
            "nor an empty window"
        );
        assert_eq!(received(&empty), None);
    }

    #[test]
    fn a_folder_goes_to_the_window_that_has_it_open() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_in(&dir);
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let wa = window(&registry, Some(&a));
        let wb = window(&registry, Some(&b));

        assert!(forward_with_registry(&registry, &a.join(".")));
        assert_eq!(received(&wa).map(|p| normalize(&p)), Some(normalize(&a)));
        assert_eq!(received(&wb), None);
    }

    #[test]
    fn a_file_outside_every_workspace_goes_to_a_running_window() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_in(&dir);
        let ws = dir.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let server = window(&registry, Some(&ws));
        let file = dir.path().join("loose.txt");
        std::fs::write(&file, "").unwrap();
        assert!(forward_with_registry(&registry, &file));
        assert_eq!(received(&server), Some(file));
    }

    #[test]
    fn relative_paths_are_made_absolute_before_sending() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_in(&dir);
        let server = window(&registry, None);
        assert!(forward_with_registry(&registry, Path::new("relative.txt")));
        let got = received(&server).unwrap();
        assert!(got.is_absolute(), "{got:?}");
        assert!(got.ends_with("relative.txt"));
    }

    #[test]
    fn no_window_stale_port_or_wrong_token_falls_back_to_a_new_window() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_in(&dir);
        assert!(
            !forward_with_registry(&registry, Path::new("a.txt")),
            "no window"
        );

        // Stale: nothing listens on the recorded port any more; the file is cleaned up.
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        std::fs::create_dir_all(&registry).unwrap();
        let stale = registry.join("stale.lock");
        std::fs::write(&stale, format!("{port}\ntok\n")).unwrap();
        assert!(
            !forward_with_registry(&registry, Path::new("a.txt")),
            "stale port"
        );
        assert!(!stale.exists());

        // Wrong token: the server ignores the request and the client gives up.
        let server = window(&registry, None);
        let lock = server.lock.clone();
        let real = std::fs::read_to_string(&lock).unwrap();
        let forged = format!("{}\nnot-the-token\n", real.lines().next().unwrap());
        std::fs::write(&lock, forged).unwrap();
        assert!(
            !forward_with_registry(&registry, Path::new("a.txt")),
            "wrong token"
        );
        assert!(server.rx.try_recv().is_err());
    }

    #[test]
    fn release_removes_only_our_own_registration() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry_in(&dir);
        let old = window(&registry, None);
        let new = window(&registry, None);
        old.release();
        assert!(!old.lock.exists());
        assert!(new.lock.exists());
        assert!(forward_with_registry(&registry, Path::new("a.txt")));
        assert!(received(&new).is_some(), "the remaining window takes it");
        assert!(old.rx.try_recv().is_err());
    }
}
