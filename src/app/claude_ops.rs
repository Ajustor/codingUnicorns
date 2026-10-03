use super::CodingUnicorns;

impl CodingUnicorns {
    /// Drain streamed events from the active turn, surface pending permission
    /// requests, and detect turn completion (clean result OR process death).
    pub(crate) fn poll_claude(&mut self, ctx: &egui::Context) {
        if let Some(turn) = self.claude_turn.take() {
            let mut got_event = false;
            let mut alive = true;
            loop {
                match turn.rx.try_recv() {
                    Ok(ev) => {
                        self.claude_session.apply(ev);
                        got_event = true;
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        alive = false;
                        break;
                    }
                }
            }
            if alive && self.claude_session.running {
                // Still streaming — keep the handle for next frame.
                self.claude_turn = Some(turn);
                if got_event {
                    // More output may be arriving — repaint now.
                    ctx.request_repaint();
                } else {
                    // Idle wait: poll again soon without busy-spinning at full FPS.
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
            } else {
                // Finished. If the process died without a final `result`,
                // clear running and note it. Dropping `turn` reaps the child.
                if self.claude_session.running {
                    self.claude_session.running = false;
                    self.claude_session
                        .transcript
                        .push(claude::session::Message {
                            role: claude::session::Role::Assistant,
                            text: "(claude ended without a result — see logs)".to_string(),
                        });
                }
                ctx.request_repaint();
            }
        }

        // Surface one pending permission request at a time. Auto-approve read-only
        // tools when `claude_auto_allow_read` is set, so they never prompt.
        if self.claude_pending.is_none() {
            if let Some(perm) = &self.claude_perm {
                if let Ok(req) = perm.requests.try_recv() {
                    let auto_allow = self.config.claude_auto_allow_read
                        && matches!(req.tool.as_str(), "Read" | "Glob" | "Grep" | "LS");
                    if auto_allow {
                        let _ = req.reply.send(claude::permission::Decision::Allow);
                    } else {
                        self.claude_pending = Some(req);
                    }
                    ctx.request_repaint();
                }
            }
        }

        // Lazily resolve the active Claude account (once) while the panel is open,
        // off the UI thread, for the header indicator.
        if self.show_claude && self.claude_account.is_none() {
            match &self.claude_account_rx {
                None => {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let binary = self.config.claude_binary.clone();
                    std::thread::spawn(move || {
                        let label = claude::account::fetch_account(&binary)
                            .unwrap_or_else(|| "unknown".to_string());
                        let _ = tx.send(label);
                    });
                    self.claude_account_rx = Some(rx);
                }
                Some(rx) => {
                    if let Ok(label) = rx.try_recv() {
                        self.claude_account = Some(label);
                        self.claude_account_rx = None;
                        ctx.request_repaint();
                    }
                }
            }
        }
    }

    /// Launch one conversation turn for `user_text`.
    pub(crate) fn start_claude_turn(&mut self, user_text: String) {
        use claude::{permission, process, session::EditorContext};

        let Some(workspace) = self.workspace_path.clone() else {
            return;
        };
        // Ensure a permission listener exists for this session.
        if self.claude_perm.is_none() {
            let token = uuid::Uuid::new_v4().to_string();
            match permission::PermissionListener::start(token) {
                Ok(l) => self.claude_perm = Some(l),
                Err(e) => {
                    log::error!("claude permission listener: {e}");
                    return;
                }
            }
        }
        let perm = self.claude_perm.as_ref().unwrap();
        let self_exe = std::env::current_exe().unwrap_or_default();
        let mcp_config =
            match permission::write_mcp_config(&std::env::temp_dir(), &self_exe, perm.port) {
                Ok(p) => p,
                Err(e) => {
                    log::error!("claude mcp-config: {e}");
                    return;
                }
            };

        let ctx = EditorContext {
            current_file: self
                .editor
                .current_path
                .as_ref()
                .map(|p| p.display().to_string()),
            selection: self.editor.selected_text_pub(),
            selection_lines: self.editor.selection_line_range_pub(),
        };
        let prompt = claude::session::ClaudeSession::build_prompt(&user_text, &ctx);

        self.claude_session
            .transcript
            .push(claude::session::Message {
                role: claude::session::Role::User,
                text: user_text,
            });
        self.claude_session.running = true;

        let req = process::TurnRequest {
            binary: self.config.claude_binary.clone(),
            prompt,
            workspace,
            session_id: self.claude_session.session_id.clone(),
            perm_port: perm.port,
            perm_token: perm.token.clone(),
            mcp_config,
            model: self.claude_model.clone(),
        };
        match process::spawn_turn(&req) {
            Ok(turn) => self.claude_turn = Some(turn),
            Err(e) => {
                self.claude_session.running = false;
                self.claude_session
                    .transcript
                    .push(claude::session::Message {
                        role: claude::session::Role::Assistant,
                        text: format!(
                            "Failed to launch `{}`: {e}. Is Claude Code installed and on PATH?",
                            self.config.claude_binary
                        ),
                    });
            }
        }
    }

    /// Route a panel input beginning with `/`. Local commands control the
    /// session/panel; any other `/command` is forwarded to `claude` (slash
    /// skills resolve server-side). `text` keeps its leading `/`.
    pub(crate) fn handle_claude_slash(&mut self, text: String) {
        let trimmed = text.trim().to_string();
        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or("").to_ascii_lowercase();
        let arg = parts
            .next()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        match name.as_str() {
            "/clear" | "/new" | "/reset" => {
                self.claude_session.reset();
                self.claude_pending = None;
                self.claude_turn = None;
                self.push_claude_note("Conversation cleared.".to_string());
            }
            "/model" => match arg {
                Some(m) => {
                    self.push_claude_note(format!("Model set to `{m}` for upcoming turns."));
                    self.claude_model = Some(m);
                }
                None => {
                    self.claude_model = None;
                    self.push_claude_note("Model override cleared (using default).".to_string());
                }
            },
            "/help" => self.push_claude_note(
                "Local commands: /clear, /model <name> (empty to reset), /help. \
                 Any other /command is forwarded to Claude as a skill."
                    .to_string(),
            ),
            // Unknown locally → forward to Claude so skills (/review, /commit, …) work.
            _ => self.start_claude_turn(text),
        }
    }

    /// Push a system/tool note into the transcript (panel-local message).
    fn push_claude_note(&mut self, text: String) {
        self.claude_session
            .transcript
            .push(claude::session::Message {
                role: claude::session::Role::Tool,
                text,
            });
    }
}
