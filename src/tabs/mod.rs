use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Tab {
    pub id: usize,
    pub path: PathBuf,
    pub title: String,
    pub is_modified: bool,
    pub is_settings: bool,
    /// Module page shown in this tab (a `[[panels]]` id), instead of a file.
    pub page: Option<String>,
    /// Pull request shown in this tab (its number), instead of a file.
    pub pr: Option<u64>,
    /// The file was deleted on disk; the buffer is kept and the title shows it.
    pub is_deleted: bool,
}

impl Tab {
    /// Shows a file (or an untitled buffer), not settings, a module page or
    /// a pull request.
    pub fn is_file(&self) -> bool {
        !self.is_settings && self.page.is_none() && self.pr.is_none()
    }
}

/// Which tabs a bulk close acts on, relative to an anchor tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseScope {
    /// Every tab but the anchor.
    Others,
    /// The tabs after the anchor.
    ToTheRight,
    /// Every tab without unsaved changes.
    Saved,
    All,
}

pub struct TabManager {
    pub tabs: Vec<Tab>,
    pub active_tab: Option<usize>,
    next_id: usize,
    /// A modified tab the user asked to close (× or middle-click): left open
    /// for the caller to confirm, like Ctrl+W.
    pub close_request: Option<usize>,
    /// A bulk close picked in a tab's context menu, with that tab's id.
    pub bulk_close: Option<(CloseScope, usize)>,
}

impl TabManager {
    pub fn new() -> Self {
        Self {
            tabs: vec![],
            active_tab: None,
            next_id: 0,
            close_request: None,
            bulk_close: None,
        }
    }

    pub fn open(&mut self, path: PathBuf, _content: String) -> usize {
        if let Some(tab) = self.tabs.iter().find(|t| t.path == path) {
            let id = tab.id;
            self.active_tab = Some(id);
            return id;
        }
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "untitled".to_string());
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab {
            id,
            path,
            title,
            is_modified: false,
            is_settings: false,
            page: None,
            pr: None,
            is_deleted: false,
        });
        self.active_tab = Some(id);
        id
    }

    pub fn open_untitled(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let title = format!("untitled-{}", id + 1);
        let path = PathBuf::from(format!("untitled-{}", id + 1));
        self.tabs.push(Tab {
            id,
            path,
            title,
            is_modified: true,
            is_settings: false,
            page: None,
            pr: None,
            is_deleted: false,
        });
        self.active_tab = Some(id);
        id
    }

    /// Point tab `id` at `path` (an untitled buffer saved under a name).
    pub fn retarget(&mut self, id: usize, path: PathBuf) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            tab.title = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "untitled".to_string());
            tab.path = path;
        }
    }

    pub fn open_settings(&mut self) -> usize {
        if let Some(existing) = self.tabs.iter().find(|t| t.is_settings) {
            let id = existing.id;
            self.active_tab = Some(id);
            return id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab {
            id,
            path: PathBuf::from("__settings__"),
            title: "Settings".to_string(),
            is_modified: false,
            is_settings: true,
            page: None,
            pr: None,
            is_deleted: false,
        });
        self.active_tab = Some(id);
        id
    }

    /// Open (or focus) the tab of module page `panel_id`.
    pub fn open_page(&mut self, panel_id: &str, title: &str) -> usize {
        if let Some(existing) = self
            .tabs
            .iter()
            .find(|t| t.page.as_deref() == Some(panel_id))
        {
            let id = existing.id;
            self.active_tab = Some(id);
            return id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab {
            id,
            path: PathBuf::from(format!("__page__{panel_id}")),
            title: title.to_string(),
            is_modified: false,
            is_settings: false,
            page: Some(panel_id.to_string()),
            pr: None,
            is_deleted: false,
        });
        self.active_tab = Some(id);
        id
    }

    /// Module page of the active tab, if it shows one.
    pub fn active_page(&self) -> Option<&str> {
        let id = self.active_tab?;
        self.tabs.iter().find(|t| t.id == id)?.page.as_deref()
    }

    /// The active tab is edited in the editor: a file or an untitled buffer,
    /// not the settings, a module page or a pull request.
    pub fn active_is_document(&self) -> bool {
        self.active_tab
            .and_then(|id| self.tabs.iter().find(|t| t.id == id))
            .is_some_and(|t| t.is_file())
    }

    /// Open (or focus) the tab of pull request `number`.
    pub fn open_pr(&mut self, number: u64, title: &str) -> usize {
        if let Some(existing) = self.tabs.iter().find(|t| t.pr == Some(number)) {
            let id = existing.id;
            self.active_tab = Some(id);
            return id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab {
            id,
            path: PathBuf::from(format!("__pr__{number}")),
            title: pr_tab_title(number, title),
            is_modified: false,
            is_settings: false,
            page: None,
            pr: Some(number),
            is_deleted: false,
        });
        self.active_tab = Some(id);
        id
    }

    /// Pull request of the active tab, if it shows one.
    pub fn active_pr(&self) -> Option<u64> {
        let id = self.active_tab?;
        self.tabs.iter().find(|t| t.id == id)?.pr
    }

    /// Close every pull request tab (they belong to the workspace's repo).
    pub fn close_prs(&mut self) {
        let gone: Vec<usize> = self
            .tabs
            .iter()
            .filter(|t| t.pr.is_some())
            .map(|t| t.id)
            .collect();
        for id in gone {
            self.close(id);
        }
    }

    /// Close the page tabs whose panel is not in `panels` any more.
    pub fn retain_pages(&mut self, panels: &[String]) {
        let gone: Vec<usize> = self
            .tabs
            .iter()
            .filter(|t| t.page.as_ref().is_some_and(|p| !panels.contains(p)))
            .map(|t| t.id)
            .collect();
        for id in gone {
            self.close(id);
        }
    }

    /// Mark (or unmark) the tab of `path` as deleted on disk. Returns true if
    /// a tab's state changed.
    pub fn set_deleted(&mut self, path: &std::path::Path, deleted: bool) -> bool {
        let mut changed = false;
        for t in self.tabs.iter_mut().filter(|t| t.path == path) {
            changed |= t.is_deleted != deleted;
            t.is_deleted = deleted;
        }
        changed
    }

    pub fn close(&mut self, id: usize) {
        self.tabs.retain(|t| t.id != id);
        if self.active_tab == Some(id) {
            self.active_tab = self.tabs.last().map(|t| t.id);
        }
    }

    /// Close the tabs of a workspace being left: all but the settings tab and
    /// the tabs with unsaved changes, `unsaved` included (a file whose edits
    /// its tab may not show yet). Returns how many stay for unsaved changes.
    pub fn close_saved_tabs(&mut self, unsaved: Option<&std::path::Path>) -> usize {
        let is_unsaved = |t: &Tab| t.is_modified || Some(t.path.as_path()) == unsaved;
        let (kept, stale): (Vec<&Tab>, Vec<&Tab>) = self
            .tabs
            .iter()
            .filter(|t| !t.is_settings)
            .partition(|t| is_unsaved(t));
        let kept = kept.len();
        let stale: Vec<usize> = stale.iter().map(|t| t.id).collect();
        for id in stale {
            self.close(id);
        }
        kept
    }

    /// Ids of the tabs `scope` covers around `anchor` (the active tab when
    /// `None`), in tab order. Tabs with unsaved changes are included: the
    /// caller decides what to do with them.
    pub fn ids_in_scope(&self, scope: CloseScope, anchor: Option<usize>) -> Vec<usize> {
        let anchor = anchor.or(self.active_tab);
        let pos = anchor.and_then(|a| self.tabs.iter().position(|t| t.id == a));
        self.tabs
            .iter()
            .enumerate()
            .filter(|(i, t)| match scope {
                CloseScope::Others => Some(t.id) != anchor,
                CloseScope::ToTheRight => pos.is_some_and(|p| *i > p),
                CloseScope::Saved => !t.is_modified,
                CloseScope::All => true,
            })
            .map(|(_, t)| t.id)
            .collect()
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        let mut to_open: Option<PathBuf> = None;
        let mut activate_id: Option<usize> = None;
        let mut to_close: Option<usize> = None;
        let active_tab = self.active_tab;

        let tabs_data: Vec<(usize, PathBuf, String, bool, Option<&'static str>)> = self
            .tabs
            .iter()
            .map(|t| {
                (
                    t.id,
                    t.path.clone(),
                    if t.is_deleted {
                        format!("{} (deleted)", t.title)
                    } else {
                        t.title.clone()
                    },
                    t.is_modified,
                    if t.pr.is_some() {
                        Some(egui_phosphor::regular::GIT_PULL_REQUEST)
                    } else if t.page.is_some() {
                        Some(egui_phosphor::regular::PUZZLE_PIECE)
                    } else if t.is_settings {
                        Some(egui_phosphor::regular::GEAR)
                    } else {
                        None
                    },
                )
            })
            .collect();

        ui.horizontal(|ui| {
            ui.style_mut().spacing.item_spacing.x = 0.0;
            for (tab_id, tab_path, tab_title, tab_modified, tab_special_icon) in &tabs_data {
                let is_active = active_tab == Some(*tab_id);
                let bg = if is_active {
                    egui::Color32::from_rgb(30, 30, 30)
                } else {
                    egui::Color32::from_rgb(45, 45, 45)
                };
                let frame_resp = egui::Frame::new().fill(bg).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(icon) = tab_special_icon {
                            ui.label(
                                egui::RichText::new(*icon)
                                    .color(egui::Color32::from_gray(160))
                                    .size(14.0),
                            );
                        } else {
                            let name = tab_path
                                .file_name()
                                .map(|n| n.to_string_lossy())
                                .unwrap_or_else(|| tab_title.as_str().into());
                            let (icon, icon_color) = crate::filetree::file_icon(&name);
                            ui.label(egui::RichText::new(icon).color(icon_color).size(14.0));
                        }
                        ui.add_space(2.0);
                        let tab_label = if *tab_modified {
                            egui::RichText::new(format!("● {}", tab_title))
                                .color(egui::Color32::from_rgb(255, 180, 50))
                        } else {
                            egui::RichText::new(tab_title.as_str()).color(if is_active {
                                egui::Color32::WHITE
                            } else {
                                egui::Color32::from_rgb(160, 160, 160)
                            })
                        };
                        let label_resp = ui.selectable_label(is_active, tab_label);
                        if label_resp.clicked() {
                            if tab_special_icon.is_some() {
                                activate_id = Some(*tab_id);
                            } else {
                                to_open = Some(tab_path.clone());
                            }
                        }
                        label_resp.context_menu(|ui| {
                            if ui.button("Close").clicked() {
                                to_close = Some(*tab_id);
                                ui.close_menu();
                            }
                            let scopes = [
                                ("Close Others", CloseScope::Others),
                                ("Close to the Right", CloseScope::ToTheRight),
                                ("Close Saved", CloseScope::Saved),
                                ("Close All", CloseScope::All),
                            ];
                            for (label, scope) in scopes {
                                if ui.button(label).clicked() {
                                    self.bulk_close = Some((scope, *tab_id));
                                    ui.close_menu();
                                }
                            }
                        });
                        if ui.small_button("×").clicked() {
                            to_close = Some(*tab_id);
                        }
                    });
                });
                // Middle-click closes the tab.
                let tab_id_copy = *tab_id;
                if ui.input(|i| {
                    i.pointer.button_released(egui::PointerButton::Middle)
                        && i.pointer
                            .hover_pos()
                            .is_some_and(|p| frame_resp.response.rect.contains(p))
                }) {
                    to_close = Some(tab_id_copy);
                }
                ui.separator();
            }
        });

        if let Some(id) = activate_id {
            self.active_tab = Some(id);
        }

        if let Some(ref path) = to_open {
            if let Some(tab) = self.tabs.iter().find(|t| &t.path == path) {
                self.active_tab = Some(tab.id);
            }
        }

        // A tab with unsaved changes is only closed once the user confirms.
        if let Some(id) =
            to_close.filter(|id| self.tabs.iter().any(|t| t.id == *id && t.is_modified))
        {
            self.close_request = Some(id);
            to_close = None;
        }
        if let Some(id) = to_close {
            let was_active = self.active_tab == Some(id);
            self.close(id);
            // If the active tab was closed, load the new active tab's file
            if was_active && to_open.is_none() {
                to_open = self
                    .active_tab
                    .and_then(|new_id| self.tabs.iter().find(|t| t.id == new_id))
                    .filter(|t| t.is_file())
                    .map(|t| t.path.clone());
            }
        }
        to_open
    }
}

/// `#58 title`, the title cut to 40 characters.
fn pr_tab_title(number: u64, title: &str) -> String {
    const MAX: usize = 40;
    let cut: String = title.chars().take(MAX).collect();
    let short = cut.trim_end();
    let ellipsis = if title.chars().count() > MAX {
        "…"
    } else {
        ""
    };
    format!("#{number} {short}{ellipsis}")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Headless egui harness ──────────────────────────────────────────────

    struct Harness {
        ctx: egui::Context,
        texts: Vec<(String, egui::Rect)>,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                ctx: egui::Context::default(),
                texts: vec![],
            }
        }

        /// Run one frame rendering the tab bar; returns `show()`'s result.
        fn frame(&mut self, tm: &mut TabManager, events: Vec<egui::Event>) -> Option<PathBuf> {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600.0, 300.0),
                )),
                events,
                ..Default::default()
            };
            let mut result = None;
            let out = self.ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    result = tm.show(ui);
                });
            });
            self.texts.clear();
            for cs in &out.shapes {
                collect_texts(&cs.shape, &mut self.texts);
            }
            result
        }

        fn rect_of(&self, text: &str) -> egui::Rect {
            self.texts
                .iter()
                .find(|(t, _)| t == text)
                .map(|(_, r)| *r)
                .unwrap_or_else(|| panic!("text {text:?} not rendered; have {:?}", self.texts))
        }

        /// Press + release `button` over `text`, returning the result of the
        /// release frame (where egui reports the click).
        fn click(
            &mut self,
            tm: &mut TabManager,
            text: &str,
            button: egui::PointerButton,
        ) -> Option<PathBuf> {
            self.frame(tm, vec![]);
            let pos = self.rect_of(text).center();
            let ev = |pressed| egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(tm, vec![egui::Event::PointerMoved(pos), ev(true)]);
            self.frame(tm, vec![ev(false)])
        }
    }

    fn collect_texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(t) => {
                out.push((t.galley.text().to_string(), t.visual_bounding_rect()));
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect_texts(s, out)),
            _ => {}
        }
    }

    fn manager_with(paths: &[&str]) -> TabManager {
        let mut tm = TabManager::new();
        for p in paths {
            tm.open(PathBuf::from(p), String::new());
        }
        tm
    }

    // ── Pure state logic ───────────────────────────────────────────────────

    #[test]
    fn open_creates_tab_titled_after_file_name_and_activates_it() {
        let mut tm = TabManager::new();
        assert!(tm.active_tab.is_none());
        let id = tm.open(PathBuf::from("src/main.rs"), "fn main() {}".into());
        assert_eq!(tm.tabs.len(), 1);
        let t = &tm.tabs[0];
        assert_eq!(t.id, id);
        assert_eq!(t.title, "main.rs");
        assert_eq!(t.path, PathBuf::from("src/main.rs"));
        assert!(!t.is_modified && !t.is_settings);
        assert_eq!(tm.active_tab, Some(id));
    }

    #[test]
    fn open_existing_path_reuses_tab_and_reactivates_it() {
        let mut tm = TabManager::new();
        let a = tm.open(PathBuf::from("a.rs"), String::new());
        let b = tm.open(PathBuf::from("b.rs"), String::new());
        assert_ne!(a, b);
        assert_eq!(tm.active_tab, Some(b));
        let again = tm.open(PathBuf::from("a.rs"), String::new());
        assert_eq!(again, a);
        assert_eq!(tm.tabs.len(), 2);
        assert_eq!(tm.active_tab, Some(a));
    }

    #[test]
    fn open_path_without_file_name_is_titled_untitled() {
        let mut tm = TabManager::new();
        tm.open(PathBuf::from(".."), String::new());
        assert_eq!(tm.tabs[0].title, "untitled");
    }

    #[test]
    fn open_untitled_creates_distinct_modified_tabs() {
        let mut tm = TabManager::new();
        let a = tm.open_untitled();
        let b = tm.open_untitled();
        assert_ne!(a, b);
        assert_eq!(tm.tabs[0].title, "untitled-1");
        assert_eq!(tm.tabs[1].title, "untitled-2");
        assert_ne!(tm.tabs[0].path, tm.tabs[1].path);
        assert!(tm.tabs.iter().all(|t| t.is_modified && !t.is_settings));
        assert_eq!(tm.active_tab, Some(b));
    }

    #[test]
    fn active_is_document_for_files_and_untitled_buffers_only() {
        let mut tm = TabManager::new();
        assert!(!tm.active_is_document());
        tm.open_untitled();
        assert!(tm.active_is_document());
        tm.open_settings();
        assert!(!tm.active_is_document());
        tm.open_page("docker.images", "Docker");
        assert!(!tm.active_is_document());
        tm.open(PathBuf::from("/a.rs"), String::new());
        assert!(tm.active_is_document());
    }

    #[test]
    fn close_saved_tabs_keeps_settings_and_unsaved_changes() {
        let mut tm = TabManager::new();
        tm.open(PathBuf::from("/ws/a.rs"), String::new());
        let b = tm.open(PathBuf::from("/ws/b.rs"), String::new());
        let settings = tm.open_settings();
        tm.open_page("docker.images", "Docker");
        let untitled = tm.open_untitled();
        let c = tm.open(PathBuf::from("/ws/c.rs"), String::new());
        tm.tabs.iter_mut().find(|t| t.id == b).unwrap().is_modified = true;
        // c.rs has edits its tab doesn't show yet.
        let kept = tm.close_saved_tabs(Some(std::path::Path::new("/ws/c.rs")));
        assert_eq!(kept, 3);
        let left: Vec<usize> = tm.tabs.iter().map(|t| t.id).collect();
        assert_eq!(left, vec![b, settings, untitled, c]);
        assert_eq!(tm.active_tab, Some(c));

        // Nothing unsaved: only the settings tab stays.
        let mut tm = TabManager::new();
        tm.open(PathBuf::from("/ws/a.rs"), String::new());
        let settings = tm.open_settings();
        tm.open(PathBuf::from("/ws/b.rs"), String::new());
        assert_eq!(tm.close_saved_tabs(None), 0);
        assert_eq!(tm.tabs.len(), 1);
        assert_eq!(tm.active_tab, Some(settings));
    }

    #[test]
    fn pages_are_singletons_and_closed_when_their_panel_goes() {
        let mut tm = TabManager::new();
        tm.open(PathBuf::from("/a.rs"), String::new());
        let p = tm.open_page("docker.images", "Docker");
        assert_eq!(tm.active_page(), Some("docker.images"));
        assert_eq!(tm.open_page("docker.images", "Docker"), p);
        assert_eq!(tm.tabs.len(), 2);
        tm.retain_pages(&["docker.images".to_string()]);
        assert_eq!(tm.tabs.len(), 2);
        tm.retain_pages(&[]);
        assert_eq!(tm.tabs.len(), 1);
        assert_eq!(tm.active_page(), None);
    }

    #[test]
    fn open_settings_is_a_singleton() {
        let mut tm = manager_with(&["a.rs"]);
        let s = tm.open_settings();
        let t = tm.tabs.iter().find(|t| t.id == s).unwrap();
        assert!(t.is_settings);
        assert_eq!(t.title, "Settings");
        assert_eq!(tm.active_tab, Some(s));

        tm.open(PathBuf::from("b.rs"), String::new());
        let s2 = tm.open_settings();
        assert_eq!(s, s2);
        assert_eq!(tm.tabs.iter().filter(|t| t.is_settings).count(), 1);
        assert_eq!(tm.active_tab, Some(s));
    }

    #[test]
    fn close_active_tab_activates_last_remaining() {
        let mut tm = manager_with(&["a.rs", "b.rs", "c.rs"]);
        let ids: Vec<usize> = tm.tabs.iter().map(|t| t.id).collect();
        tm.active_tab = Some(ids[1]);
        tm.close(ids[1]);
        assert_eq!(tm.tabs.len(), 2);
        assert_eq!(tm.active_tab, Some(ids[2]));
    }

    #[test]
    fn close_inactive_tab_keeps_active() {
        let mut tm = manager_with(&["a.rs", "b.rs"]);
        let ids: Vec<usize> = tm.tabs.iter().map(|t| t.id).collect();
        tm.active_tab = Some(ids[1]);
        tm.close(ids[0]);
        assert_eq!(tm.active_tab, Some(ids[1]));
        assert_eq!(tm.tabs.len(), 1);
    }

    #[test]
    fn close_last_tab_clears_active_and_unknown_id_is_noop() {
        let mut tm = manager_with(&["a.rs"]);
        tm.close(999);
        assert_eq!(tm.tabs.len(), 1);
        let id = tm.tabs[0].id;
        tm.close(id);
        assert!(tm.tabs.is_empty());
        assert!(tm.active_tab.is_none());
    }

    #[test]
    fn ids_are_never_reused_after_close() {
        let mut tm = manager_with(&["a.rs"]);
        let first = tm.tabs[0].id;
        tm.close(first);
        let next = tm.open(PathBuf::from("b.rs"), String::new());
        assert_ne!(first, next);
    }

    #[test]
    fn set_deleted_marks_tab_and_reports_changes() {
        let mut tm = manager_with(&["a.rs", "b.rs"]);
        let a = PathBuf::from("a.rs");
        assert!(tm.set_deleted(&a, true));
        assert!(!tm.set_deleted(&a, true), "no change the second time");
        assert!(tm.tabs[0].is_deleted && !tm.tabs[1].is_deleted);
        assert!(!tm.set_deleted(&PathBuf::from("zzz.rs"), true));
        assert!(tm.set_deleted(&a, false));
        assert!(!tm.tabs[0].is_deleted);
    }

    #[test]
    fn deleted_tab_renders_with_suffix() {
        let mut tm = manager_with(&["a.rs"]);
        tm.set_deleted(&PathBuf::from("a.rs"), true);
        let mut h = Harness::new();
        h.frame(&mut tm, vec![]);
        h.rect_of("a.rs (deleted)");
    }

    // ── Rendering / interaction ────────────────────────────────────────────

    #[test]
    fn show_renders_every_tab_without_actions_when_idle() {
        let mut tm = manager_with(&["a.rs", "b.rs"]);
        tm.tabs[0].is_modified = true;
        tm.open_settings();
        let mut h = Harness::new();
        assert!(h.frame(&mut tm, vec![]).is_none());
        // Modified tabs get a bullet prefix; others show their plain title.
        h.rect_of("● a.rs");
        h.rect_of("b.rs");
        h.rect_of("Settings");
        assert_eq!(tm.tabs.len(), 3);
    }

    #[test]
    fn clicking_file_tab_returns_its_path_and_activates_it() {
        let mut tm = manager_with(&["a.rs", "b.rs"]);
        let a_id = tm.tabs[0].id;
        let mut h = Harness::new();
        let r = h.click(&mut tm, "a.rs", egui::PointerButton::Primary);
        assert_eq!(r, Some(PathBuf::from("a.rs")));
        assert_eq!(tm.active_tab, Some(a_id));
    }

    #[test]
    fn clicking_settings_tab_activates_without_opening_a_path() {
        let mut tm = TabManager::new();
        let s = tm.open_settings();
        tm.open(PathBuf::from("a.rs"), String::new());
        let mut h = Harness::new();
        let r = h.click(&mut tm, "Settings", egui::PointerButton::Primary);
        assert!(r.is_none());
        assert_eq!(tm.active_tab, Some(s));
    }

    #[test]
    fn close_button_on_active_tab_returns_new_active_path() {
        let mut tm = manager_with(&["a.rs", "b.rs"]);
        // b.rs is active; its × is the second one rendered.
        let mut h = Harness::new();
        h.frame(&mut tm, vec![]);
        let closes: Vec<egui::Rect> = h
            .texts
            .iter()
            .filter(|(t, _)| t == "×")
            .map(|(_, r)| *r)
            .collect();
        assert_eq!(closes.len(), 2);
        let pos = closes[1].center();
        let ev = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        h.frame(&mut tm, vec![egui::Event::PointerMoved(pos), ev(true)]);
        let r = h.frame(&mut tm, vec![ev(false)]);
        assert_eq!(tm.tabs.len(), 1);
        assert_eq!(tm.tabs[0].title, "a.rs");
        assert_eq!(
            r,
            Some(PathBuf::from("a.rs")),
            "editor must load the new active tab"
        );
    }

    #[test]
    fn close_button_on_inactive_tab_returns_nothing() {
        let mut tm = manager_with(&["a.rs", "b.rs"]);
        let b_id = tm.tabs[1].id;
        let mut h = Harness::new();
        h.frame(&mut tm, vec![]);
        let pos = h
            .texts
            .iter()
            .find(|(t, _)| t == "×")
            .map(|(_, r)| r.center())
            .unwrap();
        let ev = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        h.frame(&mut tm, vec![egui::Event::PointerMoved(pos), ev(true)]);
        let r = h.frame(&mut tm, vec![ev(false)]);
        assert!(r.is_none());
        assert_eq!(tm.tabs.len(), 1);
        assert_eq!(tm.active_tab, Some(b_id));
    }

    #[test]
    fn middle_click_closes_hovered_tab() {
        let mut tm = manager_with(&["a.rs", "b.rs"]);
        let mut h = Harness::new();
        let r = h.click(&mut tm, "b.rs", egui::PointerButton::Middle);
        assert_eq!(tm.tabs.len(), 1);
        assert_eq!(tm.tabs[0].title, "a.rs");
        assert_eq!(r, Some(PathBuf::from("a.rs")));
    }

    #[test]
    fn close_scopes_pick_tabs_around_the_anchor() {
        let mut tm = manager_with(&["a.rs", "b.rs", "c.rs", "d.rs"]);
        let ids: Vec<usize> = tm.tabs.iter().map(|t| t.id).collect();
        tm.tabs[3].is_modified = true;
        let b = Some(ids[1]);
        assert_eq!(
            tm.ids_in_scope(CloseScope::Others, b),
            [ids[0], ids[2], ids[3]]
        );
        assert_eq!(tm.ids_in_scope(CloseScope::ToTheRight, b), [ids[2], ids[3]]);
        assert_eq!(tm.ids_in_scope(CloseScope::Saved, b), &ids[..3]);
        assert_eq!(tm.ids_in_scope(CloseScope::All, b), ids);
        // Without an anchor, the active tab (the last opened) is the anchor.
        assert_eq!(tm.ids_in_scope(CloseScope::Others, None), &ids[..3]);
        assert!(tm.ids_in_scope(CloseScope::ToTheRight, None).is_empty());
    }

    #[test]
    fn tab_context_menu_requests_a_bulk_close() {
        let mut tm = manager_with(&["a.rs", "b.rs", "c.rs"]);
        let a_id = tm.tabs[0].id;
        let mut h = Harness::new();
        h.click(&mut tm, "a.rs", egui::PointerButton::Secondary);
        h.click(&mut tm, "Close to the Right", egui::PointerButton::Primary);
        assert_eq!(tm.bulk_close, Some((CloseScope::ToTheRight, a_id)));
        assert_eq!(tm.tabs.len(), 3, "the app does the closing");
    }

    #[test]
    fn closing_a_modified_tab_asks_instead_of_closing() {
        let mut tm = manager_with(&["a.rs", "b.rs"]);
        tm.tabs[1].is_modified = true;
        let b_id = tm.tabs[1].id;
        let mut h = Harness::new();
        let r = h.click(&mut tm, "● b.rs", egui::PointerButton::Middle);
        assert_eq!(tm.tabs.len(), 2, "kept open until confirmed");
        assert_eq!(tm.close_request, Some(b_id));
        assert_eq!(r, None);

        // The × of a modified tab asks the same way.
        tm.close_request = None;
        h.frame(&mut tm, vec![]);
        let pos = h
            .texts
            .iter()
            .filter(|(t, _)| t == "×")
            .map(|(_, r)| r.center())
            .nth(1)
            .unwrap();
        let ev = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        h.frame(&mut tm, vec![egui::Event::PointerMoved(pos), ev(true)]);
        h.frame(&mut tm, vec![ev(false)]);
        assert_eq!(tm.tabs.len(), 2);
        assert_eq!(tm.close_request, Some(b_id));
    }

    #[test]
    fn pull_request_tabs_are_singletons_and_not_files() {
        let mut tm = TabManager::new();
        let f = tm.open(PathBuf::from("/w/a.rs"), String::new());
        let p = tm.open_pr(58, "feat: connect servers");
        assert_eq!(tm.active_pr(), Some(58));
        assert_eq!(tm.open_pr(58, "feat: connect servers"), p);
        assert_eq!(tm.tabs.len(), 2);
        let tab = tm.tabs.iter().find(|t| t.id == p).unwrap();
        assert_eq!(tab.title, "#58 feat: connect servers");
        assert!(!tab.is_file());
        assert!(tm.tabs.iter().find(|t| t.id == f).unwrap().is_file());
        tm.active_tab = Some(f);
        assert_eq!(tm.active_pr(), None);
    }

    #[test]
    fn long_pull_request_titles_are_cut() {
        let mut tm = TabManager::new();
        tm.open_pr(1, &"x".repeat(50));
        assert_eq!(tm.tabs[0].title, format!("#1 {}…", "x".repeat(40)));
    }

    #[test]
    fn clicking_a_pull_request_tab_activates_it_without_opening_a_file() {
        let mut tm = manager_with(&["/w/a.rs"]);
        let p = tm.open_pr(7, "Fix");
        tm.active_tab = Some(tm.tabs[0].id);
        let mut h = Harness::new();
        assert_eq!(
            h.click(&mut tm, "#7 Fix", egui::PointerButton::Primary),
            None
        );
        assert_eq!(tm.active_tab, Some(p));
    }

    #[test]
    fn pull_request_and_page_tabs_show_their_icon() {
        let mut tm = TabManager::new();
        tm.open_pr(3, "Fix");
        tm.open_page("docker.images", "Docker");
        let mut h = Harness::new();
        h.frame(&mut tm, vec![]);
        h.rect_of(egui_phosphor::regular::GIT_PULL_REQUEST);
        h.rect_of(egui_phosphor::regular::PUZZLE_PIECE);
    }

    #[test]
    fn close_prs_closes_only_pull_request_tabs() {
        let mut tm = TabManager::new();
        let f = tm.open(PathBuf::from("/w/a.rs"), String::new());
        tm.open_pr(1, "One");
        tm.open_pr(2, "Two");
        assert_eq!(tm.active_pr(), Some(2));
        tm.close_prs();
        assert_eq!(tm.tabs.len(), 1);
        assert_eq!(tm.tabs[0].id, f);
        assert_eq!(tm.active_tab, Some(f));
        assert_eq!(tm.active_pr(), None);
        // A later PR with the same number gets a fresh tab.
        let p = tm.open_pr(1, "One");
        assert_ne!(p, f);
    }

    #[test]
    fn is_file_tells_files_from_the_other_kinds() {
        let mut tm = TabManager::new();
        let f = tm.open(PathBuf::from("/w/a.rs"), String::new());
        let u = tm.open_untitled();
        let s = tm.open_settings();
        let g = tm.open_page("p", "Page");
        let p = tm.open_pr(1, "PR");
        let is_file = |id| tm.tabs.iter().find(|t| t.id == id).unwrap().is_file();
        assert!(is_file(f) && is_file(u));
        assert!(!is_file(s) && !is_file(g) && !is_file(p));
    }

    #[test]
    fn pull_request_title_cut_boundaries() {
        let mut tm = TabManager::new();
        tm.open_pr(1, &"x".repeat(40));
        assert_eq!(tm.tabs[0].title, format!("#1 {}", "x".repeat(40)));
        tm.open_pr(2, &"é".repeat(50));
        assert_eq!(tm.tabs[1].title, format!("#2 {}…", "é".repeat(40)));
        // Whitespace left at the cut is trimmed before the ellipsis.
        tm.open_pr(3, &format!("{} tail", "x".repeat(39)));
        assert_eq!(tm.tabs[2].title, format!("#3 {}…", "x".repeat(39)));
    }

    #[test]
    fn closing_the_active_file_tab_when_a_pr_tab_takes_over_opens_no_file() {
        let mut tm = TabManager::new();
        tm.open_pr(5, "PR");
        tm.open(PathBuf::from("/w/a.rs"), String::new());
        let mut h = Harness::new();
        h.frame(&mut tm, vec![]);
        let pos = h
            .texts
            .iter()
            .filter(|(t, _)| t == "×")
            .map(|(_, r)| r.center())
            .nth(1)
            .unwrap();
        let ev = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        h.frame(&mut tm, vec![egui::Event::PointerMoved(pos), ev(true)]);
        let r = h.frame(&mut tm, vec![ev(false)]);
        assert_eq!(tm.tabs.len(), 1);
        assert_eq!(tm.active_pr(), Some(5));
        assert_eq!(r, None);
    }

    #[test]
    fn closing_a_pull_request_tab_never_asks_for_confirmation() {
        let mut tm = manager_with(&["/w/a.rs"]);
        tm.open_pr(5, "PR");
        let mut h = Harness::new();
        let r = h.click(&mut tm, "#5 PR", egui::PointerButton::Middle);
        assert_eq!(tm.tabs.len(), 1);
        assert_eq!(tm.close_request, None);
        // The file tab becomes active: it is returned to be loaded.
        assert_eq!(r, Some(PathBuf::from("/w/a.rs")));
    }
}
