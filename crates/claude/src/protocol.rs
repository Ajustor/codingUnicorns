use serde_json::Value;

/// A parsed event from `claude -p --output-format stream-json --verbose`.
#[derive(Debug, Clone, PartialEq)]
pub enum ClaudeEvent {
    /// First event of a turn; carries the session id to reuse via `--resume`.
    Init { session_id: String },
    /// A complete assistant text block.
    AssistantText(String),
    /// Claude is invoking a tool.
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// Final event of the turn.
    Result {
        text: String,
        cost_usd: f64,
        session_id: Option<String>,
    },
}

/// Parse one stdout line into zero or more events.
/// Unknown/irrelevant lines yield an empty vec (forward-compatible).
pub fn parse_line(line: &str) -> Vec<ClaudeEvent> {
    let line = line.trim();
    if line.is_empty() {
        return vec![];
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    match v.get("type").and_then(|t| t.as_str()) {
        Some("system") if v.get("subtype").and_then(|s| s.as_str()) == Some("init") => {
            match v.get("session_id").and_then(|s| s.as_str()) {
                Some(id) => vec![ClaudeEvent::Init {
                    session_id: id.to_string(),
                }],
                None => vec![],
            }
        }
        Some("assistant") => parse_assistant(&v),
        Some("result") => vec![ClaudeEvent::Result {
            text: v
                .get("result")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string(),
            cost_usd: v
                .get("total_cost_usd")
                .and_then(|c| c.as_f64())
                .unwrap_or(0.0),
            session_id: v
                .get("session_id")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string()),
        }],
        _ => vec![],
    }
}

fn parse_assistant(v: &Value) -> Vec<ClaudeEvent> {
    let Some(content) = v
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    else {
        return vec![];
    };
    let mut out = Vec::new();
    for block in content {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    if !t.is_empty() {
                        out.push(ClaudeEvent::AssistantText(t.to_string()));
                    }
                }
            }
            Some("tool_use") => out.push(ClaudeEvent::ToolUse {
                id: block
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("")
                    .to_string(),
                name: block
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string(),
                input: block.get("input").cloned().unwrap_or(Value::Null),
            }),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_init_session_id() {
        let line = r#"{"type":"system","subtype":"init","session_id":"abc123","model":"x"}"#;
        assert_eq!(
            parse_line(line),
            vec![ClaudeEvent::Init {
                session_id: "abc123".into()
            }]
        );
    }

    #[test]
    fn parses_assistant_text_and_tool_use() {
        let line = r#"{"type":"assistant","message":{"content":[
            {"type":"text","text":"Hello"},
            {"type":"tool_use","id":"t1","name":"Edit","input":{"file":"a.rs"}}
        ]}}"#;
        let evs = parse_line(line);
        assert_eq!(evs[0], ClaudeEvent::AssistantText("Hello".into()));
        match &evs[1] {
            ClaudeEvent::ToolUse { name, .. } => assert_eq!(name, "Edit"),
            other => panic!("expected tool_use, got {other:?}"),
        }
    }

    #[test]
    fn parses_result() {
        let line = r#"{"type":"result","result":"done","total_cost_usd":0.01,"session_id":"abc"}"#;
        assert_eq!(
            parse_line(line),
            vec![ClaudeEvent::Result {
                text: "done".into(),
                cost_usd: 0.01,
                session_id: Some("abc".into())
            }]
        );
    }

    #[test]
    fn ignores_unknown_and_blank() {
        assert!(parse_line("").is_empty());
        assert!(parse_line("not json").is_empty());
        assert!(parse_line(r#"{"type":"stream_event"}"#).is_empty());
    }
}
