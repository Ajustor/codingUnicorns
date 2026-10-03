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
}
