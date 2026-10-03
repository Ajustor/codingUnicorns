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
