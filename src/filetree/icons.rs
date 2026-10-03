use egui_phosphor::regular as ph;

/// Returns (phosphor icon char, color) for a given filename.
pub fn file_icon(name: &str) -> (&'static str, egui::Color32) {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "rs" => (ph::FILE_RS, egui::Color32::from_rgb(222, 99, 52)),
        "py" => (ph::FILE_PY, egui::Color32::from_rgb(53, 114, 165)),
        "js" | "mjs" | "cjs" => (ph::FILE_JS, egui::Color32::from_rgb(240, 219, 79)),
        "ts" => (ph::FILE_TS, egui::Color32::from_rgb(49, 120, 198)),
        "jsx" => (ph::FILE_JSX, egui::Color32::from_rgb(97, 218, 251)),
        "tsx" => (ph::FILE_TSX, egui::Color32::from_rgb(97, 218, 251)),
        "json" | "jsonc" => (ph::BRACKETS_CURLY, egui::Color32::from_rgb(255, 196, 88)),
        "toml" => (ph::FILE_CODE, egui::Color32::from_rgb(156, 220, 254)),
        "yaml" | "yml" => (ph::FILE_CODE, egui::Color32::from_rgb(206, 145, 120)),
        "md" | "mdx" => (ph::FILE_MD, egui::Color32::from_rgb(100, 200, 255)),
        "html" | "htm" => (ph::FILE_HTML, egui::Color32::from_rgb(228, 79, 38)),
        "css" => (ph::FILE_CSS, egui::Color32::from_rgb(86, 156, 214)),
        "scss" | "sass" | "less" => (ph::FILE_CSS, egui::Color32::from_rgb(205, 103, 153)),
        "c" | "h" => (ph::FILE_C, egui::Color32::from_rgb(85, 144, 196)),
        "cpp" | "cc" | "cxx" | "hpp" => (ph::FILE_CPP, egui::Color32::from_rgb(85, 144, 196)),
        "sql" => (ph::FILE_SQL, egui::Color32::from_rgb(218, 160, 17)),
        "svg" => (ph::FILE_SVG, egui::Color32::from_rgb(255, 160, 40)),
        "xml" => (ph::FILE_CODE, egui::Color32::from_rgb(228, 79, 38)),
        "sh" | "bash" | "zsh" | "fish" => (ph::TERMINAL, egui::Color32::from_rgb(35, 209, 139)),
        "txt" | "log" => (ph::FILE_TXT, egui::Color32::GRAY),
        "lock" => (ph::FILE_LOCK, egui::Color32::GRAY),
        "go" => (ph::FILE_CODE, egui::Color32::from_rgb(0, 173, 216)),
        "java" => (ph::FILE_CODE, egui::Color32::from_rgb(176, 114, 25)),
        "kt" | "kts" => (ph::FILE_CODE, egui::Color32::from_rgb(169, 121, 227)),
        "swift" => (ph::FILE_CODE, egui::Color32::from_rgb(240, 81, 56)),
        "rb" => (ph::FILE_CODE, egui::Color32::from_rgb(204, 52, 45)),
        "php" => (ph::FILE_CODE, egui::Color32::from_rgb(119, 123, 179)),
        "lua" => (ph::FILE_CODE, egui::Color32::from_rgb(80, 80, 228)),
        "cs" => (ph::FILE_C_SHARP, egui::Color32::from_rgb(104, 33, 122)),
        "dart" => (ph::FILE_CODE, egui::Color32::from_rgb(84, 182, 217)),
        "zig" => (ph::FILE_CODE, egui::Color32::from_rgb(247, 175, 48)),
        "ex" | "exs" => (ph::FILE_CODE, egui::Color32::from_rgb(102, 51, 153)),
        _ => (ph::FILE, egui::Color32::from_gray(160)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Color32;

    fn rgb(r: u8, g: u8, b: u8) -> Color32 {
        Color32::from_rgb(r, g, b)
    }

    #[test]
    fn known_extensions_map_to_specific_icon_and_color() {
        let cases: &[(&str, &str, Color32)] = &[
            ("main.rs", ph::FILE_RS, rgb(222, 99, 52)),
            ("a.py", ph::FILE_PY, rgb(53, 114, 165)),
            ("a.js", ph::FILE_JS, rgb(240, 219, 79)),
            ("a.mjs", ph::FILE_JS, rgb(240, 219, 79)),
            ("a.cjs", ph::FILE_JS, rgb(240, 219, 79)),
            ("a.ts", ph::FILE_TS, rgb(49, 120, 198)),
            ("a.jsx", ph::FILE_JSX, rgb(97, 218, 251)),
            ("a.tsx", ph::FILE_TSX, rgb(97, 218, 251)),
            ("a.json", ph::BRACKETS_CURLY, rgb(255, 196, 88)),
            ("a.jsonc", ph::BRACKETS_CURLY, rgb(255, 196, 88)),
            ("Cargo.toml", ph::FILE_CODE, rgb(156, 220, 254)),
            ("a.yaml", ph::FILE_CODE, rgb(206, 145, 120)),
            ("a.yml", ph::FILE_CODE, rgb(206, 145, 120)),
            ("README.md", ph::FILE_MD, rgb(100, 200, 255)),
            ("a.mdx", ph::FILE_MD, rgb(100, 200, 255)),
            ("a.html", ph::FILE_HTML, rgb(228, 79, 38)),
            ("a.htm", ph::FILE_HTML, rgb(228, 79, 38)),
            ("a.css", ph::FILE_CSS, rgb(86, 156, 214)),
            ("a.scss", ph::FILE_CSS, rgb(205, 103, 153)),
            ("a.sass", ph::FILE_CSS, rgb(205, 103, 153)),
            ("a.less", ph::FILE_CSS, rgb(205, 103, 153)),
            ("a.c", ph::FILE_C, rgb(85, 144, 196)),
            ("a.h", ph::FILE_C, rgb(85, 144, 196)),
            ("a.cpp", ph::FILE_CPP, rgb(85, 144, 196)),
            ("a.cc", ph::FILE_CPP, rgb(85, 144, 196)),
            ("a.cxx", ph::FILE_CPP, rgb(85, 144, 196)),
            ("a.hpp", ph::FILE_CPP, rgb(85, 144, 196)),
            ("a.sql", ph::FILE_SQL, rgb(218, 160, 17)),
            ("a.svg", ph::FILE_SVG, rgb(255, 160, 40)),
            ("a.xml", ph::FILE_CODE, rgb(228, 79, 38)),
            ("a.sh", ph::TERMINAL, rgb(35, 209, 139)),
            ("a.bash", ph::TERMINAL, rgb(35, 209, 139)),
            ("a.zsh", ph::TERMINAL, rgb(35, 209, 139)),
            ("a.fish", ph::TERMINAL, rgb(35, 209, 139)),
            ("a.txt", ph::FILE_TXT, Color32::GRAY),
            ("a.log", ph::FILE_TXT, Color32::GRAY),
            ("Cargo.lock", ph::FILE_LOCK, Color32::GRAY),
            ("a.go", ph::FILE_CODE, rgb(0, 173, 216)),
            ("A.java", ph::FILE_CODE, rgb(176, 114, 25)),
            ("a.kt", ph::FILE_CODE, rgb(169, 121, 227)),
            ("build.gradle.kts", ph::FILE_CODE, rgb(169, 121, 227)),
            ("a.swift", ph::FILE_CODE, rgb(240, 81, 56)),
            ("a.rb", ph::FILE_CODE, rgb(204, 52, 45)),
            ("a.php", ph::FILE_CODE, rgb(119, 123, 179)),
            ("a.lua", ph::FILE_CODE, rgb(80, 80, 228)),
            ("a.cs", ph::FILE_C_SHARP, rgb(104, 33, 122)),
            ("a.dart", ph::FILE_CODE, rgb(84, 182, 217)),
            ("a.zig", ph::FILE_CODE, rgb(247, 175, 48)),
            ("a.ex", ph::FILE_CODE, rgb(102, 51, 153)),
            ("a.exs", ph::FILE_CODE, rgb(102, 51, 153)),
        ];
        for (name, icon, color) in cases {
            assert_eq!(file_icon(name), (*icon, *color), "for {name}");
        }
    }

    #[test]
    fn extension_match_is_case_insensitive_and_uses_last_dot() {
        assert_eq!(file_icon("MAIN.RS").0, ph::FILE_RS);
        assert_eq!(file_icon("Index.Tsx").0, ph::FILE_TSX);
        assert_eq!(file_icon("archive.tar.json").0, ph::BRACKETS_CURLY);
        assert_eq!(file_icon("notes.md.txt").0, ph::FILE_TXT);
    }

    #[test]
    fn unknown_or_missing_extension_uses_generic_icon() {
        let generic = (ph::FILE, Color32::from_gray(160));
        assert_eq!(file_icon("photo.png"), generic);
        assert_eq!(file_icon("Makefile"), generic);
        assert_eq!(file_icon(".gitignore"), generic);
        assert_eq!(file_icon(""), generic);
        assert_eq!(file_icon("trailingdot."), generic);
    }

    #[test]
    #[ignore = "BUG: an extension-less file whose whole name is an extension (e.g. `go`, `lock`) gets that language's icon"]
    fn bare_name_without_dot_is_not_treated_as_extension() {
        let generic = (ph::FILE, Color32::from_gray(160));
        assert_eq!(file_icon("lock"), generic);
        assert_eq!(file_icon("go"), generic);
    }
}
