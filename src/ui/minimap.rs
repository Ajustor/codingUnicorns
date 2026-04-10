use crate::lsp::client::{DiagSeverity, Diagnostic};

const MINIMAP_WIDTH: f32 = 80.0;
const LINE_PX: f32 = 1.5; // height per line in minimap (pixels)

/// Pre-computed per-line data for the minimap code structure view.
pub struct LineShape {
    /// Indentation in characters.
    pub indent: usize,
    /// Content length (non-whitespace) in characters.
    pub content_len: usize,
    /// True if the line is blank.
    pub blank: bool,
}

pub struct MinimapData<'a> {
    pub total_lines: usize,
    pub first_visible: usize,
    pub visible_count: usize,
    pub diagnostics: &'a [Diagnostic],
    pub line_diff: &'a [u8],
    pub find_matches: &'a [usize],
    pub cursor_row: usize,
    pub extra_cursor_rows: Vec<usize>,
    pub line_height: f32,
    pub bg_color: egui::Color32,
    pub accent_color: egui::Color32,
    /// Per-line shape data for the code structure rendering.
    pub lines: &'a [LineShape],
    /// Fold regions: `(start_line, end_line)` pairs.
    pub fold_regions: &'a [(usize, usize)],
}

/// Renders the minimap overlay on the right side of the given rect.
/// Returns the new scroll offset Y if the user clicked/dragged the minimap.
pub fn render(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    editor_rect: egui::Rect,
    data: &MinimapData<'_>,
) -> Option<f32> {
    if data.total_lines == 0 {
        return None;
    }

    let minimap_rect = egui::Rect::from_min_size(
        egui::pos2(editor_rect.max.x - MINIMAP_WIDTH, editor_rect.min.y),
        egui::vec2(MINIMAP_WIDTH, editor_rect.height()),
    );

    // ── Hover detection & opacity animation ──────────────────────────────
    let hover_id = ui.id().with("minimap_hover");
    let hovered = ui
        .ctx()
        .input(|i| i.pointer.hover_pos().is_some_and(|p| minimap_rect.contains(p)));
    let target_alpha = if hovered { 0.75 } else { 0.18 };
    let alpha = ui
        .ctx()
        .animate_value_with_time(hover_id, target_alpha, 0.15);

    let line_height = data.line_height;
    let bg_color = data.bg_color;
    let accent_color = data.accent_color;

    // ── Background ───────────────────────────────────────────────────────
    let bg = egui::Color32::from_rgba_unmultiplied(
        bg_color.r().saturating_sub(8),
        bg_color.g().saturating_sub(8),
        bg_color.b().saturating_sub(8),
        (alpha * 255.0) as u8,
    );
    painter.rect_filled(minimap_rect, 0.0, bg);

    // ── Scale ────────────────────────────────────────────────────────────
    // Use fixed LINE_PX per line, but clamp so the map fits in the rect.
    let natural_h = data.total_lines as f32 * LINE_PX;
    let scale = if natural_h > minimap_rect.height() {
        minimap_rect.height() / data.total_lines as f32
    } else {
        LINE_PX
    };
    let line_h = scale.max(1.0);

    let content_x = minimap_rect.min.x + 6.0;
    let content_w = minimap_rect.width() - 12.0;
    let max_chars = 120.0_f32; // normalize line width to ~120 chars
    let char_px = content_w / max_chars;

    // ── Fold regions (background bands showing code blocks) ────────────
    // Sort by size (largest first) so smaller nested regions draw on top.
    let mut sorted_regions: Vec<(usize, usize)> = data.fold_regions.to_vec();
    sorted_regions.sort_by(|a, b| (b.1 - b.0).cmp(&(a.1 - a.0)));

    // Compute nesting depth per region for color variation.
    for &(start, end) in &sorted_regions {
        let span = end.saturating_sub(start);
        if span < 3 {
            continue;
        }
        // Nesting depth: count how many other regions fully contain this one
        let depth = sorted_regions
            .iter()
            .filter(|&&(os, oe)| os < start && oe > end)
            .count();
        let depth_clamped = depth.min(5);

        let y_start = minimap_rect.min.y + start as f32 * scale;
        let y_end = minimap_rect.min.y + (end + 1) as f32 * scale;
        if y_end < minimap_rect.min.y || y_start > minimap_rect.max.y {
            continue;
        }

        // Alternate colors by depth for visual distinction
        let base_a = (alpha * 18.0).min(30.0) as u8;
        let region_color = match depth_clamped % 4 {
            0 => egui::Color32::from_rgba_unmultiplied(80, 140, 220, base_a),  // blue
            1 => egui::Color32::from_rgba_unmultiplied(160, 120, 200, base_a), // purple
            2 => egui::Color32::from_rgba_unmultiplied(80, 180, 160, base_a),  // teal
            _ => egui::Color32::from_rgba_unmultiplied(180, 160, 80, base_a),  // gold
        };

        // Left indent bar (thin vertical line marking the block boundary)
        let indent_level = data
            .lines
            .get(start)
            .map(|l| l.indent)
            .unwrap_or(0);
        let bar_x_pos = content_x + indent_level as f32 * char_px;

        // Background band
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(bar_x_pos, y_start),
                egui::pos2(content_x + content_w, y_end.min(minimap_rect.max.y)),
            ),
            0.0,
            region_color,
        );

        // Left edge line (slightly brighter to show the block boundary)
        let edge_a = (alpha * 60.0).min(80.0) as u8;
        let edge_color = match depth_clamped % 4 {
            0 => egui::Color32::from_rgba_unmultiplied(80, 140, 220, edge_a),
            1 => egui::Color32::from_rgba_unmultiplied(160, 120, 200, edge_a),
            2 => egui::Color32::from_rgba_unmultiplied(80, 180, 160, edge_a),
            _ => egui::Color32::from_rgba_unmultiplied(180, 160, 80, edge_a),
        };
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(bar_x_pos, y_start),
                egui::pos2(bar_x_pos + 1.0, y_end.min(minimap_rect.max.y)),
            ),
            0.0,
            edge_color,
        );
    }

    // ── Code structure (per-line rendering) ──────────────────────────────
    let code_alpha = (alpha * 1.4).min(1.0);
    let code_color = egui::Color32::from_rgba_unmultiplied(
        bg_color.r().saturating_add(60),
        bg_color.g().saturating_add(60),
        bg_color.b().saturating_add(60),
        (code_alpha * 200.0) as u8,
    );

    for (i, line) in data.lines.iter().enumerate() {
        if line.blank {
            continue;
        }
        let y = minimap_rect.min.y + i as f32 * scale;
        if y + line_h < minimap_rect.min.y || y > minimap_rect.max.y {
            continue;
        }
        let x_offset = line.indent as f32 * char_px;
        let w = (line.content_len as f32 * char_px).min(content_w - x_offset).max(2.0);
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(content_x + x_offset, y),
                egui::vec2(w, line_h),
            ),
            0.0,
            code_color,
        );
    }

    // ── Region markers (drawn on top of code structure) ──────────────────
    let marker_alpha = (alpha * 2.2).min(1.0);
    let marker_h = line_h.max(2.0);

    let draw_marker = |line: usize, color: egui::Color32, side: bool| {
        let y = minimap_rect.min.y + line as f32 * scale;
        if y < minimap_rect.min.y || y > minimap_rect.max.y {
            return;
        }
        let c = egui::Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            (marker_alpha * color.a() as f32) as u8,
        );
        if side {
            // Thin bar on the right edge (diagnostics, search)
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(minimap_rect.max.x - 4.0, y),
                    egui::vec2(3.0, marker_h),
                ),
                0.0,
                c,
            );
        } else {
            // Full-width subtle highlight
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(content_x, y),
                    egui::vec2(content_w, marker_h),
                ),
                0.0,
                c,
            );
        }
    };

    // Git diff: thin bar on left edge
    for (line, &status) in data.line_diff.iter().enumerate() {
        let color = match status {
            1 => egui::Color32::from_rgb(80, 200, 80),  // added
            2 => egui::Color32::from_rgb(80, 160, 230), // modified
            _ => continue,
        };
        let y = minimap_rect.min.y + line as f32 * scale;
        if y >= minimap_rect.min.y && y <= minimap_rect.max.y {
            let c = egui::Color32::from_rgba_unmultiplied(
                color.r(), color.g(), color.b(), (marker_alpha * 255.0) as u8,
            );
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(minimap_rect.min.x + 1.0, y),
                    egui::vec2(3.0, marker_h),
                ),
                0.0,
                c,
            );
        }
    }

    // Diagnostics: right edge markers
    for diag in data.diagnostics {
        let color = match diag.severity {
            DiagSeverity::Error => egui::Color32::from_rgb(240, 60, 60),
            DiagSeverity::Warning => egui::Color32::from_rgb(230, 180, 40),
            DiagSeverity::Info | DiagSeverity::Hint => egui::Color32::from_rgb(70, 170, 220),
        };
        draw_marker(diag.line as usize, color, true);
    }

    // Find matches: right edge markers
    for &line in data.find_matches {
        draw_marker(line, egui::Color32::from_rgb(230, 160, 40), true);
    }

    // Cursor lines: subtle full-width highlight
    draw_marker(data.cursor_row, accent_color, false);
    for &row in &data.extra_cursor_rows {
        draw_marker(row, accent_color, false);
    }

    // ── Viewport rectangle ───────────────────────────────────────────────
    let vp_y = minimap_rect.min.y + data.first_visible as f32 * scale;
    let vp_h = (data.visible_count as f32 * scale).max(6.0);
    let vp_rect = egui::Rect::from_min_size(
        egui::pos2(minimap_rect.min.x, vp_y),
        egui::vec2(minimap_rect.width(), vp_h),
    );
    painter.rect_filled(
        vp_rect,
        1.0,
        egui::Color32::from_rgba_unmultiplied(180, 180, 180, (alpha * 50.0) as u8),
    );
    painter.rect_stroke(
        vp_rect,
        1.0,
        egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(180, 180, 180, (alpha * 100.0) as u8),
        ),
        egui::StrokeKind::Inside,
    );

    // ── Click / drag to scroll ───────────────────────────────────────────
    let mut new_scroll = None;
    let resp = ui.interact(
        minimap_rect,
        ui.id().with("minimap_click"),
        egui::Sense::click_and_drag(),
    );
    if resp.clicked() || resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let rel_y = pos.y - minimap_rect.min.y;
            let target_line = rel_y / scale;
            let center_offset = (data.visible_count as f32 / 2.0) * line_height;
            let target_scroll = (target_line * line_height - center_offset).max(0.0);
            let max_scroll =
                (data.total_lines as f32 * line_height - editor_rect.height()).max(0.0);
            new_scroll = Some(target_scroll.min(max_scroll));
        }
    }

    new_scroll
}
