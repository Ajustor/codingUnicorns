//! External file change detection.
//!
//! The workspace is watched recursively on a background thread (via `notify`);
//! raw events are debounced and delivered as [`FsChanges`] batches that the UI
//! drains once per frame. On each batch (or every [`POLL_INTERVAL`] when the
//! watcher could not be started) the open tabs are compared against the
//! size/mtime recorded when we last loaded or saved them:
//! - unmodified active tab → reloaded silently, keeping cursor/scroll;
//! - modified active tab → non-blocking "Reload / Keep mine" prompt;
//! - deleted file → tab marked "(deleted)", buffer kept.
//!
//! Saves are guarded too: writing over a file that changed on disk since we
//! loaded it asks first (see [`CodingUnicorns::save_editor_guarded`]).

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime};

use notify::event::ModifyKind;
use notify::{EventKind, RecursiveMode, Watcher};

use super::CodingUnicorns;

/// Quiet period after the last raw event before a batch is delivered.
pub const DEBOUNCE: Duration = Duration::from_millis(200);
/// Upper bound on how long a continuous stream of events can delay a batch.
const MAX_BATCH_DELAY: Duration = Duration::from_secs(1);
/// Open-file mtime polling interval when the watcher is unavailable.
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Directories whose events are ignored (VCS metadata, build output).
const IGNORED_DIRS: &[&str] = &[".git", "target", "node_modules"];

/// Size + modification time of a file, used to tell whether it changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub mtime: Option<SystemTime>,
    pub len: u64,
}

impl FileStamp {
    /// Stamp of the regular file at `path`, or `None` if it doesn't exist.
    pub fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        meta.is_file().then(|| Self {
            mtime: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

/// A debounced batch of filesystem changes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FsChanges {
    pub paths: HashSet<PathBuf>,
    /// A file or folder was created, removed or renamed (file tree is stale).
    pub structure_changed: bool,
}

impl FsChanges {
    fn is_empty(&self) -> bool {
        self.paths.is_empty() && !self.structure_changed
    }

    fn merge(&mut self, other: FsChanges) {
        self.paths.extend(other.paths);
        self.structure_changed |= other.structure_changed;
    }

    /// Fold one raw `notify` event in, dropping ignored paths.
    fn add_event(&mut self, event: notify::Event, root: &Path) {
        let Some(structural) = classify(&event.kind) else {
            return;
        };
        let mut any = false;
        for p in event.paths {
            if !is_ignored(&p, root) {
                self.paths.insert(p);
                any = true;
            }
        }
        if any && structural {
            self.structure_changed = true;
        }
    }
}

/// Whether events for `path` should be ignored (inside `.git/`, `target/`, …
/// relative to the watched `root`).
pub fn is_ignored(path: &Path, root: &Path) -> bool {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components().any(|c| match c {
        Component::Normal(name) => IGNORED_DIRS.iter().any(|d| name == *d),
        _ => false,
    })
}

/// `None` for events that never change content (access); otherwise whether
/// the event changes the tree structure (create / remove / rename).
pub fn classify(kind: &EventKind) -> Option<bool> {
    match kind {
        EventKind::Access(_) => None,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_)) => {
            Some(true)
        }
        _ => Some(false),
    }
}

/// A running recursive watcher. Dropping it stops the watch and its thread.
pub struct FileWatcher {
    _watcher: notify::RecommendedWatcher,
    rx: Receiver<FsChanges>,
}

impl FileWatcher {
    pub fn start(
        root: &Path,
        debounce: Duration,
        waker: Option<egui::Context>,
    ) -> notify::Result<Self> {
        let (raw_tx, raw_rx) = mpsc::channel::<notify::Event>();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
                Ok(ev) => {
                    let _ = raw_tx.send(ev);
                }
                Err(e) => log::warn!("file watcher error: {e}"),
            })?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        let (tx, rx) = mpsc::channel();
        let root = root.to_path_buf();
        std::thread::Builder::new()
            .name("fs-watch-debounce".into())
            .spawn(move || debounce_loop(raw_rx, tx, &root, debounce, waker))
            .map_err(notify::Error::io)?;
        Ok(Self {
            _watcher: watcher,
            rx,
        })
    }

    /// All batches delivered since the last call, merged.
    pub fn drain(&self) -> Option<FsChanges> {
        let mut all = FsChanges::default();
        while let Ok(c) = self.rx.try_recv() {
            all.merge(c);
        }
        (!all.is_empty()).then_some(all)
    }
}

fn debounce_loop(
    raw_rx: Receiver<notify::Event>,
    tx: Sender<FsChanges>,
    root: &Path,
    debounce: Duration,
    waker: Option<egui::Context>,
) {
    let mut pending = FsChanges::default();
    let mut first_at = Instant::now();
    let flush = |pending: &mut FsChanges| -> bool {
        if pending.is_empty() {
            return true;
        }
        let ok = tx.send(std::mem::take(pending)).is_ok();
        if let Some(ctx) = &waker {
            ctx.request_repaint();
        }
        ok
    };
    loop {
        let event = if pending.is_empty() {
            match raw_rx.recv() {
                Ok(ev) => {
                    first_at = Instant::now();
                    ev
                }
                Err(_) => return, // watcher dropped
            }
        } else {
            let wait = debounce.min(MAX_BATCH_DELAY.saturating_sub(first_at.elapsed()));
            match raw_rx.recv_timeout(wait) {
                Ok(ev) => ev,
                Err(RecvTimeoutError::Timeout) => {
                    if !flush(&mut pending) {
                        return;
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    flush(&mut pending);
                    return;
                }
            }
        };
        pending.add_event(event, root);
        if !pending.is_empty() && first_at.elapsed() >= MAX_BATCH_DELAY && !flush(&mut pending) {
            return;
        }
    }
}

/// Text normalised for "same content?" checks: no BOM, `\n` line endings.
pub fn normalize_text(s: &str) -> String {
    s.strip_prefix('\u{feff}')
        .unwrap_or(s)
        .replace("\r\n", "\n")
}

/// Whether the file at `path` holds `text` (modulo BOM / line endings).
fn disk_matches(path: &Path, text: &str) -> bool {
    std::fs::read_to_string(path).is_ok_and(|disk| normalize_text(&disk) == normalize_text(text))
}

/// What to do with an open tab after comparing its file to the recorded stamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskAction {
    /// Nothing changed.
    None,
    /// The file is gone: mark the tab.
    MarkDeleted,
    /// Changed, but nothing to do beyond remembering the new stamp.
    AcceptStamp,
    /// Unmodified buffer: reload it from disk.
    Reload,
    /// Unsaved edits would be lost: ask the user.
    Conflict,
}

/// Decide how to react to the on-disk state of an open file. `same_content`
/// is only evaluated (it reads the file) when the stamp changed for the
/// active tab.
pub fn decide(
    recorded: Option<FileStamp>,
    now: Option<FileStamp>,
    was_deleted: bool,
    is_active: bool,
    modified: bool,
    same_content: impl FnOnce() -> bool,
) -> DiskAction {
    match now {
        None if was_deleted => DiskAction::None,
        None => DiskAction::MarkDeleted,
        Some(now) if recorded == Some(now) => DiskAction::None,
        // Inactive tabs are re-read from disk when activated.
        Some(_) if !is_active => DiskAction::AcceptStamp,
        Some(_) if same_content() => DiskAction::AcceptStamp,
        Some(_) if !modified => DiskAction::Reload,
        Some(_) => DiskAction::Conflict,
    }
}

/// A pending "file changed on disk" prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    pub path: PathBuf,
    /// Raised by an explicit save (offer Overwrite / Cancel).
    pub on_save: bool,
}

pub struct FileWatchState {
    watcher: Option<FileWatcher>,
    last_poll: Instant,
    stamps: HashMap<PathBuf, FileStamp>,
    pub conflict: Option<Conflict>,
    waker: Option<egui::Context>,
    /// Editor (path, is_modified) seen last frame — detects our own saves.
    last_editor: Option<(PathBuf, bool)>,
}

impl FileWatchState {
    pub fn new(waker: Option<egui::Context>) -> Self {
        Self {
            watcher: None,
            last_poll: Instant::now(),
            stamps: HashMap::new(),
            conflict: None,
            waker,
            last_editor: None,
        }
    }
}

fn is_ctrl_s(e: &egui::Event) -> bool {
    matches!(
        e,
        egui::Event::Key { key: egui::Key::S, pressed: true, modifiers, .. } if modifiers.ctrl
    )
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

enum Choice {
    Reload,
    KeepMine,
    Overwrite,
    Cancel,
}

impl CodingUnicorns {
    /// (Re)start watching `root`; falls back to polling open files on failure.
    pub fn start_file_watcher(&mut self, root: &Path) {
        self.file_watch.watcher = None;
        match FileWatcher::start(root, DEBOUNCE, self.file_watch.waker.clone()) {
            Ok(w) => self.file_watch.watcher = Some(w),
            Err(e) => log::warn!(
                "could not watch {} ({e}); polling open files every {}s instead",
                root.display(),
                POLL_INTERVAL.as_secs()
            ),
        }
    }

    /// Remember the on-disk stamp of `path` (after loading or saving it).
    pub fn record_file_stamp(&mut self, path: &Path) {
        match FileStamp::of(path) {
            Some(s) => {
                self.file_watch.stamps.insert(path.to_path_buf(), s);
            }
            None => {
                self.file_watch.stamps.remove(path);
            }
        }
    }

    /// Per-frame driver; call before the UI renders (it may swallow a Ctrl+S
    /// that would overwrite external changes).
    pub fn tick_file_watch(&mut self, ctx: &egui::Context) {
        // Our own saves flip is_modified true → false: adopt the new stamp so
        // the resulting watcher event isn't mistaken for an external change.
        let cur = self
            .editor
            .current_path
            .clone()
            .map(|p| (p, self.editor.is_modified));
        if let (Some((lp, true)), Some((cp, false))) = (&self.file_watch.last_editor, &cur) {
            if lp == cp {
                let p = cp.clone();
                self.record_file_stamp(&p);
            }
        }
        self.file_watch.last_editor = cur;

        let mut check = false;
        if let Some(w) = &self.file_watch.watcher {
            if let Some(changes) = w.drain() {
                if changes.structure_changed {
                    self.file_tree.reload_children();
                }
                check = true;
            }
        } else if !self.tab_manager.tabs.is_empty() {
            if self.file_watch.last_poll.elapsed() >= POLL_INTERVAL {
                self.file_watch.last_poll = Instant::now();
                check = true;
            }
            ctx.request_repaint_after(POLL_INTERVAL);
        }
        if check {
            self.check_open_files();
        }

        self.intercept_conflicting_save(ctx);
        self.show_conflict_window(ctx);
    }

    /// Compare every open file tab with its on-disk state and react.
    pub fn check_open_files(&mut self) {
        let tabs: Vec<(PathBuf, bool)> = self
            .tab_manager
            .tabs
            .iter()
            .filter(|t| !t.is_settings && t.path.is_absolute())
            .map(|t| (t.path.clone(), t.is_deleted))
            .collect();
        for (path, was_deleted) in tabs {
            let now = FileStamp::of(&path);
            if now.is_some() && was_deleted {
                self.tab_manager.set_deleted(&path, false);
            }
            let is_active = self.editor.current_path.as_deref() == Some(path.as_path());
            let action = decide(
                self.file_watch.stamps.get(&path).copied(),
                now,
                was_deleted,
                is_active,
                self.editor.is_modified,
                || disk_matches(&path, &self.editor.buffer.to_string()),
            );
            match action {
                DiskAction::None => {}
                DiskAction::MarkDeleted => {
                    self.tab_manager.set_deleted(&path, true);
                    self.file_watch.stamps.remove(&path);
                    if is_active {
                        self.toast(format!("{} was deleted on disk", file_name(&path)));
                    }
                }
                DiskAction::AcceptStamp => self.record_file_stamp(&path),
                DiskAction::Reload => {
                    self.reload_from_disk(path.clone());
                    self.toast(format!("Reloaded {} (changed on disk)", file_name(&path)));
                }
                DiskAction::Conflict => {
                    if self.file_watch.conflict.as_ref().map(|c| &c.path) != Some(&path) {
                        self.file_watch.conflict = Some(Conflict {
                            path,
                            on_save: false,
                        });
                    }
                }
            }
        }
    }

    /// Reload the editor's file from disk, keeping cursor/scroll (clamped)
    /// and without stealing keyboard focus.
    fn reload_from_disk(&mut self, path: PathBuf) {
        let focus = self.editor.focus_requested;
        self.open_file(path);
        self.editor.focus_requested = focus;
    }

    /// True when saving the editor to `path` would overwrite changes made on
    /// disk since we loaded/saved it (and the user hasn't chosen to keep theirs).
    fn has_disk_conflict(&mut self, path: &Path) -> bool {
        if self
            .file_watch
            .conflict
            .as_ref()
            .is_some_and(|c| c.path == path)
        {
            return true;
        }
        let Some(now) = FileStamp::of(path) else {
            return false; // deleted: saving recreates it
        };
        if self.file_watch.stamps.get(path) == Some(&now) {
            return false;
        }
        if disk_matches(path, &self.editor.buffer.to_string()) {
            self.record_file_stamp(path);
            return false;
        }
        true
    }

    /// Save the main editor unless that would silently overwrite external
    /// changes, in which case the conflict prompt is raised instead. `explicit`
    /// is false for auto-saves. Returns true if the file was written.
    pub fn save_editor_guarded(&mut self, explicit: bool) -> bool {
        let path = self.editor.current_path.clone();
        if let Some(path) = &path {
            if self.has_disk_conflict(path) {
                let on_save = explicit
                    || self
                        .file_watch
                        .conflict
                        .as_ref()
                        .is_some_and(|c| &c.path == path && c.on_save);
                self.file_watch.conflict = Some(Conflict {
                    path: path.clone(),
                    on_save,
                });
                return false;
            }
        }
        let ok = self.editor.save().is_ok();
        if let Some(path) = &path {
            self.record_file_stamp(path);
        }
        ok
    }

    /// The editor saves itself on Ctrl+S; swallow that key when the save would
    /// overwrite external changes and ask instead.
    fn intercept_conflicting_save(&mut self, ctx: &egui::Context) {
        if self.editor2.is_some() && self.active_pane == 1 {
            return;
        }
        if !ctx.input(|i| i.events.iter().any(is_ctrl_s)) {
            return;
        }
        let Some(path) = self.editor.current_path.clone() else {
            return;
        };
        if self.has_disk_conflict(&path) {
            ctx.input_mut(|i| i.events.retain(|e| !is_ctrl_s(e)));
            self.file_watch.conflict = Some(Conflict {
                path,
                on_save: true,
            });
        }
    }

    fn show_conflict_window(&mut self, ctx: &egui::Context) {
        let Some(conflict) = self.file_watch.conflict.clone() else {
            return;
        };
        // The buffer the prompt was about is gone (tab switched or closed).
        if self.editor.current_path.as_deref() != Some(conflict.path.as_path()) {
            self.file_watch.conflict = None;
            return;
        }
        let mut choice = None;
        egui::Window::new("File changed on disk")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, [-16.0, 48.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "\"{}\" was changed on disk.",
                    file_name(&conflict.path)
                ));
                if conflict.on_save {
                    ui.label("Saving now would overwrite those changes.");
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .button("Reload")
                        .on_hover_text("Discard your edits and load the file from disk")
                        .clicked()
                    {
                        choice = Some(Choice::Reload);
                    }
                    if conflict.on_save {
                        if ui.button("Overwrite").clicked() {
                            choice = Some(Choice::Overwrite);
                        }
                        if ui.button("Cancel").clicked() {
                            choice = Some(Choice::Cancel);
                        }
                    } else if ui.button("Keep mine (overwrite on save)").clicked() {
                        choice = Some(Choice::KeepMine);
                    }
                });
            });
        let path = conflict.path;
        match choice {
            None => {}
            Some(Choice::Reload) => {
                self.file_watch.conflict = None;
                self.reload_from_disk(path);
            }
            Some(Choice::KeepMine) => {
                self.file_watch.conflict = None;
                self.record_file_stamp(&path);
            }
            Some(Choice::Overwrite) => {
                self.file_watch.conflict = None;
                self.record_file_stamp(&path);
                if self.save_editor_guarded(true) {
                    self.toast("Saved");
                }
            }
            Some(Choice::Cancel) => self.file_watch.conflict = None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(len: u64) -> Option<FileStamp> {
        Some(FileStamp {
            mtime: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(len)),
            len,
        })
    }

    #[test]
    fn decide_covers_every_case() {
        let never = || -> bool { panic!("content must not be read") };
        use DiskAction as A;
        assert_eq!(
            decide(stamp(1), stamp(1), false, true, true, never),
            A::None
        );
        assert_eq!(
            decide(stamp(1), None, false, true, false, never),
            A::MarkDeleted
        );
        assert_eq!(decide(None, None, true, true, false, never), A::None);
        assert_eq!(
            decide(stamp(1), stamp(2), false, false, true, never),
            A::AcceptStamp
        );
        assert_eq!(
            decide(stamp(1), stamp(2), false, true, true, || true),
            A::AcceptStamp
        );
        assert_eq!(
            decide(stamp(1), stamp(2), false, true, false, || false),
            A::Reload
        );
        assert_eq!(
            decide(stamp(1), stamp(2), false, true, true, || false),
            A::Conflict
        );
        // File came back after deletion (no recorded stamp) with new content.
        assert_eq!(
            decide(None, stamp(3), true, true, false, || false),
            A::Reload
        );
    }

    #[test]
    fn file_stamp_tracks_size_and_existence() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        assert_eq!(FileStamp::of(&f), None);
        assert_eq!(FileStamp::of(dir.path()), None, "directories have no stamp");
        std::fs::write(&f, "abc").unwrap();
        let s1 = FileStamp::of(&f).unwrap();
        assert_eq!(s1.len, 3);
        std::fs::write(&f, "abcdef").unwrap();
        assert_ne!(FileStamp::of(&f), Some(s1));
    }

    #[test]
    fn normalize_text_ignores_bom_and_crlf() {
        assert_eq!(normalize_text("\u{feff}a\r\nb\n"), "a\nb\n");
        assert_eq!(normalize_text("a\nb"), "a\nb");
    }

    #[test]
    fn disk_matches_compares_normalized_content() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, "x\r\ny\r\n").unwrap();
        assert!(disk_matches(&f, "x\ny\n"));
        assert!(!disk_matches(&f, "x\nz\n"));
        assert!(!disk_matches(&dir.path().join("missing"), ""));
    }

    #[test]
    fn ignored_dirs_are_relative_to_root() {
        let root = Path::new("/ws/target/proj");
        assert!(!is_ignored(Path::new("/ws/target/proj/src/a.rs"), root));
        assert!(is_ignored(Path::new("/ws/target/proj/.git/index"), root));
        assert!(is_ignored(
            Path::new("/ws/target/proj/target/debug/x"),
            root
        ));
        assert!(is_ignored(
            Path::new("/ws/target/proj/web/node_modules/m/i.js"),
            root
        ));
    }

    #[test]
    fn classify_separates_structural_and_content_events() {
        use notify::event::{AccessKind, CreateKind, DataChange, RemoveKind, RenameMode};
        assert_eq!(classify(&EventKind::Access(AccessKind::Any)), None);
        assert_eq!(classify(&EventKind::Create(CreateKind::File)), Some(true));
        assert_eq!(classify(&EventKind::Remove(RemoveKind::Any)), Some(true));
        assert_eq!(
            classify(&EventKind::Modify(ModifyKind::Name(RenameMode::Both))),
            Some(true)
        );
        assert_eq!(
            classify(&EventKind::Modify(ModifyKind::Data(DataChange::Content))),
            Some(false)
        );
    }

    #[test]
    fn add_event_drops_ignored_paths() {
        use notify::event::{CreateKind, DataChange};
        let root = Path::new("/ws");
        let mut c = FsChanges::default();
        c.add_event(
            notify::Event::new(EventKind::Create(CreateKind::File)).add_path("/ws/.git/x".into()),
            root,
        );
        assert!(c.is_empty(), "ignored create must not mark the tree stale");
        c.add_event(
            notify::Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Any)))
                .add_path("/ws/a.rs".into()),
            root,
        );
        assert!(c.paths.contains(Path::new("/ws/a.rs")));
        assert!(!c.structure_changed);
    }

    /// Wait (bounded) for the watcher to report a batch matching `pred`.
    fn wait_for(w: &FileWatcher, pred: impl Fn(&FsChanges) -> bool) -> Option<FsChanges> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut all = FsChanges::default();
        while Instant::now() < deadline {
            if let Some(c) = w.drain() {
                all.merge(c);
                if pred(&all) {
                    return Some(all);
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        None
    }

    #[test]
    fn watcher_reports_debounced_changes_and_ignores_git_dir() {
        let dir = tempfile::tempdir().unwrap();
        // Canonical root so reported paths compare equal (e.g. macOS /private).
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let w = FileWatcher::start(&root, Duration::from_millis(50), None).unwrap();

        std::fs::write(root.join(".git").join("index"), "x").unwrap();
        let f = root.join("a.txt");
        std::fs::write(&f, "hello").unwrap();
        let got = wait_for(&w, |c| c.paths.iter().any(|p| p.ends_with("a.txt")))
            .expect("watcher should report the new file");
        assert!(got.structure_changed, "file creation is structural");
        assert!(
            got.paths.iter().all(|p| !is_ignored(p, &root)),
            "ignored paths leaked: {:?}",
            got.paths
        );
    }
}
