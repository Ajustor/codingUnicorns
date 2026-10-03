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
    /// For very large files we tokenize only the visible window (+ margin) instead
    /// of the whole document. This is the `[start, end)` line range currently cached
    /// in viewport mode (`None` in full-document mode).
    viewport: Option<(usize, usize)>,
}

impl Highlighter {
    pub fn new() -> Self {
        Self {
            language: String::new(),
            line_tokens: Vec::new(),
            last_version: -1,
            viewport: None,
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
        self.viewport = None;
    }

    /// Returns true when the cached tokens are stale and need rebuilding.
    pub fn needs_update(&self, version: i32) -> bool {
        version != self.last_version
    }

    /// In viewport mode: whether the visible window needs re-tokenizing — either the
    /// content changed, or the visible range scrolled outside the cached window.
    pub fn viewport_stale(&self, version: i32, vis_start: usize, vis_end: usize) -> bool {
        if version != self.last_version {
            return true;
        }
        match self.viewport {
            None => true,
            Some((s, e)) => vis_start < s || vis_end > e,
        }
    }

    /// Tokenize only `window_src` (the text of lines `[start_line, start_line+N)`)
    /// and place the resulting tokens at their absolute line indices. Off-window
    /// lines are left empty (rendered plain — they're off-screen anyway). Used for
    /// very large files where re-tokenizing the whole document each frame lags.
    pub fn highlight_viewport(
        &mut self,
        window_src: &str,
        start_line: usize,
        total_lines: usize,
        version: i32,
        cached_range: (usize, usize),
        plugin_manager: Option<&crate::plugin::manager::PluginManager>,
    ) {
        let lang = self.language.clone();
        let window_tokens = plugin_manager.and_then(|pm| pm.tokenize_document(&lang, window_src));

        let mut lines = vec![Vec::new(); total_lines];
        if let Some(win) = window_tokens {
            for (i, toks) in win.into_iter().enumerate() {
                let abs = start_line + i;
                if abs < total_lines {
                    lines[abs] = toks;
                }
            }
        }
        self.line_tokens = lines;
        self.last_version = version;
        self.viewport = Some(cached_range);
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
        self.viewport = None;

        let lang = self.language.clone();

        // 1. Document-level tokenizer (extensions with embedded tree-sitter).
        if let Some(pm) = plugin_manager {
            if let Some(doc_tokens) = pm.tokenize_document(&lang, source) {
                if !doc_tokens.is_empty() {
                    self.line_tokens = doc_tokens;
                    return;
                }
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
                self.line_tokens = tokens;
                return;
            }
        }

        // 3. Plain-text fallback.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::manager::PluginManager;
    use crate::plugin::Plugin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn tok(text: &str, kind: TokenKind) -> Token {
        Token {
            text: text.to_string(),
            kind,
        }
    }

    /// Language plugin double: optional document tokenizer, optional line tokenizer
    /// producing a single token of `line_kind`, counts tokenizer resets.
    struct FakeLang {
        doc: Option<Vec<Vec<Token>>>,
        line_kind: Option<TokenKind>,
        resets: Arc<AtomicUsize>,
    }

    impl Plugin for FakeLang {
        fn name(&self) -> &str {
            "fake"
        }
        fn file_extensions(&self) -> &[&str] {
            &["fk"]
        }
        fn tokenize_document(&self, lang: &str, _text: &str) -> Option<Vec<Vec<Token>>> {
            if lang == "fk" {
                self.doc.clone()
            } else {
                None
            }
        }
        fn tokenize_line(&self, lang: &str, line: &str) -> Option<Vec<Token>> {
            if lang != "fk" {
                return None;
            }
            self.line_kind.map(|k| vec![tok(line, k)])
        }
        fn reset_tokenizer(&self) {
            self.resets.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn manager(
        doc: Option<Vec<Vec<Token>>>,
        line_kind: Option<TokenKind>,
    ) -> (PluginManager, Arc<AtomicUsize>) {
        let resets = Arc::new(AtomicUsize::new(0));
        let mut pm = PluginManager::new();
        pm.register(Box::new(FakeLang {
            doc,
            line_kind,
            resets: resets.clone(),
        }));
        (pm, resets)
    }

    fn fk_highlighter() -> Highlighter {
        let mut h = Highlighter::new();
        h.set_language("fk");
        h
    }

    fn kinds(tokens: &[Token]) -> Vec<TokenKind> {
        tokens.iter().map(|t| t.kind).collect()
    }

    #[test]
    fn token_colors_are_distinct_where_expected() {
        use TokenKind::*;
        assert_eq!(Keyword.color(), egui::Color32::from_rgb(197, 134, 192));
        assert_eq!(Normal.color(), egui::Color32::from_rgb(212, 212, 212));
        assert_eq!(Function.color(), Macro.color());
        assert_eq!(Operator.color(), Normal.color());
        let all = [
            Keyword,
            KeywordType,
            String,
            Comment,
            Number,
            Function,
            Normal,
            Class,
            TypeParam,
            Property,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.color(), b.color(), "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn set_language_lowercases_and_invalidates() {
        let mut h = Highlighter::new();
        h.highlight_document("a", 3, None);
        assert!(!h.needs_update(3));
        h.set_language("RS");
        assert_eq!(h.language, "rs");
        assert!(h.needs_update(3), "language change forces re-highlight");

        // Same language again keeps the cache.
        h.highlight_document("a", 4, None);
        h.set_language("rs");
        assert!(!h.needs_update(4));
    }

    #[test]
    fn set_language_from_filename_handles_special_names() {
        let mut h = Highlighter::new();
        let cases = [
            ("Dockerfile", "dockerfile"),
            ("dockerfile.dev", "dockerfile"),
            ("Makefile", "makefile"),
            ("GNUmakefile", "makefile"),
            ("main.RS", "rs"),
            ("archive.tar.gz", "gz"),
            ("README", "readme"),
        ];
        for (name, lang) in cases {
            h.set_language_from_filename(name);
            assert_eq!(h.language, lang, "for {name}");
        }
    }

    #[test]
    fn invalidate_forces_update_and_drops_viewport() {
        let mut h = Highlighter::new();
        h.highlight_viewport("x", 0, 1, 7, (0, 10), None);
        assert!(!h.viewport_stale(7, 0, 10));
        h.invalidate();
        assert!(h.needs_update(7));
        assert!(h.viewport_stale(7, 0, 10));
    }

    #[test]
    fn viewport_stale_checks_version_and_range() {
        let mut h = Highlighter::new();
        assert!(h.viewport_stale(-1, 0, 1), "no viewport cached yet");
        h.highlight_viewport("a\nb", 10, 100, 2, (5, 50), None);
        assert!(!h.viewport_stale(2, 5, 50));
        assert!(!h.viewport_stale(2, 10, 40));
        assert!(h.viewport_stale(3, 10, 40), "content changed");
        assert!(h.viewport_stale(2, 4, 40), "scrolled above window");
        assert!(h.viewport_stale(2, 10, 51), "scrolled below window");
        // Full-document mode clears the viewport.
        h.highlight_document("a", 9, None);
        assert!(h.viewport_stale(9, 0, 1));
    }

    #[test]
    fn highlight_viewport_places_tokens_at_absolute_lines() {
        let doc = vec![
            vec![tok("fn", TokenKind::Keyword)],
            vec![tok("x", TokenKind::Number)],
            vec![tok("overflow", TokenKind::Comment)],
        ];
        let (pm, _) = manager(Some(doc), None);
        let mut h = fk_highlighter();
        h.highlight_viewport("fn\nx\noverflow", 3, 5, 1, (3, 5), Some(&pm));
        assert!(!h.needs_update(1));
        assert_eq!(h.line_tokens.len(), 5);
        assert!(h.line_tokens[..3].iter().all(Vec::is_empty));
        assert_eq!(kinds(&h.line_tokens[3]), vec![TokenKind::Keyword]);
        assert_eq!(kinds(&h.line_tokens[4]), vec![TokenKind::Number]);
    }

    #[test]
    fn highlight_viewport_without_plugin_leaves_lines_empty() {
        let mut h = fk_highlighter();
        h.highlight_viewport("a\nb", 0, 3, 1, (0, 3), None);
        assert_eq!(h.line_tokens.len(), 3);
        assert!(h.line_tokens.iter().all(Vec::is_empty));
    }

    #[test]
    fn highlight_document_without_plugins_is_plain_text() {
        let mut h = Highlighter::new();
        h.highlight_document("let x;\n\nfoo", 1, None);
        assert_eq!(h.line_tokens.len(), 3);
        assert_eq!(h.line_tokens[0].len(), 1);
        assert_eq!(h.line_tokens[0][0].text, "let x;");
        assert_eq!(h.line_tokens[0][0].kind, TokenKind::Normal);
        assert!(h.line_tokens[1].is_empty(), "empty line has no tokens");
    }

    #[test]
    fn highlight_document_is_noop_for_same_version() {
        let mut h = Highlighter::new();
        h.highlight_document("one", 5, None);
        h.highlight_document("two\nthree", 5, None);
        assert_eq!(h.line_tokens.len(), 1);
        assert_eq!(h.line_tokens[0][0].text, "one");
    }

    #[test]
    fn highlight_document_prefers_document_tokenizer() {
        let doc = vec![vec![
            tok("fn", TokenKind::Keyword),
            tok(" a", TokenKind::Normal),
        ]];
        let (pm, resets) = manager(Some(doc), Some(TokenKind::Comment));
        let mut h = fk_highlighter();
        h.highlight_document("fn a", 1, Some(&pm));
        assert_eq!(
            kinds(&h.line_tokens[0]),
            vec![TokenKind::Keyword, TokenKind::Normal]
        );
        assert_eq!(resets.load(Ordering::SeqCst), 0, "line path not taken");
    }

    #[test]
    fn highlight_document_falls_back_to_line_tokenizer() {
        // Empty document tokens count as "unsupported".
        let (pm, resets) = manager(Some(vec![]), Some(TokenKind::Comment));
        let mut h = fk_highlighter();
        h.highlight_document("// a\n// b", 1, Some(&pm));
        assert_eq!(resets.load(Ordering::SeqCst), 1);
        assert_eq!(h.line_tokens.len(), 2);
        assert_eq!(kinds(&h.line_tokens[1]), vec![TokenKind::Comment]);
        assert_eq!(h.line_tokens[1][0].text, "// b");
    }

    #[test]
    fn highlight_document_uses_plain_when_line_tokenizer_is_trivial() {
        // Line tokenizer only returns Normal tokens: treated as no highlighting.
        let (pm, resets) = manager(None, Some(TokenKind::Normal));
        let mut h = fk_highlighter();
        h.highlight_document("a\nb", 1, Some(&pm));
        assert_eq!(resets.load(Ordering::SeqCst), 1);
        assert_eq!(h.line_tokens.len(), 2);
        assert_eq!(kinds(&h.line_tokens[0]), vec![TokenKind::Normal]);

        // A plugin that doesn't handle the language falls back to plain per line.
        let (pm, resets) = manager(None, Some(TokenKind::Keyword));
        let mut h = Highlighter::new();
        h.set_language("other");
        h.highlight_document("x", 1, Some(&pm));
        assert_eq!(resets.load(Ordering::SeqCst), 0);
        assert_eq!(kinds(&h.line_tokens[0]), vec![TokenKind::Normal]);
    }

    #[test]
    fn tokens_for_line_uses_cache_then_plugin_then_plain() {
        let (pm, _) = manager(None, Some(TokenKind::Keyword));
        let mut h = fk_highlighter();
        h.highlight_document("cached", 1, Some(&pm));

        // Cache hit (ignores the passed text).
        let t = h.tokens_for_line(0, "ignored", None);
        assert_eq!(t[0].text, "cached");
        assert_eq!(t[0].kind, TokenKind::Keyword);

        // Cache miss: plugin tokenizer.
        let t = h.tokens_for_line(5, "fresh", Some(&pm));
        assert_eq!(t[0].text, "fresh");
        assert_eq!(t[0].kind, TokenKind::Keyword);

        // Cache miss, no plugin: plain.
        let t = h.tokens_for_line(5, "fresh", None);
        assert_eq!(kinds(&t), vec![TokenKind::Normal]);

        // Plugin that doesn't handle the language: plain.
        let (pm_none, _) = manager(None, None);
        let t = h.tokens_for_line(5, "fresh", Some(&pm_none));
        assert_eq!(kinds(&t), vec![TokenKind::Normal]);
    }

    #[test]
    fn tokenize_line_is_plain() {
        let h = Highlighter::new();
        let t = h.tokenize_line("fn main() {}");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].text, "fn main() {}");
        assert_eq!(t[0].kind, TokenKind::Normal);
        assert!(h.tokenize_line("").is_empty());
    }

    #[test]
    fn no_builtin_keywords() {
        assert!(keywords_for_language("rs").is_empty());
        assert!(keywords_for_language("").is_empty());
    }
}
