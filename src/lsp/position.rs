//! Columns of LSP positions. The editor counts characters (Unicode scalar
//! values); a server counts in the `positionEncoding` it picked at
//! initialization, UTF-16 code units unless it says otherwise (LSP 3.17). The
//! client offers UTF-32 (characters: nothing to convert) first, and otherwise
//! converts columns with the text of the line, as the server was sent it.

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};

use serde_json::Value;

use super::client::{normalized_path_key, uri_to_path};

/// The encodings the client accepts, preferred first
/// (`general.positionEncodings` of `initialize`).
pub const OFFERED: [&str; 2] = ["utf-32", "utf-16"];

/// How a server counts columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PositionEncoding {
    Utf8,
    #[default]
    Utf16,
    Utf32,
}

impl PositionEncoding {
    /// The encoding a server picked (`positionEncoding` of the capabilities
    /// it answers `initialize` with): UTF-16 when it says nothing.
    pub fn from_capabilities(capabilities: &Value) -> Self {
        match capabilities.get("positionEncoding").and_then(Value::as_str) {
            Some("utf-32") => Self::Utf32,
            Some("utf-8") => Self::Utf8,
            _ => Self::Utf16,
        }
    }

    fn units(self, c: char) -> u32 {
        match self {
            Self::Utf8 => c.len_utf8() as u32,
            Self::Utf16 => c.len_utf16() as u32,
            Self::Utf32 => 1,
        }
    }
}

/// Column `col` (characters) of `line` in `enc` code units. A column past the
/// end of the line keeps its excess.
pub fn encode_col(line: &str, col: u32, enc: PositionEncoding) -> u32 {
    let mut units = 0;
    let mut chars = 0;
    for c in line.chars().take(col as usize) {
        units += enc.units(c);
        chars += 1;
    }
    units + (col - chars)
}

/// Column `units` (`enc` code units) of `line` in characters. A column inside
/// a character (half a surrogate pair) lands after it; a column past the end
/// of the line keeps its excess.
pub fn decode_col(line: &str, units: u32, enc: PositionEncoding) -> u32 {
    let mut seen = 0;
    let mut chars = 0;
    for c in line.chars() {
        if seen >= units {
            return chars;
        }
        seen += enc.units(c);
        chars += 1;
    }
    chars + units.saturating_sub(seen)
}

/// How many documents keep their text here: the ones being edited. Positions
/// in other files are converted with the file on disk.
const KEPT_DOCS: usize = 8;

/// Text of the documents last sent to the server (`didOpen`, `didChange`),
/// most recent first.
#[derive(Default)]
pub struct OpenDocs(VecDeque<(String, String)>);

impl OpenDocs {
    pub fn set(&mut self, uri: &str, text: &str) {
        let key = normalized_path_key(uri);
        self.0.retain(|(k, _)| *k != key);
        self.0.push_front((key, text.to_string()));
        self.0.truncate(KEPT_DOCS);
    }

    fn get(&self, uri: &str) -> Option<&str> {
        let key = normalized_path_key(uri);
        self.0
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, text)| text.as_str())
    }

    /// Column `col` (characters) of `line` of document `uri`, in `enc` units.
    pub fn encode(&self, uri: &str, line: u32, col: u32, enc: PositionEncoding) -> u32 {
        if enc == PositionEncoding::Utf32 {
            return col;
        }
        match self.get(uri).and_then(|t| t.split('\n').nth(line as usize)) {
            Some(text) => encode_col(text, col, enc),
            None => col,
        }
    }
}

/// A document's text and the offset where each of its lines starts.
type Indexed<'a> = (Cow<'a, str>, Vec<usize>);

/// The lines that decode the positions of a batch of messages, from the open
/// documents or else the files on disk (each read once per batch).
pub struct Lines<'a> {
    enc: PositionEncoding,
    docs: &'a OpenDocs,
    /// By document; `None` when its columns need no conversion (ASCII text)
    /// or its text is unknown.
    cache: HashMap<String, Option<Indexed<'a>>>,
}

impl<'a> Lines<'a> {
    pub fn new(enc: PositionEncoding, docs: &'a OpenDocs) -> Self {
        Self {
            enc,
            docs,
            cache: HashMap::new(),
        }
    }

    fn load(docs: &'a OpenDocs, uri: &str) -> Option<Indexed<'a>> {
        let text = match docs.get(uri) {
            Some(text) => Cow::Borrowed(text),
            None => Cow::Owned(std::fs::read_to_string(uri_to_path(uri)).ok()?),
        };
        if text.is_ascii() {
            return None;
        }
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Some((text, starts))
    }

    fn decode(&mut self, uri: &str, line: u64, units: u64) -> u64 {
        let docs = self.docs;
        let entry = self
            .cache
            .entry(normalized_path_key(uri))
            .or_insert_with(|| Self::load(docs, uri));
        let Some((text, starts)) = entry else {
            return units;
        };
        let Some(&start) = starts.get(line as usize) else {
            return units;
        };
        let end = starts.get(line as usize + 1).map_or(text.len(), |&e| e - 1);
        decode_col(&text[start..end], units as u32, self.enc).into()
    }
}

/// Rewrite, in place, the column of every position under `value` from the
/// server's encoding to characters. A position belongs to the nearest
/// enclosing document (`uri`, `targetUri`, `textDocument.uri`, a key of a
/// workspace edit's `changes`), else to `uri`, the request's document. `data`
/// and `arguments` are left as they are: the server reads them back.
pub fn decode_positions(value: &mut Value, uri: Option<&str>, lines: &mut Lines) {
    match value {
        Value::Array(items) => {
            for item in items {
                decode_positions(item, uri, lines);
            }
        }
        Value::Object(map) => {
            let line = map.get("line").and_then(Value::as_u64);
            if let (Some(line), Some(units)) = (line, map.get("character").and_then(Value::as_u64))
            {
                if let Some(uri) = uri {
                    map.insert("character".into(), lines.decode(uri, line, units).into());
                }
                return;
            }
            let own = ["uri", "targetUri"]
                .iter()
                .find_map(|k| map.get(*k))
                .or_else(|| map.get("textDocument").and_then(|d| d.get("uri")))
                .and_then(Value::as_str)
                .map(str::to_string);
            let doc = own.as_deref().or(uri);
            for (key, v) in map.iter_mut() {
                match key.as_str() {
                    "data" | "arguments" => {}
                    // A location link's origin is in the request's document.
                    "originSelectionRange" => decode_positions(v, uri, lines),
                    "changes" if v.is_object() => {
                        for (edited, edits) in v.as_object_mut().into_iter().flatten() {
                            decode_positions(edits, Some(edited), lines);
                        }
                    }
                    _ => decode_positions(v, doc, lines),
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use PositionEncoding::*;

    #[test]
    fn encoding_comes_from_the_capabilities_and_defaults_to_utf16() {
        let caps = |e: &str| json!({ "positionEncoding": e });
        assert_eq!(PositionEncoding::from_capabilities(&caps("utf-32")), Utf32);
        assert_eq!(PositionEncoding::from_capabilities(&caps("utf-8")), Utf8);
        assert_eq!(PositionEncoding::from_capabilities(&caps("utf-16")), Utf16);
        assert_eq!(PositionEncoding::from_capabilities(&json!({})), Utf16);
    }

    #[test]
    fn columns_convert_both_ways() {
        // 😀 is 2 UTF-16 units and 4 UTF-8 bytes, é is 1 and 2.
        let line = "a😀é b";
        for (col, utf16, utf8) in [(0, 0, 0), (1, 1, 1), (2, 3, 5), (3, 4, 7), (5, 6, 9)] {
            assert_eq!(encode_col(line, col, Utf16), utf16, "utf-16 of {col}");
            assert_eq!(encode_col(line, col, Utf8), utf8, "utf-8 of {col}");
            assert_eq!(encode_col(line, col, Utf32), col);
            assert_eq!(decode_col(line, utf16, Utf16), col, "from utf-16 {utf16}");
            assert_eq!(decode_col(line, utf8, Utf8), col, "from utf-8 {utf8}");
            assert_eq!(decode_col(line, col, Utf32), col);
        }
        // Inside the emoji: after it.
        assert_eq!(decode_col(line, 2, Utf16), 2);
        // Past the end of the line: the excess is kept.
        assert_eq!(encode_col(line, 8, Utf16), 9);
        assert_eq!(decode_col(line, 9, Utf16), 8);
        assert_eq!(decode_col("", 3, Utf16), 3);
    }

    #[test]
    fn open_docs_encode_with_the_last_text_sent() {
        let mut docs = OpenDocs::default();
        docs.set("file:///a.rs", "let x;\nlet s = \"😀\"; y");
        assert_eq!(docs.encode("file:///a.rs", 1, 13, Utf16), 14);
        assert_eq!(docs.encode("file:///a.rs", 1, 13, Utf32), 13);
        assert_eq!(docs.encode("file:///a.rs", 0, 4, Utf16), 4);
        // Unknown document or line: unchanged.
        assert_eq!(docs.encode("file:///b.rs", 1, 13, Utf16), 13);
        assert_eq!(docs.encode("file:///a.rs", 7, 13, Utf16), 13);
        docs.set("file:///a.rs", "plain");
        assert_eq!(docs.encode("file:///a.rs", 1, 13, Utf16), 13);
    }

    #[test]
    fn open_docs_keep_the_most_recent_few() {
        let mut docs = OpenDocs::default();
        for i in 0..KEPT_DOCS + 2 {
            docs.set(&format!("file:///{i}.rs"), "😀");
        }
        assert!(docs.get("file:///0.rs").is_none());
        assert!(docs.get("file:///2.rs").is_some());
        assert_eq!(docs.0.len(), KEPT_DOCS);
    }

    #[test]
    fn decode_positions_follows_the_document_of_each_range() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("other.rs");
        std::fs::write(&other, "x\r\n😀😀 z\n").unwrap();
        let other_uri = crate::lsp::client::path_to_uri(&other);
        let mut docs = OpenDocs::default();
        docs.set("file:///a.rs", "é = 1; w");
        let pos = |line, character| json!({ "line": line, "character": character });
        let range = |line, s, e| json!({ "start": pos(line, s), "end": pos(line, e) });
        let mut result = json!({
            // Positions without a document: the request's.
            "range": range(0, 7, 8),
            "items": [
                { "uri": other_uri, "range": range(1, 5, 6) },
                { "targetUri": other_uri, "targetRange": range(1, 4, 4),
                  "originSelectionRange": range(0, 7, 8) },
                { "textDocument": { "uri": other_uri }, "edits": [{ "range": range(1, 2, 4) }] }
            ],
            "edit": { "changes": { other_uri.clone(): [{ "range": range(1, 4, 6) }] } },
            "data": range(0, 7, 8),
            "command": { "arguments": [range(0, 7, 8)] }
        });
        let mut lines = Lines::new(Utf16, &docs);
        decode_positions(&mut result, Some("file:///a.rs"), &mut lines);
        assert_eq!(result["range"], range(0, 7, 8), "é is 1 UTF-16 unit");
        assert_eq!(result["items"][0]["range"], range(1, 3, 4));
        assert_eq!(result["items"][1]["targetRange"], range(1, 2, 2));
        assert_eq!(result["items"][1]["originSelectionRange"], range(0, 7, 8));
        assert_eq!(result["items"][2]["edits"][0]["range"], range(1, 1, 2));
        assert_eq!(
            result["edit"]["changes"][&other_uri][0]["range"],
            range(1, 2, 4)
        );
        // Read back by the server: untouched.
        assert_eq!(result["data"], range(0, 7, 8));
        assert_eq!(result["command"]["arguments"][0], range(0, 7, 8));

        // UTF-8 on the open document: é is 2 bytes.
        let mut result = json!({ "range": range(0, 8, 9) });
        decode_positions(
            &mut result,
            Some("file:///a.rs"),
            &mut Lines::new(Utf8, &docs),
        );
        assert_eq!(result["range"], range(0, 7, 8));
    }

    #[test]
    fn decode_positions_leaves_unknown_documents_and_lines_alone() {
        let docs = OpenDocs::default();
        let mut lines = Lines::new(Utf16, &docs);
        let original = json!({ "uri": "file:///definitely/missing.rs",
            "range": { "start": { "line": 3, "character": 9 }, "end": { "line": 3, "character": 12 } } });
        let mut v = original.clone();
        decode_positions(&mut v, None, &mut lines);
        assert_eq!(v, original);
        let mut v = json!({ "line": 3, "character": 9 });
        decode_positions(&mut v, None, &mut lines);
        assert_eq!(v["character"], 9, "no document at all");
    }
}
