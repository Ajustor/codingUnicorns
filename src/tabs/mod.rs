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
    /// The file was deleted on disk; the buffer is kept and the title shows it.
    pub is_deleted: bool,
}

pub struct TabManager {
    pub tabs: Vec<Tab>,
    pub active_tab: Option<usize>,
    next_id: usize,
}

impl TabManager {
    pub fn new() -> Self {
        Self {
            tabs: vec![],
            active_tab: None,
            next_id: 0,
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
            is_deleted: false,
        });
        self.active_tab = Some(id);
        id
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

    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        let mut to_open: Option<PathBuf> = None;
        let mut activate_id: Option<usize> = None;
        let mut to_close: Option<usize> = None;
        let active_tab = self.active_tab;

        let tabs_data: Vec<(usize, PathBuf, String, bool, bool)> = self
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
                    t.is_settings || t.page.is_some(),
                )
            })
            .collect();

        ui.horizontal(|ui| {
            ui.style_mut().spacing.item_spacing.x = 0.0;
            for (tab_id, tab_path, tab_title, tab_modified, tab_is_settings) in &tabs_data {
                let is_active = active_tab == Some(*tab_id);
                let bg = if is_active {
                    egui::Color32::from_rgb(30, 30, 30)
                } else {
                    egui::Color32::from_rgb(45, 45, 45)
                };
                let frame_resp = egui::Frame::new().fill(bg).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if *tab_is_settings {
                            let icon = if tab_path.starts_with("__page__") {
                                egui_phosphor::regular::PUZZLE_PIECE
                            } else {
                                egui_phosphor::regular::GEAR
                            };
                            ui.label(
                                egui::RichText::new(icon)
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
                        if ui.selectable_label(is_active, tab_label).clicked() {
                            if *tab_is_settings {
                                activate_id = Some(*tab_id);
                            } else {
                                to_open = Some(tab_path.clone());
                            }
                        }
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

        if let Some(id) = to_close {
            let was_active = self.active_tab == Some(id);
            self.close(id);
            // If the active tab was closed, load the new active tab's file
            if was_active && to_open.is_none() {
                to_open = self
                    .active_tab
                    .and_then(|new_id| self.tabs.iter().find(|t| t.id == new_id))
                    .filter(|t| !t.is_settings && t.page.is_none())
                    .map(|t| t.path.clone());
            }
        }
        to_open
    }
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
}
