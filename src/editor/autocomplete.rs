use std::collections::HashSet;

use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;

/// A single completion entry: the text to insert plus an optional kind tag
/// (e.g. "Function", "Keyword") used to render a colored badge in the popup.
#[derive(Clone)]
pub struct Suggestion {
    pub label: String,
    pub kind: Option<String>,
    pub match_indices: Vec<usize>,
}

impl Suggestion {
    fn local(label: String, kind: Option<&str>, match_indices: Vec<usize>) -> Self {
        Self {
            label,
            kind: kind.map(|k| k.to_string()),
            match_indices,
        }
    }
}

pub struct Autocomplete {
    pub visible: bool,
    pub query: String,
    pub suggestions: Vec<Suggestion>,
    pub selected: usize,
    pub cursor_screen_pos: egui::Pos2,
    matcher: SkimMatcherV2,
}

impl Autocomplete {
    pub fn new() -> Self {
        Self {
            visible: false,
            query: String::new(),
            suggestions: Vec::new(),
            selected: 0,
            cursor_screen_pos: egui::Pos2::ZERO,
            matcher: SkimMatcherV2::default(),
        }
    }

    /// Recompute suggestions based on the partial word being typed, ranked by a
    /// fuzzy-match score (subsequence match, not just prefix).
    pub fn update(&mut self, word: &str, buffer_words: &[String], lang_keywords: &[&str]) {
        if word.chars().count() < 2 {
            self.visible = false;
            return;
        }

        self.query = word.to_string();

        let mut seen: HashSet<&str> = HashSet::new();
        // (score, label, kind, match_indices)
        let mut scored: Vec<(i64, &str, Option<&str>, Vec<usize>)> = Vec::new();

        for &kw in lang_keywords {
            if kw == word || !seen.insert(kw) {
                continue;
            }
            if let Some((score, idx)) = self.matcher.fuzzy_indices(kw, word) {
                scored.push((score, kw, Some("Keyword"), idx));
            }
        }
        for bw in buffer_words {
            if bw == word || !seen.insert(bw.as_str()) {
                continue;
            }
            if let Some((score, idx)) = self.matcher.fuzzy_indices(bw, word) {
                scored.push((score, bw.as_str(), None, idx));
            }
        }

        // Highest score first. `sort_by` is stable, so ties keep insertion order
        // (keywords before buffer words).
        scored.sort_by_key(|b| std::cmp::Reverse(b.0));
        scored.truncate(50);

        self.suggestions = scored
            .into_iter()
            .map(|(_, label, kind, idx)| Suggestion::local(label.to_string(), kind, idx))
            .collect();
        self.selected = 0;
        self.visible = !self.suggestions.is_empty();
    }

    /// Returns the label to confirm, if any.
    pub fn confirm(&self) -> Option<&str> {
        if self.visible && !self.suggestions.is_empty() {
            Some(self.suggestions[self.selected].label.as_str())
        } else {
            None
        }
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if !self.suggestions.is_empty() {
            self.selected = (self.selected + 1).min(self.suggestions.len() - 1);
        }
    }

    /// Populate suggestions directly from LSP completion items (already ranked by
    /// the server) and show the popup.
    pub fn set_lsp_suggestions(&mut self, items: Vec<Suggestion>) {
        if items.is_empty() {
            return;
        }
        self.suggestions = items;
        self.selected = 0;
        self.visible = true;
    }

    /// Render the popup using an egui Area (does not consume keyboard focus).
    pub fn show(
        &self,
        ctx: &egui::Context,
        palette: crate::ui::theme::Palette,
        spacing: crate::ui::theme::Spacing,
    ) {
        if !self.visible || self.suggestions.is_empty() {
            return;
        }

        const ITEM_HEIGHT: f32 = 20.0;
        const POPUP_WIDTH: f32 = 240.0;
        const BADGE_W: f32 = 22.0;
        const MAX_VISIBLE: usize = 8;

        // Compute which window of suggestions to show.
        let scroll_start = if self.selected >= MAX_VISIBLE {
            self.selected + 1 - MAX_VISIBLE
        } else {
            0
        };
        let end = (scroll_start + MAX_VISIBLE).min(self.suggestions.len());
        let visible = &self.suggestions[scroll_start..end];

        // Decide whether to show below or above the cursor.
        let screen_height = ctx.screen_rect().height();
        let popup_height = visible.len() as f32 * ITEM_HEIGHT + 8.0;
        let pos = if self.cursor_screen_pos.y + popup_height > screen_height {
            egui::pos2(
                self.cursor_screen_pos.x,
                self.cursor_screen_pos.y - popup_height - ITEM_HEIGHT,
            )
        } else {
            self.cursor_screen_pos
        };

        egui::Area::new(egui::Id::new("autocomplete_popup"))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                crate::ui::widgets::popup_frame(palette, spacing).show(ui, |ui| {
                    for (i, suggestion) in visible.iter().enumerate() {
                        let actual_idx = scroll_start + i;
                        let is_selected = actual_idx == self.selected;

                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(POPUP_WIDTH, ITEM_HEIGHT),
                            egui::Sense::hover(),
                        );

                        if is_selected {
                            ui.painter().rect_filled(rect, 2.0, palette.accent_muted);
                        }

                        // Kind badge (left gutter).
                        if let Some(kind) = &suggestion.kind {
                            let (badge, color) = kind_badge(kind);
                            if !badge.is_empty() {
                                ui.painter().text(
                                    egui::pos2(rect.min.x + 5.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    badge,
                                    egui::FontId::monospace(12.0),
                                    color,
                                );
                            }
                        }

                        let mut job = egui::text::LayoutJob::default();
                        for (ci, ch) in suggestion.label.chars().enumerate() {
                            let matched = suggestion.match_indices.contains(&ci);
                            job.append(
                                &ch.to_string(),
                                0.0,
                                egui::text::TextFormat {
                                    font_id: egui::FontId::monospace(13.0),
                                    color: if matched {
                                        palette.accent
                                    } else {
                                        palette.text
                                    },
                                    ..Default::default()
                                },
                            );
                        }
                        let galley = ui.fonts(|f| f.layout_job(job));
                        let text_pos = egui::pos2(
                            rect.min.x + BADGE_W,
                            rect.center().y - galley.size().y / 2.0,
                        );
                        ui.painter().galley(text_pos, galley, palette.text);
                    }
                });
            });
    }
}

/// Map a completion kind to a compact badge + color for the popup gutter.
/// Accepts LSP kind names ("Function", "Class", …) and local tags ("Keyword").
fn kind_badge(kind: &str) -> (&'static str, egui::Color32) {
    use egui::Color32;
    match kind {
        "Function" | "Method" => ("ƒ", Color32::from_rgb(220, 220, 170)),
        "Constructor" | "Class" | "Interface" => ("C", Color32::from_rgb(78, 201, 176)),
        "Field" | "Property" => ("○", Color32::from_rgb(156, 220, 254)),
        "Variable" => ("v", Color32::from_rgb(156, 220, 254)),
        "Module" => ("M", Color32::from_rgb(197, 134, 192)),
        "Keyword" => ("kw", Color32::from_rgb(197, 134, 192)),
        "Snippet" => ("◇", Color32::from_gray(180)),
        "Text" => ("", Color32::from_gray(140)),
        _ => ("•", Color32::from_gray(140)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(ws: &[&str]) -> Vec<String> {
        ws.iter().map(|w| w.to_string()).collect()
    }

    fn labels(ac: &Autocomplete) -> Vec<&str> {
        ac.suggestions.iter().map(|s| s.label.as_str()).collect()
    }

    fn sugg(label: &str, kind: Option<&str>) -> Suggestion {
        Suggestion {
            label: label.to_string(),
            kind: kind.map(str::to_string),
            match_indices: vec![],
        }
    }

    #[test]
    fn new_is_hidden_and_empty() {
        let ac = Autocomplete::new();
        assert!(!ac.visible);
        assert!(ac.suggestions.is_empty());
        assert_eq!(ac.selected, 0);
        assert!(ac.confirm().is_none());
    }

    #[test]
    fn short_words_hide_the_popup() {
        let mut ac = Autocomplete::new();
        ac.visible = true;
        ac.update("a", &words(&["abc"]), &[]);
        assert!(!ac.visible);
        ac.update("", &words(&["abc"]), &[]);
        assert!(!ac.visible);
    }

    #[test]
    fn fuzzy_matches_are_ranked_and_non_matches_dropped() {
        let mut ac = Autocomplete::new();
        ac.update(
            "prn",
            &words(&["pr", "println", "spring_run", "other", "nrp"]),
            &[],
        );
        assert!(ac.visible);
        assert_eq!(ac.query, "prn");
        let l = labels(&ac);
        assert!(l.contains(&"println"));
        assert!(
            l.contains(&"spring_run"),
            "subsequence match, not just prefix"
        );
        assert!(!l.contains(&"other"));
        assert!(!l.contains(&"pr"), "missing the 'n'");
        assert!(!l.contains(&"nrp"), "order matters");
        // Every suggestion records which chars matched the query.
        for s in &ac.suggestions {
            assert_eq!(s.match_indices.len(), 3, "{}", s.label);
            assert!(s.kind.is_none());
        }
    }

    #[test]
    fn prefix_matches_rank_above_scattered_matches() {
        let mut ac = Autocomplete::new();
        ac.update("foo", &words(&["xfxoxo", "foobar"]), &[]);
        assert_eq!(labels(&ac)[0], "foobar");
    }

    #[test]
    fn keywords_are_tagged_and_deduplicated_against_buffer_words() {
        let mut ac = Autocomplete::new();
        ac.update("re", &words(&["return", "result"]), &["return", "ref"]);
        let ret: Vec<_> = ac
            .suggestions
            .iter()
            .filter(|s| s.label == "return")
            .collect();
        assert_eq!(ret.len(), 1, "duplicates collapse to one entry");
        assert_eq!(ret[0].kind.as_deref(), Some("Keyword"));
        let result = ac.suggestions.iter().find(|s| s.label == "result").unwrap();
        assert!(result.kind.is_none());
        assert!(labels(&ac).contains(&"ref"));
    }

    #[test]
    fn exact_word_is_not_suggested() {
        let mut ac = Autocomplete::new();
        ac.update("let", &words(&["let", "letter"]), &["let"]);
        assert_eq!(labels(&ac), vec!["letter"]);
    }

    #[test]
    fn no_matches_hides_popup() {
        let mut ac = Autocomplete::new();
        ac.update("zz", &words(&["abc", "def"]), &["fn"]);
        assert!(!ac.visible);
        assert!(ac.suggestions.is_empty());
    }

    #[test]
    fn suggestions_are_capped_at_50() {
        let many: Vec<String> = (0..80).map(|i| format!("item{i:02}")).collect();
        let mut ac = Autocomplete::new();
        ac.update("it", &many, &[]);
        assert_eq!(ac.suggestions.len(), 50);
    }

    #[test]
    fn update_resets_selection() {
        let mut ac = Autocomplete::new();
        ac.update("ab", &words(&["abc", "abd", "abe"]), &[]);
        ac.move_down();
        ac.move_down();
        assert_eq!(ac.selected, 2);
        ac.update("ab", &words(&["abc", "abd"]), &[]);
        assert_eq!(ac.selected, 0);
    }

    #[test]
    fn move_up_and_down_clamp_to_bounds() {
        let mut ac = Autocomplete::new();
        ac.move_down();
        assert_eq!(ac.selected, 0, "no suggestions: stays at 0");
        ac.set_lsp_suggestions(vec![sugg("a", None), sugg("b", None), sugg("c", None)]);
        ac.move_up();
        assert_eq!(ac.selected, 0);
        ac.move_down();
        ac.move_down();
        ac.move_down();
        assert_eq!(ac.selected, 2);
        assert_eq!(ac.confirm(), Some("c"));
        ac.move_up();
        assert_eq!(ac.confirm(), Some("b"));
    }

    #[test]
    fn confirm_requires_visible_popup() {
        let mut ac = Autocomplete::new();
        ac.set_lsp_suggestions(vec![sugg("value", Some("Variable"))]);
        assert_eq!(ac.confirm(), Some("value"));
        ac.visible = false;
        assert_eq!(ac.confirm(), None);
    }

    #[test]
    fn set_lsp_suggestions_replaces_list_and_ignores_empty() {
        let mut ac = Autocomplete::new();
        ac.set_lsp_suggestions(vec![]);
        assert!(!ac.visible);

        ac.set_lsp_suggestions(vec![sugg("x", None), sugg("y", Some("Function"))]);
        ac.move_down();
        assert!(ac.visible);
        assert_eq!(ac.selected, 1);

        ac.set_lsp_suggestions(vec![sugg("z", None)]);
        assert_eq!(labels(&ac), vec!["z"]);
        assert_eq!(ac.selected, 0);

        // An empty LSP response keeps the previous list.
        ac.set_lsp_suggestions(vec![]);
        assert_eq!(labels(&ac), vec!["z"]);
    }

    #[test]
    fn kind_badges() {
        use egui::Color32;
        assert_eq!(kind_badge("Function").0, "ƒ");
        assert_eq!(kind_badge("Method").0, "ƒ");
        assert_eq!(kind_badge("Class").0, "C");
        assert_eq!(kind_badge("Constructor").0, "C");
        assert_eq!(kind_badge("Interface").0, "C");
        assert_eq!(kind_badge("Field").0, "○");
        assert_eq!(kind_badge("Property").0, "○");
        assert_eq!(kind_badge("Variable").0, "v");
        assert_eq!(kind_badge("Module").0, "M");
        assert_eq!(
            kind_badge("Keyword"),
            ("kw", Color32::from_rgb(197, 134, 192))
        );
        assert_eq!(kind_badge("Snippet").0, "◇");
        assert_eq!(kind_badge("Text").0, "");
        assert_eq!(kind_badge("Unknown"), ("•", Color32::from_gray(140)));
    }

    // ── Rendering (headless egui) ───────────────────────────────────────────

    fn palette() -> crate::ui::theme::Palette {
        crate::ui::theme::Palette::from_theme(&crate::config::Theme {
            name: "test".into(),
            background: [30, 30, 30],
            foreground: [212, 212, 212],
            accent: [0, 122, 204],
        })
    }

    /// Render the popup for a couple of frames and return its area rect, if shown.
    fn render(ac: &Autocomplete) -> Option<egui::Rect> {
        let ctx = egui::Context::default();
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        for _ in 0..2 {
            let _ = ctx.run(input(), |ctx| {
                ac.show(ctx, palette(), crate::ui::theme::Spacing::default());
            });
        }
        ctx.memory(|m| m.area_rect(egui::Id::new("autocomplete_popup")))
    }

    fn popup_with(n: usize) -> Autocomplete {
        let kinds = [Some("Function"), Some("Text"), None, Some("Keyword")];
        let mut ac = Autocomplete::new();
        ac.set_lsp_suggestions(
            (0..n)
                .map(|i| Suggestion {
                    label: format!("item{i}"),
                    kind: kinds[i % kinds.len()].map(str::to_string),
                    match_indices: vec![0, 1],
                })
                .collect(),
        );
        ac
    }

    #[test]
    fn show_does_nothing_when_hidden() {
        let mut ac = popup_with(3);
        ac.visible = false;
        assert!(render(&ac).is_none());
        assert!(render(&Autocomplete::new()).is_none());
    }

    #[test]
    fn show_places_popup_below_cursor_when_it_fits() {
        let mut ac = popup_with(3);
        ac.cursor_screen_pos = egui::pos2(10.0, 100.0);
        let rect = render(&ac).expect("popup rendered");
        assert_eq!(rect.min, egui::pos2(10.0, 100.0));
        assert!(rect.height() >= 60.0 && rect.height() < 80.0, "{rect:?}");
    }

    #[test]
    fn show_flips_popup_above_cursor_near_bottom_edge() {
        let mut ac = popup_with(3);
        ac.cursor_screen_pos = egui::pos2(10.0, 590.0);
        let rect = render(&ac).expect("popup rendered");
        // 590 - (3 * 20 + 8) - 20
        assert_eq!(rect.min, egui::pos2(10.0, 502.0));
    }

    #[test]
    fn show_limits_visible_rows_and_scrolls_to_selection() {
        let mut ac = popup_with(12);
        for _ in 0..10 {
            ac.move_down();
        }
        assert_eq!(ac.selected, 10);
        ac.cursor_screen_pos = egui::pos2(0.0, 0.0);
        let rect = render(&ac).expect("popup rendered");
        // At most 8 rows are drawn: same height as a popup with exactly 8 items,
        // taller than one with 7.
        let eight = render(&popup_with(8)).unwrap();
        let seven = render(&popup_with(7)).unwrap();
        assert_eq!(rect.height(), eight.height(), "{rect:?}");
        assert!(rect.height() > seven.height() + 19.0);
    }
}
