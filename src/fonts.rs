//! Font set construction. Centralised so both startup (`main`) and the live
//! "change font" action in Settings build the exact same definitions.

use egui::{FontData, FontDefinitions, FontFamily};

/// Build the application's font definitions.
///
/// Always includes the Phosphor icon font (sidebar/UI glyphs) and Symbola as a
/// Unicode fallback. When `custom_path` points to a readable `.ttf`/`.otf`, it is
/// registered as the **primary monospace** font, so the editor and the integrated
/// terminal render with it — this is how a Nerd Font's glyphs (e.g. an oh-my-posh
/// prompt) show up. It is deliberately NOT added to the proportional family, so the
/// Phosphor icons used across the UI keep rendering from their own font.
pub fn build_font_definitions(custom_path: Option<&str>) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);

    // Symbola fallback for emoji/symbols (🦄, ●, ⚙, …).
    fonts.font_data.insert(
        "Symbola".to_owned(),
        FontData::from_static(include_bytes!("../assets/Symbola.ttf")).into(),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.push("Symbola".to_owned());
        }
    }

    // Optional user-chosen monospace font (loaded from a file on disk).
    if let Some(path) = custom_path.filter(|p| !p.trim().is_empty()) {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("custom-mono".to_owned(), FontData::from_owned(bytes).into());
            if let Some(list) = fonts.families.get_mut(&FontFamily::Monospace) {
                list.insert(0, "custom-mono".to_owned());
            }
        }
    }

    fonts
}
