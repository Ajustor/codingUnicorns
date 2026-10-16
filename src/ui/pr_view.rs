//! The tab of a pull request: its description, the files it changes and
//! their side-by-side diff with the review threads. Loads in the background.

use super::diff_layout::unfold_for_threads;
use super::diff_view::{kind_icon, DiffEvent, DiffInput, DiffView};
use super::theme::Palette;
use crate::git::compare::{self, DiffContent, FileDiff};
use crate::github::{self, PullDetail, PullSummary, RepoRef, ReviewThread};
use crate::plugin::manager::PluginManager;
use egui::RichText;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use egui_phosphor::regular;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Mutex;
use std::time::Duration;

/// Fetches write to the repository: one at a time, whatever the tab.
static FETCH: Mutex<()> = Mutex::new(());

/// Everything a pull request tab shows.
pub struct PrData {
    pub detail: PullDetail,
    pub files: Vec<FileDiff>,
    pub threads: Vec<ReviewThread>,
    /// HEAD is the pull request's head: opened files match the diff.
    pub checked_out: bool,
}

/// What the tab asks the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrAction {
    /// Open `path` (relative to the repository root) at 0-based `line`.
    OpenFile { path: String, line: usize },
    /// Check the pull request out on local branch `branch`, tracking
    /// `merge_ref` on origin (see `compare::pull_merge_ref`).
    Checkout {
        branch: String,
        head_sha: String,
        merge_ref: String,
    },
}

/// Frames a click is given to scroll the diff before the list follows the
/// reader again.
const SETTLE_FRAMES: u8 = 6;

type Loaded = Receiver<Result<PrData, String>>;

enum State {
    /// First load, nothing to show yet.
    Loading(Loaded),
    Failed(String),
    Ready(Box<PrData>),
}

/// A message under the header.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    Info(String),
    Error(String),
}

/// A "Show anyway" in flight.
struct FullText {
    file: usize,
    /// The blobs the load was started for: a refresh may have replaced them.
    old: Option<git2::Oid>,
    new: Option<git2::Oid>,
    rx: Receiver<Result<DiffContent, String>>,
}

pub struct PrView {
    pub number: u64,
    title: String,
    workdir: PathBuf,
    repo: RepoRef,
    config_token: String,
    state: State,
    /// A refresh in flight while `state` is `Ready`.
    refresh: Option<Loaded>,
    /// Files marked viewed: path → the content they were viewed with.
    viewed: HashMap<String, Option<git2::Oid>>,
    /// Collapse chosen with the chevron: path → collapsed. Wins over the
    /// viewed state until the file's checkbox is toggled.
    collapsed: HashMap<String, bool>,
    /// File highlighted in the list: the last one clicked, then the one the
    /// reader scrolls to.
    current_file: Option<usize>,
    /// File under the top of the diff at the last frame.
    last_top: Option<usize>,
    /// Frames left after a click during which the list ignores where the diff
    /// ends up (a header that can't reach the top, the end of the content).
    settling: u8,
    diff: DiffView,
    /// Review threads of each file, as `files`.
    thread_counts: Vec<usize>,
    full_text: Option<FullText>,
    notice: Option<Notice>,
    /// Rects of the header labels, for the layout tests.
    #[cfg(test)]
    probe: Vec<(&'static str, egui::Rect)>,
    /// Rects of the rows of the files list, for the click tests.
    #[cfg(test)]
    file_rows: Vec<egui::Rect>,
}

impl PrView {
    /// Open pull request `pull` of `repo` and start loading it.
    pub fn open(workdir: PathBuf, repo: RepoRef, pull: &PullSummary, config_token: String) -> Self {
        let mut view = Self::with_state(
            workdir,
            repo,
            pull.number,
            pull.title.clone(),
            State::Failed(String::new()),
        );
        view.config_token = config_token;
        view.reload();
        view
    }

    /// A view showing `data`, loading nothing.
    pub fn from_data(workdir: PathBuf, repo: RepoRef, data: PrData) -> Self {
        let number = data.detail.summary.number;
        let title = data.detail.summary.title.clone();
        let mut view = Self::with_state(workdir, repo, number, title, State::Failed(String::new()));
        view.set_data(data);
        view
    }

    fn with_state(
        workdir: PathBuf,
        repo: RepoRef,
        number: u64,
        title: String,
        state: State,
    ) -> Self {
        Self {
            number,
            title,
            workdir,
            repo,
            config_token: String::new(),
            state,
            refresh: None,
            viewed: HashMap::new(),
            collapsed: HashMap::new(),
            current_file: None,
            last_top: None,
            settling: 0,
            diff: DiffView::default(),
            thread_counts: Vec::new(),
            full_text: None,
            notice: None,
            #[cfg(test)]
            probe: Vec::new(),
            #[cfg(test)]
            file_rows: Vec::new(),
        }
    }

    /// Show `data` and forget what was computed from the previous one.
    fn set_data(&mut self, data: PrData) {
        self.thread_counts = thread_counts(&data.files, &data.threads);
        self.state = State::Ready(Box::new(data));
        self.refresh = None;
        self.full_text = None;
        self.notice = None;
        self.current_file = None;
        self.last_top = None;
        self.diff.reset();
    }

    /// Load (again) the pull request in the background; "viewed" marks are
    /// kept. When it is showing, it stays on screen until the new data is in.
    pub fn reload(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (workdir, repo, token, number) = (
            self.workdir.clone(),
            self.repo.clone(),
            self.config_token.clone(),
            self.number,
        );
        let spawned = std::thread::Builder::new()
            .name(format!("pull-{number}"))
            .spawn(move || {
                let _ = tx.send(load(&workdir, repo, number, &token));
            });
        let showing = matches!(self.state, State::Ready(_));
        match (spawned, showing) {
            (Ok(_), true) => {
                self.refresh = Some(rx);
                self.notice = None;
            }
            (Ok(_), false) => {
                self.state = State::Loading(rx);
                self.notice = None;
            }
            (Err(e), true) => {
                self.notice = Some(Notice::Error(format!("Could not start loading: {e}")));
            }
            (Err(e), false) => {
                self.state = State::Failed(format!("Could not start loading: {e}"));
            }
        }
    }

    /// The F7 stop reached last.
    pub fn current_change(&self) -> Option<usize> {
        self.diff.current_change()
    }

    pub fn diff(&self) -> &DiffView {
        &self.diff
    }

    /// After a checkout: whether HEAD is now the pull request's head.
    pub fn set_checked_out(&mut self, on_head: bool) {
        if let State::Ready(data) = &mut self.state {
            data.checked_out = on_head;
        }
    }

    /// Mark file `file` viewed, or not viewed any more.
    pub fn toggle_viewed(&mut self, file: usize) {
        let State::Ready(data) = &self.state else {
            return;
        };
        let Some(f) = data.files.get(file) else {
            return;
        };
        if is_viewed(&self.viewed, f) {
            self.viewed.remove(f.path());
        } else {
            self.viewed.insert(f.path().to_string(), f.version());
        }
        // Viewed folds the file, not viewed unfolds it: the chevron's choice
        // is forgotten.
        self.collapsed.remove(f.path());
        self.diff.invalidate_stops();
        self.diff.collapse_anchor(file);
    }

    /// Collapse file `file`, or expand it, whatever its viewed mark.
    pub fn toggle_collapsed(&mut self, file: usize) {
        let State::Ready(data) = &self.state else {
            return;
        };
        let Some(f) = data.files.get(file) else {
            return;
        };
        let now = is_collapsed(&self.viewed, &self.collapsed, f);
        self.collapsed.insert(f.path().to_string(), !now);
        self.diff.invalidate_stops();
        self.diff.collapse_anchor(file);
    }

    /// A click on the row of `file` in the list: it becomes the current file,
    /// is expanded if need be, and its header goes to the top of the diff.
    pub fn go_to_file(&mut self, file: usize) {
        let State::Ready(data) = &self.state else {
            return;
        };
        let Some(f) = data.files.get(file) else {
            return;
        };
        self.current_file = Some(file);
        self.settling = SETTLE_FRAMES;
        if is_collapsed(&self.viewed, &self.collapsed, f) {
            self.collapsed.insert(f.path().to_string(), false);
            self.diff.invalidate_stops();
        }
        self.diff.scroll_to_file(file);
    }

    /// File highlighted in the list.
    pub fn current_file(&self) -> Option<usize> {
        self.current_file
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        plugins: Option<&PluginManager>,
        md_cache: &mut CommonMarkCache,
    ) -> Option<PrAction> {
        self.poll();
        // The host removes the spacing between widgets: this tab needs it.
        ui.spacing_mut().item_spacing = ui.ctx().style().spacing.item_spacing;
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(12, 0))
            .show(ui, |ui| self.show_content(ui, palette, plugins, md_cache))
            .inner
    }

    fn show_content(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        plugins: Option<&PluginManager>,
        md_cache: &mut CommonMarkCache,
    ) -> Option<PrAction> {
        match &self.state {
            State::Loading(_) => {
                ui.add_space(24.0);
                ui.label(
                    RichText::new(format!("#{} {}", self.number, self.title))
                        .heading()
                        .strong(),
                );
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Loading pull request #{}…", self.number));
                });
                ui.ctx().request_repaint_after(Duration::from_millis(100));
                return None;
            }
            State::Failed(message) => {
                let message = message.clone();
                ui.add_space(24.0);
                ui.label(RichText::new(message).color(palette.error));
                if ui.button("Retry").clicked() {
                    self.reload();
                }
                return None;
            }
            State::Ready(_) => {}
        }
        if self.full_text.is_some() || self.refresh.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        let mut action = None;
        let mut refresh = false;
        let mut toggled = None;
        let mut clicked = None;
        let mut events = Vec::new();
        {
            let refreshing = self.refresh.is_some();
            let Self {
                number,
                repo,
                state: State::Ready(data),
                viewed,
                collapsed,
                current_file,
                last_top,
                settling,
                diff,
                notice,
                thread_counts,
                #[cfg(test)]
                probe,
                #[cfg(test)]
                file_rows,
                ..
            } = self
            else {
                return None;
            };
            let number = *number;
            let viewed_flags: Vec<bool> = data.files.iter().map(|f| is_viewed(viewed, f)).collect();
            let collapsed_flags: Vec<bool> = data
                .files
                .iter()
                .map(|f| is_collapsed(viewed, collapsed, f))
                .collect();
            #[cfg(test)]
            file_rows.clear();
            let d = &data.detail;

            // ── Header ──
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!("#{} {}", d.summary.number, d.summary.title))
                    .heading()
                    .strong(),
            );
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !refreshing,
                        egui::Button::new(format!("{} Refresh", regular::ARROW_CLOCKWISE)),
                    )
                    .clicked()
                {
                    refresh = true;
                }
                if refreshing {
                    ui.spinner();
                }
                if ui
                    .button(format!("{} Checkout", regular::GIT_BRANCH))
                    .on_hover_text("Check the pull request out on a local branch")
                    .clicked()
                {
                    action = Some(checkout_action(d, repo));
                }
                if ui
                    .button(format!("{} Open on GitHub", regular::ARROW_SQUARE_OUT))
                    .clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(&d.html_url));
                }
            });
            ui.horizontal_wrapped(|ui| {
                let author = ui.label(RichText::new(&d.summary.author).strong());
                let meta = ui.label(
                    RichText::new(format!("{} → {}", d.summary.head_ref, d.summary.base_ref))
                        .color(palette.text_muted),
                );
                #[cfg(test)]
                {
                    probe.clear();
                    probe.push(("author", author.rect));
                    probe.push(("meta", meta.rect));
                }
                #[cfg(not(test))]
                let _ = (author, meta);
                if d.summary.draft {
                    ui.label(RichText::new("Draft").small().color(palette.warning));
                }
            });
            let hint = if data.checked_out {
                "Double-click a line to open it in the editor."
            } else {
                "Double-click a line to open it in the editor. Your working copy is not on this pull request's head: the file may differ."
            };
            ui.label(RichText::new(hint).small().color(palette.text_faint));
            match notice {
                Some(Notice::Info(n)) => {
                    ui.label(RichText::new(n.as_str()).small().color(palette.text_muted));
                }
                Some(Notice::Error(n)) => {
                    ui.label(RichText::new(n.as_str()).small().color(palette.error));
                }
                None => {}
            }
            egui::CollapsingHeader::new("Description")
                .id_salt(("pr-description", number))
                .default_open(true)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("pr-description-scroll", number))
                        .max_height(220.0)
                        .show(ui, |ui| {
                            if d.body.trim().is_empty() {
                                ui.label(
                                    RichText::new("No description")
                                        .italics()
                                        .color(palette.text_muted),
                                );
                            } else {
                                CommonMarkViewer::new().show(ui, md_cache, &d.body);
                            }
                        });
                });
            ui.separator();

            // ── F7 / Shift+F7 ──
            if !ui.ctx().wants_keyboard_input() {
                let (next, prev) = ui.input(|i| {
                    let f7 = i.key_pressed(egui::Key::F7);
                    (f7 && !i.modifiers.shift, f7 && i.modifiers.shift)
                });
                if next {
                    diff.request_next();
                } else if prev {
                    diff.request_prev();
                }
            }

            // ── Files ──
            let max_width = (ui.available_width() * 0.5).max(160.0);
            egui::SidePanel::left(egui::Id::new(("pr-files", number)))
                .resizable(true)
                .default_width(260.0_f32.min(max_width))
                .width_range(160.0..=max_width.min(600.0))
                .show_inside(ui, |ui| {
                    ui.label(
                        RichText::new(format!("FILES ({})", data.files.len()))
                            .strong()
                            .small(),
                    );
                    egui::ScrollArea::vertical()
                        .id_salt(("pr-files-scroll", number))
                        .show(ui, |ui| {
                            for (i, f) in data.files.iter().enumerate() {
                                // The whole row selects the file. Made
                                // first, so the checkbox on top still gets
                                // its own clicks; the labels don't sense
                                // anything, or they would take the click.
                                let row_rect = egui::Rect::from_min_size(
                                    ui.cursor().min,
                                    egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                                );
                                #[cfg(test)]
                                file_rows.push(row_rect);
                                let row = ui.interact(
                                    row_rect,
                                    ui.id().with(("pr-file-row", i)),
                                    egui::Sense::click(),
                                );
                                if *current_file == Some(i) {
                                    ui.painter().rect_filled(
                                        row_rect,
                                        2.0,
                                        palette.accent.gamma_multiply(0.25),
                                    );
                                } else if row.hovered() {
                                    ui.painter()
                                        .rect_filled(row_rect, 2.0, palette.surface_raised);
                                }
                                if row.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                ui.horizontal(|ui| {
                                    let mut seen = viewed_flags[i];
                                    if ui.checkbox(&mut seen, "").on_hover_text("Viewed").changed()
                                    {
                                        toggled = Some(i);
                                    }
                                    let (icon, color) = kind_icon(f.kind, &palette);
                                    let plain =
                                        |text: RichText| egui::Label::new(text).selectable(false);
                                    ui.add(plain(
                                        RichText::new(icon).monospace().small().color(color),
                                    ));
                                    if matches!(f.content, DiffContent::Text(_)) {
                                        ui.add(plain(
                                            RichText::new(format!("+{}", f.additions))
                                                .small()
                                                .color(palette.git_added),
                                        ));
                                        ui.add(plain(
                                            RichText::new(format!("−{}", f.deletions))
                                                .small()
                                                .color(palette.git_removed),
                                        ));
                                    }
                                    let threads = thread_counts.get(i).copied().unwrap_or(0);
                                    if threads > 0 {
                                        ui.add(plain(
                                            RichText::new(format!(
                                                "{} {threads}",
                                                regular::CHAT_CIRCLE
                                            ))
                                            .small()
                                            .color(palette.text_muted),
                                        ));
                                    }
                                    ui.add(
                                        plain(RichText::new(f.display_path()).small()).truncate(),
                                    );
                                });
                                if row.clicked() {
                                    clicked = Some(i);
                                }
                                row.on_hover_text(f.display_path());
                            }
                        });
                });

            // ── Diff ──
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    let input = DiffInput {
                        files: &data.files,
                        threads: &data.threads,
                        viewed: &viewed_flags,
                        collapsed: &collapsed_flags,
                        palette,
                        plugins,
                    };
                    events = ui
                        .push_id(("pr", number), |ui| diff.show(ui, &input, md_cache))
                        .inner;
                    // The list follows the reader once they scroll.
                    let top = diff.top_file();
                    if *settling > 0 {
                        *settling -= 1;
                        *last_top = top;
                    } else if top != *last_top {
                        if top.is_some() {
                            *current_file = top;
                        }
                        *last_top = top;
                    }
                });
        }
        if let Some(file) = toggled {
            self.toggle_viewed(file);
        }
        if let Some(file) = clicked {
            self.go_to_file(file);
        }
        if let Some(a) = self.handle_events(events) {
            action = Some(a);
        }
        if refresh {
            self.reload();
        }
        action
    }

    /// Apply what the diff reported; an event may ask the app to open a file.
    fn handle_events(&mut self, events: Vec<DiffEvent>) -> Option<PrAction> {
        let mut action = None;
        for event in events {
            match event {
                DiffEvent::ToggleViewed(file) => self.toggle_viewed(file),
                DiffEvent::ToggleCollapsed(file) => self.toggle_collapsed(file),
                DiffEvent::Unfold { file, row } => {
                    let State::Ready(data) = &mut self.state else {
                        continue;
                    };
                    if let Some(DiffContent::Text(text)) =
                        data.files.get_mut(file).map(|f| &mut f.content)
                    {
                        text.unfold(row);
                    }
                    self.diff.invalidate();
                }
                DiffEvent::ShowAnyway(file) => self.start_full_text(file),
                DiffEvent::OpenLine { file, line } => {
                    let State::Ready(data) = &self.state else {
                        continue;
                    };
                    if let Some(path) = data.files.get(file).and_then(|f| f.new_path.clone()) {
                        action = Some(PrAction::OpenFile { path, line });
                    }
                }
            }
        }
        action
    }

    /// Take the finished background loads, if any.
    fn poll(&mut self) {
        match &self.state {
            State::Loading(rx) => match recv(rx) {
                Some(Ok(data)) => self.set_data(data),
                Some(Err(e)) => self.state = State::Failed(e),
                None => {}
            },
            State::Ready(_) => {
                let finished = self.refresh.as_ref().and_then(recv);
                match finished {
                    Some(Ok(data)) => self.set_data(data),
                    Some(Err(e)) => {
                        self.refresh = None;
                        self.notice = Some(Notice::Error(e));
                    }
                    None => {}
                }
            }
            State::Failed(_) => {}
        }
        self.poll_full_text();
    }

    fn poll_full_text(&mut self) {
        let Some(full) = &self.full_text else {
            return;
        };
        let result = match full.rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("Loading the file stopped unexpectedly.".into()),
        };
        let Some(FullText { file, old, new, .. }) = self.full_text.take() else {
            return;
        };
        if matches!(self.notice, Some(Notice::Info(_))) {
            self.notice = None;
        }
        let State::Ready(data) = &mut self.state else {
            return;
        };
        match result {
            Ok(content) => {
                // A refresh may have replaced the file meanwhile.
                let Some(f) = data
                    .files
                    .get_mut(file)
                    .filter(|f| f.old_blob == old && f.new_blob == new)
                else {
                    return;
                };
                f.set_content(content);
                unfold_for_threads(std::slice::from_mut(f), &data.threads);
                self.thread_counts = thread_counts(&data.files, &data.threads);
                self.diff.invalidate_stops();
            }
            Err(e) => self.notice = Some(Notice::Error(e)),
        }
    }

    /// Load a large file's diff anyway, in the background.
    fn start_full_text(&mut self, file: usize) {
        if self.full_text.is_some() {
            return;
        }
        let State::Ready(data) = &self.state else {
            return;
        };
        let Some(f) = data.files.get(file) else {
            return;
        };
        let (workdir, old, new) = (self.workdir.clone(), f.old_blob, f.new_blob);
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("pull-file".into())
            .spawn(move || {
                let _ = tx.send(compare::load_text(&workdir, old, new));
            });
        match spawned {
            Ok(_) => {
                self.full_text = Some(FullText { file, old, new, rx });
                self.notice = Some(Notice::Info("Loading the full file…".into()));
            }
            Err(e) => self.notice = Some(Notice::Error(format!("Could not load the file: {e}"))),
        }
    }
}

/// The result of a background load, once it is in. A load whose thread died
/// without answering is an error.
fn recv<T>(rx: &Receiver<Result<T, String>>) -> Option<Result<T, String>> {
    match rx.try_recv() {
        Ok(result) => Some(result),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(Err("Loading stopped unexpectedly.".into())),
    }
}

/// What the Checkout button asks for.
fn checkout_action(detail: &PullDetail, repo: &RepoRef) -> PrAction {
    let from_fork = detail
        .head_repo
        .as_deref()
        .is_none_or(|r| !r.eq_ignore_ascii_case(&repo.full_name()));
    let (n, head) = (detail.summary.number, detail.summary.head_ref.as_str());
    PrAction::Checkout {
        branch: compare::pull_branch_name(n, head, from_fork),
        head_sha: detail.head_sha.clone(),
        merge_ref: compare::pull_merge_ref(n, head, from_fork),
    }
}

/// Review threads of each file: by its current path, else by its old one
/// (the rule of `diff_layout`). Threads of files not in the diff count nowhere.
fn thread_counts(files: &[FileDiff], threads: &[ReviewThread]) -> Vec<usize> {
    let mut counts = vec![0; files.len()];
    for thread in threads {
        let file = files
            .iter()
            .position(|f| f.path() == thread.path)
            .or_else(|| {
                files
                    .iter()
                    .position(|f| f.old_path.as_deref() == Some(thread.path.as_str()))
            });
        if let Some(file) = file {
            counts[file] += 1;
        }
    }
    counts
}

/// `f` is marked viewed, with the content it has now.
fn is_viewed(viewed: &HashMap<String, Option<git2::Oid>>, f: &FileDiff) -> bool {
    viewed
        .get(f.path())
        .is_some_and(|version| *version == f.version())
}

/// `f` is folded to its header: the chevron's choice if any, else its viewed
/// state.
fn is_collapsed(
    viewed: &HashMap<String, Option<git2::Oid>>,
    collapsed: &HashMap<String, bool>,
    f: &FileDiff,
) -> bool {
    collapsed
        .get(f.path())
        .copied()
        .unwrap_or_else(|| is_viewed(viewed, f))
}

/// Fetch and diff pull request `number`, with its review threads.
fn load(workdir: &Path, repo: RepoRef, number: u64, config_token: &str) -> Result<PrData, String> {
    let client = github::Client::new(repo, github::auth::resolve(config_token));
    let detail = client.pull_detail(number).map_err(|e| e.to_string())?;
    let (threads, files) = std::thread::scope(|s| {
        let threads = s.spawn(|| client.review_threads(number));
        let fetched = {
            let _one_at_a_time = FETCH.lock().unwrap_or_else(|e| e.into_inner());
            compare::fetch_pull(workdir, number, &detail.summary.base_ref)
        };
        let files =
            fetched.and_then(|()| compare::diff_pull(workdir, &detail.base_sha, &detail.head_sha));
        (threads.join(), files)
    });
    let mut files = files?;
    let threads = threads
        .map_err(|_| "Loading the review comments stopped unexpectedly.".to_string())?
        .map_err(|e| e.to_string())?;
    unfold_for_threads(&mut files, &threads);
    let checked_out = git2::Repository::discover(workdir)
        .ok()
        .and_then(|r| r.head().ok()?.target())
        .is_some_and(|oid| oid.to_string() == detail.head_sha);
    Ok(PrData {
        detail,
        files,
        threads,
        checked_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::compare::text_file;
    use crate::ui::diff_layout::Item;

    fn repo() -> RepoRef {
        RepoRef {
            owner: "me".into(),
            name: "proj".into(),
        }
    }

    fn fixture() -> PrView {
        PrView::from_data(PathBuf::from("."), repo(), fixture_data())
    }

    fn fixture_data() -> PrData {
        PrData {
            detail: PullDetail {
                summary: PullSummary {
                    number: 7,
                    title: "Fix".into(),
                    author: "me".into(),
                    head_ref: "fix".into(),
                    base_ref: "main".into(),
                    draft: false,
                },
                body: "Body".into(),
                head_sha: "a".repeat(40),
                base_sha: "b".repeat(40),
                head_repo: Some("me/proj".into()),
                html_url: "https://github.com/me/proj/pull/7".into(),
            },
            // Two stops in the first file, one in the second.
            files: vec![
                text_file("a.rs", "a\nb\nc\nd\n", "A\nb\nc\n"),
                text_file("b.rs", "x\n", "y\n"),
            ],
            threads: vec![],
            checked_out: false,
        }
    }

    fn frame(
        ctx: &egui::Context,
        view: &mut PrView,
        cache: &mut CommonMarkCache,
        events: Vec<egui::Event>,
    ) -> Option<PrAction> {
        // Like a real backend: the held modifiers accompany the key events.
        let modifiers = events
            .iter()
            .find_map(|e| match e {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let raw = egui::RawInput {
            modifiers,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let palette = Palette::from_theme(&crate::config::Theme::default());
        let mut action = None;
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                action = view.show(ui, palette, None, cache);
            });
        });
        action
    }

    fn f7(shift: bool) -> egui::Event {
        egui::Event::Key {
            key: egui::Key::F7,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: if shift {
                egui::Modifiers::SHIFT
            } else {
                egui::Modifiers::NONE
            },
        }
    }

    #[test]
    fn f7_walks_through_the_changes() {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let mut view = fixture();
        frame(&ctx, &mut view, &mut cache, vec![]); // lays the diff out
        frame(&ctx, &mut view, &mut cache, vec![f7(false)]);
        assert_eq!(view.current_change(), Some(0));
        frame(&ctx, &mut view, &mut cache, vec![f7(false)]);
        assert_eq!(view.current_change(), Some(1));
        frame(&ctx, &mut view, &mut cache, vec![f7(true)]);
        assert_eq!(view.current_change(), Some(0));
    }

    #[test]
    fn a_viewed_file_collapses_to_its_header() {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let mut view = fixture();
        frame(&ctx, &mut view, &mut cache, vec![]);
        view.toggle_viewed(0);
        frame(&ctx, &mut view, &mut cache, vec![]);
        let items = &view.diff().layout().unwrap().items();
        assert!(!items.iter().any(|i| matches!(i, Item::Row { file: 0, .. })));
        assert!(items.iter().any(|i| matches!(i, Item::Row { file: 1, .. })));
        view.toggle_viewed(0);
        frame(&ctx, &mut view, &mut cache, vec![]);
        let items = &view.diff().layout().unwrap().items();
        assert!(items.iter().any(|i| matches!(i, Item::Row { file: 0, .. })));
    }

    #[test]
    fn a_new_version_of_a_viewed_file_is_unchecked() {
        let mut view = fixture();
        view.toggle_viewed(0);
        let State::Ready(data) = &mut view.state else {
            panic!("not ready")
        };
        assert!(is_viewed(&view.viewed, &data.files[0]));
        data.files[0].new_blob = Some(git2::Oid::from_str(&"c".repeat(40)).unwrap());
        assert!(!is_viewed(&view.viewed, &data.files[0]));
    }

    fn data(view: &PrView) -> &PrData {
        let State::Ready(data) = &view.state else {
            panic!("not ready")
        };
        data
    }

    fn data_mut(view: &mut PrView) -> &mut PrData {
        let State::Ready(data) = &mut view.state else {
            panic!("not ready")
        };
        data
    }

    fn blob(c: &str) -> Option<git2::Oid> {
        Some(git2::Oid::from_str(&c.repeat(40)).unwrap())
    }

    #[test]
    fn the_header_keeps_its_spacing_when_the_host_removes_it() {
        for width in [1200.0, 260.0] {
            let ctx = egui::Context::default();
            let mut cache = CommonMarkCache::default();
            let mut view = fixture();
            let palette = Palette::from_theme(&crate::config::Theme::default());
            for _ in 0..2 {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 800.0),
                    )),
                    ..Default::default()
                };
                let _ = ctx.run(raw, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        // What the host does before routing tab content.
                        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                        view.show(ui, palette, None, &mut cache);
                    });
                });
            }
            let rect = |name: &str| {
                view.probe
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, r)| *r)
                    .unwrap()
            };
            assert!(
                !rect("author").intersects(rect("meta")),
                "width {width}: {:?} touches {:?}",
                rect("author"),
                rect("meta")
            );
        }
    }

    #[test]
    fn checkout_action_by_origin_of_the_head() {
        let mut detail = data(&fixture()).detail.clone();
        let expect = |from_fork: bool| PrAction::Checkout {
            branch: compare::pull_branch_name(7, "fix", from_fork),
            head_sha: "a".repeat(40),
            merge_ref: compare::pull_merge_ref(7, "fix", from_fork),
        };
        // Same repository, whatever the case.
        detail.head_repo = Some("ME/Proj".into());
        assert_eq!(checkout_action(&detail, &repo()), expect(false));
        // A fork.
        detail.head_repo = Some("you/proj".into());
        let action = checkout_action(&detail, &repo());
        assert_eq!(action, expect(true));
        assert_eq!(
            action,
            PrAction::Checkout {
                branch: "pr/7".into(),
                head_sha: "a".repeat(40),
                merge_ref: "refs/pull/7/head".into(),
            }
        );
        // A deleted fork counts as a fork.
        detail.head_repo = None;
        assert_eq!(checkout_action(&detail, &repo()), expect(true));
    }

    #[test]
    fn opening_a_line_opens_the_new_path_of_the_file() {
        let mut view = fixture();
        let action = view.handle_events(vec![DiffEvent::OpenLine { file: 1, line: 4 }]);
        assert_eq!(
            action,
            Some(PrAction::OpenFile {
                path: "b.rs".into(),
                line: 4
            })
        );
        data_mut(&mut view).files[1].new_path = None; // deleted
        let action = view.handle_events(vec![DiffEvent::OpenLine { file: 1, line: 4 }]);
        assert_eq!(action, None);
        let action = view.handle_events(vec![DiffEvent::OpenLine { file: 9, line: 0 }]);
        assert_eq!(action, None);
    }

    #[test]
    fn a_toggle_event_marks_the_file_viewed() {
        let mut view = fixture();
        assert_eq!(view.handle_events(vec![DiffEvent::ToggleViewed(0)]), None);
        assert!(is_viewed(&view.viewed, &data(&view).files[0]));
    }

    #[test]
    fn a_finished_first_load_shows_the_pull_request() {
        let mut view = fixture();
        let loaded = PrData {
            files: vec![text_file("z.rs", "a\n", "b\n")],
            ..fixture_data()
        };
        let (tx, rx) = std::sync::mpsc::channel();
        view.state = State::Loading(rx);
        view.poll();
        assert!(matches!(view.state, State::Loading(_)), "still loading");
        tx.send(Ok(loaded)).unwrap();
        view.poll();
        assert_eq!(data(&view).files.len(), 1);
        assert_eq!(view.thread_counts, vec![0]);
    }

    #[test]
    fn a_failed_first_load_shows_the_error() {
        let mut view = fixture();
        let (tx, rx) = std::sync::mpsc::channel();
        view.state = State::Loading(rx);
        tx.send(Err("no network".into())).unwrap();
        view.poll();
        assert!(matches!(&view.state, State::Failed(m) if m == "no network"));
    }

    #[test]
    fn a_load_that_dies_is_reported() {
        let mut view = fixture();
        let (tx, rx) = std::sync::mpsc::channel();
        view.state = State::Loading(rx);
        drop(tx);
        view.poll();
        assert!(matches!(&view.state, State::Failed(m) if m == "Loading stopped unexpectedly."));
    }

    #[test]
    fn a_failed_refresh_keeps_the_view() {
        let mut view = fixture();
        view.toggle_viewed(0);
        let (tx, rx) = std::sync::mpsc::channel();
        view.refresh = Some(rx);
        view.poll();
        assert!(view.refresh.is_some(), "still refreshing");
        tx.send(Err("rate limited".into())).unwrap();
        view.poll();
        assert_eq!(data(&view).files.len(), 2);
        assert!(view.refresh.is_none());
        assert_eq!(view.notice, Some(Notice::Error("rate limited".into())));
        assert!(is_viewed(&view.viewed, &data(&view).files[0]));
    }

    #[test]
    fn a_refresh_replaces_the_data_and_keeps_the_viewed_marks() {
        let mut view = fixture();
        view.toggle_viewed(0);
        let (tx, rx) = std::sync::mpsc::channel();
        view.refresh = Some(rx);
        tx.send(Ok(fixture_data())).unwrap();
        view.poll();
        assert!(view.refresh.is_none());
        assert_eq!(view.notice, None);
        assert!(is_viewed(&view.viewed, &data(&view).files[0]));
    }

    fn full_text_in_flight(
        view: &mut PrView,
        file: usize,
        old: Option<git2::Oid>,
        new: Option<git2::Oid>,
    ) -> std::sync::mpsc::Sender<Result<DiffContent, String>> {
        let (tx, rx) = std::sync::mpsc::channel();
        view.full_text = Some(FullText { file, old, new, rx });
        view.notice = Some(Notice::Info("Loading the full file…".into()));
        tx
    }

    fn text(old: &str, new: &str) -> DiffContent {
        DiffContent::Text(compare::TextDiff::new(old, new))
    }

    #[test]
    fn show_anyway_applies_the_loaded_content() {
        let mut view = fixture();
        data_mut(&mut view).files[1].content = DiffContent::TooLarge;
        let tx = full_text_in_flight(&mut view, 1, None, None);
        view.poll();
        assert!(view.full_text.is_some(), "still loading");
        assert!(matches!(view.notice, Some(Notice::Info(_))));
        tx.send(Ok(text("x\n", "y\nz\n"))).unwrap();
        view.poll();
        let f = &data(&view).files[1];
        assert!(matches!(f.content, DiffContent::Text(_)));
        assert!(f.additions > 0);
        assert!(view.full_text.is_none());
        assert_eq!(view.notice, None);
    }

    #[test]
    fn show_anyway_ignores_a_file_replaced_meanwhile() {
        let mut view = fixture();
        data_mut(&mut view).files[1].content = DiffContent::TooLarge;
        // Started for other blobs than the file has now.
        let tx = full_text_in_flight(&mut view, 1, blob("1"), blob("2"));
        tx.send(Ok(text("x\n", "y\n"))).unwrap();
        view.poll();
        assert!(matches!(
            data(&view).files[1].content,
            DiffContent::TooLarge
        ));
        assert!(view.full_text.is_none());
    }

    #[test]
    fn show_anyway_failure_is_an_error_notice() {
        let mut view = fixture();
        let tx = full_text_in_flight(&mut view, 1, None, None);
        tx.send(Err("blob missing".into())).unwrap();
        view.poll();
        assert_eq!(view.notice, Some(Notice::Error("blob missing".into())));
    }

    #[test]
    fn a_second_show_anyway_waits_for_the_first() {
        let mut view = fixture();
        let _tx = full_text_in_flight(&mut view, 1, None, None);
        view.start_full_text(0);
        assert_eq!(view.full_text.as_ref().map(|f| f.file), Some(1));
    }

    #[test]
    fn threads_are_counted_by_path_then_by_old_path() {
        use crate::github::api::{thread, Side};
        let mut renamed = text_file("new.rs", "a\n", "b\n");
        renamed.old_path = Some("old.rs".into());
        let files = vec![text_file("a.rs", "a\n", "b\n"), renamed];
        let threads = vec![
            thread("a.rs", Side::Right, Some(1)),
            thread("a.rs", Side::Right, Some(1)),
            thread("old.rs", Side::Left, Some(1)),
            thread("new.rs", Side::Right, Some(1)),
            thread("gone.rs", Side::Right, Some(1)),
        ];
        assert_eq!(thread_counts(&files, &threads), vec![2, 2]);
    }

    fn long_text_file(path: &str) -> FileDiff {
        let old: String = (0..200).map(|i| format!("l{i}\n")).collect();
        let mut new = old.clone();
        for i in [0, 100, 199] {
            new = new.replace(&format!("l{i}\n"), &format!("L{i}\n"));
        }
        text_file(path, &old, &new)
    }

    fn long_fixture() -> PrView {
        PrView::from_data(
            PathBuf::from("."),
            repo(),
            PrData {
                files: vec![
                    long_text_file("a.rs"),
                    long_text_file("b.rs"),
                    long_text_file("c.rs"),
                    long_text_file("d.rs"),
                ],
                ..fixture_data()
            },
        )
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn click_at(
        ctx: &egui::Context,
        view: &mut PrView,
        cache: &mut CommonMarkCache,
        pos: egui::Pos2,
    ) {
        frame(ctx, view, cache, vec![egui::Event::PointerMoved(pos)]);
        frame(ctx, view, cache, vec![pointer(pos, true)]);
        frame(ctx, view, cache, vec![pointer(pos, false)]);
    }

    fn header_is_at_the_top(view: &PrView, file: usize) -> bool {
        let layout = view.diff().layout().unwrap();
        let top = layout.top(layout.header_item(file).unwrap());
        let (viewport_top, _) = view.diff().viewport().unwrap();
        (viewport_top - top).abs() < 1.0
    }

    #[test]
    fn a_click_anywhere_on_a_file_row_scrolls_the_diff_to_its_header() {
        // Near the checkbox-free right edge, and on the path text.
        for dx in [
            |r: egui::Rect| r.right() - 3.0,
            |r: egui::Rect| r.left() + 110.0,
        ] {
            let ctx = egui::Context::default();
            let mut cache = CommonMarkCache::default();
            let mut view = long_fixture();
            frame(&ctx, &mut view, &mut cache, vec![]);
            frame(&ctx, &mut view, &mut cache, vec![]);
            let row = view.file_rows[1];
            click_at(
                &ctx,
                &mut view,
                &mut cache,
                egui::pos2(dx(row), row.center().y),
            );
            for _ in 0..4 {
                frame(&ctx, &mut view, &mut cache, vec![]);
            }
            assert_eq!(view.current_file(), Some(1));
            assert!(header_is_at_the_top(&view, 1), "b.rs at the top");
        }
    }

    #[test]
    fn a_click_before_the_first_diff_frame_scrolls_too() {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let mut view = long_fixture();
        frame(&ctx, &mut view, &mut cache, vec![]);
        view.go_to_file(2);
        for _ in 0..4 {
            frame(&ctx, &mut view, &mut cache, vec![]);
        }
        assert!(header_is_at_the_top(&view, 2), "c.rs at the top");
        // Right after an invalidation as well.
        view.diff.invalidate();
        view.go_to_file(1);
        frame(&ctx, &mut view, &mut cache, vec![]);
        frame(&ctx, &mut view, &mut cache, vec![]);
        assert!(header_is_at_the_top(&view, 1), "b.rs at the top");
    }

    #[test]
    fn the_checkbox_of_a_row_marks_viewed_without_scrolling() {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let mut view = long_fixture();
        frame(&ctx, &mut view, &mut cache, vec![]);
        frame(&ctx, &mut view, &mut cache, vec![]);
        let row = view.file_rows[1];
        click_at(
            &ctx,
            &mut view,
            &mut cache,
            egui::pos2(row.left() + 10.0, row.center().y),
        );
        assert!(is_viewed(&view.viewed, &data(&view).files[1]));
        assert_eq!(view.current_file(), Some(0), "the row was not clicked");
    }

    fn has_rows(view: &PrView, file: usize) -> bool {
        view.diff()
            .layout()
            .unwrap()
            .items()
            .iter()
            .any(|i| matches!(i, Item::Row { file: f, .. } if *f == file))
    }

    #[test]
    fn the_chevron_collapses_an_unviewed_file_and_expands_a_viewed_one() {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let mut view = fixture();
        frame(&ctx, &mut view, &mut cache, vec![]);
        view.handle_events(vec![DiffEvent::ToggleCollapsed(0)]);
        frame(&ctx, &mut view, &mut cache, vec![]);
        assert!(!has_rows(&view, 0));
        assert!(has_rows(&view, 1));
        assert!(!is_viewed(&view.viewed, &data(&view).files[0]));
        // Viewed, collapsed by the mark, expanded by the chevron.
        view.toggle_viewed(1);
        frame(&ctx, &mut view, &mut cache, vec![]);
        assert!(!has_rows(&view, 1));
        view.handle_events(vec![DiffEvent::ToggleCollapsed(1)]);
        frame(&ctx, &mut view, &mut cache, vec![]);
        assert!(has_rows(&view, 1));
        assert!(is_viewed(&view.viewed, &data(&view).files[1]));
    }

    #[test]
    fn checking_viewed_after_a_chevron_override_collapses_and_clears_it() {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let mut view = fixture();
        frame(&ctx, &mut view, &mut cache, vec![]);
        // Expanded by hand while unviewed: override false.
        view.toggle_collapsed(0);
        view.toggle_collapsed(0);
        assert_eq!(view.collapsed.get("a.rs"), Some(&false));
        view.toggle_viewed(0);
        assert!(view.collapsed.is_empty());
        frame(&ctx, &mut view, &mut cache, vec![]);
        assert!(!has_rows(&view, 0));
        // Unchecking expands, even after a chevron collapse.
        view.toggle_collapsed(0); // expand a viewed file
        view.toggle_viewed(0); // unviewed
        frame(&ctx, &mut view, &mut cache, vec![]);
        assert!(view.collapsed.is_empty());
        assert!(has_rows(&view, 0));
    }

    #[test]
    fn a_refresh_keeps_the_chevron_choices() {
        let mut view = fixture();
        view.toggle_collapsed(0);
        let (tx, rx) = std::sync::mpsc::channel();
        view.refresh = Some(rx);
        tx.send(Ok(fixture_data())).unwrap();
        view.poll();
        assert_eq!(view.collapsed.get("a.rs"), Some(&true));
    }

    #[test]
    fn the_current_file_follows_the_click_then_the_reader() {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let mut view = long_fixture();
        frame(&ctx, &mut view, &mut cache, vec![]);
        frame(&ctx, &mut view, &mut cache, vec![]);
        assert_eq!(view.current_file(), Some(0));
        let row = view.file_rows[2];
        click_at(
            &ctx,
            &mut view,
            &mut cache,
            egui::pos2(row.right() - 3.0, row.center().y),
        );
        for _ in 0..10 {
            frame(&ctx, &mut view, &mut cache, vec![]);
        }
        assert_eq!(view.current_file(), Some(2));
        // The last file can't reach the top: it stays the current one.
        let row = view.file_rows[3];
        click_at(
            &ctx,
            &mut view,
            &mut cache,
            egui::pos2(row.right() - 3.0, row.center().y),
        );
        for _ in 0..10 {
            frame(&ctx, &mut view, &mut cache, vec![]);
        }
        assert_eq!(view.current_file(), Some(3));
        // The reader scrolls back to the first file.
        view.diff.scroll_to_file(0);
        for _ in 0..4 {
            frame(&ctx, &mut view, &mut cache, vec![]);
        }
        assert_eq!(view.current_file(), Some(0));
    }

    #[test]
    fn going_to_a_collapsed_file_expands_it() {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let mut view = long_fixture();
        frame(&ctx, &mut view, &mut cache, vec![]);
        view.toggle_viewed(1);
        frame(&ctx, &mut view, &mut cache, vec![]);
        assert!(!has_rows(&view, 1));
        view.go_to_file(1);
        for _ in 0..4 {
            frame(&ctx, &mut view, &mut cache, vec![]);
        }
        assert!(has_rows(&view, 1));
        assert!(is_viewed(&view.viewed, &data(&view).files[1]));
        assert!(header_is_at_the_top(&view, 1));
    }
}
