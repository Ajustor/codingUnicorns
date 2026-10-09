//! Hand files and folders opened from Explorer (or `cu <path>`) to the IDE window that is
//! already running, instead of starting a second window.
//!
//! The running instance listens on a loopback TCP port and records `port` + a random token
//! in `instance.lock` in the config dir. A new process launched with a path reads that file,
//! sends `token\npath\n` and exits once the running instance answers `ok`. Anything that
//! goes wrong (no lock file, stale port, wrong token, timeout) falls back to a normal start.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const LOCK_FILE: &str = "instance.lock";
const TIMEOUT: Duration = Duration::from_millis(800);
/// Paths longer than this are rejected so a bogus client can't make us buffer forever.
const MAX_LINE: u64 = 64 * 1024;

/// Command-line flag that skips forwarding and always opens a new window.
pub const NEW_WINDOW_FLAG: &str = "--new-window";

fn default_lock_path() -> Option<PathBuf> {
    Some(
        dirs_next::config_dir()?
            .join("coding-unicorns")
            .join(LOCK_FILE),
    )
}

/// Contents of the lock file: where the running instance listens and the token it expects.
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

/// Send `path` to the running instance. Returns true when it accepted it, in which case
/// this process should exit.
pub fn forward(path: &Path) -> bool {
    default_lock_path().is_some_and(|lock| forward_with_lock(&lock, path))
}

fn forward_with_lock(lock: &Path, path: &Path) -> bool {
    let Some(info) = std::fs::read_to_string(lock)
        .ok()
        .and_then(|s| LockInfo::parse(&s))
    else {
        return false;
    };
    // The running instance has another working directory: always send an absolute path.
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, info.port));
    let send = || -> std::io::Result<bool> {
        let mut stream = TcpStream::connect_timeout(&addr, TIMEOUT)?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        writeln!(stream, "{}", info.token)?;
        writeln!(stream, "{}", path.display())?;
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply)?;
        Ok(reply.trim() == "ok")
    };
    send().unwrap_or(false)
}

/// Listener owned by the running instance. Dropping it does not stop the accept thread,
/// but [`Server::release`] removes the lock file so no new client will find it.
pub struct Server {
    lock: PathBuf,
    info: LockInfo,
    /// Paths sent by other processes, to be opened by the UI.
    pub rx: Receiver<PathBuf>,
}

impl Server {
    /// Start listening and record the lock file. `on_path` runs on the accept thread after
    /// each received path (used to wake the UI). Returns `None` if anything fails; the app
    /// then simply works without forwarding.
    pub fn start(on_path: impl Fn() + Send + 'static) -> Option<Self> {
        Self::start_with_lock(default_lock_path()?, on_path)
    }

    fn start_with_lock(lock: PathBuf, on_path: impl Fn() + Send + 'static) -> Option<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
        let info = LockInfo {
            port: listener.local_addr().ok()?.port(),
            token: uuid::Uuid::new_v4().simple().to_string(),
        };
        if let Some(dir) = lock.parent() {
            std::fs::create_dir_all(dir).ok()?;
        }
        std::fs::write(&lock, info.render()).ok()?;

        let (tx, rx) = mpsc::channel();
        let token = info.token.clone();
        std::thread::Builder::new()
            .name("single-instance".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    if let Some(path) = handle_client(stream, &token) {
                        if tx.send(path).is_err() {
                            break; // UI gone
                        }
                        on_path();
                    }
                }
            })
            .ok()?;
        Some(Self { lock, info, rx })
    }

    /// Stop advertising this instance (called on exit, before any relaunch, so the new
    /// process starts its own window instead of forwarding to us).
    pub fn release(&self) {
        // Only remove the file if it still points at us: a newer instance may own it now.
        let ours = std::fs::read_to_string(&self.lock)
            .ok()
            .and_then(|s| LockInfo::parse(&s))
            .is_some_and(|i| i == self.info);
        if ours {
            let _ = std::fs::remove_file(&self.lock);
        }
    }
}

/// Read one request; returns the path when the token matches.
fn handle_client(stream: TcpStream, token: &str) -> Option<PathBuf> {
    stream.set_read_timeout(Some(TIMEOUT)).ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?).take(MAX_LINE);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    if line.trim_end() != token {
        return None;
    }
    line.clear();
    reader.read_line(&mut line).ok()?;
    let path = PathBuf::from(line.trim_end_matches(['\r', '\n']));
    if path.as_os_str().is_empty() {
        return None;
    }
    let mut stream = stream;
    let _ = stream.write_all(b"ok\n");
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn lock_in(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("nested").join(LOCK_FILE)
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
    fn forwarded_path_reaches_the_server_and_wakes_it() {
        let dir = tempfile::tempdir().unwrap();
        let lock = lock_in(&dir);
        let woken = Arc::new(AtomicUsize::new(0));
        let w = woken.clone();
        let server = Server::start_with_lock(lock.clone(), move || {
            w.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

        let file = dir.path().join("notes with spaces.txt");
        assert!(forward_with_lock(&lock, &file));
        let got = server.rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(got, file);
        // The server wakes the UI right after sending the path: wait for it.
        let start = std::time::Instant::now();
        while woken.load(Ordering::SeqCst) == 0 && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(woken.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn relative_paths_are_made_absolute_before_sending() {
        let dir = tempfile::tempdir().unwrap();
        let lock = lock_in(&dir);
        let server = Server::start_with_lock(lock.clone(), || {}).unwrap();
        assert!(forward_with_lock(&lock, Path::new("relative.txt")));
        let got = server.rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(got.is_absolute(), "{got:?}");
        assert!(got.ends_with("relative.txt"));
    }

    #[test]
    fn no_lock_stale_port_or_wrong_token_falls_back_to_a_new_window() {
        let dir = tempfile::tempdir().unwrap();
        let lock = lock_in(&dir);
        assert!(
            !forward_with_lock(&lock, Path::new("a.txt")),
            "no lock file"
        );

        // Stale: nothing listens on the recorded port any more.
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
        std::fs::write(&lock, format!("{port}\ntok\n")).unwrap();
        assert!(!forward_with_lock(&lock, Path::new("a.txt")), "stale port");

        // Wrong token: the server ignores the request and the client gives up.
        let server = Server::start_with_lock(lock.clone(), || {}).unwrap();
        let real = std::fs::read_to_string(&lock).unwrap();
        let forged = format!("{}\nnot-the-token\n", real.lines().next().unwrap());
        std::fs::write(&lock, forged).unwrap();
        assert!(!forward_with_lock(&lock, Path::new("a.txt")), "wrong token");
        assert!(server.rx.try_recv().is_err());
    }

    #[test]
    fn release_removes_only_our_own_lock() {
        let dir = tempfile::tempdir().unwrap();
        let lock = lock_in(&dir);
        let server = Server::start_with_lock(lock.clone(), || {}).unwrap();
        server.release();
        assert!(!lock.exists());
        assert!(
            !forward_with_lock(&lock, Path::new("a.txt")),
            "a released instance is no longer offered"
        );

        // A newer instance took over the lock: releasing the old one keeps it.
        let old = Server::start_with_lock(lock.clone(), || {}).unwrap();
        let _new = Server::start_with_lock(lock.clone(), || {}).unwrap();
        old.release();
        assert!(lock.exists());
    }
}
