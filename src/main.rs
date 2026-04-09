#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

mod app;
mod config;
pub mod dap;
mod editor;
pub mod extension;
mod filetree;
mod git;
mod lsp;
mod nav_history;
pub mod plugin;
pub mod runner;
mod tabs;
mod terminal;
mod ui;

use app::WritingUnicorns;

fn load_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../assets/icon.png");
    // Decode PNG manually (raw RGBA from our handcrafted PNG)
    let mut pos = 8usize; // skip PNG signature
    let mut width = 0u32;
    let mut height = 0u32;
    let mut idat = Vec::new();
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
        let tag = &bytes[pos + 4..pos + 8];
        let data = &bytes[pos + 8..pos + 8 + len];
        match tag {
            b"IHDR" => {
                width = u32::from_be_bytes(data[0..4].try_into().ok()?);
                height = u32::from_be_bytes(data[4..8].try_into().ok()?);
            }
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&idat).ok()?;
    let stride = width as usize * 4 + 1;
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for row in 0..height as usize {
        let start = row * stride;
        // filter byte at start (we always used 0=None in our generator)
        for px in 0..width as usize {
            let o = start + 1 + px * 4;
            rgba.extend_from_slice(&raw[o..o + 4]);
        }
    }
    Some(egui::IconData {
        rgba,
        width,
        height,
    })
}

fn main() -> eframe::Result<()> {
    env_logger::init();

    let args: Vec<String> = std::env::args().collect();
    let initial_path = args.get(1).map(std::path::PathBuf::from);

    let icon = load_icon();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Writing Unicorns")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([600.0, 400.0]);
    if let Some(icon_data) = icon {
        viewport = viewport.with_icon(std::sync::Arc::new(icon_data));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Writing Unicorns",
        options,
        Box::new(|cc| {
            // Load Phosphor icon font so sidebar icons render correctly
            let mut fonts = egui::FontDefinitions::default();
            egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);

            // Add Symbola as a fallback so emoji and symbols (🦄, ●, ⚙, etc.) render correctly
            fonts.font_data.insert(
                "Symbola".to_owned(),
                egui::FontData::from_static(include_bytes!("../assets/Symbola.ttf")).into(),
            );
            fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .push("Symbola".to_owned());
            fonts
                .families
                .get_mut(&egui::FontFamily::Monospace)
                .unwrap()
                .push("Symbola".to_owned());

            cc.egui_ctx.set_fonts(fonts);

            Ok(Box::new(WritingUnicorns::new(cc, initial_path)))
        }),
    )
}
