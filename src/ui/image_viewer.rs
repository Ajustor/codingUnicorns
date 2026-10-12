//! Image viewer: zoom (wheel, buttons, keys), pan (drag), rotate / flip (view
//! only, the file is never written), a choice of backdrop for transparent
//! images and a pixel inspector under the pointer.

use crate::app::ImageData;
use crate::ui::theme::Palette;
use egui::{Color32, Pos2, Rect, Vec2};
use egui_phosphor::regular as ph;

pub const MIN_ZOOM: f32 = 0.01;
pub const MAX_ZOOM: f32 = 64.0;
/// Zoom factor of one button press / key press.
const ZOOM_STEP: f32 = 1.25;
/// Side of a transparency checkerboard square, in points.
const CHECKER_CELL: f32 = 8.0;

/// What is drawn behind the image (visible through transparent pixels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backdrop {
    #[default]
    Checker,
    Dark,
    Light,
}

impl Backdrop {
    fn next(self) -> Self {
        match self {
            Self::Checker => Self::Dark,
            Self::Dark => Self::Light,
            Self::Light => Self::Checker,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Checker => "Checkerboard",
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }
}

/// How the current image is shown. Reset (except the backdrop) when another
/// image is opened.
#[derive(Default)]
pub struct ImageView {
    /// Points per image pixel; `None` fits the image in the view (never
    /// enlarging it past 100%).
    zoom: Option<f32>,
    /// Offset of the image centre from the view centre, in points.
    pan: Vec2,
    /// Clockwise quarter turns (0..4).
    quarter_turns: u8,
    flip_h: bool,
    flip_v: bool,
    backdrop: Backdrop,
    /// 2×2 checkerboard, repeated behind transparent images.
    checker: Option<egui::TextureHandle>,
}

/// Largest zoom showing all of an image of `size` (rotated) in `view`, at most 1.
pub fn fit_zoom(size: Vec2, view: Vec2) -> f32 {
    if size.x <= 0.0 || size.y <= 0.0 {
        return 1.0;
    }
    (view.x / size.x).min(view.y / size.y).clamp(MIN_ZOOM, 1.0)
}

impl ImageView {
    /// Back to "fit, upright" for a newly opened image; the backdrop is kept.
    pub fn reset(&mut self) {
        *self = Self {
            backdrop: self.backdrop,
            checker: self.checker.take(),
            ..Self::default()
        };
    }

    /// Image size once rotated (width and height swap on odd quarter turns).
    pub fn rotated_size(&self, size: Vec2) -> Vec2 {
        if self.quarter_turns % 2 == 1 {
            Vec2::new(size.y, size.x)
        } else {
            size
        }
    }

    /// The zoom in effect for an image of `size` in a view of `view`.
    pub fn effective_zoom(&self, size: Vec2, view: Vec2) -> f32 {
        self.zoom
            .unwrap_or_else(|| fit_zoom(self.rotated_size(size), view))
    }

    /// Image-space offset (from the image centre, in pixels) → view-space
    /// offset (from the image centre, in pixels): flip, then rotate.
    pub fn image_to_view(&self, p: Vec2) -> Vec2 {
        let mut p = Vec2::new(
            if self.flip_h { -p.x } else { p.x },
            if self.flip_v { -p.y } else { p.y },
        );
        for _ in 0..self.quarter_turns % 4 {
            p = Vec2::new(-p.y, p.x); // 90° clockwise (y points down)
        }
        p
    }

    /// Inverse of [`Self::image_to_view`].
    pub fn view_to_image(&self, p: Vec2) -> Vec2 {
        let mut p = p;
        for _ in 0..self.quarter_turns % 4 {
            p = Vec2::new(p.y, -p.x);
        }
        Vec2::new(
            if self.flip_h { -p.x } else { p.x },
            if self.flip_v { -p.y } else { p.y },
        )
    }

    /// The image pixel under `offset` (points from the image centre on
    /// screen), or `None` outside the image.
    pub fn pixel_at(&self, size: Vec2, zoom: f32, offset: Vec2) -> Option<(u32, u32)> {
        let p = self.view_to_image(offset / zoom) + size / 2.0;
        (p.x >= 0.0 && p.y >= 0.0 && p.x < size.x && p.y < size.y)
            .then_some((p.x as u32, p.y as u32))
    }

    /// Zoom by `factor` keeping the point `anchor` (points from the view
    /// centre) fixed on screen.
    pub fn zoom_by(&mut self, factor: f32, anchor: Vec2, current: f32) {
        let new = (current * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let ratio = new / current;
        self.pan = anchor - (anchor - self.pan) * ratio;
        self.zoom = Some(new);
    }

    fn set_fit(&mut self) {
        self.zoom = None;
        self.pan = Vec2::ZERO;
    }

    fn set_actual_size(&mut self) {
        self.zoom = Some(1.0);
        self.pan = Vec2::ZERO;
    }

    fn rotate(&mut self, clockwise: bool) {
        self.quarter_turns = (self.quarter_turns + if clockwise { 1 } else { 3 }) % 4;
        self.pan = Vec2::ZERO;
    }

    /// Keep at least `margin` points of the image inside the view.
    fn clamp_pan(&mut self, shown: Vec2, view: Vec2) {
        let margin = 48.0;
        let max = ((shown + view) / 2.0 - Vec2::splat(margin)).max(Vec2::ZERO);
        self.pan = self.pan.clamp(-max, max);
    }
}

/// Toolbar + canvas + status line for `texture` (pixels in `data`).
pub fn show(
    ui: &mut egui::Ui,
    view: &mut ImageView,
    texture: &egui::TextureHandle,
    data: &ImageData,
    palette: Palette,
) {
    let size = Vec2::new(data.width as f32, data.height as f32);

    toolbar(ui, view, data, size, palette);

    let status_h = ui.text_style_height(&egui::TextStyle::Small) + 8.0;
    let canvas_size = (ui.available_size() - Vec2::new(0.0, status_h)).max(Vec2::splat(1.0));
    let (canvas, resp) = ui.allocate_exact_size(canvas_size, egui::Sense::click_and_drag());
    let view_size = canvas.size();
    let mut zoom = view.effective_zoom(size, view_size);

    // ── Interaction ──────────────────────────────────────────────────────────
    let pointer = resp.hover_pos();
    if let Some(pos) = pointer {
        let anchor = pos - canvas.center();
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = (scroll * 0.004).exp() * pinch;
        if (factor - 1.0).abs() > f32::EPSILON {
            view.zoom_by(factor, anchor, zoom);
        }
        if !ui.ctx().wants_keyboard_input() {
            let (zin, zout, fit, actual) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals),
                    i.key_pressed(egui::Key::Minus),
                    i.key_pressed(egui::Key::Num0),
                    i.key_pressed(egui::Key::Num1),
                )
            });
            if zin {
                view.zoom_by(ZOOM_STEP, Vec2::ZERO, zoom);
            }
            if zout {
                view.zoom_by(1.0 / ZOOM_STEP, Vec2::ZERO, zoom);
            }
            if fit {
                view.set_fit();
            }
            if actual {
                view.set_actual_size();
            }
        }
    }
    if resp.double_clicked() {
        if view.zoom.is_none() {
            // Fit → 100% around the clicked point.
            let anchor = pointer.map_or(Vec2::ZERO, |p| p - canvas.center());
            view.zoom_by(1.0 / zoom, anchor, zoom);
        } else {
            view.set_fit();
        }
    }
    if resp.dragged() {
        if view.zoom.is_none() {
            view.zoom = Some(zoom);
        }
        view.pan += resp.drag_delta();
    }
    if resp.hovered() {
        let icon = if resp.dragged() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        };
        ui.ctx().set_cursor_icon(icon);
    }
    zoom = view.effective_zoom(size, view_size);
    let shown = view.rotated_size(size) * zoom;
    if view.zoom.is_some() {
        view.clamp_pan(shown, view_size);
    }

    // ── Painting ─────────────────────────────────────────────────────────────
    let painter = ui.painter_at(canvas);
    painter.rect_filled(canvas, 0.0, palette.bg);
    let center = canvas.center() + view.pan;
    let image_rect = Rect::from_center_size(center, shown);
    paint_backdrop(ui, &painter, view, image_rect);

    let mut mesh = egui::Mesh::with_texture(texture.id());
    for uv in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
        let local = Vec2::new((uv.0 - 0.5) * size.x, (uv.1 - 0.5) * size.y);
        let pos = center + view.image_to_view(local) * zoom;
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv: Pos2::new(uv.0, uv.1),
            color: Color32::WHITE,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));

    // ── Status line: hovered pixel ───────────────────────────────────────────
    let hovered = pointer.and_then(|p| view.pixel_at(size, zoom, p - center));
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.set_min_height(status_h);
        let small = |s: String| egui::RichText::new(s).small().color(palette.text_muted);
        match hovered {
            Some((x, y)) => {
                let i = ((y * data.width + x) * 4) as usize;
                let [r, g, b, a] = [
                    data.pixels[i],
                    data.pixels[i + 1],
                    data.pixels[i + 2],
                    data.pixels[i + 3],
                ];
                ui.label(small(format!("{x}, {y}")));
                let (swatch, _) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(swatch, 2.0, Color32::from_rgba_unmultiplied(r, g, b, a));
                ui.painter().rect_stroke(
                    swatch,
                    2.0,
                    egui::Stroke::new(1.0_f32, palette.border_strong),
                    egui::StrokeKind::Inside,
                );
                let hex = format!("#{r:02X}{g:02X}{b:02X}{a:02X}");
                let text = ui
                    .label(small(format!("{hex}   rgba({r}, {g}, {b}, {a})")))
                    .on_hover_text("Click to copy the hex colour");
                if text.clicked() {
                    ui.ctx().copy_text(hex);
                }
            }
            None => {
                ui.label(
                    egui::RichText::new("Scroll to zoom · drag to pan · double-click: fit / 100%")
                        .small()
                        .color(palette.text_faint),
                );
            }
        }
    });
}

fn toolbar(
    ui: &mut egui::Ui,
    view: &mut ImageView,
    data: &ImageData,
    size: Vec2,
    palette: Palette,
) {
    // The canvas size isn't known yet; the fit zoom shown is for the space below.
    let approx_view = ui.available_size() - Vec2::new(0.0, 56.0);
    let zoom = view.effective_zoom(size, approx_view.max(Vec2::splat(1.0)));
    egui::Frame::new()
        .fill(palette.surface)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let button = |ui: &mut egui::Ui, icon: &str, tip: &str, on: bool| {
                    ui.add(egui::Button::new(icon).frame(false).selected(on))
                        .on_hover_text(tip)
                        .clicked()
                };
                if button(ui, ph::MAGNIFYING_GLASS_MINUS, "Zoom out (-)", false) {
                    view.zoom_by(1.0 / ZOOM_STEP, Vec2::ZERO, zoom);
                }
                ui.add_sized(
                    [48.0, ui.spacing().interact_size.y],
                    egui::Label::new(
                        egui::RichText::new(format!("{:.0}%", zoom * 100.0))
                            .small()
                            .color(palette.text_muted),
                    ),
                );
                if button(ui, ph::MAGNIFYING_GLASS_PLUS, "Zoom in (+)", false) {
                    view.zoom_by(ZOOM_STEP, Vec2::ZERO, zoom);
                }
                ui.separator();
                if button(ui, ph::ARROWS_IN, "Fit to view (0)", view.zoom.is_none()) {
                    view.set_fit();
                }
                if button(
                    ui,
                    ph::FRAME_CORNERS,
                    "Actual size (1)",
                    view.zoom == Some(1.0),
                ) {
                    view.set_actual_size();
                }
                ui.separator();
                if button(ui, ph::ARROW_COUNTER_CLOCKWISE, "Rotate left", false) {
                    view.rotate(false);
                }
                if button(ui, ph::ARROW_CLOCKWISE, "Rotate right", false) {
                    view.rotate(true);
                }
                if button(ui, ph::FLIP_HORIZONTAL, "Flip horizontally", view.flip_h) {
                    view.flip_h = !view.flip_h;
                }
                if button(ui, ph::FLIP_VERTICAL, "Flip vertically", view.flip_v) {
                    view.flip_v = !view.flip_v;
                }
                ui.separator();
                let tip = format!("Background: {} (click to change)", view.backdrop.label());
                if button(ui, ph::CHECKERBOARD, &tip, false) {
                    view.backdrop = view.backdrop.next();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut info = format!("{} × {}", data.width, data.height);
                    if let Some(bytes) = data.file_size {
                        info.push_str(&format!("  ·  {}", format_size(bytes)));
                    }
                    if !data.format.is_empty() {
                        info.push_str(&format!("  ·  {}", data.format));
                    }
                    ui.label(egui::RichText::new(info).small().color(palette.text_muted));
                });
            });
        });
}

fn paint_backdrop(ui: &egui::Ui, painter: &egui::Painter, view: &mut ImageView, rect: Rect) {
    match view.backdrop {
        Backdrop::Dark => {
            painter.rect_filled(rect, 0.0, Color32::from_gray(24));
        }
        Backdrop::Light => {
            painter.rect_filled(rect, 0.0, Color32::from_gray(240));
        }
        Backdrop::Checker => {
            let tex = view.checker.get_or_insert_with(|| {
                let (a, b) = (Color32::from_gray(204), Color32::from_gray(153));
                let img = egui::ColorImage {
                    size: [2, 2],
                    pixels: vec![a, b, b, a],
                };
                ui.ctx().load_texture(
                    "image_viewer_checker",
                    img,
                    egui::TextureOptions::NEAREST_REPEAT,
                )
            });
            // Only the visible part: the image can be far larger than the view.
            let visible = rect.intersect(painter.clip_rect());
            if visible.is_positive() {
                let cells = 2.0 * CHECKER_CELL;
                let uv = Rect::from_min_max(
                    ((visible.min - rect.min) / cells).to_pos2(),
                    ((visible.max - rect.min) / cells).to_pos2(),
                );
                painter.image(tex.id(), visible, uv, Color32::WHITE);
            }
        }
    }
}

/// "1.2 MB" style size.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Vec2 = Vec2::new(40.0, 20.0);

    fn view(turns: u8, flip_h: bool, flip_v: bool) -> ImageView {
        ImageView {
            quarter_turns: turns,
            flip_h,
            flip_v,
            ..Default::default()
        }
    }

    #[test]
    fn fit_zoom_shrinks_but_never_enlarges() {
        assert_eq!(
            fit_zoom(Vec2::new(200.0, 100.0), Vec2::new(100.0, 100.0)),
            0.5
        );
        assert_eq!(
            fit_zoom(Vec2::new(10.0, 10.0), Vec2::new(500.0, 500.0)),
            1.0
        );
        assert_eq!(fit_zoom(Vec2::ZERO, Vec2::new(10.0, 10.0)), 1.0);
    }

    #[test]
    fn rotation_swaps_the_fitted_size() {
        let v = view(1, false, false);
        assert_eq!(v.rotated_size(SIZE), Vec2::new(20.0, 40.0));
        // 20×40 in a 100×20 view: height limits the zoom.
        assert_eq!(v.effective_zoom(SIZE, Vec2::new(100.0, 20.0)), 0.5);
    }

    #[test]
    fn to_view_and_view_to_image_are_inverse_for_every_orientation() {
        let p = Vec2::new(3.0, -7.0);
        for turns in 0..4 {
            for (h, v) in [(false, false), (true, false), (false, true), (true, true)] {
                let iv = view(turns, h, v);
                assert_eq!(
                    iv.view_to_image(iv.image_to_view(p)),
                    p,
                    "turns={turns} h={h} v={v}"
                );
            }
        }
    }

    #[test]
    fn quarter_turn_is_clockwise() {
        // The image's top-left corner ends up top-right after one turn.
        let tl = Vec2::new(-SIZE.x / 2.0, -SIZE.y / 2.0);
        assert_eq!(
            view(1, false, false).image_to_view(tl),
            Vec2::new(10.0, -20.0)
        );
    }

    #[test]
    fn pixel_at_maps_screen_offsets_to_pixels() {
        let v = view(0, false, false);
        // Centre of the image, at 2× zoom.
        assert_eq!(v.pixel_at(SIZE, 2.0, Vec2::ZERO), Some((20, 10)));
        // Top-left pixel.
        assert_eq!(v.pixel_at(SIZE, 2.0, Vec2::new(-39.0, -19.0)), Some((0, 0)));
        // Outside.
        assert_eq!(v.pixel_at(SIZE, 2.0, Vec2::new(-41.0, 0.0)), None);
        assert_eq!(v.pixel_at(SIZE, 1.0, Vec2::new(20.0, 0.0)), None);

        // Flipped horizontally: the screen's left edge shows the last column.
        let f = view(0, true, false);
        assert_eq!(f.pixel_at(SIZE, 1.0, Vec2::new(-19.5, 0.0)), Some((39, 10)));

        // One clockwise turn: the screen's top-right shows pixel (0, 0).
        let r = view(1, false, false);
        assert_eq!(r.pixel_at(SIZE, 1.0, Vec2::new(9.5, -19.5)), Some((0, 0)));
    }

    #[test]
    fn zoom_by_keeps_the_anchor_fixed_and_clamps() {
        let mut v = ImageView::default();
        let anchor = Vec2::new(30.0, -10.0);
        // The image point under the anchor before zooming…
        let before = v.view_to_image((anchor - v.pan) / 1.0);
        v.zoom_by(2.0, anchor, 1.0);
        assert_eq!(v.zoom, Some(2.0));
        // …is still under it after.
        assert_eq!(v.view_to_image((anchor - v.pan) / 2.0), before);

        v.zoom_by(1e9, Vec2::ZERO, 2.0);
        assert_eq!(v.zoom, Some(MAX_ZOOM));
        v.zoom_by(1e-9, Vec2::ZERO, MAX_ZOOM);
        assert_eq!(v.zoom, Some(MIN_ZOOM));
    }

    #[test]
    fn reset_keeps_only_the_backdrop() {
        let mut v = view(3, true, true);
        v.zoom = Some(4.0);
        v.pan = Vec2::new(5.0, 5.0);
        v.backdrop = Backdrop::Light;
        v.reset();
        assert_eq!(v.zoom, None);
        assert_eq!(v.pan, Vec2::ZERO);
        assert_eq!((v.quarter_turns, v.flip_h, v.flip_v), (0, false, false));
        assert_eq!(v.backdrop, Backdrop::Light);
    }

    #[test]
    fn clamp_pan_keeps_part_of_the_image_visible() {
        let mut v = ImageView {
            pan: Vec2::new(10_000.0, -10_000.0),
            ..Default::default()
        };
        v.clamp_pan(Vec2::new(100.0, 100.0), Vec2::new(200.0, 200.0));
        assert_eq!(v.pan, Vec2::new(102.0, -102.0));
    }

    #[test]
    fn format_size_uses_binary_units() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MB");
    }
}
