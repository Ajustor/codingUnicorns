// Syntax highlighting. Language tokenisation is provided exclusively by installed
// extensions (FFI language modules); the editor binary embeds no grammars.

// ── Token types ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TokenKind {
    Keyword,
    KeywordType,
    String,
    Comment,
    Number,
    Function,
    Macro,
    Normal,
    Class,     // teal — struct/enum/trait/class names (upper-case identifiers)
    TypeParam, // lighter teal — single-letter generics like T, U
    Operator,
    Property,
}

impl TokenKind {
    pub fn color(self) -> egui::Color32 {
        match self {
            TokenKind::Keyword => egui::Color32::from_rgb(197, 134, 192),
            TokenKind::KeywordType => egui::Color32::from_rgb(86, 156, 214),
            TokenKind::String => egui::Color32::from_rgb(206, 145, 120),
            TokenKind::Comment => egui::Color32::from_rgb(106, 153, 85),
            TokenKind::Number => egui::Color32::from_rgb(181, 206, 168),
            TokenKind::Function => egui::Color32::from_rgb(220, 220, 170),
            TokenKind::Macro => egui::Color32::from_rgb(220, 220, 170),
            TokenKind::Normal => egui::Color32::from_rgb(212, 212, 212),
            TokenKind::Class => egui::Color32::from_rgb(78, 201, 176),
            TokenKind::TypeParam => egui::Color32::from_rgb(180, 220, 220),
            TokenKind::Operator => egui::Color32::from_rgb(212, 212, 212),
            TokenKind::Property => egui::Color32::from_rgb(156, 220, 254),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Token {
    pub text: String,
    pub kind: TokenKind,
}

// ── Highlighter ───────────────────────────────────────────────────────────────

pub struct Highlighter {
    pub language: String,
    /// Pre-computed token list per line, rebuilt when `content_version` changes.
    line_tokens: Vec<Vec<Token>>,
    /// The `content_version` for which `line_tokens` was last computed.
    last_version: i32,
}

impl Highlighter {
    pub fn new() -> Self {
        Self {
            language: String::new(),
            line_tokens: Vec::new(),
            last_version: -1,
        }
    }

    pub fn set_language(&mut self, ext: &str) {
        let new_lang = ext.to_lowercase();
        if self.language != new_lang {
            self.language = new_lang;
            self.last_version = -1; // force re-highlight on next frame
            self.line_tokens.clear();
        }
    }

    pub fn set_language_from_filename(&mut self, filename: &str) {
        let lower = filename.to_lowercase();
        if lower == "dockerfile" || lower.starts_with("dockerfile.") {
            self.set_language("dockerfile");
            return;
        }
        if lower == "makefile" || lower == "gnumakefile" {
            self.set_language("makefile");
            return;
        }
        let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
        self.set_language(&ext);
    }

    /// Force the token cache to be rebuilt on the next frame.
    pub fn invalidate(&mut self) {
        self.last_version = -1;
        self.line_tokens.clear();
    }

    /// Returns true when the cached tokens are stale and need rebuilding.
    pub fn needs_update(&self, version: i32) -> bool {
        version != self.last_version
    }

    /// Rebuild the per-line token cache from `source`.
    /// No-op if `version` matches the last-computed version.
    /// Tokenisation comes from the installed language extension (document-level
    /// tokenizer preferred, then line-by-line); otherwise the text is left plain.
    pub fn highlight_document(
        &mut self,
        source: &str,
        version: i32,
        plugin_manager: Option<&crate::plugin::manager::PluginManager>,
    ) {
        if version == self.last_version {
            return;
        }
        self.last_version = version;

        let lang = self.language.clone();
        hl_log(&format!(
            "highlight lang={lang:?} src_lines={} pm={}",
            source.lines().count(),
            plugin_manager.is_some()
        ));

        // 1. Document-level tokenizer (extensions with embedded tree-sitter).
        if let Some(pm) = plugin_manager {
            match pm.tokenize_document(&lang, source) {
                Some(doc_tokens) if !doc_tokens.is_empty() => {
                    let strings = doc_tokens
                        .iter()
                        .flatten()
                        .filter(|t| t.kind == TokenKind::String)
                        .count();
                    hl_log(&format!(
                        "  -> DOC path: {} lines, {} string tokens",
                        doc_tokens.len(),
                        strings
                    ));
                    self.line_tokens = doc_tokens;
                    return;
                }
                Some(_) => hl_log("  doc tokenizer returned EMPTY"),
                None => hl_log("  doc tokenizer returned None (no plugin for this ext)"),
            }
        }

        // 2. Line-by-line tokenizer (legacy FFI modules).
        if let Some(pm) = plugin_manager {
            pm.reset_tokenizer(&lang);
            let tokens: Vec<Vec<Token>> = source
                .lines()
                .map(|line| {
                    pm.tokenize_line(&lang, line)
                        .unwrap_or_else(|| tokenize_line_plain(line))
                })
                .collect();
            if tokens.iter().any(|line_toks| {
                line_toks.len() > 1
                    || line_toks
                        .first()
                        .map(|t| t.kind != TokenKind::Normal)
                        .unwrap_or(false)
            }) {
                hl_log("  -> LINE path (per-line tokenizer)");
                self.line_tokens = tokens;
                return;
            }
        }

        // 3. Plain-text fallback.
        hl_log("  -> PLAIN fallback (no colors)");
        self.line_tokens = source.lines().map(tokenize_line_plain).collect();
    }

    /// Return tokens for `line_idx`, falling back to on-the-fly tokenization if the
    /// cache doesn't cover that line.
    pub fn tokens_for_line(
        &self,
        line_idx: usize,
        line_text: &str,
        plugin_manager: Option<&crate::plugin::manager::PluginManager>,
    ) -> Vec<Token> {
        match self.line_tokens.get(line_idx) {
            Some(toks) if !toks.is_empty() => toks.clone(),
            _ => {
                if let Some(pm) = plugin_manager {
                    if let Some(tokens) = pm.tokenize_line(&self.language, line_text) {
                        return tokens;
                    }
                }
                tokenize_line_plain(line_text)
            }
        }
    }

    /// Single-line tokeniser for contexts without plugin access (hover popups, etc.)
    pub fn tokenize_line(&self, line: &str) -> Vec<Token> {
        tokenize_line_plain(line)
    }
}

// ── Plain-text fallback (no embedded language tokenisers) ────────────────────

/// Returns the line as a single Normal token.
/// Language-specific tokenisation is provided exclusively by installed extensions.
fn tokenize_line_plain(line: &str) -> Vec<Token> {
    if line.is_empty() {
        return vec![];
    }
    vec![Token {
        text: line.to_string(),
        kind: TokenKind::Normal,
    }]
}

/// Returns keywords for autocomplete. Language support comes from extensions only.
pub fn keywords_for_language(_lang: &str) -> &'static [&'static str] {
    &[]
}

/// Temporary diagnostic: append a highlighting trace line to `%TEMP%/cu-hl.log`.
fn hl_log(msg: &str) {
    use std::io::Write;
    let path = std::env::temp_dir().join("cu-hl.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{msg}");
    }
}
