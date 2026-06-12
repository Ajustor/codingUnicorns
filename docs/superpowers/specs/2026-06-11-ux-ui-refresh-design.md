# Coding Unicorns — UX/UI Refresh Design

**Date:** 2026-06-11
**Scope:** A system-level visual + UX refresh delivered in 4 dependent layers. Introduces a semantic design-token layer derived from the existing 3-colour theme, rewires the UI onto it, then layers editor-readability and ergonomics/discoverability improvements on top.

---

## Goals

Make the editor feel *designed* and consistent, without changing the lightweight philosophy or steamrolling the author's recent hand-tuning.

- **Visual finish** — colours and contrast are coherent everywhere; the status bar follows the theme; selection and diagnostics are legible; surfaces have a clear hierarchy.
- **Editor readability** — current line, active line number, per-severity diagnostics, active indent guide, breadcrumbs.
- **Ergonomics** — destructive-action confirmation, action feedback (toasts), unified popups.
- **Discoverability** — richer command palette (descriptions, shortcuts, more commands).

## Layers Overview

| # | Layer | Depends on | Risk | Visible win |
|---|-------|-----------|------|-------------|
| 1 | Design token layer (foundation) | — | Low (additive) | Indirect |
| 2 | High-visibility chrome | 1 | Low | High |
| 3 | Editor readability | 1 | Medium | High |
| 4 | Ergonomics & discoverability | 1, 2 | Low | Medium |

Each layer is independently shippable. The order respects dependencies (tokens first).

## Guiding Decisions

- **The user still edits only 3 colours.** `bg` / `fg` / `accent` (plus presets) remain the *seed*. Everything else is *derived*. Existing themes, presets and custom colours keep working.
- **UI typography is decoupled from the editor font.** UI text uses egui's native `text_styles` at fixed, legible sizes; it does not scale with `config.font.size` (which would make the chrome huge at 24px). A separate "UI scale" setting may come later, out of scope here.
- **Names:** the colour set is `Palette`; the spacing/rounding scale is `Spacing`.
- **Light themes become viable.** Derivation detects light vs dark via background luminance and lifts surfaces toward white (dark) or black (light), replacing the additive `saturating_add(+7/+15/+30)` model that collapses on light backgrounds.
- **Syntax-highlighting colours (`editor/highlight.rs`) are intentionally left untouched** this pass — they are recently hand-tuned. They stay their own concern; centralising them into `Palette` is explicitly out of scope.
- **The recent indent-guide tuning is preserved** (the `-2px` horizontal nudge and faint stroke stay).

---

## Layer 1 — Design Token Layer (the foundation)

### Goal
Add the missing semantic layer between the 3 seed colours and the UI, so consistency is automatic and every renderer reads from one source of truth.

### New module `src/ui/theme.rs`

`config::Theme` (the 3-colour seed) is unchanged. The new module owns derivation, the token types, and the egui `Visuals` mapping. `dark_visuals` moves here from `layout.rs`.

```rust
/// Semantic colours derived from the 3-colour Theme seed. Single source of truth for UI rendering.
pub struct Palette {
    // Surfaces — depth hierarchy
    pub bg: Color32,             // editor background (seed bg)
    pub surface: Color32,        // panels, sidebar, tab bar
    pub surface_raised: Color32, // popups: autocomplete, hover, palette, find
    pub overlay: Color32,        // modal scrim
    pub border: Color32,         // dividers, popup borders
    pub border_strong: Color32,  // focused field border

    // Text
    pub text: Color32,           // primary (seed fg)
    pub text_muted: Color32,     // secondary labels, inactive line numbers
    pub text_faint: Color32,     // disabled, placeholders

    // Accent
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_muted: Color32,   // low-alpha accent for subtle fills
    pub on_accent: Color32,      // text legible *on top of* accent (auto black/white)

    // Semantic
    pub error: Color32,
    pub warning: Color32,
    pub info: Color32,
    pub success: Color32,
    pub hint: Color32,

    // Git (centralised from editor/mod.rs)
    pub git_added: Color32,
    pub git_modified: Color32,
    pub git_removed: Color32,

    // Editor
    pub line_highlight: Color32, // subtle fill behind the cursor line
    pub selection: Color32,
    pub selection_inactive: Color32,
}

/// Spacing + corner-rounding scale. Replaces scattered magic numbers.
pub struct Spacing {
    pub xs: f32,        // 2
    pub sm: f32,        // 4
    pub md: f32,        // 8
    pub lg: f32,        // 12
    pub round_sm: f32,  // 4
    pub round_md: f32,  // 6
    pub round_lg: f32,  // 8
}
```

### Derivation

Pure helpers (unit-testable):

- `mix(a, b, t) -> Color32` — per-channel linear interpolation.
- `luminance(c) -> f32` — perceptual `(0.299·r + 0.587·g + 0.114·b) / 255`.
- `is_dark(bg) = luminance(bg) < 0.5`; `toward = if is_dark { WHITE } else { BLACK }`.
- `on(c) = if luminance(c) > 0.55 { near-black } else { WHITE }` — chooses readable text over a fill.

Default mapping (verified against the **Dark** preset `bg=[30,30,30]`, `fg=[212,212,212]`, `accent=[0,122,204]`, so the look stays recognisable):

| Token | Formula | Dark value | Note |
|-------|---------|-----------|------|
| `surface` | `mix(bg, toward, 0.04)` | ≈ 39 | current `+7` → 37 |
| `surface_raised` | `mix(bg, toward, 0.08)` | ≈ 48 | current autocomplete ≈ 45 |
| `border` | `mix(bg, toward, 0.14)` | ≈ 61 | |
| `border_strong` | `mix(bg, toward, 0.25)` | ≈ 86 | |
| `text_muted` | `mix(fg, bg, 0.40)` | ≈ 139 | brighter than current `fg/2`=106, more legible |
| `text_faint` | `mix(fg, bg, 0.65)` | ≈ 89 | |
| `accent_hover` | `mix(accent, toward, 0.15)` | — | |
| `accent_muted` | `accent` @ alpha 48 | — | subtle fills |
| `on_accent` | `on(accent)` | WHITE | Monokai accent → black, fixes status bar |
| `line_highlight` | `mix(bg, toward, 0.06)` | ≈ 43 | subtle |
| `selection` | `accent` @ alpha 95 | — | up from 60–80 |
| `selection_inactive` | `accent` @ alpha 45 | — | |

Semantic colours are fixed, contrast-tuned hues (kept simple this pass): `error=[229,83,75]`, `warning=[229,181,67]`, `info=[86,156,214]`, `success=[80,200,120]`, `hint = text_muted`. Git colours centralise the current values: `git_added=[80,200,80]`, `git_modified=[80,150,255]`, `git_removed=[220,80,80]`.

### egui Visuals mapping

`apply_theme(ctx: &Context, config: &Config) -> (Palette, Spacing)`:

- base = `Visuals::light()` if the seed bg is light, else `Visuals::dark()`.
- `panel_fill = surface`, `window_fill = surface_raised`, `extreme_bg_color = bg` (text-edit backgrounds), `override_text_color = text`.
- `selection.bg_fill = selection`, `selection.stroke = (1.0, accent)`.
- widget `inactive/hovered/active.weak_bg_fill` from `surface`/`border`/`border_strong`.
- `window_stroke = (1.0, border)`, `window_rounding = round_lg`, `menu_rounding = round_md`.
- Configure `ctx.style().text_styles`: `Body=13`, `Small=12`, `Button=13`, `Heading=16` (decoupled from editor font; `Monospace` left to the editor's own font sizing).

### Lifecycle & boundaries

- `Palette` + `Spacing` are stored on the app and **rebuilt only when the theme changes**. Settings already persists on every theme change (`ui/settings.rs`); that path also calls `apply_theme` and stores the result. Rebuild cost is ~30 colour mixes — negligible against the 30–80 MB / perf target.
- `config/mod.rs` — `Theme` seed, unchanged.
- `ui/theme.rs` (new) — derivation + `Palette` + `Spacing` + `apply_theme`. One responsibility.
- All other renderers **read** `&Palette` / `&Spacing`; no renderer derives colours locally anymore.

---

## Layer 2 — High-visibility Chrome

### Goal
Rewire the most-seen surfaces onto `Palette`; unify every floating element's material.

### Status bar (`ui/statusbar.rs`)
- Fill = `palette.accent` (replaces hard-coded `rgb(0,122,204)`).
- Text = `palette.on_accent` — automatically legible on any accent (today `Color32::WHITE` is forced and becomes unreadable on light accents like Monokai).
- LSP dots keep their meaning but use `palette.warning/success/error`.
- UI type style instead of raw `.small()`.
- **Optional:** a diagnostics counter `⛌ N  ⚠ M` left of `Ln/Col` (small discoverability win).

### Unified popups — one shared `popup_frame`
New `src/ui/widgets.rs` holds reusable building blocks. `popup_frame(&palette, &spacing) -> egui::Frame` = fill `surface_raised`, stroke `border`, rounding `round_md`, a light egui shadow. Applied to:

- **Autocomplete** (`editor/autocomplete.rs`) — replaces `rgb(40,44,52)` / border `rgb(80,80,120)` / selected `rgb(30,80,140)` with `surface_raised` / `border` / `accent_muted`. **Bonus:** highlight the fuzzy-matched characters in each label (the matcher already returns match indices) so the user sees *why* a suggestion ranked.
- **Hover & signature help** (`editor/mod.rs`) — same frame, `surface_raised`.
- **Find & Replace + Go to Line** (`editor/mod.rs`) — active toggles use `accent` (not the local blue); matches highlighted via tokens.
- **Command palette** (`ui/palette.rs`) — same frame; selected item `accent_muted`.

Result: all floats share fill / border / rounding / shadow → instant coherence.

---

## Layer 3 — Editor Readability

All in `editor/mod.rs`, reading `Palette`.

- **Current-line highlight** — a `line_highlight` rect behind the cursor line; togglable (new `config.editor.highlight_current_line`, default on). One rect per cursor in multi-cursor mode. **Suppressed while a selection is active** to avoid noise. egui rect → no measurable cost.
- **Active line number** — current line's number in `text` (bright); others in `text_muted`. Today all use `fg/2`.
- **Per-severity diagnostics** — coloured squiggle: `error` red, `warning` yellow, `info`/`hint` blue/grey (today there is no visible distinction). The gutter marker (currently the yellow lightbulb `255,220,50`) becomes severity-coloured.
- **Active indent guide** — the guide for the cursor's enclosing block is one step more visible (`accent_muted`) than the rest. The recent `-2px` nudge and faint stroke for the others are preserved.
- **Selection legibility** — alpha 60 → ~95 via `palette.selection`. Word-occurrence highlight uses `accent_muted` instead of the near-invisible alpha-15.
- **Breadcrumbs** (announced in the README, currently absent) — a thin bar at the top of the editor (between tab bar and text), fill `surface`, showing `workspace-relative path › enclosing symbol`. New `src/ui/breadcrumbs.rs`. Symbol comes from LSP document symbols (already requested via `request_document_symbols`) resolved against the cursor line; path is split on separators. **Display first**; click-to-navigate is a follow-up. This is the largest piece of the layer.

---

## Layer 4 — Ergonomics & Discoverability

- **Delete confirmation** — file/folder deletion in the tree currently happens immediately (data-loss risk). Add `pending_delete: Option<PathBuf>` and a small confirm modal (a `confirm_dialog` helper in `ui/widgets.rs`, using the `overlay` scrim). Delete only on confirm.
- **Command palette enrichment** (`ui/palette.rs`) — each command gains a `description` and optional `shortcut`. The fuzzy matcher indexes label **and** description (so "toggle Claude" is reachable by typing "assistant" if the description says so). The bound shortcut is rendered right-aligned in `text_muted` (learn shortcuts by using the palette). Expose more of the existing menu/keybinding actions as palette commands.
- **Transient feedback (toasts)** — a small bottom bar (`surface_raised`) for "Formatted", "Saved", "LSP restarted", etc. App holds `Vec<Toast { message, born: Instant }>`; auto-dismiss after ~2 s with an alpha-only fade (no expensive animation). A `self.toast(msg)` helper fires them from action sites that are currently silent. Lives in `ui/widgets.rs`.
- **Light preset** — now that derivation supports light backgrounds, add a **Light** preset to Settings (e.g. `bg=[246,246,246]`, `fg=[40,40,40]`, `accent=[0,103,184]`). Near-free given Layer 1.

---

## Cross-cutting Constraints

- **Lightweight / performance.** No heavy animation; the only motion is toast alpha fade. `Palette` recomputes only on theme change. No new background threads. Respects the 30–80 MB target.
- **Preserve recent hand-tuning.** Indent-guide geometry, syntax-token colours (`highlight.rs`), and existing editor behaviour are not disturbed.
- **egui 0.31 idioms** (matches the pinned dependency).
- **3-colour user-facing model preserved** — settings UI and config format are unchanged except for the added Light preset and the `highlight_current_line` flag.

## Out of Scope (deferred)

Explicitly **not** in this pass, to keep it focused: per-file minimap auto-hide; Ctrl+wheel / session zoom; side-by-side diff view; go-to-line history; settings-search fuzzy / "did you mean"; configurable tokenization thresholds; find-bar resize; theme preview-before-apply; click-to-navigate breadcrumbs (display only here); folding syntax colours into `Palette`.

## Testing

- **Pure derivation is unit-tested** in `ui/theme.rs`: `mix`, `luminance`, `is_dark`, `on` (assert `on_accent` flips to black for the Monokai accent and white for the Dark accent), and a few snapshot assertions on derived values for each preset to catch regressions.
- **Breadcrumb resolution is unit-tested**: given a symbol list + cursor line, the correct enclosing symbol is returned.
- **Visual behaviour** (current line, popup material, severity colours, status-bar contrast across all presets including Light) is verified by running the app and screenshotting during implementation — egui rendering is not meaningfully unit-testable.

## File Change Map

| File | Change |
|------|--------|
| `src/ui/theme.rs` | **New.** `Palette`, `Spacing`, derivation helpers, `apply_theme`; `dark_visuals` moves here. |
| `src/ui/widgets.rs` | **New.** `popup_frame`, `confirm_dialog`, toast rendering. |
| `src/ui/breadcrumbs.rs` | **New.** Breadcrumb bar + symbol resolution. |
| `src/ui/layout.rs` | Remove `dark_visuals`; call `apply_theme`; render breadcrumbs; thread `&Palette`/`&Spacing`. |
| `src/ui/statusbar.rs` | Accent fill + `on_accent` text; semantic LSP dots; optional diagnostics counter. |
| `src/ui/palette.rs` | `popup_frame`; command `description`/`shortcut`; index descriptions; render shortcuts; more commands. |
| `src/ui/settings.rs` | Add **Light** preset; `highlight_current_line` toggle. |
| `src/editor/mod.rs` | Current-line highlight; active line number; per-severity diagnostics; active indent guide; selection alpha; popups via `popup_frame`; centralised git/diagnostic colours via `Palette`. |
| `src/editor/autocomplete.rs` | `popup_frame`; fuzzy-match character highlight. |
| `src/config/mod.rs` | `editor.highlight_current_line: bool` (default true). `Theme` seed unchanged. |
| `src/app/mod.rs` (+ `app/*`) | Store `palette`/`spacing`; rebuild on theme change; `pending_delete` + confirm flow; `toast()` helper + toast state. |
```
