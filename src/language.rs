//! Language key of a file: the string that selects its syntax highlighting,
//! LSP server and debugger (`rs`, `py`, `dockerfile`…).
//!
//! Usually the file extension, but some files are known by their name
//! (`Dockerfile`, `docker-compose.yml`): modules declare such names in their
//! manifest (`file_names`), published here when extensions are loaded.

use std::path::Path;
use std::sync::RwLock;

/// `(pattern, language)` pairs declared by the installed modules. Patterns are
/// matched case-insensitively against the file name; `*` matches any run of
/// characters.
static FILE_NAMES: RwLock<Vec<(String, String)>> = RwLock::new(Vec::new());

/// Replace the file name patterns (called after the modules are (re)loaded).
pub fn set_file_names(patterns: Vec<(String, String)>) {
    if let Ok(mut table) = FILE_NAMES.write() {
        *table = patterns
            .into_iter()
            .map(|(p, lang)| (p.to_lowercase(), lang))
            .collect();
    }
}

/// Language key of `path`: a module's file name pattern, then the built-in
/// names without extension (`Dockerfile`, `Makefile`), then the extension.
pub fn language_key(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let lower = name.to_lowercase();
    if let Ok(table) = FILE_NAMES.read() {
        if let Some((_, lang)) = table.iter().find(|(p, _)| glob_match(p, &lower)) {
            return Some(lang.clone());
        }
    }
    if lower == "dockerfile" || lower.starts_with("dockerfile.") {
        return Some("dockerfile".into());
    }
    if lower == "makefile" || lower == "gnumakefile" {
        return Some("makefile".into());
    }
    path.extension()?.to_str().map(str::to_string)
}

/// `*`-only glob match (`docker-compose.*.yml`).
fn glob_match(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches_literals_and_wildcards() {
        assert!(glob_match("dockerfile", "dockerfile"));
        assert!(!glob_match("dockerfile", "dockerfile.dev"));
        assert!(glob_match("dockerfile.*", "dockerfile.dev"));
        assert!(glob_match(
            "docker-compose.*.yml",
            "docker-compose.prod.yml"
        ));
        assert!(!glob_match("docker-compose.*.yml", "docker-compose.yml"));
        assert!(glob_match("*.dockerfile", "app.dockerfile"));
        assert!(glob_match("a*b*c", "axxbyyc"));
        assert!(!glob_match("a*b*c", "axxc"));
        assert!(!glob_match("ab*ba", "aba"));
    }

    #[test]
    fn language_key_uses_module_names_then_builtins_then_extension() {
        set_file_names(vec![
            ("Containerfile".into(), "dockerfile".into()),
            ("docker-compose.yml".into(), "compose".into()),
        ]);
        let key = |p: &str| language_key(Path::new(p));
        assert_eq!(key("/w/Containerfile").as_deref(), Some("dockerfile"));
        assert_eq!(key("/w/Docker-Compose.yml").as_deref(), Some("compose"));
        assert_eq!(key("/w/Dockerfile").as_deref(), Some("dockerfile"));
        assert_eq!(key("/w/dockerfile.dev").as_deref(), Some("dockerfile"));
        assert_eq!(key("/w/GNUmakefile").as_deref(), Some("makefile"));
        assert_eq!(key("/w/ci.yml").as_deref(), Some("yml"));
        assert_eq!(key("/w/main.rs").as_deref(), Some("rs"));
        assert_eq!(key("/w/README"), None);
    }
}
