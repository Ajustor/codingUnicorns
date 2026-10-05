//! On-disk text format of an open file: line ending and UTF-8 BOM. The buffer
//! always holds LF-only text without a BOM; the original format is detected on
//! load and re-applied on save.

const BOM: char = '\u{FEFF}';

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    #[default]
    Lf,
    Crlf,
}

impl LineEnding {
    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::Crlf => "CRLF",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextFormat {
    pub line_ending: LineEnding,
    pub bom: bool,
}

/// Detect the format of `content` and return it normalized: BOM stripped and
/// every CRLF turned into LF. The line ending is the majority one (ties and
/// files without line breaks are LF). Lone CRs are left untouched.
pub fn normalize(content: String) -> (String, TextFormat) {
    let (bom, body) = match content.strip_prefix(BOM) {
        Some(rest) => (true, rest),
        None => (false, content.as_str()),
    };
    let crlf = body.matches("\r\n").count();
    let lf = body.matches('\n').count() - crlf;
    let line_ending = if crlf > lf {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    };
    let format = TextFormat { line_ending, bom };
    let text = if crlf > 0 {
        body.replace("\r\n", "\n")
    } else if bom {
        body.to_string()
    } else {
        content
    };
    (text, format)
}

/// Bytes to write for LF-only `text` in `format`.
pub fn encode(text: &str, format: TextFormat) -> Vec<u8> {
    let mut out = String::with_capacity(text.len() + 3);
    if format.bom {
        out.push(BOM);
    }
    match format.line_ending {
        LineEnding::Lf => out.push_str(text),
        LineEnding::Crlf => out.push_str(&text.replace('\n', "\r\n")),
    }
    out.into_bytes()
}

/// Decode file bytes as UTF-8. Invalid UTF-8 falls back to lossy decoding
/// (invalid sequences become U+FFFD) and returns `true` as the second value so
/// the caller can warn that saving would not round-trip the original bytes.
pub fn decode(bytes: Vec<u8>) -> (String, bool) {
    match String::from_utf8(bytes) {
        Ok(s) => (s, false),
        Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), true),
    }
}

/// Read a text file: UTF-8 with a lossy fallback (see `decode`). BOM and line
/// endings are left in place — `Editor::set_content` normalizes them.
pub fn read_text_file(path: &std::path::Path) -> std::io::Result<(String, bool)> {
    Ok(decode(std::fs::read(path)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(line_ending: LineEnding, bom: bool) -> TextFormat {
        TextFormat { line_ending, bom }
    }

    #[test]
    fn plain_lf_text_is_unchanged() {
        let (t, f) = normalize("a\nb\n".to_string());
        assert_eq!(t, "a\nb\n");
        assert_eq!(f, TextFormat::default());
        assert_eq!(normalize(String::new()).1, TextFormat::default());
    }

    #[test]
    fn crlf_is_detected_and_stripped() {
        let (t, f) = normalize("a\r\nb\r\n".to_string());
        assert_eq!(t, "a\nb\n");
        assert_eq!(f, fmt(LineEnding::Crlf, false));
    }

    #[test]
    fn mixed_endings_pick_the_majority_and_ties_pick_lf() {
        let (t, f) = normalize("a\r\nb\r\nc\nd".to_string());
        assert_eq!(t, "a\nb\nc\nd");
        assert_eq!(f.line_ending, LineEnding::Crlf);
        assert_eq!(
            normalize("a\r\nb\n".to_string()).1.line_ending,
            LineEnding::Lf
        );
        assert_eq!(
            normalize("a\nb\nc\r\n".to_string()).1.line_ending,
            LineEnding::Lf
        );
    }

    #[test]
    fn bom_is_detected_and_stripped() {
        let (t, f) = normalize("\u{FEFF}x\r\ny".to_string());
        assert_eq!(t, "x\ny");
        assert_eq!(f, fmt(LineEnding::Crlf, true));
        let (t, f) = normalize("\u{FEFF}x".to_string());
        assert_eq!(t, "x");
        assert_eq!(f, fmt(LineEnding::Lf, true));
    }

    #[test]
    fn encode_reapplies_line_ending_and_bom() {
        assert_eq!(encode("a\nb", TextFormat::default()), b"a\nb");
        assert_eq!(
            encode("a\nb\n", fmt(LineEnding::Crlf, false)),
            b"a\r\nb\r\n"
        );
        assert_eq!(
            encode("a\nb", fmt(LineEnding::Crlf, true)),
            b"\xEF\xBB\xBFa\r\nb"
        );
    }

    #[test]
    fn normalize_then_encode_round_trips() {
        for src in ["x\r\ny\r\n", "\u{FEFF}x\ny", "\u{FEFF}é\r\n", "no newline"] {
            let (t, f) = normalize(src.to_string());
            assert_eq!(encode(&t, f), src.as_bytes(), "{src:?}");
        }
    }

    #[test]
    fn decode_falls_back_to_lossy_on_invalid_utf8() {
        assert_eq!(decode(b"ok".to_vec()), ("ok".to_string(), false));
        let (s, lossy) = decode(b"caf\xE9\n".to_vec());
        assert!(lossy);
        assert_eq!(s, "caf\u{FFFD}\n");
    }

    #[test]
    fn read_text_file_reads_and_errors_on_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.txt");
        std::fs::write(&p, b"\xFF\xFEa").unwrap();
        let (s, lossy) = read_text_file(&p).unwrap();
        assert!(lossy);
        assert!(s.ends_with('a'));
        assert!(read_text_file(&dir.path().join("missing")).is_err());
    }
}
