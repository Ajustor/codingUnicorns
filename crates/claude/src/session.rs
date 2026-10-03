use super::protocol::ClaudeEvent;

#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct EditorContext {
    /// Workspace-relative or absolute path of the current file, if any.
    pub current_file: Option<String>,
    /// Selected text, if any.
    pub selection: Option<String>,
    /// 1-based inclusive selection line range, if a selection exists.
    pub selection_lines: Option<(usize, usize)>,
}

#[derive(Default)]
pub struct ClaudeSession {
    pub transcript: Vec<Message>,
    pub session_id: Option<String>,
    pub running: bool,
    pub cost_usd: f64,
}

impl ClaudeSession {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the full prompt sent to `claude`: a compact context header (only the
    /// parts that exist) followed by the user's message.
    pub fn build_prompt(user_text: &str, ctx: &EditorContext) -> String {
        let mut header = String::new();
        if let Some(file) = &ctx.current_file {
            header.push_str(&format!("[current file: {file}]\n"));
        }
        if let (Some(sel), Some((a, b))) = (&ctx.selection, ctx.selection_lines) {
            header.push_str(&format!("[selection lines {a}-{b}]\n```\n{sel}\n```\n"));
        }
        if header.is_empty() {
            user_text.to_string()
        } else {
            format!("{header}\n{user_text}")
        }
    }

    /// Start a new conversation (drop session continuity + transcript).
    pub fn reset(&mut self) {
        self.transcript.clear();
        self.session_id = None;
        self.cost_usd = 0.0;
        self.running = false;
    }

    /// Apply one streamed event to the transcript / session state.
    pub fn apply(&mut self, ev: ClaudeEvent) {
        match ev {
            ClaudeEvent::Init { session_id } => {
                if self.session_id.is_none() {
                    self.session_id = Some(session_id);
                }
            }
            ClaudeEvent::AssistantText(text) => {
                self.transcript.push(Message {
                    role: Role::Assistant,
                    text,
                });
            }
            ClaudeEvent::ToolUse { name, input, .. } => {
                self.transcript.push(Message {
                    role: Role::Tool,
                    text: format!("{name} {input}"),
                });
            }
            ClaudeEvent::Result {
                cost_usd,
                session_id,
                ..
            } => {
                self.cost_usd += cost_usd;
                if let Some(id) = session_id {
                    self.session_id = Some(id);
                }
                self.running = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_with_no_context_is_message_only() {
        let ctx = EditorContext::default();
        assert_eq!(ClaudeSession::build_prompt("hi", &ctx), "hi");
    }

    #[test]
    fn prompt_includes_file_and_selection() {
        let ctx = EditorContext {
            current_file: Some("src/a.rs".into()),
            selection: Some("let x = 1;".into()),
            selection_lines: Some((10, 10)),
        };
        let p = ClaudeSession::build_prompt("explain", &ctx);
        assert!(p.contains("[current file: src/a.rs]"));
        assert!(p.contains("[selection lines 10-10]"));
        assert!(p.contains("let x = 1;"));
        assert!(p.trim_end().ends_with("explain"));
    }

    #[test]
    fn init_sets_session_id_once() {
        let mut s = ClaudeSession::new();
        s.apply(ClaudeEvent::Init {
            session_id: "first".into(),
        });
        s.apply(ClaudeEvent::Init {
            session_id: "second".into(),
        });
        assert_eq!(s.session_id.as_deref(), Some("first"));
    }

    #[test]
    fn result_clears_running_and_adds_cost() {
        let mut s = ClaudeSession::new();
        s.running = true;
        s.apply(ClaudeEvent::Result {
            text: "ok".into(),
            cost_usd: 0.02,
            session_id: None,
        });
        assert!(!s.running);
        assert!((s.cost_usd - 0.02).abs() < 1e-9);
    }
    #[test]
    fn prompt_with_file_only() {
        let ctx = EditorContext {
            current_file: Some("main.rs".into()),
            ..Default::default()
        };
        assert_eq!(
            ClaudeSession::build_prompt("fix it", &ctx),
            "[current file: main.rs]\n\nfix it"
        );
    }

    #[test]
    fn prompt_exact_layout_with_selection() {
        let ctx = EditorContext {
            current_file: Some("a.rs".into()),
            selection: Some("x".into()),
            selection_lines: Some((3, 5)),
        };
        assert_eq!(
            ClaudeSession::build_prompt("q", &ctx),
            "[current file: a.rs]\n[selection lines 3-5]\n```\nx\n```\n\nq"
        );
    }

    #[test]
    fn selection_without_line_range_is_omitted() {
        let ctx = EditorContext {
            current_file: None,
            selection: Some("text".into()),
            selection_lines: None,
        };
        assert_eq!(ClaudeSession::build_prompt("hi", &ctx), "hi");
        let ctx = EditorContext {
            current_file: None,
            selection: None,
            selection_lines: Some((1, 2)),
        };
        assert_eq!(ClaudeSession::build_prompt("hi", &ctx), "hi");
    }

    #[test]
    fn selection_only_has_header() {
        let ctx = EditorContext {
            current_file: None,
            selection: Some("s".into()),
            selection_lines: Some((1, 2)),
        };
        assert_eq!(
            ClaudeSession::build_prompt("go", &ctx),
            "[selection lines 1-2]\n```\ns\n```\n\ngo"
        );
    }

    #[test]
    fn new_session_is_empty() {
        let s = ClaudeSession::new();
        assert!(s.transcript.is_empty());
        assert!(s.session_id.is_none());
        assert!(!s.running);
        assert_eq!(s.cost_usd, 0.0);
    }

    #[test]
    fn assistant_text_and_tool_use_append_to_transcript() {
        let mut s = ClaudeSession::new();
        s.apply(ClaudeEvent::AssistantText("hello".into()));
        s.apply(ClaudeEvent::ToolUse {
            id: "t".into(),
            name: "Edit".into(),
            input: serde_json::json!({"file":"a.rs"}),
        });
        assert_eq!(s.transcript.len(), 2);
        assert_eq!(s.transcript[0].role, Role::Assistant);
        assert_eq!(s.transcript[0].text, "hello");
        assert_eq!(s.transcript[1].role, Role::Tool);
        assert_eq!(s.transcript[1].text, r#"Edit {"file":"a.rs"}"#);
    }

    #[test]
    fn result_session_id_overrides_and_costs_accumulate() {
        let mut s = ClaudeSession::new();
        s.apply(ClaudeEvent::Init {
            session_id: "init".into(),
        });
        s.running = true;
        s.apply(ClaudeEvent::Result {
            text: String::new(),
            cost_usd: 0.5,
            session_id: Some("resumed".into()),
        });
        assert_eq!(s.session_id.as_deref(), Some("resumed"));
        s.running = true;
        s.apply(ClaudeEvent::Result {
            text: String::new(),
            cost_usd: 0.25,
            session_id: None,
        });
        // None keeps the previous id
        assert_eq!(s.session_id.as_deref(), Some("resumed"));
        assert!((s.cost_usd - 0.75).abs() < 1e-9);
        assert!(!s.running);
        // Result does not add a transcript entry
        assert!(s.transcript.is_empty());
    }

    #[test]
    fn reset_clears_everything() {
        let mut s = ClaudeSession::new();
        s.apply(ClaudeEvent::Init {
            session_id: "id".into(),
        });
        s.apply(ClaudeEvent::AssistantText("x".into()));
        s.cost_usd = 1.0;
        s.running = true;
        s.reset();
        assert!(s.transcript.is_empty());
        assert!(s.session_id.is_none());
        assert_eq!(s.cost_usd, 0.0);
        assert!(!s.running);
        // After reset a new Init is accepted again.
        s.apply(ClaudeEvent::Init {
            session_id: "new".into(),
        });
        assert_eq!(s.session_id.as_deref(), Some("new"));
    }
}
