use std::collections::BTreeMap;
use std::ffi::{c_char, CStr, CString};
use std::path::Path;

use super::manifest::{ExtensionManifest, PanelSpec};
use crate::editor::highlight::{Token, TokenKind};
use crate::plugin::{Plugin, PluginContext, PluginResponse};

type StrFn2 = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_char;
type StrFn3 = unsafe extern "C" fn(*const c_char, *const c_char, *const c_char) -> *mut c_char;

/// A language plugin loaded from a compiled `.so`/`.dll` extension.
///
/// Wraps the five C FFI functions exported by each language module:
/// `language_id`, `file_extensions`, `tokenize_line_ffi`, `hover_info_ffi`, `free_string`.
pub struct FfiLangPlugin {
    // Must be kept alive — dropping it would unload the library.
    _lib: libloading::Library,
    language_id: String,
    extensions: Vec<String>,
    lsp_server: Option<String>,
    lsp_args: Vec<String>,
    /// Per-language servers (`[capabilities.lsp_servers]`).
    lsp_servers: BTreeMap<String, (String, Vec<String>)>,
    /// Interfaces declared in the manifest (`[[panels]]`).
    panels: Vec<PanelSpec>,
    /// `tokenize_line_lang_ffi(lang, line)`: preferred over `tokenize_line_ffi`
    /// by modules handling several languages.
    tokenize_line_lang_fn: Option<StrFn2>,
    /// `tokenize_document_lang_ffi(lang, text)`.
    tokenize_document_lang_fn: Option<StrFn2>,
    /// `hover_info_lang_ffi(lang, word, content)`.
    hover_lang_fn: Option<StrFn3>,
    /// `ui_view_ffi(panel_id)`: JSON view of a panel.
    ui_view_fn: Option<unsafe extern "C" fn(*const c_char) -> *mut c_char>,
    /// `ui_event_ffi(panel_id, event_json)`: JSON actions, or null.
    ui_event_fn: Option<StrFn2>,
    tokenize_fn: Option<unsafe extern "C" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char>,
    free_fn: Option<unsafe extern "C" fn(*mut std::ffi::c_char)>,
    hover_fn: Option<
        unsafe extern "C" fn(
            *const std::ffi::c_char,
            *const std::ffi::c_char,
        ) -> *mut std::ffi::c_char,
    >,
    reset_tokenizer_fn: Option<unsafe extern "C" fn()>,
    tokenize_document_fn:
        Option<unsafe extern "C" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char>,
    tokenize_document_tsx_fn:
        Option<unsafe extern "C" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char>,
}

// Safety: all raw fn pointers are Send + Sync as long as the underlying C functions are
// thread-safe, which they are by convention for these pure-computation FFI modules.
unsafe impl Send for FfiLangPlugin {}
unsafe impl Sync for FfiLangPlugin {}

impl FfiLangPlugin {
    /// Load a language extension from `lib_path`; language servers and
    /// panels come from its `manifest`.
    pub fn load(lib_path: &Path, manifest: &ExtensionManifest) -> anyhow::Result<Self> {
        let caps = &manifest.capabilities;
        let lsp_server = caps.lsp_server.clone();
        let lsp_args = caps.lsp_args.clone();
        let lsp_servers = caps
            .lsp_servers
            .iter()
            .map(|(lang, s)| (lang.clone(), (s.command.clone(), s.args.clone())))
            .collect();
        let panels = manifest.panels.clone();
        unsafe {
            let lib = libloading::Library::new(lib_path)?;

            // language_id() → static C string (no allocation, no free needed)
            let language_id: String = {
                let sym: libloading::Symbol<unsafe extern "C" fn() -> *const std::ffi::c_char> =
                    lib.get(b"language_id\0")?;
                let ptr = sym();
                if ptr.is_null() {
                    anyhow::bail!("language_id() returned null");
                }
                CStr::from_ptr(ptr).to_str()?.to_string()
            };

            // file_extensions() → comma-separated static C string (e.g. "py,pyw")
            let extensions: Vec<String> = {
                let sym: libloading::Symbol<unsafe extern "C" fn() -> *const std::ffi::c_char> =
                    lib.get(b"file_extensions\0")?;
                let ptr = sym();
                if ptr.is_null() {
                    vec![]
                } else {
                    CStr::from_ptr(ptr)
                        .to_str()
                        .unwrap_or("")
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                }
            };

            // Optional symbols — missing symbols are non-fatal.
            let tokenize_fn = lib
                .get::<unsafe extern "C" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char>(
                    b"tokenize_line_ffi\0",
                )
                .ok()
                .map(|s| *s);

            let free_fn = lib
                .get::<unsafe extern "C" fn(*mut std::ffi::c_char)>(b"free_string\0")
                .ok()
                .map(|s| *s);

            let hover_fn = lib
                .get::<unsafe extern "C" fn(
                    *const std::ffi::c_char,
                    *const std::ffi::c_char,
                ) -> *mut std::ffi::c_char>(b"hover_info_ffi\0")
                .ok()
                .map(|s| *s);

            let reset_tokenizer_fn = lib
                .get::<unsafe extern "C" fn()>(b"reset_tokenizer\0")
                .ok()
                .map(|s| *s);

            let tokenize_document_fn = lib
                .get::<unsafe extern "C" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char>(
                    b"tokenize_document_ffi\0",
                )
                .ok()
                .map(|s| *s);

            let tokenize_document_tsx_fn = lib
                .get::<unsafe extern "C" fn(*const std::ffi::c_char) -> *mut std::ffi::c_char>(
                    b"tokenize_document_tsx_ffi\0",
                )
                .ok()
                .map(|s| *s);

            let tokenize_line_lang_fn = lib
                .get::<StrFn2>(b"tokenize_line_lang_ffi\0")
                .ok()
                .map(|s| *s);
            let tokenize_document_lang_fn = lib
                .get::<StrFn2>(b"tokenize_document_lang_ffi\0")
                .ok()
                .map(|s| *s);
            let hover_lang_fn = lib.get::<StrFn3>(b"hover_info_lang_ffi\0").ok().map(|s| *s);
            let ui_view_fn = lib
                .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(b"ui_view_ffi\0")
                .ok()
                .map(|s| *s);
            let ui_event_fn = lib.get::<StrFn2>(b"ui_event_ffi\0").ok().map(|s| *s);

            Ok(Self {
                _lib: lib,
                language_id,
                extensions,
                lsp_server,
                lsp_args,
                lsp_servers,
                panels,
                tokenize_line_lang_fn,
                tokenize_document_lang_fn,
                hover_lang_fn,
                ui_view_fn,
                ui_event_fn,
                tokenize_fn,
                free_fn,
                hover_fn,
                reset_tokenizer_fn,
                tokenize_document_fn,
                tokenize_document_tsx_fn,
            })
        }
    }

    fn call_tokenize(&self, line: &str) -> Option<String> {
        let tokenize = self.tokenize_fn?;
        let free = self.free_fn?;
        let c_line = CString::new(line).ok()?;
        unsafe {
            let ptr = tokenize(c_line.as_ptr());
            if ptr.is_null() {
                return None;
            }
            let result = CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string());
            free(ptr);
            result
        }
    }

    fn call_tokenize_document(&self, text: &str, tsx: bool) -> Option<String> {
        let func = if tsx {
            self.tokenize_document_tsx_fn
                .or(self.tokenize_document_fn)?
        } else {
            self.tokenize_document_fn?
        };
        let free = self.free_fn?;
        let c_text = CString::new(text).ok()?;
        unsafe {
            let ptr = func(c_text.as_ptr());
            if ptr.is_null() {
                return None;
            }
            let result = CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string());
            free(ptr);
            result
        }
    }

    /// Call a string-returning FFI function with C string `args`; the result
    /// is copied and released with the module's `free_string`.
    fn call_str(
        &self,
        args: &[&str],
        f: impl FnOnce(&[*const c_char]) -> *mut c_char,
    ) -> Option<String> {
        let free = self.free_fn?;
        let c_args = args
            .iter()
            .map(|a| CString::new(*a))
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let ptrs: Vec<*const c_char> = c_args.iter().map(|c| c.as_ptr()).collect();
        let ptr = f(&ptrs);
        if ptr.is_null() {
            return None;
        }
        unsafe {
            let result = CStr::from_ptr(ptr).to_str().ok().map(str::to_string);
            free(ptr);
            result
        }
    }

    fn call_hover(&self, word: &str, content: &str) -> Option<String> {
        let hover = self.hover_fn?;
        let free = self.free_fn?;
        let c_word = CString::new(word).ok()?;
        let c_content = CString::new(content).ok()?;
        unsafe {
            let ptr = hover(c_word.as_ptr(), c_content.as_ptr());
            if ptr.is_null() {
                return None;
            }
            let result = CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string());
            free(ptr);
            result
        }
    }
}

impl Plugin for FfiLangPlugin {
    fn name(&self) -> &str {
        &self.language_id
    }

    fn tokenize_line(&self, lang: &str, line: &str) -> Option<Vec<Token>> {
        if !self.extensions.iter().any(|e| e == lang) {
            return None;
        }
        let json = match self.tokenize_line_lang_fn {
            Some(f) => self.call_str(&[lang, line], |a| unsafe { f(a[0], a[1]) })?,
            None => self.call_tokenize(line)?,
        };
        parse_token_json(&json)
    }

    fn tokenize_document(&self, lang: &str, text: &str) -> Option<Vec<Vec<Token>>> {
        if !self.extensions.iter().any(|e| e == lang) {
            return None;
        }
        let json = match self.tokenize_document_lang_fn {
            Some(f) => self.call_str(&[lang, text], |a| unsafe { f(a[0], a[1]) })?,
            None => self.call_tokenize_document(text, lang == "tsx" || lang == "jsx")?,
        };
        parse_document_json(&json)
    }

    fn hover_info(&self, lang: &str, word: &str, file_content: &str) -> Option<String> {
        if !self.extensions.iter().any(|e| e == lang) {
            return None;
        }
        match self.hover_lang_fn {
            Some(f) => self.call_str(&[lang, word, file_content], |a| unsafe {
                f(a[0], a[1], a[2])
            }),
            None => self.call_hover(word, file_content),
        }
    }

    fn ui_panels(&self) -> Vec<PanelSpec> {
        if self.ui_view_fn.is_some() {
            self.panels.clone()
        } else {
            Vec::new()
        }
    }

    fn ui_view(&self, panel_id: &str) -> Option<String> {
        let f = self.ui_view_fn?;
        self.panels.iter().find(|p| p.id == panel_id)?;
        self.call_str(&[panel_id], |a| unsafe { f(a[0]) })
    }

    fn ui_event(&self, panel_id: &str, event: &str) -> Option<String> {
        let f = self.ui_event_fn?;
        self.panels.iter().find(|p| p.id == panel_id)?;
        self.call_str(&[panel_id, event], |a| unsafe { f(a[0], a[1]) })
    }

    fn file_extensions(&self) -> &[&str] {
        // We can't return &[&str] from Vec<String> directly without a self-referential
        // borrow, so we leak a small slice once. Extensions are loaded once per process.
        // This is intentional — extension lifetimes match the process lifetime.
        Box::leak(
            self.extensions
                .iter()
                .map(|s| Box::leak(s.clone().into_boxed_str()) as &str)
                .collect::<Vec<&str>>()
                .into_boxed_slice(),
        )
    }

    fn reset_tokenizer(&self) {
        if let Some(reset) = self.reset_tokenizer_fn {
            unsafe { reset() };
        }
    }

    fn lsp_server_command(&self) -> Option<(String, Vec<String>)> {
        let server = self.lsp_server.clone()?;
        Some((server, self.lsp_args.clone()))
    }

    fn lsp_server_command_for(&self, lang: &str) -> Option<(String, Vec<String>)> {
        match self.lsp_servers.get(lang) {
            Some(cmd) => Some(cmd.clone()),
            None => self.lsp_server_command(),
        }
    }

    fn update(&mut self, _ctx: &PluginContext) -> PluginResponse {
        PluginResponse::default()
    }
}

// ── JSON token parser ─────────────────────────────────────────────────────────

/// Parse the JSON array returned by `tokenize_line_ffi`.
/// Format: `[{"text":"...","kind":"keyword"}, ...]`
fn parse_token_json(json: &str) -> Option<Vec<Token>> {
    let json = json.trim();
    if !json.starts_with('[') || !json.ends_with(']') {
        return None;
    }
    let inner = &json[1..json.len() - 1];
    let mut tokens = Vec::new();
    let mut rest = inner.trim();

    while !rest.is_empty() {
        // Each element is a JSON object: {"text":"...","kind":"..."}
        if !rest.starts_with('{') {
            break;
        }
        // Find the object's closing '}' while respecting quoted strings — a token's
        // text may itself contain '{' or '}' (C# braces, string interpolation, …),
        // so a naive find('}') would truncate the object and drop the whole line.
        let bytes = rest.as_bytes();
        let mut depth = 0usize;
        let mut in_str = false;
        let mut end = 0usize;
        let mut i = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'"' if !in_str => in_str = true,
                b'"' if in_str => in_str = false,
                b'\\' if in_str => i += 1, // skip escaped char
                b'{' if !in_str => depth += 1,
                b'}' if !in_str => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        if end == 0 {
            break; // malformed / no closing brace
        }
        let obj = &rest[1..end - 1]; // strip braces
        rest = rest[end..].trim_start_matches([',', ' ']);

        let text = extract_json_str(obj, "text")?;
        let kind_str = extract_json_str(obj, "kind")?;
        let kind = match kind_str.as_str() {
            "keyword" => TokenKind::Keyword,
            "type" => TokenKind::KeywordType,
            "string" => TokenKind::String,
            "comment" => TokenKind::Comment,
            "number" => TokenKind::Number,
            "function" => TokenKind::Function,
            "macro" => TokenKind::Macro,
            "property" => TokenKind::Property,
            "operator" => TokenKind::Operator,
            "class" => TokenKind::Class,
            _ => TokenKind::Normal,
        };
        tokens.push(Token { text, kind });
    }

    Some(tokens)
}

/// Parse the JSON array-of-arrays returned by `tokenize_document_ffi`.
/// Format: `[[{"text":"...","kind":"..."}, ...], ...]`
fn parse_document_json(json: &str) -> Option<Vec<Vec<Token>>> {
    let json = json.trim();
    if !json.starts_with('[') || !json.ends_with(']') {
        return None;
    }
    // The outer array contains line arrays. We find each inner [...] by bracket matching.
    let bytes = json.as_bytes();
    let mut lines = Vec::new();
    let mut i = 1; // skip outer '['
    while i < bytes.len() {
        // Skip whitespace and commas
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b',' || bytes[i] == b'\n') {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b']' {
            break;
        }
        if bytes[i] != b'[' {
            break;
        }
        // Find matching ']' respecting nested strings
        let start = i;
        let mut depth = 0;
        let mut in_str = false;
        while i < bytes.len() {
            match bytes[i] {
                b'"' if !in_str => in_str = true,
                b'"' if in_str => in_str = false,
                b'\\' if in_str => {
                    i += 1;
                } // skip escaped char
                b'[' if !in_str => depth += 1,
                b']' if !in_str => {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        let line_json = &json[start..i];
        lines.push(parse_token_json(line_json).unwrap_or_default());
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

/// Extract a string value for `key` from a flat JSON object body (no nested objects).
fn extract_json_str(obj: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\":", key);
    let start = obj.find(&needle)? + needle.len();
    let rest = obj[start..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let rest = &rest[1..]; // skip opening quote
    let mut result = String::new();
    let mut chars = rest.chars();
    loop {
        match chars.next()? {
            '"' => break,
            '\\' => match chars.next()? {
                '"' => result.push('"'),
                '\\' => result.push('\\'),
                'n' => result.push('\n'),
                'r' => result.push('\r'),
                't' => result.push('\t'),
                c => result.push(c),
            },
            c => result.push(c),
        }
    }
    Some(result)
}

#[cfg(test)]
mod token_parse_tests {
    use super::*;

    #[test]
    fn token_text_with_braces_does_not_truncate_line() {
        // A token whose text is "}" (block close, C# interpolation, …) must not
        // truncate object parsing and drop the rest of the line's tokens.
        let json = r#"[{"text":"a","kind":"keyword"},{"text":"}","kind":"normal"},{"text":"\"s\"","kind":"string"}]"#;
        let toks = parse_token_json(json).expect("should parse");
        assert_eq!(toks.len(), 3, "all tokens recovered despite '}}' in text");
        assert_eq!(toks[1].text, "}");
        assert!(toks.iter().any(|t| t.kind == TokenKind::String));
    }

    #[test]
    fn interpolated_string_keeps_string_tokens() {
        let json = r#"[{"text":"$","kind":"string"},{"text":"\"hi \"","kind":"string"},{"text":"{","kind":"normal"},{"text":"name","kind":"normal"},{"text":"}","kind":"normal"},{"text":" x\"","kind":"string"}]"#;
        let toks = parse_token_json(json).expect("should parse");
        assert_eq!(toks.len(), 6);
        assert_eq!(
            toks.iter().filter(|t| t.kind == TokenKind::String).count(),
            3
        );
    }

    #[test]
    fn all_kinds_are_mapped() {
        let json = r#"[{"text":"a","kind":"keyword"},{"text":"b","kind":"type"},{"text":"c","kind":"string"},{"text":"d","kind":"comment"},{"text":"e","kind":"number"},{"text":"f","kind":"function"},{"text":"g","kind":"macro"},{"text":"h","kind":"weird"},{"text":"i","kind":"property"},{"text":"j","kind":"operator"},{"text":"k","kind":"class"}]"#;
        let kinds: Vec<TokenKind> = parse_token_json(json)
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Keyword,
                TokenKind::KeywordType,
                TokenKind::String,
                TokenKind::Comment,
                TokenKind::Number,
                TokenKind::Function,
                TokenKind::Macro,
                TokenKind::Normal,
                TokenKind::Property,
                TokenKind::Operator,
                TokenKind::Class,
            ]
        );
    }

    #[test]
    fn whitespace_and_empty_arrays() {
        assert_eq!(parse_token_json("  []  ").unwrap().len(), 0);
        let toks = parse_token_json(
            r#"[ {"text": "x", "kind": "number"} , {"text":"y","kind":"normal"} ]"#,
        )
        .unwrap();
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].text, "x");
        assert_eq!(toks[0].kind, TokenKind::Number);
    }

    #[test]
    fn non_array_input_is_rejected() {
        assert!(parse_token_json("").is_none());
        assert!(parse_token_json("{}").is_none());
        assert!(parse_token_json("[{").is_none());
        assert!(parse_token_json("null").is_none());
    }

    #[test]
    fn malformed_elements_stop_or_fail_parsing() {
        // A non-object element stops parsing; earlier tokens are kept.
        let toks = parse_token_json(r#"[{"text":"a","kind":"keyword"},42]"#).unwrap();
        assert_eq!(toks.len(), 1);
        // Unterminated object → stop.
        let toks = parse_token_json(r#"[{"text":"a","kind":"keyword"},{"text":"b"]"#).unwrap();
        assert_eq!(toks.len(), 1);
        // Missing required key → whole line rejected.
        assert!(parse_token_json(r#"[{"text":"a"}]"#).is_none());
        assert!(parse_token_json(r#"[{"kind":"keyword"}]"#).is_none());
        // Non-string value → rejected.
        assert!(parse_token_json(r#"[{"text":1,"kind":"keyword"}]"#).is_none());
    }

    #[test]
    fn extract_json_str_unescapes() {
        let obj = r#""text":"a\"b\\c\nd\re\tf\/g","kind":"x""#;
        assert_eq!(extract_json_str(obj, "text").unwrap(), "a\"b\\c\nd\re\tf/g");
        assert_eq!(extract_json_str(obj, "kind").unwrap(), "x");
        assert!(extract_json_str(obj, "missing").is_none());
        assert!(extract_json_str(r#""text":"unterminated"#, "text").is_none());
        assert!(extract_json_str(r#""text":"bad escape\"#, "text").is_none());
        assert_eq!(
            extract_json_str(r#""text":   "spaced""#, "text").unwrap(),
            "spaced"
        );
    }

    #[test]
    fn parse_document_json_lines() {
        let json = "[[{\"text\":\"fn\",\"kind\":\"keyword\"}],\n[],[{\"text\":\"]\",\"kind\":\"normal\"},{\"text\":\"\\\"\",\"kind\":\"string\"}]]";
        let lines = parse_document_json(json).unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0][0].text, "fn");
        assert!(lines[1].is_empty());
        assert_eq!(
            lines[2][0].text, "]",
            "bracket inside string does not end the line"
        );
        assert_eq!(lines[2][1].text, "\"");
    }

    #[test]
    fn parse_document_json_edge_cases() {
        assert!(parse_document_json("[]").is_none());
        assert!(parse_document_json("  [ ]  ").is_none());
        assert!(parse_document_json("nope").is_none());
        assert!(parse_document_json("[[").is_none());
        // Non-array element stops parsing.
        let lines = parse_document_json(r#"[[{"text":"a","kind":"normal"}], 5]"#).unwrap();
        assert_eq!(lines.len(), 1);
        // A malformed line becomes an empty token list rather than failing the document.
        let lines =
            parse_document_json(r#"[[{"text":"a"}],[{"text":"b","kind":"normal"}]]"#).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].is_empty());
        assert_eq!(lines[1][0].text, "b");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_char;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static RESETS: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn tokenize(line: *const c_char) -> *mut c_char {
        let s = CStr::from_ptr(line).to_str().unwrap();
        if s == "null" {
            return std::ptr::null_mut();
        }
        CString::new(format!(r#"[{{"text":"{s}","kind":"keyword"}}]"#))
            .unwrap()
            .into_raw()
    }

    unsafe extern "C" fn document(text: *const c_char) -> *mut c_char {
        let s = CStr::from_ptr(text).to_str().unwrap();
        let lines: Vec<String> = s
            .lines()
            .map(|l| format!(r#"[{{"text":"{l}","kind":"normal"}}]"#))
            .collect();
        CString::new(format!("[{}]", lines.join(",")))
            .unwrap()
            .into_raw()
    }

    unsafe extern "C" fn document_tsx(_text: *const c_char) -> *mut c_char {
        CString::new(r#"[[{"text":"tsx","kind":"keyword"}]]"#)
            .unwrap()
            .into_raw()
    }

    unsafe extern "C" fn null_doc(_text: *const c_char) -> *mut c_char {
        std::ptr::null_mut()
    }

    unsafe extern "C" fn hover(word: *const c_char, content: *const c_char) -> *mut c_char {
        let w = CStr::from_ptr(word).to_str().unwrap();
        let c = CStr::from_ptr(content).to_str().unwrap();
        if w == "missing" {
            return std::ptr::null_mut();
        }
        CString::new(format!("{w} in {} bytes", c.len()))
            .unwrap()
            .into_raw()
    }

    unsafe extern "C" fn free_string(p: *mut c_char) {
        drop(CString::from_raw(p));
    }

    unsafe extern "C" fn reset() {
        RESETS.fetch_add(1, Ordering::SeqCst);
    }

    unsafe extern "C" fn ui_view(panel: *const c_char) -> *mut c_char {
        let p = CStr::from_ptr(panel).to_str().unwrap();
        CString::new(format!(
            r#"{{"children":[{{"type":"text","text":"{p}"}}]}}"#
        ))
        .unwrap()
        .into_raw()
    }

    unsafe extern "C" fn ui_event(panel: *const c_char, ev: *const c_char) -> *mut c_char {
        let p = CStr::from_ptr(panel).to_str().unwrap();
        let e = CStr::from_ptr(ev).to_str().unwrap();
        CString::new(format!("{p}|{e}")).unwrap().into_raw()
    }

    unsafe extern "C" fn line_lang(lang: *const c_char, line: *const c_char) -> *mut c_char {
        let l = CStr::from_ptr(lang).to_str().unwrap();
        let s = CStr::from_ptr(line).to_str().unwrap();
        CString::new(format!(r#"[{{"text":"{l}:{s}","kind":"string"}}]"#))
            .unwrap()
            .into_raw()
    }

    unsafe extern "C" fn doc_lang(lang: *const c_char, _text: *const c_char) -> *mut c_char {
        let l = CStr::from_ptr(lang).to_str().unwrap();
        CString::new(format!(r#"[[{{"text":"{l}","kind":"keyword"}}]]"#))
            .unwrap()
            .into_raw()
    }

    unsafe extern "C" fn hover_lang(
        lang: *const c_char,
        word: *const c_char,
        _content: *const c_char,
    ) -> *mut c_char {
        let l = CStr::from_ptr(lang).to_str().unwrap();
        let w = CStr::from_ptr(word).to_str().unwrap();
        CString::new(format!("{l}/{w}")).unwrap().into_raw()
    }

    #[test]
    fn language_aware_exports_receive_the_language() {
        let mut p = full_plugin();
        p.tokenize_line_lang_fn = Some(line_lang);
        p.tokenize_document_lang_fn = Some(doc_lang);
        p.hover_lang_fn = Some(hover_lang);
        assert_eq!(p.tokenize_line("tl", "x").unwrap()[0].text, "tl:x");
        assert_eq!(p.tokenize_document("tsx", "x").unwrap()[0][0].text, "tsx");
        assert_eq!(p.hover_info("jsx", "w", "").as_deref(), Some("jsx/w"));
        assert!(p.tokenize_line("rs", "x").is_none(), "unhandled language");
    }

    #[test]
    fn per_language_server_overrides_the_module_server() {
        let p = full_plugin();
        assert_eq!(p.lsp_server_command_for("tsx").unwrap().0, "tsx-lsp");
        assert_eq!(p.lsp_server_command_for("tl").unwrap().0, "tl-lsp");
        assert!(bare_plugin().lsp_server_command_for("tl").is_none());
    }

    #[test]
    fn panels_are_routed_to_the_ui_exports() {
        let p = full_plugin();
        assert_eq!(p.ui_panels().len(), 1);
        assert!(p.ui_view("tl.panel").unwrap().contains("tl.panel"));
        assert_eq!(p.ui_event("tl.panel", "{}").as_deref(), Some("tl.panel|{}"));
        assert!(p.ui_view("other").is_none(), "undeclared panel");
        assert!(p.ui_event("other", "{}").is_none());
        // Panels declared without the exports are not offered.
        let mut p = full_plugin();
        p.ui_view_fn = None;
        assert!(p.ui_panels().is_empty());
        assert!(bare_plugin().ui_view("tl.panel").is_none());
    }

    /// A handle to the running test binary — keeps `_lib` valid without a real extension.
    fn this_lib() -> libloading::Library {
        #[cfg(windows)]
        let l = libloading::os::windows::Library::this().unwrap();
        #[cfg(unix)]
        let l = libloading::os::unix::Library::this();
        l.into()
    }

    fn full_plugin() -> FfiLangPlugin {
        FfiLangPlugin {
            _lib: this_lib(),
            language_id: "testlang".into(),
            extensions: vec!["tl".into(), "tsx".into(), "jsx".into()],
            lsp_server: Some("tl-lsp".into()),
            lsp_args: vec!["--stdio".into()],
            lsp_servers: BTreeMap::from([("tsx".to_string(), ("tsx-lsp".to_string(), vec![]))]),
            panels: vec![PanelSpec {
                id: "tl.panel".into(),
                title: "TL".into(),
                icon: None,
                location: Default::default(),
            }],
            tokenize_line_lang_fn: None,
            tokenize_document_lang_fn: None,
            hover_lang_fn: None,
            ui_view_fn: Some(ui_view),
            ui_event_fn: Some(ui_event),
            tokenize_fn: Some(tokenize),
            free_fn: Some(free_string),
            hover_fn: Some(hover),
            reset_tokenizer_fn: Some(reset),
            tokenize_document_fn: Some(document),
            tokenize_document_tsx_fn: Some(document_tsx),
        }
    }

    fn bare_plugin() -> FfiLangPlugin {
        FfiLangPlugin {
            _lib: this_lib(),
            language_id: "bare".into(),
            extensions: vec!["tl".into()],
            lsp_server: None,
            lsp_args: vec![],
            lsp_servers: BTreeMap::new(),
            panels: vec![],
            tokenize_line_lang_fn: None,
            tokenize_document_lang_fn: None,
            hover_lang_fn: None,
            ui_view_fn: None,
            ui_event_fn: None,
            tokenize_fn: None,
            free_fn: None,
            hover_fn: None,
            reset_tokenizer_fn: None,
            tokenize_document_fn: None,
            tokenize_document_tsx_fn: None,
        }
    }

    #[test]
    fn metadata_comes_from_fields() {
        let p = full_plugin();
        assert_eq!(p.name(), "testlang");
        assert_eq!(p.file_extensions(), &["tl", "tsx", "jsx"]);
        assert_eq!(
            p.lsp_server_command(),
            Some(("tl-lsp".to_string(), vec!["--stdio".to_string()]))
        );
        assert!(bare_plugin().lsp_server_command().is_none());
    }

    #[test]
    fn tokenize_line_calls_ffi_for_handled_languages() {
        let p = full_plugin();
        let toks = p.tokenize_line("tl", "let").unwrap();
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].text, "let");
        assert_eq!(toks[0].kind, TokenKind::Keyword);
        assert!(p.tokenize_line("rs", "let").is_none(), "unhandled language");
        assert!(
            p.tokenize_line("tl", "null").is_none(),
            "null pointer result"
        );
        assert!(p.tokenize_line("tl", "nul\0byte").is_none(), "interior NUL");
    }

    #[test]
    fn tokenize_document_selects_tsx_variant() {
        let p = full_plugin();
        let lines = p.tokenize_document("tl", "a\nb").unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1][0].text, "b");
        for lang in ["tsx", "jsx"] {
            let lines = p.tokenize_document(lang, "x").unwrap();
            assert_eq!(lines[0][0].text, "tsx");
        }
        assert!(p.tokenize_document("rs", "x").is_none());
        assert!(p.tokenize_document("tl", "bad\0").is_none());
    }

    #[test]
    fn tsx_falls_back_to_plain_document_tokenizer() {
        let mut p = full_plugin();
        p.tokenize_document_tsx_fn = None;
        let lines = p.tokenize_document("tsx", "plain").unwrap();
        assert_eq!(lines[0][0].text, "plain");
        p.tokenize_document_fn = Some(null_doc);
        assert!(p.tokenize_document("tl", "x").is_none());
    }

    #[test]
    fn hover_info_calls_ffi() {
        let p = full_plugin();
        assert_eq!(
            p.hover_info("tl", "foo", "abcd").as_deref(),
            Some("foo in 4 bytes")
        );
        assert!(p.hover_info("tl", "missing", "").is_none());
        assert!(p.hover_info("rs", "foo", "").is_none());
        assert!(p.hover_info("tl", "f\0o", "").is_none());
        assert!(p.hover_info("tl", "foo", "c\0").is_none());
    }

    #[test]
    fn missing_symbols_disable_features() {
        let p = bare_plugin();
        assert!(p.tokenize_line("tl", "x").is_none());
        assert!(p.tokenize_document("tl", "x").is_none());
        assert!(p.tokenize_document("tsx", "x").is_none());
        assert!(p.hover_info("tl", "x", "").is_none());
        p.reset_tokenizer(); // no-op without the symbol

        // A producer without a matching free function is never called (would leak).
        let mut p = full_plugin();
        p.free_fn = None;
        assert!(p.tokenize_line("tl", "x").is_none());
        assert!(p.tokenize_document("tl", "x").is_none());
        assert!(p.hover_info("tl", "x", "").is_none());
    }

    #[test]
    fn reset_tokenizer_calls_ffi() {
        let p = full_plugin();
        let before = RESETS.load(Ordering::SeqCst);
        p.reset_tokenizer();
        assert_eq!(RESETS.load(Ordering::SeqCst), before + 1);
    }

    #[test]
    fn update_returns_empty_response() {
        let mut p = full_plugin();
        let ctx = PluginContext {
            buffer_text: "",
            filename: None,
            cursor_row: 0,
            cursor_col: 0,
            is_modified: false,
            hovered_word: None,
        };
        let r = p.update(&ctx);
        assert!(r.status_text.is_none() && r.notifications.is_empty());
    }

    fn test_manifest() -> ExtensionManifest {
        ExtensionManifest::parse(
            "[extension]\nid = \"t\"\nname = \"T\"\nversion = \"1.0.0\"\ndescription = \"\"\n",
        )
        .unwrap()
    }

    /// Loads a real module: `CU_E2E_MODULE_DIR` holds its `manifest.toml` and
    /// library (e.g. writing-unicorns-modules' docker-lang, built in release).
    #[test]
    #[ignore = "needs a built module (CU_E2E_MODULE_DIR)"]
    fn real_module_from_env() {
        let dir = std::path::PathBuf::from(std::env::var("CU_E2E_MODULE_DIR").unwrap());
        let manifest =
            ExtensionManifest::parse(&std::fs::read_to_string(dir.join("manifest.toml")).unwrap())
                .unwrap();
        let lib = super::super::registry::find_platform_lib(&dir).expect("module library");
        let p = FfiLangPlugin::load(&lib, &manifest).unwrap();
        for lang in &manifest.capabilities.languages {
            assert!(p.file_extensions().contains(&lang.as_str()), "{lang}");
            let (cmd, _) = p.lsp_server_command_for(lang).unwrap();
            println!("{lang}: LSP {cmd}");
        }
        // `lang|text`, with `\n` written as two characters.
        if let Ok(doc) = std::env::var("CU_E2E_DOC") {
            let (lang, text) = doc.split_once('|').unwrap();
            let lines = p
                .tokenize_document(lang, &text.replace("\\n", "\n"))
                .unwrap();
            println!("{lang} tokens: {lines:?}");
        }
        for panel in p.ui_panels() {
            let mut view = String::new();
            // The first view starts the work; later ones show its result.
            for _ in 0..40 {
                view = p.ui_view(&panel.id).unwrap();
                if view.contains("image(s)") || view.contains("\"error\"") {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            let parsed: super::super::ui_host::View = serde_json::from_str(&view).unwrap();
            let entries = view.matches("\"actions\":[{").count();
            println!(
                "{} ({:?}): {} nodes, {entries} entries with actions",
                panel.id,
                panel.location,
                parsed.children.len()
            );
        }
    }

    #[test]
    fn load_missing_library_fails() {
        let p = std::env::temp_dir().join(format!("cu-missing-{}.dll", uuid::Uuid::new_v4()));
        assert!(FfiLangPlugin::load(&p, &test_manifest()).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn load_library_without_language_id_fails() {
        assert!(FfiLangPlugin::load(Path::new("kernel32.dll"), &test_manifest()).is_err());
    }
}
