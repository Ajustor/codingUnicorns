//! The PULL REQUESTS section of the Git panel: the open pull requests of
//! `origin`'s GitHub repository, loaded in the background when the section
//! is first expanded.

use crate::git::GitStatus;
use crate::github::{self, GithubError, PullSummary, RepoRef};
use egui_phosphor::regular;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

type Loaded = Result<(RepoRef, Vec<PullSummary>), GithubError>;

pub struct PrList {
    /// Folder the list belongs to; another workspace starts over.
    loaded_for: Option<PathBuf>,
    loading: Option<Receiver<Loaded>>,
    repo: Option<RepoRef>,
    pulls: Vec<PullSummary>,
    error: Option<String>,
    /// The pull request clicked, for the app to open.
    pub open_request: Option<(RepoRef, PullSummary)>,
}

impl PrList {
    pub fn new() -> Self {
        Self {
            loaded_for: None,
            loading: None,
            repo: None,
            pulls: Vec::new(),
            error: None,
            open_request: None,
        }
    }

    /// A list already loaded for `path`, for tests of the panel.
    #[cfg(test)]
    pub fn loaded_for_test(path: PathBuf) -> Self {
        let mut list = Self::new();
        list.loaded_for = Some(path);
        list.repo = Some(RepoRef {
            owner: "acme".into(),
            name: "app".into(),
        });
        list.pulls = vec![PullSummary {
            number: 7,
            title: "A very long pull request title that cannot possibly fit in the sidebar".into(),
            author: "someone-with-a-long-login".into(),
            head_ref: "feature/a-very-long-branch-name-for-testing".into(),
            base_ref: "release/another-very-long-branch-name".into(),
            draft: true,
        }];
        list
    }

    #[cfg(test)]
    pub fn pulls_len(&self) -> usize {
        self.pulls.len()
    }

    #[cfg(test)]
    pub fn is_loaded_for(&self, path: &std::path::Path) -> bool {
        self.loaded_for.as_deref() == Some(path)
    }

    /// Load the list for the repository holding `path`, in the background.
    pub fn start(&mut self, path: PathBuf, config_token: String) {
        let (tx, rx) = std::sync::mpsc::channel();
        let folder = path.clone();
        let spawned = std::thread::Builder::new()
            .name("github-pulls".into())
            .spawn(move || {
                let result = github::origin_repo(&folder).and_then(|repo| {
                    let client =
                        github::Client::new(repo.clone(), github::auth::resolve(&config_token));
                    client.list_open_pulls().map(|pulls| (repo, pulls))
                });
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => {
                self.loading = Some(rx);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Could not start loading: {e}")),
        }
        self.loaded_for = Some(path);
    }

    /// Take the finished load, if any. A failure keeps the previous list.
    fn poll(&mut self) {
        let Some(rx) = &self.loading else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.loading = None;
                self.error = Some("Loading stopped unexpectedly.".into());
                return;
            }
        };
        self.loading = None;
        match result {
            Ok((repo, pulls)) => {
                self.repo = Some(repo);
                self.pulls = pulls;
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, git: &GitStatus, config_token: &str) {
        let Some(path) = git.repo_path.as_deref() else {
            return;
        };
        if self.loaded_for.as_deref().is_some_and(|p| p != path) {
            // Another workspace was opened: its pull requests, not these.
            *self = Self::new();
        }
        self.poll();
        if self.loading.is_some() {
            // Also while collapsed: the result is picked up by `poll`.
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        let title = if self.repo.is_some() {
            format!("PULL REQUESTS ({})", self.pulls.len())
        } else {
            "PULL REQUESTS".to_string()
        };
        egui::CollapsingHeader::new(egui::RichText::new(title).strong().small())
            .id_salt("pull-requests")
            .default_open(false)
            .show(ui, |ui| {
                if self.loaded_for.is_none() {
                    self.start(path.to_path_buf(), config_token.to_string());
                }
                ui.horizontal(|ui| {
                    let idle = self.loading.is_none();
                    let refresh =
                        egui::Button::new(format!("{} Refresh", regular::ARROW_CLOCKWISE)).small();
                    if ui.add_enabled(idle, refresh).clicked() {
                        self.start(path.to_path_buf(), config_token.to_string());
                    }
                    if !idle {
                        ui.spinner();
                    }
                });
                if let Some(err) = &self.error {
                    ui.label(
                        egui::RichText::new(err)
                            .small()
                            .color(egui::Color32::from_rgb(220, 80, 80)),
                    );
                }
                if self.pulls.is_empty() && self.error.is_none() && self.loading.is_none() {
                    ui.label(
                        egui::RichText::new("No open pull requests")
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                }
                let mut clicked = None;
                egui::ScrollArea::vertical()
                    .id_salt("pull-requests-list")
                    .max_height(250.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for pull in &self.pulls {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("#{}", pull.number))
                                        .monospace()
                                        .small()
                                        .color(egui::Color32::GRAY),
                                );
                                let title = ui
                                    .add(
                                        egui::Label::new(egui::RichText::new(&pull.title).small())
                                            .truncate()
                                            .selectable(false)
                                            .sense(egui::Sense::click()),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                                if title.clicked() {
                                    clicked = Some(pull.clone());
                                }
                            });
                            let draft = if pull.draft { " · Draft" } else { "" };
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} · {} → {}{draft}",
                                    pull.author, pull.head_ref, pull.base_ref
                                ))
                                .small()
                                .color(egui::Color32::GRAY),
                            );
                            ui.add_space(2.0);
                        }
                    });
                if let (Some(pull), Some(repo)) = (clicked, &self.repo) {
                    self.open_request = Some((repo.clone(), pull));
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_repository_without_a_github_origin_says_so() {
        let r = crate::git::tests::TestRepo::new();
        let mut list = PrList::new();
        list.start(r.path().to_path_buf(), String::new());
        let deadline = Instant::now() + Duration::from_secs(5);
        while list.loading.is_some() && Instant::now() < deadline {
            list.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(list.loading.is_none(), "load still running after 5 s");
        assert_eq!(
            list.error.as_deref(),
            Some("The origin remote is not a GitHub repository.")
        );
        assert!(list.pulls.is_empty());
        assert!(list.open_request.is_none());
    }

    fn pull(number: u64) -> PullSummary {
        PullSummary {
            number,
            title: format!("PR {number}"),
            author: "a".into(),
            head_ref: "h".into(),
            base_ref: "b".into(),
            draft: false,
        }
    }

    fn repo() -> RepoRef {
        RepoRef {
            owner: "o".into(),
            name: "n".into(),
        }
    }

    fn list_receiving(result: Option<Loaded>) -> PrList {
        let (tx, rx) = std::sync::mpsc::channel();
        if let Some(result) = result {
            tx.send(result).unwrap();
        }
        let mut list = PrList::new();
        list.loading = Some(rx);
        list.repo = Some(repo());
        list.pulls = vec![pull(1)];
        list
    }

    #[test]
    fn a_failed_load_keeps_the_previous_list() {
        let mut list = list_receiving(Some(Err(GithubError::Network("offline".into()))));
        list.poll();
        assert!(list.loading.is_none());
        assert_eq!(list.pulls, vec![pull(1)]);
        assert_eq!(list.repo, Some(repo()));
        assert!(list.error.is_some());
    }

    #[test]
    fn a_crashed_load_is_reported() {
        let mut list = list_receiving(None);
        list.poll();
        assert!(list.loading.is_none());
        assert_eq!(list.error.as_deref(), Some("Loading stopped unexpectedly."));
        assert_eq!(list.pulls, vec![pull(1)]);
    }

    #[test]
    fn a_successful_load_replaces_the_list_and_clears_the_error() {
        let other = RepoRef {
            owner: "x".into(),
            name: "y".into(),
        };
        let mut list = list_receiving(Some(Ok((other.clone(), vec![pull(2), pull(3)]))));
        list.error = Some("old".into());
        list.poll();
        assert!(list.loading.is_none());
        assert_eq!(list.pulls, vec![pull(2), pull(3)]);
        assert_eq!(list.repo, Some(other));
        assert!(list.error.is_none());
    }
}
