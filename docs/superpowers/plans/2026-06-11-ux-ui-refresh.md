# UX/UI Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **⚠ Commit policy (user override):** The repo owner's standing rule is *never commit without explicit approval*. Each task ends with a **Checkpoint** step: stage the listed files and **ask the owner** before running `git commit`. The suggested commit message is provided for when approval is given.

**Goal:** Introduce a semantic design-token layer derived from the existing 3-colour theme, rewire the UI onto it, and layer editor-readability + ergonomics/discoverability improvements on top.

**Architecture:** A new `ui::theme` module derives a `Palette` (Copy) and `Spacing` (Copy) from `config::Theme` and maps them to egui `Visuals` + `text_styles`. `layout::render` rebuilds them each frame (≈30 colour mixes, negligible) and stores them on `CodingUnicorns`. Every renderer reads `app.palette` instead of deriving colours locally. New shared widgets (`popup_frame`, `confirm_dialog`, toasts) and a breadcrumbs bar build on the palette.

**Tech Stack:** Rust, egui/eframe 0.31, fuzzy-matcher (SkimMatcherV2), existing LSP `DiagSeverity`.

**Reference spec:** `docs/superpowers/specs/2026-06-11-ux-ui-refresh-design.md`

---

## Phase 1 — Token Foundation

### Task 1: Colour-derivation helpers

**Files:**
- Create: `src/ui/theme.rs`
- Modify: `src/ui/mod.rs` (register module)

- [ ] **Step 1: Create the module with helpers + tests**

Create `src/ui/theme.rs`:

```rust
use egui::Color32;

/// Per-channel linear interpolation. `t` is clamped to [0,1].
pub(crate) fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Perceptual relative luminance in [0,1].
pub(crate) fn luminance(c: Color32) -> f32 {
    (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32) / 255.0
}

/// True if a colour is dark enough to want light text/surfaces lifted toward white.
pub(crate) fn is_dark(c: Color32) -> bool {
    luminance(c) < 0.5
}

/// Pick a readable text colour to draw *on top of* `c` (near-black on light fills, white on dark).
pub(crate) fn on(c: Color32) -> Color32 {
    if luminance(c) > 0.55 {
        Color32::from_rgb(20, 20, 20)
    } else {
        Color32::WHITE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_midpoint_is_grey() {
        let m = mix(Color32::BLACK, Color32::WHITE, 0.5);
        assert!((127..=128).contains(&m.r()));
    }

    #[test]
    fn luminance_bounds() {
        assert!(luminance(Color32::WHITE) > 0.99);
        assert!(luminance(Color32::BLACK) < 0.01);
    }

    #[test]
    fn dark_detection() {
        assert!(is_dark(Color32::from_rgb(30, 30, 30)));
        assert!(!is_dark(Color32::from_rgb(246, 246, 246)));
    }

    #[test]
    fn on_picks_contrasting_text() {
        // Dark blue accent -> white text
        assert_eq!(on(Color32::from_rgb(0, 122, 204)), Color32::WHITE);
        // Bright Monokai green accent -> near-black text
        assert_eq!(on(Color32::from_rgb(166, 226, 46)), Color32::from_rgb(20, 20, 20));
    }
}
```

- [ ] **Step 2: Register the module**

In `src/ui/mod.rs`, add alongside the other `pub mod` lines (e.g. after `pub mod statusbar;`):

```rust
pub mod theme;
```

- [ ] **Step 3: Run the tests**

Run: `rtk cargo test --lib ui::theme`
Expected: 4 tests pass.

- [ ] **Step 4: Checkpoint** — stage `src/ui/theme.rs src/ui/mod.rs`, ask for approval, then:
`git commit -m "feat(ui): add colour-derivation helpers for the theme token layer"`

---

### Task 2: `Palette` struct + derivation

**Files:**
- Modify: `src/ui/theme.rs`

- [ ] **Step 1: Add the `Palette` type, derivation, and tests**

Append to `src/ui/theme.rs` (above the `tests` module):

```rust
/// Semantic colours derived from the 3-colour `config::Theme` seed.
/// Single source of truth for UI rendering. `Copy` so renderers can take it by value.
#[derive(Clone, Copy)]
pub struct Palette {
    // Surfaces (depth hierarchy)
    pub bg: Color32,
    pub surface: Color32,
    pub surface_raised: Color32,
    pub overlay: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    // Text
    pub text: Color32,
    pub text_muted: Color32,
    pub text_faint: Color32,
    // Accent
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_muted: Color32,
    pub on_accent: Color32,
    // Semantic
    pub error: Color32,
    pub warning: Color32,
    pub info: Color32,
    pub success: Color32,
    pub hint: Color32,
    // Git
    pub git_added: Color32,
    pub git_modified: Color32,
    pub git_removed: Color32,
    // Editor
    pub line_highlight: Color32,
    pub selection: Color32,
    pub selection_inactive: Color32,
}

impl Palette {
    pub fn from_theme(theme: &crate::config::Theme) -> Self {
        let bg = Color32::from_rgb(theme.background[0], theme.background[1], theme.background[2]);
        let fg = Color32::from_rgb(theme.foreground[0], theme.foreground[1], theme.foreground[2]);
        let accent = Color32::from_rgb(theme.accent[0], theme.accent[1], theme.accent[2]);
        let toward = if is_dark(bg) { Color32::WHITE } else { Color32::BLACK };
        let alpha = |c: Color32, a: u8| Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a);
        Self {
            bg,
            surface: mix(bg, toward, 0.04),
            surface_raised: mix(bg, toward, 0.08),
            overlay: Color32::from_rgba_unmultiplied(0, 0, 0, 150),
            border: mix(bg, toward, 0.14),
            border_strong: mix(bg, toward, 0.25),
            text: fg,
            text_muted: mix(fg, bg, 0.40),
            text_faint: mix(fg, bg, 0.65),
            accent,
            accent_hover: mix(accent, toward, 0.15),
            accent_muted: alpha(accent, 48),
            on_accent: on(accent),
            error: Color32::from_rgb(229, 83, 75),
            warning: Color32::from_rgb(229, 181, 67),
            info: Color32::from_rgb(86, 156, 214),
            success: Color32::from_rgb(80, 200, 120),
            hint: mix(fg, bg, 0.40),
            git_added: Color32::from_rgb(80, 200, 80),
            git_modified: Color32::from_rgb(80, 150, 255),
            git_removed: Color32::from_rgb(220, 80, 80),
            line_highlight: mix(bg, toward, 0.06),
            selection: alpha(accent, 95),
            selection_inactive: alpha(accent, 45),
        }
    }
}
```

Add these tests inside the existing `tests` module:

```rust
    fn dark_theme() -> crate::config::Theme {
        crate::config::Theme {
            name: "dark".into(),
            background: [30, 30, 30],
            foreground: [212, 212, 212],
            accent: [0, 122, 204],
        }
    }

    #[test]
    fn dark_palette_preserves_current_look() {
        let p = Palette::from_theme(&dark_theme());
        assert_eq!(p.surface.r(), 39); // ~ current +7 (37)
        assert_eq!(p.text_muted.r(), 139); // brighter than old fg/2 (106), more legible
        assert_eq!(p.on_accent, Color32::WHITE);
        assert_eq!(p.selection.a(), 95);
    }

    #[test]
    fn light_background_lifts_surfaces_downward() {
        let light = crate::config::Theme {
            name: "light".into(),
            background: [246, 246, 246],
            foreground: [40, 40, 40],
            accent: [0, 103, 184],
        };
        let p = Palette::from_theme(&light);
        // On a light bg, surfaces mix toward black -> darker than bg (contrast preserved,
        // unlike the old saturating_add which collapsed at 255).
        assert!(p.surface.r() < p.bg.r());
        assert!(p.border.r() < p.surface.r());
    }
```

- [ ] **Step 2: Run the tests**

Run: `rtk cargo test --lib ui::theme`
Expected: 6 tests pass.

- [ ] **Step 3: Checkpoint** — stage `src/ui/theme.rs`, ask, then:
`git commit -m "feat(ui): derive a semantic Palette from the theme seed"`

---

### Task 3: `Spacing` scale

**Files:**
- Modify: `src/ui/theme.rs`

- [ ] **Step 1: Add the `Spacing` type**

Append to `src/ui/theme.rs` (above `tests`):

```rust
/// Spacing + corner-rounding scale. Replaces scattered magic numbers. `Copy`.
#[derive(Clone, Copy)]
pub struct Spacing {
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub round_sm: f32,
    pub round_md: f32,
    pub round_lg: f32,
}

impl Default for Spacing {
    fn default() -> Self {
        Self { xs: 2.0, sm: 4.0, md: 8.0, lg: 12.0, round_sm: 4.0, round_md: 6.0, round_lg: 8.0 }
    }
}
```

- [ ] **Step 2: Build**

Run: `rtk cargo build`
Expected: compiles (the type is unused for now — that is fine; it is consumed in Task 4).

- [ ] **Step 3: Checkpoint** — stage `src/ui/theme.rs`, ask, then:
`git commit -m "feat(ui): add the Spacing scale"`

---

### Task 4: `apply_theme` + wire onto the app

**Files:**
- Modify: `src/ui/theme.rs` (add `apply_theme`, move visuals here)
- Modify: `src/app/mod.rs` (add `palette`/`spacing` fields + init)
- Modify: `src/ui/layout.rs:24` (call `apply_theme`), remove `dark_visuals` (`src/ui/layout.rs:1435-1474`)

- [ ] **Step 1: Add `apply_theme` and the visuals mapping to `theme.rs`**

Append to `src/ui/theme.rs` (above `tests`):

```rust
/// Rebuild the palette + spacing from config, push the matching egui `Visuals`
/// and UI `text_styles` into the context, and return them for renderers to read.
/// Called once per frame at the top of `layout::render`; cost is negligible.
pub fn apply_theme(ctx: &egui::Context, config: &crate::config::Config) -> (Palette, Spacing) {
    let p = Palette::from_theme(&config.theme);
    let spacing = Spacing::default();

    let mut v = if is_dark(p.bg) { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.surface;
    v.window_fill = p.surface_raised;
    v.extreme_bg_color = p.bg;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.selection;
    v.selection.stroke = egui::Stroke::new(1.0, p.accent);
    v.widgets.inactive.weak_bg_fill = p.surface;
    v.widgets.hovered.weak_bg_fill = p.border;
    v.widgets.active.weak_bg_fill = p.border_strong;
    v.window_stroke = egui::Stroke::new(1.0, p.border);
    ctx.set_visuals(v);

    ctx.style_mut(|style| {
        use egui::{FontFamily, FontId, TextStyle};
        style.text_styles.insert(TextStyle::Small, FontId::new(12.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Button, FontId::new(13.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Heading, FontId::new(16.0, FontFamily::Proportional));
    });

    (p, spacing)
}
```

- [ ] **Step 2: Add `palette`/`spacing` fields to the app struct**

In `src/app/mod.rs`, add fields to the `CodingUnicorns` struct (near `status_bar` at `src/app/mod.rs:56`):

```rust
    pub palette: crate::ui::theme::Palette,
    pub spacing: crate::ui::theme::Spacing,
```

In the app constructor (near `status_bar: StatusBar::new(),` at `src/app/mod.rs:230`), initialise from the loaded config (the `config` local must already exist at that point):

```rust
            palette: crate::ui::theme::Palette::from_theme(&config.theme),
            spacing: crate::ui::theme::Spacing::default(),
```

> If `config` is not in scope at that exact line, use `crate::ui::theme::Palette::from_theme(&crate::config::Config::default().theme)`; it is overwritten on the first frame by Step 3 anyway.

- [ ] **Step 3: Call `apply_theme` in `layout::render` and store the result**

In `src/ui/layout.rs`, replace line 24:

```rust
    ctx.set_visuals(dark_visuals(&app.config));
```

with:

```rust
    let (palette, spacing) = crate::ui::theme::apply_theme(ctx, &app.config);
    app.palette = palette;
    app.spacing = spacing;
```

- [ ] **Step 4: Delete the obsolete `dark_visuals`**

In `src/ui/layout.rs`, delete the entire `fn dark_visuals(config: &Config) -> egui::Visuals { … }` (lines 1435-1474). If `Config` becomes an unused import after this, remove it from the `use` at the top.

- [ ] **Step 5: Build and visually verify**

Run: `rtk cargo build`
Expected: compiles, no warnings about unused `dark_visuals`/`Config`.
Then run the app (`cargo run`) with the default **Dark** theme and confirm it looks essentially unchanged (panels slightly cleaner, no regression). This is the foundation-regression gate.

- [ ] **Step 6: Checkpoint** — stage `src/ui/theme.rs src/app/mod.rs src/ui/layout.rs`, ask, then:
`git commit -m "feat(ui): apply theme via the token layer; remove dark_visuals"`

---

## Phase 2 — High-visibility Chrome

### Task 5: Shared `popup_frame` widget

**Files:**
- Create: `src/ui/widgets.rs`
- Modify: `src/ui/mod.rs`

- [ ] **Step 1: Create the widgets module**

Create `src/ui/widgets.rs`:

```rust
use crate::ui::theme::{Palette, Spacing};

/// The shared material for every floating element (autocomplete, hover, palette, find).
/// Gives all popups the same fill / border / rounding / shadow.
pub fn popup_frame(palette: Palette, spacing: Spacing) -> egui::Frame {
    egui::Frame::new()
        .fill(palette.surface_raised)
        .stroke(egui::Stroke::new(1.0, palette.border))
        .corner_radius(spacing.round_md)
        .inner_margin(egui::Margin::same(spacing.xs as i8))
        .shadow(egui::epaint::Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: egui::Color32::from_black_alpha(60),
        })
}
```

> egui 0.31 note: `Frame::corner_radius` takes an `impl Into<CornerRadius>` (f32 works); `Shadow` fields are `offset: [i8;2]`, `blur: u8`, `spread: u8`. If the exact field types differ in the pinned version, adjust the literals to match — the values are what matter.

- [ ] **Step 2: Register the module**

In `src/ui/mod.rs` add:

```rust
pub mod widgets;
```

- [ ] **Step 3: Build**

Run: `rtk cargo build`
Expected: compiles (unused-function warning for `popup_frame` is acceptable; consumed next task).

- [ ] **Step 4: Checkpoint** — stage `src/ui/widgets.rs src/ui/mod.rs`, ask, then:
`git commit -m "feat(ui): add shared popup_frame widget"`

---

### Task 6: Status bar onto the palette

**Files:**
- Modify: `src/ui/statusbar.rs`
- Modify: `src/ui/layout.rs:215`

- [ ] **Step 1: Thread the palette into `StatusBar::show`**

In `src/ui/statusbar.rs`, change the signature (line 22) to accept the palette:

```rust
    pub fn show(
        &self,
        ui: &mut egui::Ui,
        editor: &Editor,
        git: &GitStatus,
        lsp_status: LspStatus,
        palette: crate::ui::theme::Palette,
    ) {
        let bg = palette.accent;
```

- [ ] **Step 2: Use `on_accent` for all text and semantic colours for LSP dots**

In the same function, replace every `egui::Color32::WHITE` text colour with `palette.on_accent`, and the LSP dot colours:
- `Connecting` dot: `palette.warning`
- `Loading` dot: `palette.warning`
- `Ready` dot: `palette.success`
- `Error` dot: `palette.error`

(Leave the `.small()` calls; they now resolve to the 12px UI `Small` style configured in `apply_theme`.)

- [ ] **Step 3: Update the call site**

In `src/ui/layout.rs:215-216`, pass the palette:

```rust
            app.status_bar
                .show(ui, &app.editor, &app.git_status, lsp_status, app.palette);
```

- [ ] **Step 4: Build and verify across themes**

Run: `rtk cargo build`
Then run the app, open **Settings → Theme**, and switch to **Monokai**. Confirm the status-bar text is now dark and legible on the green accent (previously forced white = unreadable), and the bar follows the accent on every preset.

- [ ] **Step 5: Checkpoint** — stage `src/ui/statusbar.rs src/ui/layout.rs`, ask, then:
`git commit -m "feat(ui): status bar follows the theme accent with legible text"`

---

### Task 7: Thread the palette into the editor + line-number colours

**Files:**
- Modify: `src/editor/mod.rs:346` (signature), `:567-568` (line-number colour)
- Modify: `src/ui/layout.rs:1107` and `:1384` (both `Editor::show` call sites)

- [ ] **Step 1: Add a `palette` parameter to `Editor::show`**

In `src/editor/mod.rs:346-353`, add the parameter at the end:

```rust
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        config: &crate::config::Config,
        plugin_manager: &crate::plugin::manager::PluginManager,
        lsp_hover: Option<String>,
        breakpoint_lines: &std::collections::HashSet<usize>,
        palette: crate::ui::theme::Palette,
    ) {
```

- [ ] **Step 2: Drive line-number colour from the palette (active line stands out)**

In `src/editor/mod.rs:567-568`, replace:

```rust
        let line_num_color =
            egui::Color32::from_rgb(fg_color.r() / 2, fg_color.g() / 2, fg_color.b() / 2);
```

with:

```rust
        let line_num_color = palette.text_muted;
        let line_num_color_active = palette.text;
        let (cur_row_for_gutter, _) = self.cursor.position();
```

Then at the two line-number paint sites (`src/editor/mod.rs:2171` and `:2187`), change the colour argument from `line_num_color` to:

```rust
                            if line_idx == cur_row_for_gutter { line_num_color_active } else { line_num_color },
```

- [ ] **Step 3: Update both call sites**

In `src/ui/layout.rs:1107`, the `app.editor.show(` call — add `app.palette` as the final argument. In `src/ui/layout.rs:1384`, the `.show(ui, &app.config, &app.plugin_manager, lsp_hover, &bp_lines)` call (editor2 pane) — add `, app.palette` as the final argument.

> Borrow note: `Palette` is `Copy`, so passing `app.palette` by value alongside `&mut app.editor` does not conflict.

- [ ] **Step 4: Build and verify**

Run: `rtk cargo build`
Then run the app and confirm the current line's number is brighter than the rest.

- [ ] **Step 5: Checkpoint** — stage `src/editor/mod.rs src/ui/layout.rs`, ask, then:
`git commit -m "feat(editor): thread palette into the editor; highlight active line number"`

---

### Task 8: Autocomplete popup onto the palette + fuzzy-match highlight

**Files:**
- Modify: `src/editor/autocomplete.rs`
- Modify: `src/editor/mod.rs:3203` (call site)

- [ ] **Step 1: Store fuzzy-match indices on each suggestion**

In `src/editor/autocomplete.rs`, extend `Suggestion` (line 8) and its constructor:

```rust
#[derive(Clone)]
pub struct Suggestion {
    pub label: String,
    pub kind: Option<String>,
    pub match_indices: Vec<usize>,
}

impl Suggestion {
    fn local(label: String, kind: Option<&str>, match_indices: Vec<usize>) -> Self {
        Self { label, kind: kind.map(|k| k.to_string()), match_indices }
    }
}
```

In `update` (lines 56-83), capture indices via `fuzzy_indices` instead of `fuzzy_match`, and carry them through:

```rust
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

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.truncate(50);

        self.suggestions = scored
            .into_iter()
            .map(|(_, label, kind, idx)| Suggestion::local(label.to_string(), kind, idx))
            .collect();
```

Also update `set_lsp_suggestions` callers: LSP items have no indices, so wherever a `Suggestion` is built from LSP data, pass `match_indices: vec![]`. (Search the codebase for `Suggestion {` / `Suggestion::` construction outside this file and add the field.)

- [ ] **Step 2: Take the palette in `show` and use `popup_frame` + per-char highlight**

In `src/editor/autocomplete.rs`, change `show` (line 119) to `pub fn show(&self, ctx: &egui::Context, palette: crate::ui::theme::Palette, spacing: crate::ui::theme::Spacing)`.

Replace the hard-coded `egui::Frame::new().fill(...).stroke(...).inner_margin(...)` (lines 154-157) with:

```rust
                crate::ui::widgets::popup_frame(palette, spacing)
```

Replace the selected-row fill (line 172) `egui::Color32::from_rgb(30, 80, 140)` with `palette.accent_muted`.

Replace the single-colour label paint (lines 190-196) with a `LayoutJob` that bolds/recolours matched characters:

```rust
                            let mut job = egui::text::LayoutJob::default();
                            for (ci, ch) in suggestion.label.chars().enumerate() {
                                let matched = suggestion.match_indices.contains(&ci);
                                job.append(
                                    &ch.to_string(),
                                    0.0,
                                    egui::text::TextFormat {
                                        font_id: egui::FontId::monospace(13.0),
                                        color: if matched { palette.accent } else { palette.text },
                                        ..Default::default()
                                    },
                                );
                            }
                            let galley = ui.fonts(|f| f.layout_job(job));
                            ui.painter().galley(
                                egui::pos2(rect.min.x + BADGE_W, rect.center().y - galley.size().y / 2.0),
                                galley,
                                palette.text,
                            );
```

- [ ] **Step 3: Give `Editor::show` a `spacing` param, then update the autocomplete call**

`Editor::show` does not yet hold a `Spacing`. First add one, mirroring Task 7: append `spacing: crate::ui::theme::Spacing` to the `Editor::show` signature (`src/editor/mod.rs:346`) and pass `app.spacing` at both call sites (`src/ui/layout.rs:1107` and `:1384`).

Then in `src/editor/mod.rs:3203`, change `self.autocomplete.show(ui.ctx());` to:

```rust
                self.autocomplete.show(ui.ctx(), palette, spacing);
```

- [ ] **Step 4: Build and verify**

Run: `rtk cargo build`
Then run the app, trigger autocomplete (type 2+ chars), and confirm: popup uses the unified surface/border/shadow; the typed characters are highlighted in the accent colour within each suggestion.

- [ ] **Step 5: Checkpoint** — stage `src/editor/autocomplete.rs src/editor/mod.rs src/ui/layout.rs`, ask, then:
`git commit -m "feat(editor): unify autocomplete popup; highlight fuzzy matches"`

---

### Task 9: Remaining popups + command-palette styling

**Files:**
- Modify: `src/editor/mod.rs` (Find/Replace window ~`:364`, Go-to-Line window, hover/signature frames)
- Modify: `src/ui/palette.rs` (`show` signature + selected-item colour)
- Modify: `src/app/mod.rs:1059` (palette call site)

- [ ] **Step 1: Find/Replace & Go-to-Line active toggles use the accent**

In `src/editor/mod.rs`, the Find/Replace window (`egui::Window::new("Find")`, line 364) and Go-to-Line window: where toggle buttons (case-sensitive `Aa`, regex `.*`) colour themselves blue when active, replace the local blue with `palette.accent`. Where the hover/signature tooltip frames set a fill, use `palette.surface_raised`. (Search within `Editor::show` for `Color32::from_rgb` fills/strokes on these overlays and swap to palette tokens; `palette` is in scope from Task 7.)

- [ ] **Step 2: Command palette takes the palette and highlights the selected item**

In `src/ui/palette.rs:129`, add `palette: crate::ui::theme::Palette` as a parameter to `show`. In the command-row rendering (line 305-307), the `RichText` colour `Color32::from_rgb(180, 200, 255)` becomes `palette.accent`. (The egui `Window` already uses `window_fill = surface_raised` from `apply_theme`, so the palette body is themed automatically.)

- [ ] **Step 3: Update the call site**

In `src/app/mod.rs:1059`, add `, self.palette` to the `command_palette.show(ctx, &mut self.file_tree, &mut self.workspace_path)` call.

- [ ] **Step 4: Build and verify**

Run: `rtk cargo build`
Then run the app; open the command palette (Ctrl+Shift+P) and the find bar; confirm consistent surfaces and accent-coloured active toggles/selection.

- [ ] **Step 5: Checkpoint** — stage `src/editor/mod.rs src/ui/palette.rs src/app/mod.rs`, ask, then:
`git commit -m "feat(ui): theme find/goto/command-palette popups via the palette"`

---

## Phase 3 — Editor Readability

### Task 10: Current-line highlight + config toggle

**Files:**
- Modify: `src/config/mod.rs:271` (`EditorConfig`), `:300` (`Default`)
- Modify: `src/editor/mod.rs` (render loop)
- Modify: `src/ui/settings.rs:101-103` area (toggle)

- [ ] **Step 1: Add the config flag**

In `src/config/mod.rs`, add to `EditorConfig` (after `show_minimap`, line 270):

```rust
    #[serde(default = "default_true")]
    pub highlight_current_line: bool,
```

And to the `Default for Config` editor block (after `show_minimap: true,`, line 300):

```rust
                highlight_current_line: true,
```

- [ ] **Step 2: Paint the highlight in the per-line loop**

In `src/editor/mod.rs`, inside the per-line render loop, immediately before the line-number block at `:2178` (`if config.editor.line_numbers {`), add:

```rust
                    // Current-line highlight (suppressed while a selection is active, to avoid noise).
                    let (hl_row, _) = self.cursor.position();
                    let selection_active = sel_range.is_some()
                        || self.extra_cursors.iter().any(|c| c.selection_range().is_some());
                    if config.editor.highlight_current_line && line_idx == hl_row && !selection_active {
                        painter.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(rect.min.x, y),
                                egui::pos2(rect.max.x, y + line_height),
                            ),
                            0.0,
                            palette.line_highlight,
                        );
                    }
```

> If `sel_range` is not yet bound at this point in the loop, use only the `extra_cursors` check plus `self.cursor.selection_range().is_some()`. Verify against the variable in scope (`sel_range` is used later at `:2390`).

- [ ] **Step 3: Add the settings toggle**

In `src/ui/settings.rs`, after the minimap checkbox (lines 101-103), add:

```rust
                if setting_matches(&q, &["current", "line", "highlight", "cursor"]) {
                    changed |= checkbox(ui, &mut config.editor.highlight_current_line, "Highlight current line");
                }
```

Also add `"current line"`, `"highlight"` to the editor-section keyword list at `src/ui/settings.rs:50-54`.

- [ ] **Step 4: Build and verify**

Run: `rtk cargo build`
Then run the app: the cursor line shows a subtle highlight; making a selection removes it; toggling the setting off disables it.

- [ ] **Step 5: Checkpoint** — stage `src/config/mod.rs src/editor/mod.rs src/ui/settings.rs`, ask, then:
`git commit -m "feat(editor): subtle current-line highlight with a settings toggle"`

---

### Task 11: Centralise gutter / diff / diagnostic colours

**Files:**
- Modify: `src/editor/mod.rs` (`:2199-2200` diff bar, `:2262` lightbulb, `:2682-2688` diagnostic squiggle)

- [ ] **Step 1: Diff gutter bar via palette**

In `src/editor/mod.rs:2198-2202`, replace:

```rust
                        let bar_color = match diff_status {
                            DIFF_ADDED => Some(egui::Color32::from_rgb(80, 200, 80)),
                            DIFF_MODIFIED => Some(egui::Color32::from_rgb(80, 150, 255)),
                            _ => None,
                        };
```

with:

```rust
                        let bar_color = match diff_status {
                            DIFF_ADDED => Some(palette.git_added),
                            DIFF_MODIFIED => Some(palette.git_modified),
                            _ => None,
                        };
```

- [ ] **Step 2: Lightbulb marker becomes severity-aware**

In `src/editor/mod.rs:2249-2265`, replace the fixed yellow lightbulb colour `egui::Color32::from_rgb(255, 220, 50)` with the highest severity present on the line:

```rust
                            let sev_color = self
                                .diagnostics
                                .iter()
                                .filter(|d| d.line as usize == line_idx)
                                .map(|d| match d.severity {
                                    crate::lsp::client::DiagSeverity::Error => palette.error,
                                    crate::lsp::client::DiagSeverity::Warning => palette.warning,
                                    _ => palette.info,
                                })
                                .next()
                                .unwrap_or(palette.warning);
```

and pass `sev_color` as the lightbulb text colour.

- [ ] **Step 3: Diagnostic squiggle colours via palette**

In `src/editor/mod.rs:2681-2689`, replace the colour `match`:

```rust
                            let color = match diag.severity {
                                crate::lsp::client::DiagSeverity::Error => palette.error,
                                crate::lsp::client::DiagSeverity::Warning => palette.warning,
                                _ => palette.info,
                            };
```

- [ ] **Step 4: Build and verify**

Run: `rtk cargo build`
Then run the app on a file with errors and warnings (any LSP-backed file); confirm red errors / yellow warnings and that the gutter lightbulb matches the severity.

- [ ] **Step 5: Checkpoint** — stage `src/editor/mod.rs`, ask, then:
`git commit -m "refactor(editor): centralise diff/diagnostic/gutter colours into the palette"`

---

### Task 12: Selection legibility

**Files:**
- Modify: `src/editor/mod.rs` (`:2420-2426` main selection, `:2382` word occurrence, extra-cursor selection block)

- [ ] **Step 1: Main + extra-cursor selection use `palette.selection`**

In `src/editor/mod.rs`, replace each selection-fill `egui::Color32::from_rgba_premultiplied(accent_color.r(), accent_color.g(), accent_color.b(), 60)` (the main one at `:2420-2425` and the equivalent in the extra-cursor loop) with `palette.selection`.

- [ ] **Step 2: Word-occurrence highlight uses `accent_muted`**

In `src/editor/mod.rs:2376-2384` (the word-occurrence highlight at alpha 15), replace the `from_rgba_premultiplied(accent…, 15)` fill with `palette.accent_muted`.

- [ ] **Step 3: Build and verify**

Run: `rtk cargo build`
Then run the app; select text and confirm the selection is clearly visible; double-click a word and confirm other occurrences are visibly (but not harshly) tinted.

- [ ] **Step 4: Checkpoint** — stage `src/editor/mod.rs`, ask, then:
`git commit -m "feat(editor): more legible selection and word-occurrence highlight"`

---

### Task 13: Active indent guide

**Files:**
- Modify: `src/editor/mod.rs` (new helper + indent-guide render at `:2542-2573`)

- [ ] **Step 1: Write the failing test for the enclosing-block helper**

Add to the test module of `src/editor/mod.rs` (or create one if absent — `#[cfg(test)] mod tests { use super::*; … }`):

```rust
    #[test]
    fn active_indent_block_spans_the_enclosing_block() {
        // Lines (indent levels): 0:fn  1:  a   2:  b   3:} -> cursor on line 1 at level 1
        let lines = vec![
            "fn x() {".to_string(),
            "    a();".to_string(),
            "    b();".to_string(),
            "}".to_string(),
        ];
        // level 1 (4 spaces / indent_size 4), block covers rows 1..=2
        let got = active_indent_block(&lines, 1, 4, true);
        assert_eq!(got, Some((1, 1, 2)));
    }
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `rtk cargo test --lib active_indent_block`
Expected: FAIL — `active_indent_block` not found.

- [ ] **Step 3: Implement the helper**

Add to `src/editor/mod.rs` (free function near the bottom of the file, or an `impl`-free `fn`):

```rust
/// Returns `(level, start_row, end_row)` of the indentation block enclosing `cursor_row`,
/// where `level` is the 1-based indent depth of the cursor line. Lines are the buffer's
/// text lines; `indent_size` is the configured tab width; `spaces` selects space vs tab counting.
/// `None` when the cursor line has no indentation (level 0).
pub(crate) fn active_indent_block(
    lines: &[String],
    cursor_row: usize,
    indent_size: usize,
    spaces: bool,
) -> Option<(usize, usize, usize)> {
    let ind = |s: &str| -> usize {
        let size = indent_size.max(1);
        if spaces {
            s.chars().take_while(|&c| c == ' ').count() / size
        } else {
            s.chars().take_while(|&c| c == '\t').count()
        }
    };
    let level = ind(lines.get(cursor_row)?);
    if level == 0 {
        return None;
    }
    let mut start = cursor_row;
    while start > 0 && ind(&lines[start - 1]) >= level {
        start -= 1;
    }
    let mut end = cursor_row;
    while end + 1 < lines.len() && ind(&lines[end + 1]) >= level {
        end += 1;
    }
    Some((level, start, end))
}
```

- [ ] **Step 4: Run the test to confirm it passes**

Run: `rtk cargo test --lib active_indent_block`
Expected: PASS.

- [ ] **Step 5: Use it in the indent-guide render**

In `src/editor/mod.rs`, just before the per-line loop, compute once:

```rust
        let active_block = {
            let lines: Vec<String> = (0..self.buffer.num_lines())
                .map(|i| self.buffer.line(i))
                .collect();
            let (cr, _) = self.cursor.position();
            active_indent_block(&lines, cr, self.detected_indent_size.max(1), self.detected_indent_spaces)
        };
```

> Performance: building a `Vec<String>` of the whole buffer each frame is fine for typical files but wasteful on very large ones. If the buffer exposes a cheaper line accessor, scan rows directly instead. Keep the simple version unless profiling shows a problem.

Then in the indent-guide block (`:2562-2571`), choose the stroke colour per guide `g`:

```rust
                            let guide_color = match active_block {
                                Some((lvl, s, e)) if g == lvl && line_idx >= s && line_idx <= e => {
                                    palette.accent_muted
                                }
                                _ => egui::Color32::from_rgba_unmultiplied(130, 130, 145, 50),
                            };
                            painter.line_segment(
                                [egui::pos2(gx, y), egui::pos2(gx, y + line_height)],
                                egui::Stroke::new(1.0, guide_color),
                            );
```

(Preserve the existing `gx` computation with the `- char_width * 0.5 - 2.0` nudge.)

- [ ] **Step 6: Build and verify**

Run: `rtk cargo build && rtk cargo test --lib active_indent_block`
Then run the app: the indent guide of the block the cursor is in is more visible than the others.

- [ ] **Step 7: Checkpoint** — stage `src/editor/mod.rs`, ask, then:
`git commit -m "feat(editor): highlight the active indent guide"`

---

### Task 14: Breadcrumbs bar

**Files:**
- Create: `src/ui/breadcrumbs.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/layout.rs` (render between tab bar and editor, around `:1107`)

- [ ] **Step 1: Write the failing test for symbol resolution**

Create `src/ui/breadcrumbs.rs`:

```rust
use crate::lsp::client::DocumentSymbol;
use crate::ui::theme::Palette;

/// The deepest document symbol whose start line is at or above `cursor_line`
/// (symbols are assumed ordered by line). Returns its name, or None.
pub(crate) fn enclosing_symbol<'a>(symbols: &'a [DocumentSymbol], cursor_line: u32) -> Option<&'a str> {
    symbols
        .iter()
        .filter(|s| s.line <= cursor_line)
        .max_by_key(|s| s.line)
        .map(|s| s.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::client::DocumentSymbol;

    fn sym(name: &str, line: u32) -> DocumentSymbol {
        DocumentSymbol { name: name.to_string(), kind: "Function".into(), line }
    }

    #[test]
    fn picks_nearest_symbol_at_or_above_cursor() {
        let syms = vec![sym("alpha", 0), sym("beta", 10), sym("gamma", 20)];
        assert_eq!(enclosing_symbol(&syms, 14), Some("beta"));
        assert_eq!(enclosing_symbol(&syms, 0), Some("alpha"));
        assert_eq!(enclosing_symbol(&[], 5), None);
    }
}
```

- [ ] **Step 2: Run it to confirm it fails, then passes**

Run: `rtk cargo test --lib breadcrumbs`
Expected: FAIL until the module is registered, then PASS. Register in `src/ui/mod.rs`:

```rust
pub mod breadcrumbs;
```

Re-run: `rtk cargo test --lib breadcrumbs` → PASS.

> Confirm `DocumentSymbol`'s fields (`name`, `kind`, `line`) match `src/lsp/client.rs:19-24`. They do as of this writing.

- [ ] **Step 3: Add the render function**

Append to `src/ui/breadcrumbs.rs`:

```rust
/// Render a thin breadcrumb bar: workspace-relative path › enclosing symbol.
/// Display-only for now (no click-to-navigate).
pub fn render(
    ui: &mut egui::Ui,
    palette: Palette,
    path: Option<&std::path::Path>,
    workspace: Option<&std::path::Path>,
    symbols: &[DocumentSymbol],
    cursor_line: u32,
) {
    let Some(path) = path else { return };
    egui::Frame::new()
        .fill(palette.surface)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let rel = workspace
                    .and_then(|w| path.strip_prefix(w).ok())
                    .unwrap_or(path);
                let mut first = true;
                for comp in rel.components() {
                    if !first {
                        ui.label(egui::RichText::new("›").color(palette.text_faint).small());
                    }
                    first = false;
                    ui.label(
                        egui::RichText::new(comp.as_os_str().to_string_lossy())
                            .color(palette.text_muted)
                            .small(),
                    );
                }
                if let Some(name) = enclosing_symbol(symbols, cursor_line) {
                    ui.label(egui::RichText::new("›").color(palette.text_faint).small());
                    ui.label(egui::RichText::new(name).color(palette.text).small());
                }
            });
        });
}
```

- [ ] **Step 4: Render it above the editor**

In `src/ui/layout.rs`, in the central editor area just before `app.editor.show(` (`:1107`), call the bar. The editor's current document symbols are needed — if the app already stores them (search for `document_symbols` / `outline_symbols` on `CodingUnicorns` or `Editor`), pass that slice; otherwise pass `&[]` for now (path breadcrumbs still render):

```rust
                    crate::ui::breadcrumbs::render(
                        ui,
                        app.palette,
                        app.editor.current_path.as_deref(),
                        app.workspace_path.as_deref(),
                        app.editor.outline_symbols.as_deref().unwrap_or(&[]),
                        app.editor.cursor.position().0 as u32,
                    );
```

> Adjust `app.editor.outline_symbols` to whatever field holds the LSP `documentSymbol` results. If none exists yet, pass `&[]`; the path portion is still useful and the symbol portion lights up once symbols are wired (out of scope to add the storage here if absent).

- [ ] **Step 5: Build and verify**

Run: `rtk cargo build && rtk cargo test --lib breadcrumbs`
Then run the app; a thin path bar appears above the editor and updates as you switch files.

- [ ] **Step 6: Checkpoint** — stage `src/ui/breadcrumbs.rs src/ui/mod.rs src/ui/layout.rs`, ask, then:
`git commit -m "feat(ui): add breadcrumbs bar (path + enclosing symbol)"`

---

## Phase 4 — Ergonomics & Discoverability

### Task 15: Delete confirmation

**Files:**
- Modify: `src/ui/widgets.rs` (add `confirm_dialog`)
- Modify: `src/app/mod.rs` (add `pending_delete` field)
- Modify: `src/ui/layout.rs:569-576` (defer the delete) + render the dialog

- [ ] **Step 1: Add a `confirm_dialog` helper**

Append to `src/ui/widgets.rs`:

```rust
/// A small centred modal. Returns `Some(true)` if confirmed, `Some(false)` if cancelled,
/// `None` while still open. Caller clears its trigger state on a `Some(_)`.
pub fn confirm_dialog(
    ctx: &egui::Context,
    palette: Palette,
    title: &str,
    message: &str,
) -> Option<bool> {
    let mut result = None;
    // Dim the background.
    egui::Area::new(egui::Id::new("confirm_scrim"))
        .order(egui::Order::Background)
        .show(ctx, |ui| {
            let r = ctx.screen_rect();
            ui.painter().rect_filled(r, 0.0, palette.overlay);
        });
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(message).color(palette.text));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    result = Some(false);
                }
                if ui
                    .add(egui::Button::new(egui::RichText::new("Delete").color(palette.on_accent))
                        .fill(palette.error))
                    .clicked()
                {
                    result = Some(true);
                }
            });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        result = Some(false);
    }
    result
}
```

- [ ] **Step 2: Add `pending_delete` to the app**

In `src/app/mod.rs`, add a field to `CodingUnicorns`:

```rust
    pub pending_delete: Option<std::path::PathBuf>,
```

and initialise it in the constructor: `pending_delete: None,`.

- [ ] **Step 3: Defer the delete instead of doing it immediately**

In `src/ui/layout.rs:569-576`, replace the `FileTreeAction::Delete(path)` arm:

```rust
                                FileTreeAction::Delete(path) => {
                                    app.pending_delete = Some(path);
                                }
```

- [ ] **Step 4: Render the confirmation and act on it**

In `src/ui/layout.rs`, near the end of `render` (after the editor/panels, before the closing of the function), add:

```rust
    if let Some(path) = app.pending_delete.clone() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let kind = if path.is_dir() { "folder" } else { "file" };
        match crate::ui::widgets::confirm_dialog(
            ctx,
            app.palette,
            "Confirm delete",
            &format!("Delete {kind} \"{name}\"? This cannot be undone."),
        ) {
            Some(true) => {
                if path.is_dir() {
                    let _ = std::fs::remove_dir_all(&path);
                } else {
                    let _ = std::fs::remove_file(&path);
                }
                app.file_tree.reload_children();
                app.pending_delete = None;
            }
            Some(false) => app.pending_delete = None,
            None => {}
        }
    }
```

- [ ] **Step 5: Build and verify**

Run: `rtk cargo build`
Then run the app, right-click a file in the tree → Delete; confirm the dialog appears and only deletes on confirm.

- [ ] **Step 6: Checkpoint** — stage `src/ui/widgets.rs src/app/mod.rs src/ui/layout.rs`, ask, then:
`git commit -m "feat(ui): confirm before deleting files/folders"`

---

### Task 16: Command-palette enrichment

**Files:**
- Modify: `src/ui/palette.rs` (descriptions + index)
- Modify: `src/app/mod.rs:1066-1090` (handlers for any new commands)

- [ ] **Step 1: Add descriptions and index them in the matcher**

In `src/ui/palette.rs`, add a `description` method beside `label`/`shortcut`:

```rust
    pub fn description(&self) -> &'static str {
        match self {
            Self::ToggleTerminal => "open close integrated shell console",
            Self::ToggleClaude => "ai assistant chat panel",
            Self::ToggleSidebar => "explorer file tree side panel",
            Self::GoToLine => "jump navigate to line number",
            Self::SaveFile => "write persist current document",
            Self::NewFile => "create blank document",
            Self::OpenFolder => "open workspace project directory",
            Self::OpenSettings => "preferences configuration options theme",
            Self::Find => "search text in current file",
            Self::FindReplace => "search and substitute replace text",
            Self::RestartLsp => "restart language server diagnostics",
        }
    }
```

Then in both command-matching loops (`:165-174` and `:198-207`), match against label **and** description. Replace each:

```rust
                if effective_query.is_empty()
                    || self.matcher.fuzzy_match(cmd.label(), &effective_query).is_some()
                {
```

with:

```rust
                let haystack = format!("{} {}", cmd.label(), cmd.description());
                if effective_query.is_empty()
                    || self.matcher.fuzzy_match(&haystack, &effective_query).is_some()
                {
```

- [ ] **Step 2: (Optional, only if adding commands) wire new variants**

If you add variants to `PaletteCommand` (e.g. `Format`, `GoToDefinition`, `FindReferences`, `ToggleBlame`, `ToggleSplit`), you MUST: add them to `all()` (`:25-39`), `label`, `shortcut`, `description`, and add a handler arm in `src/app/mod.rs:1066-1090` calling the existing method (e.g. `PaletteCommand::Format => self.editor_format_document(),` — use the actual method names already in the app; grep for the matching keybinding handler to find them). Skip this step if not adding commands.

- [ ] **Step 3: Build and verify**

Run: `rtk cargo build`
Then run the app; open commands (Ctrl+Shift+P), type "assistant" and confirm "Toggle Claude panel" appears (matched via description).

- [ ] **Step 4: Checkpoint** — stage `src/ui/palette.rs src/app/mod.rs`, ask, then:
`git commit -m "feat(palette): index command descriptions for better discoverability"`

---

### Task 17: Transient toasts

**Files:**
- Modify: `src/ui/widgets.rs` (Toast type + render)
- Modify: `src/app/mod.rs` (state + `toast()` helper + fire sites)
- Modify: `src/ui/layout.rs` (render toasts)

- [ ] **Step 1: Add the Toast type + render to widgets.rs**

Append to `src/ui/widgets.rs`:

```rust
pub struct Toast {
    pub message: String,
    pub born: std::time::Instant,
}

const TOAST_TTL_MS: u128 = 2000;
const TOAST_FADE_MS: u128 = 400;

/// Draw active toasts bottom-centre and drop expired ones. Returns true if any remain
/// (so the caller can request a repaint).
pub fn render_toasts(ctx: &egui::Context, palette: Palette, spacing: Spacing, toasts: &mut Vec<Toast>) -> bool {
    toasts.retain(|t| t.born.elapsed().as_millis() < TOAST_TTL_MS);
    if toasts.is_empty() {
        return false;
    }
    egui::Area::new(egui::Id::new("toasts"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -48.0])
        .show(ctx, |ui| {
            for t in toasts.iter() {
                let elapsed = t.born.elapsed().as_millis();
                let remaining = TOAST_TTL_MS.saturating_sub(elapsed);
                let a = if remaining < TOAST_FADE_MS {
                    (remaining as f32 / TOAST_FADE_MS as f32 * 255.0) as u8
                } else {
                    255
                };
                popup_frame(palette, spacing).show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(&t.message)
                            .color(palette.text.gamma_multiply(a as f32 / 255.0)),
                    );
                });
                ui.add_space(spacing.sm);
            }
        });
    true
}
```

- [ ] **Step 2: Add state + helper to the app**

In `src/app/mod.rs`, add a field `pub toasts: Vec<crate::ui::widgets::Toast>,` (init `toasts: Vec::new(),`) and a helper:

```rust
    pub fn toast(&mut self, message: impl Into<String>) {
        self.toasts.push(crate::ui::widgets::Toast {
            message: message.into(),
            born: std::time::Instant::now(),
        });
    }
```

> `std::time::Instant::now()` is available in normal app code (this is not a workflow script).

- [ ] **Step 3: Fire toasts from currently-silent actions**

Add `self.toast("…")` calls at: the save path (after a successful save — search `self.editor.save()` call sites in `src/app/`), the format-document action, and the LSP restart handler (`PaletteCommand::RestartLsp`, `src/app/mod.rs:1086`). Examples:
- after save: `self.toast("Saved");`
- after restart: `self.toast("LSP restarted");`
- after format: `self.toast("Formatted");`

- [ ] **Step 4: Render the toasts each frame**

In `src/ui/layout.rs`, at the end of `render`:

```rust
    if crate::ui::widgets::render_toasts(ctx, app.palette, app.spacing, &mut app.toasts) {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
```

- [ ] **Step 5: Build and verify**

Run: `rtk cargo build`
Then run the app, save a file (Ctrl+S), and confirm a "Saved" toast appears bottom-centre and fades out after ~2s.

- [ ] **Step 6: Checkpoint** — stage `src/ui/widgets.rs src/app/mod.rs src/ui/layout.rs`, ask, then:
`git commit -m "feat(ui): transient toast feedback for silent actions"`

---

### Task 18: Light theme preset

**Files:**
- Modify: `src/ui/settings.rs:165-170`

- [ ] **Step 1: Add the Light preset**

In `src/ui/settings.rs`, add to the `PRESETS` array (after the One Dark row, line 169):

```rust
                        ThemePreset { name: "light",          label: "Light",     bg: [246,246,246], fg: [40,40,40],   accent: [0,103,184] },
```

Add `"light"` to the preset keyword list at `src/ui/settings.rs:164` so it is searchable.

- [ ] **Step 2: Build and verify**

Run: `rtk cargo build`
Then run the app, open Settings → Theme → **Light**, and confirm the whole UI flips to a legible light scheme (surfaces darker than the background, readable text, themed status bar). This exercises the light branch of `apply_theme` end-to-end.

- [ ] **Step 3: Checkpoint** — stage `src/ui/settings.rs`, ask, then:
`git commit -m "feat(ui): add a Light theme preset"`

---

## Final Verification

- [ ] Run the full test suite: `rtk cargo test`
- [ ] Run clippy: `rtk cargo clippy` — fix any new warnings introduced by these changes.
- [ ] Run the app and walk all four presets + Light, exercising: status bar contrast, popups (autocomplete/find/palette), current-line highlight, diagnostics, indent guide, breadcrumbs, delete confirmation, a toast. Screenshot for the record.

## Notes for the implementer

- **Borrowing:** `Palette` and `Spacing` are `Copy`. Pass them by value; this avoids conflicts with `&mut app.editor` / `&mut self`.
- **egui 0.31 surface area:** `Frame::corner_radius`, `Shadow` field names, and `gamma_multiply` are the most likely version-sensitive spots. If a call does not type-check, adjust to the pinned API — the intent (rounded, shadowed, faded) is what matters, not the exact symbol.
- **Out of scope (do NOT add):** minimap auto-hide, zoom, diff view, go-to-line history, settings-search "did you mean", configurable tokenization thresholds, find-bar resize, theme preview, click-to-navigate breadcrumbs, folding syntax colours into the palette. These are deferred per the spec.
- **Preserve:** the indent-guide nudge geometry (`- char_width * 0.5 - 2.0`) and the syntax-highlight colours in `editor/highlight.rs` (untouched this pass).
```
