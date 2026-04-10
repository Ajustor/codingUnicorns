use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent};

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

// ── Tree-sitter highlight name → TokenKind mapping ───────────────────────────

/// The ordered list of capture names recognised by our renderer.
/// Order matters: tree-sitter-highlight resolves captures hierarchically
/// (e.g. "keyword.function" falls back to "keyword" if not listed).
const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",             // 0  → Macro  (#[derive], decorators)
    "comment",               // 1  → Comment
    "comment.documentation", // 2 → Comment
    "constant",              // 3  → Keyword (true / false / nil)
    "constant.builtin",      // 4  → Keyword
    "constructor",           // 5  → Class
    "function",              // 6  → Function
    "function.builtin",      // 7  → Function
    "function.macro",        // 8  → Macro
    "function.method",       // 9  → Function
    "keyword",               // 10 → Keyword
    "keyword.function",      // 11 → Keyword
    "keyword.operator",      // 12 → Operator
    "keyword.return",        // 13 → Keyword
    "keyword.storage",       // 14 → Keyword
    "label",                 // 15 → Normal
    "number",                // 16 → Number
    "operator",              // 17 → Operator
    "property",              // 18 → Property
    "punctuation",           // 19 → Normal
    "punctuation.bracket",   // 20 → Normal
    "punctuation.delimiter", // 21 → Normal
    "string",                // 22 → String
    "string.escape",         // 23 → String
    "string.special",        // 24 → String
    "type",                  // 25 → KeywordType
    "type.builtin",          // 26 → KeywordType
    "variable",              // 27 → Normal
    "variable.builtin",      // 28 → Keyword  (self / this / super)
    "variable.parameter",    // 29 → Normal
];

fn capture_idx_to_kind(idx: usize) -> TokenKind {
    match idx {
        0 => TokenKind::Macro,
        1 | 2 => TokenKind::Comment,
        3 | 4 => TokenKind::Keyword,
        5 => TokenKind::Class,
        6..=9 => TokenKind::Function,
        10 | 11 | 13 | 14 => TokenKind::Keyword,
        12 | 17 => TokenKind::Operator,
        15 => TokenKind::Normal,
        16 => TokenKind::Number,
        18 => TokenKind::Property,
        19..=21 => TokenKind::Normal,
        22..=24 => TokenKind::String,
        25 | 26 => TokenKind::KeywordType,
        27 | 29 => TokenKind::Normal,
        28 => TokenKind::Keyword,
        _ => TokenKind::Normal,
    }
}

/// Normalise an extension to its "primary" tree-sitter language key.
fn primary_lang(lang: &str) -> &str {
    match lang {
        "jsx" => "tsx",          // JSX uses TSX grammar (superset of JS+JSX)
        "mjs" | "cjs" => "js",  // CommonJS/ESM variants → plain JS
        "mts" | "cts" => "ts",  // TypeScript variants
        "csx" => "cs",          // C# script → C#
        "htm" => "html",
        _ => lang,
    }
}

// ── Thread-local TS highlighter (reused across frames) ────────────────────────

thread_local! {
    static TS: std::cell::RefCell<tree_sitter_highlight::Highlighter> =
        std::cell::RefCell::new(tree_sitter_highlight::Highlighter::new());
}

// ── Highlighter ───────────────────────────────────────────────────────────────

pub struct Highlighter {
    pub language: String,
    /// Pre-computed token list per line, rebuilt when `content_version` changes.
    line_tokens: Vec<Vec<Token>>,
    /// The `content_version` for which `line_tokens` was last computed.
    last_version: i32,
    /// Tree-sitter configurations, keyed by primary language extension.
    configs: std::collections::HashMap<String, HighlightConfiguration>,
}

impl Highlighter {
    pub fn new() -> Self {
        let mut configs = std::collections::HashMap::new();

        try_add_config(
            &mut configs,
            "rs",
            tree_sitter_rust::LANGUAGE.into(),
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        );

        try_add_config(
            &mut configs,
            "js",
            tree_sitter_javascript::LANGUAGE.into(),
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        );

        try_add_config(
            &mut configs,
            "py",
            tree_sitter_python::LANGUAGE.into(),
            tree_sitter_python::HIGHLIGHTS_QUERY,
            "",
            "",
        );

        try_add_config(
            &mut configs,
            "go",
            tree_sitter_go::LANGUAGE.into(),
            tree_sitter_go::HIGHLIGHTS_QUERY,
            "",
            "",
        );

        try_add_config(
            &mut configs,
            "ts",
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
            "",
            tree_sitter_typescript::LOCALS_QUERY,
        );

        try_add_config(
            &mut configs,
            "tsx",
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
            "",
            tree_sitter_typescript::LOCALS_QUERY,
        );

        try_add_config(
            &mut configs,
            "html",
            tree_sitter_html::LANGUAGE.into(),
            tree_sitter_html::HIGHLIGHTS_QUERY,
            tree_sitter_html::INJECTIONS_QUERY,
            "",
        );

        try_add_config(
            &mut configs,
            "toml",
            tree_sitter_toml_ng::LANGUAGE.into(),
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            "",
            "",
        );

        Self {
            language: String::new(),
            line_tokens: Vec::new(),
            last_version: -1,
            configs,
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
    /// When `plugin_manager` is provided, uses extension-based tokenization
    /// as fallback when tree-sitter has no grammar for the language.
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
        let primary = primary_lang(&lang).to_string();

        // 1. Try plugin document-level tokenizer (extensions with embedded tree-sitter)
        if let Some(pm) = plugin_manager {
            if let Some(doc_tokens) = pm.tokenize_document(&lang, source) {
                if !doc_tokens.is_empty() {
                    self.line_tokens = doc_tokens;
                    return;
                }
            }
        }

        // 2. Try built-in tree-sitter grammars
        if let Some(config) = self.configs.get(&primary) {
            match ts_highlight(config, source) {
                Ok(spans) => {
                    self.line_tokens = spans;
                    return;
                }
                Err(e) => {
                    log::debug!("tree-sitter highlight error for {}: {e:?}", self.language);
                }
            }
        }

        // 3. Try plugin line-by-line tokenizer (legacy FFI modules)
        if let Some(pm) = plugin_manager {
            pm.reset_tokenizer(&lang);
            let tokens: Vec<Vec<Token>> = source
                .lines()
                .map(|line| {
                    pm.tokenize_line(&lang, line)
                        .unwrap_or_else(|| tokenize_line_regex(&lang, line))
                })
                .collect();
            if tokens.iter().any(|line_toks| {
                line_toks.len() > 1
                    || line_toks
                        .first()
                        .map(|t| t.kind != TokenKind::Normal)
                        .unwrap_or(false)
            }) {
                self.line_tokens = tokens;
                return;
            }
        }

        // Plain-text fallback
        self.line_tokens = source
            .lines()
            .map(|line| tokenize_line_regex(&lang, line))
            .collect();
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
                tokenize_line_regex(&self.language, line_text)
            }
        }
    }

    /// Single-line tokeniser for contexts without plugin access (hover popups, etc.)
    pub fn tokenize_line(&self, line: &str) -> Vec<Token> {
        tokenize_line_regex(&self.language, line)
    }
}

// ── Tree-sitter highlight engine ─────────────────────────────────────────────

fn try_add_config(
    map: &mut std::collections::HashMap<String, HighlightConfiguration>,
    key: &str,
    language: tree_sitter::Language,
    highlights: &str,
    injections: &str,
    locals: &str,
) {
    match HighlightConfiguration::new(language, key, highlights, injections, locals) {
        Ok(mut cfg) => {
            cfg.configure(HIGHLIGHT_NAMES);
            map.insert(key.to_string(), cfg);
        }
        Err(e) => log::warn!("Failed to build tree-sitter config for {key}: {e:?}"),
    }
}

/// Run tree-sitter-highlight on `source` and return per-line token vectors.
fn ts_highlight(
    config: &HighlightConfiguration,
    source: &str,
) -> Result<Vec<Vec<Token>>, tree_sitter_highlight::Error> {
    let source_bytes = source.as_bytes();
    let num_lines = source.lines().count().max(1);
    let mut result: Vec<Vec<Token>> = vec![Vec::new(); num_lines];

    // Build a lookup table: byte offset → line index.
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(source_bytes.iter().enumerate().filter_map(|(i, &b)| {
            if b == b'\n' {
                Some(i + 1)
            } else {
                None
            }
        }))
        .collect();

    // Collect all highlight events before releasing the thread-local borrow.
    let events: Vec<HighlightEvent> = TS.with(|cell| {
        let mut hl = cell.borrow_mut();
        let result = hl
            .highlight(config, source_bytes, None, |_| None)?
            .collect::<Result<Vec<_>, _>>();
        result
    })?;

    // Replay events to build per-line token lists.
    let mut kind_stack: Vec<TokenKind> = Vec::new();

    for event in events {
        match event {
            HighlightEvent::HighlightStart(h) => {
                kind_stack.push(capture_idx_to_kind(h.0));
            }
            HighlightEvent::HighlightEnd => {
                kind_stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                if start >= end {
                    continue;
                }
                let kind = kind_stack.last().copied().unwrap_or(TokenKind::Normal);
                let text = match source.get(start..end) {
                    Some(t) => t,
                    None => continue,
                };

                // The span may cover multiple lines — split it.
                let start_line = line_starts
                    .partition_point(|&s| s <= start)
                    .saturating_sub(1);
                let mut line_idx = start_line;

                for piece in text.split('\n') {
                    if line_idx < result.len() && !piece.is_empty() {
                        result[line_idx].push(Token {
                            text: piece.to_string(),
                            kind,
                        });
                    }
                    line_idx += 1;
                }
            }
        }
    }

    // Any line still empty gets its raw text as a single Normal token.
    for (tokens, line_text) in result.iter_mut().zip(source.lines()) {
        if tokens.is_empty() && !line_text.is_empty() {
            tokens.push(Token {
                text: line_text.to_string(),
                kind: TokenKind::Normal,
            });
        }
    }

    Ok(result)
}

// ── Plain-text fallback (no hardcoded language tokenisers) ───────────────────

/// Returns the line as a single Normal token.
/// Language-specific tokenisation is provided exclusively by installed extensions.
fn tokenize_line_regex(_lang: &str, line: &str) -> Vec<Token> {
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
