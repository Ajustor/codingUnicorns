use std::path::PathBuf;

/// Walk `workspace` searching for any of `patterns` in source files.
///
/// Returns the path and 0-indexed line number of the first match found.
/// Files in each directory are checked before recursing into subdirectories so that
/// shallower (more relevant) definitions are found first.
pub(crate) fn search_workspace_for_symbol(
    workspace: &std::path::Path,
    patterns: &[String],
    max_files: usize,
    max_depth: usize,
) -> Option<(PathBuf, usize)> {
    let mut file_count = 0usize;
    search_in_dir(
        workspace,
        patterns,
        0,
        max_depth,
        &mut file_count,
        max_files,
    )
}

pub(super) fn search_in_dir(
    dir: &std::path::Path,
    patterns: &[String],
    depth: usize,
    max_depth: usize,
    file_count: &mut usize,
    max_files: usize,
) -> Option<(PathBuf, usize)> {
    if depth > max_depth {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs: Vec<PathBuf> = Vec::new();
    let mut source_files: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.')
            || matches!(name_str.as_ref(), "target" | "node_modules" | ".git")
        {
            continue;
        }
        if path.is_dir() {
            subdirs.push(path);
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(
                ext,
                "rs" | "ts"
                    | "tsx"
                    | "js"
                    | "jsx"
                    | "py"
                    | "go"
                    | "java"
                    | "kt"
                    | "c"
                    | "cpp"
                    | "h"
            ) {
                source_files.push(path);
            }
        }
    }
    // Search files in the current directory first, then recurse into subdirectories.
    for path in source_files {
        *file_count += 1;
        if *file_count > max_files {
            return None;
        }
        if let Some(line) = search_file_for_patterns(&path, patterns) {
            return Some((path, line));
        }
    }
    for subdir in subdirs {
        if let Some(result) = search_in_dir(
            &subdir,
            patterns,
            depth + 1,
            max_depth,
            file_count,
            max_files,
        ) {
            return Some(result);
        }
    }
    None
}

/// Return the 0-indexed line number of the first line in `path` that contains any of `patterns`.
pub(super) fn search_file_for_patterns(
    path: &std::path::Path,
    patterns: &[String],
) -> Option<usize> {
    let content = std::fs::read_to_string(path).ok()?;
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        for pattern in patterns {
            if trimmed.contains(pattern.as_str()) {
                return Some(line_idx);
            }
        }
    }
    None
}

/// Returns true if `line` contains `word` as a definition token with proper word boundaries.
pub(super) fn contains_as_definition(line: &str, pattern: &str, word: &str) -> bool {
    let Some(pat_pos) = line.find(pattern) else {
        return false;
    };
    // Verify the word within the pattern has word boundaries.
    let Some(word_pos) = line[pat_pos..].find(word).map(|p| pat_pos + p) else {
        return false;
    };
    let bytes = line.as_bytes();
    let before = bytes
        .get(word_pos.saturating_sub(1))
        .copied()
        .unwrap_or(b' ');
    let after = bytes.get(word_pos + word.len()).copied().unwrap_or(b' ');
    let before_ok = !before.is_ascii_alphanumeric() && before != b'_';
    let after_ok = !after.is_ascii_alphanumeric() && after != b'_';
    before_ok && after_ok
}

pub(super) fn find_definition_in_buffer(content: &str, word: &str) -> Option<(String, usize)> {
    let patterns: &[String] = &[
        // Rust — bare fn
        format!("fn {}(", word),
        format!("fn {} (", word),
        // Rust — visibility + fn
        format!("pub fn {}(", word),
        format!("pub fn {} (", word),
        format!("pub async fn {}(", word),
        format!("pub async fn {} (", word),
        format!("pub(crate) fn {}(", word),
        format!("pub(super) fn {}(", word),
        format!("pub unsafe fn {}(", word),
        // Rust — other fn flavours
        format!("async fn {}(", word),
        format!("async fn {} (", word),
        format!("const fn {}(", word),
        format!("unsafe fn {}(", word),
        // Rust — impl-block methods (indented)
        format!("  fn {}(", word),
        format!("    fn {}(", word),
        format!("fn {}(&", word),
        format!("fn {}(&mut", word),
        // Rust — type-level definitions
        format!("struct {}", word),
        format!("enum {}", word),
        format!("trait {}", word),
        format!("impl {}", word),
        format!("type {} =", word),
        format!("const {}", word),
        format!("let {} =", word),
        format!("macro_rules! {}", word),
        // JavaScript / TypeScript
        format!("function {}(", word),
        format!("function {} (", word),
        format!("class {}", word),
        format!("interface {}", word),
        format!("export function {}", word),
        format!("export class {}", word),
        format!("export const {} =", word),
        format!("export default function {}", word),
        format!("get {}(", word),
        format!("set {}(", word),
        format!("async {}(", word),
        format!("{}:", word),
        // Python
        format!("def {}(", word),
        format!("def {} (", word),
        format!("async def {}(", word),
        format!("  def {}(", word),
        format!("    def {}(", word),
    ];

    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        for pattern in patterns {
            if contains_as_definition(trimmed, pattern, word) {
                return Some((line.to_string(), line_idx));
            }
        }
    }
    None
}
