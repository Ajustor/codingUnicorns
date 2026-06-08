#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

mod app;
mod config;
pub mod dap;
mod editor;
pub mod extension;
mod filetree;
mod git;
mod keybinds;
mod lsp;
mod nav_history;
pub mod plugin;
pub mod runner;
mod tabs;
mod terminal;
mod ui;

use app::CodingUnicorns;

/// Path of the crash log: `<config_dir>/coding-unicorns/crash.log`.
fn crash_log_path() -> std::path::PathBuf {
    let mut path = dirs_next::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    path.push("coding-unicorns");
    path.push("crash.log");
    path
}

/// Install a panic hook that appends panic message + source location to a log file.
///
/// In release builds the window has no console (`windows_subsystem = "windows"`),
/// so panics would otherwise close the app silently. `info.location()` reports the
/// Rust source file/line of the panic and does NOT depend on debug symbols, so it
/// survives `strip = true`. A backtrace is also captured (set `RUST_BACKTRACE=1`),
/// though it is only symbolicated in non-stripped builds.
fn install_panic_logger() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());
        let message = match info.payload().downcast_ref::<&str>() {
            Some(s) => (*s).to_string(),
            None => match info.payload().downcast_ref::<String>() {
                Some(s) => s.clone(),
                None => "<non-string panic payload>".to_string(),
            },
        };
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backtrace = std::backtrace::Backtrace::force_capture();
        let entry = format!(
            "\n=== PANIC @ unix:{elapsed} ===\nlocation: {location}\nmessage: {message}\nbacktrace:\n{backtrace}\n"
        );

        let path = crash_log_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            use std::io::Write;
            let _ = f.write_all(entry.as_bytes());
        }

        // Preserve default behavior (stderr) for debug builds / when a console exists.
        default_hook(info);
    }));
}

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
    // Re-invoked by `claude` as the permission MCP server — no GUI.
    let raw_args: Vec<String> = std::env::args().collect();
    if raw_args.iter().any(|a| a == "--claude-permission-server") {
        let port = raw_args
            .iter()
            .position(|a| a == "--port")
            .and_then(|i| raw_args.get(i + 1))
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(0);
        let token = std::env::var("CU_PERM_TOKEN").unwrap_or_default();
        if port != 0 {
            claude::permission::run_permission_mcp_server(port, token);
        }
        return Ok(());
    }

    install_panic_logger();
    env_logger::init();

    let args: Vec<String> = std::env::args().collect();
    let initial_path = args.get(1).map(std::path::PathBuf::from);

    let icon = load_icon();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Coding Unicorns")
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
        "Coding Unicorns",
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

            Ok(Box::new(CodingUnicorns::new(cc, initial_path)))
        }),
    )
}
