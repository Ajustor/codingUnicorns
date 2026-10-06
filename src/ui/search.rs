use std::path::{Path, PathBuf};
use std::sync::mpsc;

use regex::Regex;

/// How the query is interpreted — the same semantics as the in-file find bar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    /// Treat the query as a regular expression (replace expands `$1` captures).
    pub use_regex: bool,
    /// Only match whole words (`\b` on both sides).
    pub whole_word: bool,
}

/// Compile the query into the matcher used by both search and replace.
/// Plain mode escapes the query. Lines are matched one at a time (`^`/`$` are
/// line anchors), so the regex is built in multi-line + CRLF mode, which also
/// makes a whole-file `is_match` a safe pre-filter. Patterns that can match
/// empty text are rejected: they would "match" every line.
pub fn build_matcher(query: &str, opts: SearchOptions) -> Result<Regex, String> {
    let base = if opts.use_regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    let pattern = if opts.whole_word {
        format!(r"\b(?:{base})\b")
    } else {
        base
    };
    let re = regex::RegexBuilder::new(&pattern)
        .case_insensitive(!opts.case_sensitive)
        .multi_line(true)
        .crlf(true)
        .build()
        .map_err(|e| match e {
            regex::Error::Syntax(msg) => msg
                .lines()
                .last()
                .unwrap_or("invalid regular expression")
                .trim()
                .to_string(),
            other => other.to_string(),
        })?;
    if re.is_match("") {
        return Err("Pattern matches empty text".to_string());
    }
    Ok(re)
}

#[derive(Clone)]
pub struct SearchMatch {
    pub file_path: PathBuf,
    pub line_number: usize,
    pub line_text: String,
    pub match_start: usize,
    pub match_end: usize,
}

pub struct SearchResults {
    pub query: String,
    pub matches: Vec<SearchMatch>,
    pub searched_files: usize,
    pub elapsed_ms: u64,
}

pub struct WorkspaceSearch {
    pub query: String,
    pub case_sensitive: bool,
    pub use_regex: bool,
    pub whole_word: bool,
    /// Why the query can't be used (invalid regex), shown under the input.
    pub error: Option<String>,
    pub results: Option<SearchResults>,
    pub is_searching: bool,
    rx: Option<mpsc::Receiver<SearchResults>>,
    pub selected_match: Option<usize>,
    // ── Replace ────────────────────────────────────────────────────────────
    pub replace_query: String,
    pub show_replace: bool,
    replace_rx: Option<mpsc::Receiver<usize>>,
    pub is_replacing: bool,
    pub replace_count: Option<usize>,
}

impl WorkspaceSearch {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            case_sensitive: false,
            use_regex: false,
            whole_word: false,
            error: None,
            results: None,
            is_searching: false,
            rx: None,
            selected_match: None,
            replace_query: String::new(),
            show_replace: false,
            replace_rx: None,
            is_replacing: false,
            replace_count: None,
        }
    }

    pub fn options(&self) -> SearchOptions {
        SearchOptions {
            case_sensitive: self.case_sensitive,
            use_regex: self.use_regex,
            whole_word: self.whole_word,
        }
    }

    /// Compile the current query, recording (and returning `None` on) errors.
    fn matcher(&mut self) -> Option<Regex> {
        match build_matcher(&self.query, self.options()) {
            Ok(re) => {
                self.error = None;
                Some(re)
            }
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    pub fn start_replace_all(&mut self, workspace: PathBuf) {
        if self.query.is_empty() {
            return;
        }
        let Some(re) = self.matcher() else { return };
        let replacement = self.replace_query.clone();
        let expand = self.use_regex;
        let (tx, rx) = mpsc::channel();
        self.replace_rx = Some(rx);
        self.is_replacing = true;
        self.replace_count = None;
        std::thread::spawn(move || {
            let count = replace_in_dir(&workspace, &re, &replacement, expand);
            let _ = tx.send(count);
        });
    }

    pub fn start_search(&mut self, workspace: PathBuf) {
        if self.query.is_empty() {
            self.error = None;
            return;
        }
        let Some(re) = self.matcher() else {
            // Invalid pattern: show the error instead of stale results.
            self.results = None;
            self.is_searching = false;
            self.rx = None;
            return;
        };
        self.is_searching = true;
        self.results = None;
        let query = self.query.clone();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);

        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let mut matches = Vec::new();
            let mut searched = 0;
            search_dir(&workspace, &re, &mut matches, &mut searched);
            let _ = tx.send(SearchResults {
                query,
                matches,
                searched_files: searched,
                elapsed_ms: start.elapsed().as_millis() as u64,
            });
        });
    }

    pub fn poll(&mut self) {
        if let Some(rx) = &self.rx {
            if let Ok(results) = rx.try_recv() {
                self.results = Some(results);
                self.is_searching = false;
                self.rx = None;
            }
        }
        if let Some(rx) = &self.replace_rx {
            if let Ok(count) = rx.try_recv() {
                self.replace_count = Some(count);
                self.is_replacing = false;
                self.replace_rx = None;
            }
        }
    }

    /// Render the search panel. Returns `Some((path, line))` when a result is clicked.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        workspace: Option<&PathBuf>,
    ) -> Option<(PathBuf, usize)> {
        self.poll();
        let mut open_file: Option<(PathBuf, usize)> = None;

        ui.horizontal(|ui| {
            // Toggle replace row
            let rep_icon = if self.show_replace { "▴" } else { "▾" };
            if ui
                .small_button(rep_icon)
                .on_hover_text("Toggle replace")
                .clicked()
            {
                self.show_replace = !self.show_replace;
            }
            ui.label("🔍");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Search in workspace…")
                    .desired_width(ui.available_width() - 55.0),
            );
            if (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                || resp.changed()
            {
                if self.query.is_empty() {
                    self.error = None;
                }
                if let Some(ws) = workspace {
                    if !self.query.is_empty() {
                        self.start_search(ws.clone());
                    }
                }
            }
        });
        if let Some(err) = &self.error {
            ui.label(
                egui::RichText::new(format!("⚠ {err}"))
                    .size(11.0)
                    .color(egui::Color32::from_rgb(240, 110, 110)),
            );
        }

        if self.show_replace {
            let mut do_replace_all = false;
            ui.horizontal(|ui| {
                ui.label("↔");
                // Button first, right to left, so the field fills exactly what is
                // left: a row wider than the sidebar widens it every frame.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Replace All").clicked() {
                        do_replace_all = true;
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.replace_query)
                            .hint_text("Replace with…")
                            .desired_width(f32::INFINITY),
                    );
                });
            });
            if do_replace_all {
                if let Some(ws) = workspace {
                    self.start_replace_all(ws.clone());
                }
            }
            if self.is_replacing {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new("Replacing…")
                            .size(11.0)
                            .color(egui::Color32::GRAY),
                    );
                });
            } else if let Some(count) = self.replace_count {
                ui.label(
                    egui::RichText::new(format!("Replaced {} occurrence(s)", count))
                        .size(11.0)
                        .color(egui::Color32::from_rgb(100, 220, 100)),
                );
            }
        }

        ui.horizontal(|ui| {
            let mut changed = false;
            changed |= ui
                .toggle_value(&mut self.case_sensitive, "Aa")
                .on_hover_text("Match case")
                .changed();
            changed |= ui
                .toggle_value(&mut self.whole_word, "ab")
                .on_hover_text("Match whole word")
                .changed();
            changed |= ui
                .toggle_value(&mut self.use_regex, ".*")
                .on_hover_text("Use regular expression ($1 in replace)")
                .changed();
            if ui.small_button("Search").clicked() || changed {
                if let Some(ws) = workspace {
                    if !self.query.is_empty() {
                        self.start_search(ws.clone());
                    }
                }
            }
        });

        ui.add_space(4.0);

        if self.is_searching {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    egui::RichText::new("Searching…")
                        .color(egui::Color32::GRAY)
                        .size(11.0),
                );
            });
        } else if let Some(results) = &self.results {
            let count = results.matches.len();
            ui.label(
                egui::RichText::new(format!(
                    "{} result{} in {} file{} ({} ms)",
                    count,
                    if count == 1 { "" } else { "s" },
                    results.searched_files,
                    if results.searched_files == 1 { "" } else { "s" },
                    results.elapsed_ms,
                ))
                .size(11.0)
                .color(egui::Color32::GRAY),
            );
        } else if workspace.is_none() {
            ui.label(
                egui::RichText::new("Open a folder to search")
                    .color(egui::Color32::GRAY)
                    .size(11.0),
            );
        }

        ui.add_space(4.0);
        ui.separator();

        if let Some(results) = &self.results {
            // Group matches by file while preserving order
            let mut by_file: Vec<(PathBuf, Vec<&SearchMatch>)> = Vec::new();
            for m in &results.matches {
                if let Some(entry) = by_file.iter_mut().find(|(p, _)| p == &m.file_path) {
                    entry.1.push(m);
                } else {
                    by_file.push((m.file_path.clone(), vec![m]));
                }
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                for (file_path, file_matches) in &by_file {
                    let file_name = file_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| file_path.to_string_lossy().to_string());

                    ui.collapsing(
                        egui::RichText::new(format!("{} ({})", file_name, file_matches.len()))
                            .size(12.0)
                            .color(egui::Color32::from_rgb(100, 160, 255)),
                        |ui| {
                            for m in file_matches {
                                let text = format!("{}: {}", m.line_number, m.line_text.trim());
                                let truncated = if text.len() > 80 { &text[..80] } else { &text };

                                let resp = ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(truncated)
                                            .monospace()
                                            .size(11.0)
                                            .color(egui::Color32::from_rgb(200, 200, 200)),
                                    )
                                    .sense(egui::Sense::click()),
                                );
                                if resp.clicked() {
                                    open_file = Some((
                                        m.file_path.clone(),
                                        m.line_number.saturating_sub(1),
                                    ));
                                }
                                if resp.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                            }
                        },
                    );
                }
            });
        }

        open_file
    }
}

fn search_dir(dir: &Path, re: &Regex, matches: &mut Vec<SearchMatch>, searched: &mut usize) {
    let dir_entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in dir_entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str.starts_with('.') {
            continue;
        }
        if matches!(
            name_str.as_ref(),
            "target" | "node_modules" | ".git" | "dist" | "build"
        ) {
            continue;
        }

        if path.is_dir() {
            search_dir(&path, re, matches, searched);
            if matches.len() >= MAX_MATCHES {
                return;
            }
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !is_text_extension(ext) {
                continue;
            }
            search_file(&path, re, matches);
            *searched += 1;
            if matches.len() >= MAX_MATCHES {
                return;
            }
        }
    }
}

/// Stop collecting results past this many matching lines.
const MAX_MATCHES: usize = 1000;

/// Record the first match of `re` on every matching line (byte offsets).
fn search_file(path: &Path, re: &Regex, matches: &mut Vec<SearchMatch>) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    // One whole-file scan rules out most files without touching their lines.
    if !re.is_match(&content) {
        return;
    }
    for (line_idx, line) in content.lines().enumerate() {
        if let Some(m) = re.find(line) {
            matches.push(SearchMatch {
                file_path: path.to_path_buf(),
                line_number: line_idx + 1,
                line_text: line.to_string(),
                match_start: m.start(),
                match_end: m.end(),
            });
            if matches.len() >= MAX_MATCHES {
                return;
            }
        }
    }
}

/// Replace every match of `re`, line by line (so anchors behave as in search),
/// keeping each line's `\n` / `\r\n` ending. `expand` enables `$1` captures;
/// otherwise the replacement is inserted literally. Returns the new content and
/// the number of replacements.
fn replace_in_content(
    content: &str,
    re: &Regex,
    replacement: &str,
    expand: bool,
) -> (String, usize) {
    let mut out = String::with_capacity(content.len());
    let mut n = 0;
    for chunk in content.split_inclusive('\n') {
        let body_len = chunk
            .strip_suffix("\r\n")
            .or_else(|| chunk.strip_suffix('\n'))
            .map_or(chunk.len(), str::len);
        let (body, eol) = chunk.split_at(body_len);
        let count = re.find_iter(body).count();
        if count == 0 {
            out.push_str(chunk);
            continue;
        }
        n += count;
        if expand {
            out.push_str(&re.replace_all(body, replacement));
        } else {
            out.push_str(&re.replace_all(body, regex::NoExpand(replacement)));
        }
        out.push_str(eol);
    }
    (out, n)
}

fn replace_in_dir(dir: &Path, re: &Regex, replacement: &str, expand: bool) -> usize {
    let mut count = 0;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }
        if matches!(
            name_str.as_ref(),
            "target" | "node_modules" | ".git" | "dist" | "build"
        ) {
            continue;
        }
        if path.is_dir() {
            count += replace_in_dir(&path, re, replacement, expand);
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !is_text_extension(ext) {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                if !re.is_match(&content) {
                    continue;
                }
                let (new_content, n) = replace_in_content(&content, re, replacement, expand);
                if n > 0 {
                    let _ = std::fs::write(&path, new_content);
                    count += n;
                }
            }
        }
    }
    count
}

fn is_text_extension(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "rs" | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "mjs"
            | "py"
            | "json"
            | "toml"
            | "yaml"
            | "yml"
            | "md"
            | "txt"
            | "sh"
            | "bash"
            | "zsh"
            | "html"
            | "css"
            | "scss"
            | "less"
            | "vue"
            | "svelte"
            | "go"
            | "java"
            | "kt"
            | "c"
            | "cpp"
            | "h"
            | "hpp"
            | "cs"
            | "rb"
            | "php"
            | "swift"
            | "lua"
            | "r"
            | "ex"
            | "exs"
            | "xml"
            | "ini"
            | "cfg"
            | "conf"
            | "env"
            | "gitignore"
            | "dockerfile"
            | "lock"
    )
}

impl Default for WorkspaceSearch {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(case_sensitive: bool, use_regex: bool, whole_word: bool) -> SearchOptions {
        SearchOptions {
            case_sensitive,
            use_regex,
            whole_word,
        }
    }

    fn found(re: &Regex, s: &str) -> Vec<String> {
        re.find_iter(s).map(|m| m.as_str().to_string()).collect()
    }

    #[test]
    fn plain_mode_is_literal_and_case_insensitive_by_default() {
        let re = build_matcher("a.b(", SearchOptions::default()).unwrap();
        assert_eq!(found(&re, "A.B( axb("), ["A.B("]);
        let re = build_matcher("Foo", opts(true, false, false)).unwrap();
        assert_eq!(found(&re, "foo Foo FOO"), ["Foo"]);
    }

    #[test]
    fn regex_and_whole_word_modes() {
        let re = build_matcher(r"fo+\d", opts(false, true, false)).unwrap();
        assert_eq!(found(&re, "fooo1 fx2 FO3"), ["fooo1", "FO3"]);
        let re = build_matcher("cat", opts(false, false, true)).unwrap();
        assert_eq!(found(&re, "cat concat cats Cat"), ["cat", "Cat"]);
        // Alternation is grouped inside the word boundaries.
        let re = build_matcher("is|at", opts(false, true, true)).unwrap();
        assert_eq!(found(&re, "this is at that"), ["is", "at"]);
        // Anchors are per line, also with CRLF endings.
        let re = build_matcher("^x$", opts(false, true, false)).unwrap();
        assert!(re.is_match("a\r\nx\r\nb"));
    }

    #[test]
    fn invalid_or_empty_matching_patterns_are_errors() {
        let err = build_matcher("(unclosed", opts(false, true, false)).unwrap_err();
        assert!(err.contains("unclosed"), "{err}");
        assert!(!err.contains('\n'));
        // The same text is fine as a literal.
        assert!(build_matcher("(unclosed", SearchOptions::default()).is_ok());
        assert_eq!(
            build_matcher("a*", opts(false, true, false)).unwrap_err(),
            "Pattern matches empty text"
        );
    }

    #[test]
    fn replace_expands_captures_only_in_regex_mode() {
        let re = build_matcher(r"(\w+)=(\d+)", opts(false, true, false)).unwrap();
        let (out, n) = replace_in_content("a=1 b=2\nnone\n", &re, "$2:$1", true);
        assert_eq!(out, "1:a 2:b\nnone\n");
        assert_eq!(n, 2);

        let re = build_matcher("x", SearchOptions::default()).unwrap();
        let (out, n) = replace_in_content("xX\r\nx", &re, "$1", false);
        assert_eq!(out, "$1$1\r\n$1", "literal replacement, CRLF kept");
        assert_eq!(n, 3);
    }

    #[test]
    fn replace_handles_non_ascii_case_folding() {
        // The old lowercase-offset approach could split chars here.
        let re = build_matcher("é", SearchOptions::default()).unwrap();
        let (out, n) = replace_in_content("ÉtÉ été", &re, "e", false);
        assert_eq!(out, "ete ete");
        assert_eq!(n, 4);
    }

    #[test]
    fn search_and_replace_walk_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("src/a.rs"), "let foo1 = 1;\nfoo2();\nbar\n").unwrap();
        std::fs::write(root.join("src/b.md"), "no match\n").unwrap();
        std::fs::write(root.join("target/c.rs"), "foo9\n").unwrap();
        std::fs::write(root.join("img.png"), "foo3").unwrap();

        let re = build_matcher(r"foo(\d)", opts(true, true, false)).unwrap();
        let mut matches = Vec::new();
        let mut searched = 0;
        search_dir(root, &re, &mut matches, &mut searched);
        assert_eq!(searched, 2, "build dirs and binary extensions skipped");
        let got: Vec<(usize, usize, usize)> = matches
            .iter()
            .map(|m| (m.line_number, m.match_start, m.match_end))
            .collect();
        assert_eq!(got, [(1, 4, 8), (2, 0, 4)]);

        assert_eq!(replace_in_dir(root, &re, "bar$1", true), 2);
        assert_eq!(
            std::fs::read_to_string(root.join("src/a.rs")).unwrap(),
            "let bar1 = 1;\nbar2();\nbar\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("target/c.rs")).unwrap(),
            "foo9\n"
        );
    }

    #[test]
    fn invalid_regex_sets_error_and_skips_search() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = WorkspaceSearch::new();
        s.query = "([".into();
        s.use_regex = true;
        s.start_search(dir.path().to_path_buf());
        assert!(s.error.is_some());
        assert!(!s.is_searching);
        assert!(s.results.is_none());
        s.start_replace_all(dir.path().to_path_buf());
        assert!(!s.is_replacing);

        // Same text as a literal works and clears the error.
        s.use_regex = false;
        s.start_search(dir.path().to_path_buf());
        assert!(s.error.is_none());
        assert!(s.is_searching);
        for _ in 0..200 {
            s.poll();
            if !s.is_searching {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(s.results.as_ref().map(|r| r.matches.len()), Some(0));
    }
}
