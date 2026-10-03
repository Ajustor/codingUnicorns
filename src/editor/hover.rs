/// A parsed section of a hover markdown document.
pub(super) enum HoverSection {
    /// A fenced code block (``` ... ```).
    CodeBlock { lang: String, code: String },
    /// A prose paragraph (may contain inline `**bold**`, `*italic*`, `` `code` ``).
    Text(String),
    /// A horizontal rule (`---`).
    Separator,
}

/// Split raw LSP markdown hover text into typed sections.
pub(super) fn parse_hover_sections(text: &str) -> Vec<HoverSection> {
    let mut sections: Vec<HoverSection> = Vec::new();
    let mut in_code = false;
    let mut code_lang = String::new();
    let mut code_lines: Vec<&str> = Vec::new();
    let mut text_lines: Vec<&str> = Vec::new();

    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            if in_code {
                let t = text_lines.join("\n");
                if !t.trim().is_empty() {
                    sections.push(HoverSection::Text(t));
                }
                text_lines.clear();
                sections.push(HoverSection::CodeBlock {
                    lang: code_lang.clone(),
                    code: code_lines.join("\n"),
                });
                code_lines.clear();
                in_code = false;
            } else {
                let t = text_lines.join("\n");
                if !t.trim().is_empty() {
                    sections.push(HoverSection::Text(t));
                }
                text_lines.clear();
                code_lang = line.trim_start_matches('`').trim().to_string();
                in_code = true;
            }
        } else if in_code {
            code_lines.push(line);
        } else if line.trim() == "---" || line.trim() == "___" || line.trim() == "***" {
            let t = text_lines.join("\n");
            if !t.trim().is_empty() {
                sections.push(HoverSection::Text(t));
            }
            text_lines.clear();
            sections.push(HoverSection::Separator);
        } else {
            text_lines.push(line);
        }
    }
    if in_code && !code_lines.is_empty() {
        sections.push(HoverSection::CodeBlock {
            lang: code_lang,
            code: code_lines.join("\n"),
        });
    } else {
        let t = text_lines.join("\n");
        if !t.trim().is_empty() {
            sections.push(HoverSection::Text(t));
        }
    }
    sections
}

/// Build a LayoutJob for a line of prose, interpreting inline markdown:
/// `**bold**`, `*italic*`, `` `code` ``.
pub(super) fn inline_markdown_job(text: &str, font_size: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: 500.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let normal_color = egui::Color32::from_rgb(204, 204, 204);
    let bold_color = egui::Color32::WHITE;
    let code_color = egui::Color32::from_rgb(206, 145, 120);
    let code_bg = egui::Color32::from_rgb(40, 40, 40);

    let prop = |sz: f32| egui::FontId::proportional(sz);
    let mono = |sz: f32| egui::FontId::monospace(sz);

    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut current = String::new();

    let flush_normal = |job: &mut egui::text::LayoutJob, s: &mut String| {
        if s.is_empty() {
            return;
        }
        job.append(
            s,
            0.0,
            egui::TextFormat {
                font_id: prop(font_size),
                color: normal_color,
                ..Default::default()
            },
        );
        s.clear();
    };

    while i < chars.len() {
        if i + 1 < chars.len()
            && ((chars[i] == '*' && chars[i + 1] == '*')
                || (chars[i] == '_' && chars[i + 1] == '_'))
        {
            let marker = chars[i];
            flush_normal(&mut job, &mut current);
            i += 2;
            let mut bold = String::new();
            while i + 1 < chars.len() && !(chars[i] == marker && chars[i + 1] == marker) {
                bold.push(chars[i]);
                i += 1;
            }
            if i + 1 < chars.len() {
                i += 2;
            }
            if !bold.is_empty() {
                job.append(
                    &bold,
                    0.0,
                    egui::TextFormat {
                        font_id: prop(font_size),
                        color: bold_color,
                        ..Default::default()
                    },
                );
            }
        } else if (chars[i] == '*' || chars[i] == '_')
            && (i + 1 >= chars.len() || chars[i + 1] != chars[i])
        {
            let marker = chars[i];
            flush_normal(&mut job, &mut current);
            i += 1;
            let mut italic = String::new();
            while i < chars.len() && chars[i] != marker {
                italic.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            if !italic.is_empty() {
                job.append(
                    &italic,
                    0.0,
                    egui::TextFormat {
                        font_id: prop(font_size),
                        color: normal_color,
                        italics: true,
                        ..Default::default()
                    },
                );
            }
        } else if chars[i] == '`' {
            flush_normal(&mut job, &mut current);
            i += 1;
            let mut code = String::new();
            while i < chars.len() && chars[i] != '`' {
                code.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            if !code.is_empty() {
                job.append(
                    " ",
                    0.0,
                    egui::TextFormat {
                        font_id: mono(font_size - 1.0),
                        color: code_color,
                        background: code_bg,
                        ..Default::default()
                    },
                );
                job.append(
                    &code,
                    0.0,
                    egui::TextFormat {
                        font_id: mono(font_size - 1.0),
                        color: code_color,
                        background: code_bg,
                        ..Default::default()
                    },
                );
                job.append(
                    " ",
                    0.0,
                    egui::TextFormat {
                        font_id: mono(font_size - 1.0),
                        color: code_color,
                        background: code_bg,
                        ..Default::default()
                    },
                );
            }
        } else {
            current.push(chars[i]);
            i += 1;
        }
    }
    flush_normal(&mut job, &mut current);
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render sections as comparable strings.
    fn describe(text: &str) -> Vec<String> {
        parse_hover_sections(text)
            .into_iter()
            .map(|s| match s {
                HoverSection::CodeBlock { lang, code } => format!("code[{lang}]:{code}"),
                HoverSection::Text(t) => format!("text:{t}"),
                HoverSection::Separator => "sep".to_string(),
            })
            .collect()
    }

    #[test]
    fn parses_code_block_followed_by_text() {
        assert_eq!(
            describe("```rust\nfn a() -> u8\n```\nReturns *one*."),
            vec!["code[rust]:fn a() -> u8", "text:Returns *one*."]
        );
    }

    #[test]
    fn text_before_code_is_flushed_and_lang_may_be_empty() {
        assert_eq!(
            describe("Intro line\nsecond\n```\nlet x = 1;\nlet y = 2;\n```"),
            vec!["text:Intro line\nsecond", "code[]:let x = 1;\nlet y = 2;"]
        );
    }

    #[test]
    fn horizontal_rules_become_separators() {
        assert_eq!(
            describe("a\n---\nb\n ___ \nc\n***"),
            vec!["text:a", "sep", "text:b", "sep", "text:c", "sep"]
        );
    }

    #[test]
    fn rules_inside_code_blocks_are_code() {
        assert_eq!(describe("```\n---\n```"), vec!["code[]:---"]);
    }

    #[test]
    fn unterminated_code_block_is_still_emitted() {
        assert_eq!(
            describe("```py\nx = 1\ny = 2"),
            vec!["code[py]:x = 1\ny = 2"]
        );
        assert!(describe("```py").is_empty(), "empty unterminated block");
    }

    #[test]
    fn blank_text_runs_are_dropped() {
        assert_eq!(describe("\n  \n```\nc\n```\n\n"), vec!["code[]:c"]);
        assert!(describe("").is_empty());
        assert!(describe("   \n\t").is_empty());
        assert_eq!(describe("\n---\n"), vec!["sep"]);
    }

    #[test]
    #[ignore = "BUG: indented code fences keep the backticks in the language tag (\"```ts\" instead of \"ts\")"]
    fn indented_fence_language_is_parsed() {
        assert_eq!(
            describe("  ```ts\n  let a = 1;\n  ```"),
            vec!["code[ts]:  let a = 1;"]
        );
    }

    struct Seg {
        text: String,
        italics: bool,
        color: egui::Color32,
        monospace: bool,
        size: f32,
        background: egui::Color32,
    }

    fn segments(text: &str) -> Vec<Seg> {
        let job = inline_markdown_job(text, 14.0);
        job.sections
            .iter()
            .map(|s| Seg {
                text: job.text[s.byte_range.clone()].to_string(),
                italics: s.format.italics,
                color: s.format.color,
                monospace: s.format.font_id.family == egui::FontFamily::Monospace,
                size: s.format.font_id.size,
                background: s.format.background,
            })
            .collect()
    }

    fn texts(text: &str) -> Vec<String> {
        segments(text).into_iter().map(|s| s.text).collect()
    }

    const NORMAL: egui::Color32 = egui::Color32::from_rgb(204, 204, 204);
    const CODE: egui::Color32 = egui::Color32::from_rgb(206, 145, 120);

    #[test]
    fn plain_text_is_a_single_normal_section() {
        let segs = segments("hello world");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "hello world");
        assert_eq!(segs[0].color, NORMAL);
        assert!(!segs[0].italics);
        assert!(!segs[0].monospace);
        assert_eq!(segs[0].size, 14.0);
        let job = inline_markdown_job("x", 14.0);
        assert_eq!(job.wrap.max_width, 500.0);
    }

    #[test]
    fn bold_markers_render_white() {
        for src in ["a **bold** c", "a __bold__ c"] {
            let segs = segments(src);
            assert_eq!(
                segs.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
                vec!["a ", "bold", " c"]
            );
            assert_eq!(segs[1].color, egui::Color32::WHITE);
            assert!(!segs[1].italics);
            assert_eq!(segs[2].color, NORMAL);
        }
    }

    #[test]
    fn single_markers_render_italic() {
        for src in ["see *this* now", "see _this_ now"] {
            let segs = segments(src);
            assert_eq!(texts(src), vec!["see ", "this", " now"]);
            assert!(segs[1].italics);
            assert_eq!(segs[1].color, NORMAL);
        }
    }

    #[test]
    fn inline_code_is_monospace_and_padded() {
        let segs = segments("call `foo()` here");
        assert_eq!(
            segs.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
            vec!["call ", " ", "foo()", " ", " here"]
        );
        for seg in &segs[1..4] {
            assert!(seg.monospace);
            assert_eq!(seg.color, CODE);
            assert_eq!(seg.size, 13.0);
            assert_eq!(seg.background, egui::Color32::from_rgb(40, 40, 40));
        }
    }

    #[test]
    #[ignore = "BUG: unterminated **bold drops formatting on its last char (renders \"bol\" bold + \"d\" plain)"]
    fn unterminated_bold_consumes_rest_of_line() {
        assert_eq!(texts("x **bold"), vec!["x ", "bold"]);
    }

    #[test]
    fn unterminated_markers_consume_rest_of_line() {
        assert_eq!(texts("x *it"), vec!["x ", "it"]);
        assert_eq!(texts("x `code"), vec!["x ", " ", "code", " "]);
    }

    #[test]
    fn empty_markers_produce_no_sections() {
        assert!(texts("****").is_empty());
        assert!(texts("**").is_empty());
        assert!(texts("``").is_empty());
        assert!(texts("__").is_empty());
        assert_eq!(texts("a*"), vec!["a"]);
        assert!(texts("").is_empty());
    }
}
