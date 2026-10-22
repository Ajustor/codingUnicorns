//! "Report an Issue" dialog: sends a bug report or an idea to the relay in
//! `relay/notion-reports`, which files it on the project's Notion board.

use crate::ui::theme::Palette;
use std::sync::mpsc::Receiver;

/// Relay endpoint (the Cloudflare Worker of `relay/notion-reports`). The
/// `report_url` config setting overrides it; empty turns reporting off.
pub const DEFAULT_REPORT_URL: &str =
    "https://coding-unicorns-reports.minecities.workers.dev/report";

/// Limits checked by the relay too; enforced here so the user sees them.
const MAX_TITLE: usize = 120;
const MAX_DESCRIPTION: usize = 8000;
const MAX_LOG: usize = 20000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bug,
    Idea,
}

impl Kind {
    fn wire(self) -> &'static str {
        match self {
            Kind::Bug => "bug",
            Kind::Idea => "idea",
        }
    }
}

/// What is sent to the relay.
#[derive(Debug, serde::Serialize)]
struct Payload<'a> {
    kind: &'static str,
    title: &'a str,
    description: &'a str,
    version: &'static str,
    os: String,
    log: String,
}

fn os_label() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

/// The last `max` characters of `s`, cut on a line start when possible.
fn tail_chars(s: &str, max: usize) -> &str {
    if s.chars().count() <= max {
        return s;
    }
    let start = s.char_indices().rev().nth(max - 1).map_or(0, |(i, _)| i);
    let cut = &s[start..];
    cut.find('\n').map_or(cut, |nl| &cut[nl + 1..])
}

/// Why the dialog can't send yet, if anything.
fn problem(title: &str, description: &str) -> Option<&'static str> {
    if title.trim().is_empty() {
        Some("Give the report a title.")
    } else if title.chars().count() > MAX_TITLE {
        Some("The title is too long (120 characters max).")
    } else if description.trim().is_empty() {
        Some("Describe the problem or the idea.")
    } else if description.chars().count() > MAX_DESCRIPTION {
        Some("The description is too long (8000 characters max).")
    } else {
        None
    }
}

/// POST `body` to `url`; the relay's error message on failure.
fn post(url: &str, body: String) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(crate::extension::remote_registry::tls_config())
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut resp = agent
        .post(url)
        .header(
            "User-Agent",
            concat!("coding-unicorns/", env!("CARGO_PKG_VERSION")),
        )
        .header("Content-Type", "application/json")
        .send(body)
        .map_err(|e| format!("Could not reach the report server: {e}"))?;
    if resp.status().is_success() {
        return Ok(());
    }
    let status = resp.status();
    let text = resp
        .body_mut()
        .with_config()
        .limit(16 * 1024)
        .read_to_string()
        .unwrap_or_default();
    Err(relay_error(&text).unwrap_or_else(|| format!("The report server answered {status}.")))
}

/// The `error` field of a relay response.
fn relay_error(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get("error")?.as_str().map(str::to_string)
}

pub struct ReportDialog {
    pub open: bool,
    kind: Kind,
    title: String,
    description: String,
    attach_log: bool,
    sending: Option<Receiver<Result<(), String>>>,
    error: Option<String>,
    /// Give the title the focus on the next frame (just opened).
    focus_title: bool,
}

impl Default for ReportDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl ReportDialog {
    pub fn new() -> Self {
        Self {
            open: false,
            kind: Kind::Bug,
            title: String::new(),
            description: String::new(),
            attach_log: true,
            sending: None,
            error: None,
            focus_title: false,
        }
    }

    pub fn open(&mut self) {
        self.open = true;
        self.error = None;
        self.focus_title = true;
    }

    fn send(&mut self, url: &str, ctx: &egui::Context) {
        let log = if self.attach_log {
            tail_chars(&crate::launch_log::tail(300), MAX_LOG).to_string()
        } else {
            String::new()
        };
        let payload = Payload {
            kind: self.kind.wire(),
            title: self.title.trim(),
            description: self.description.trim(),
            version: env!("CARGO_PKG_VERSION"),
            os: os_label(),
            log,
        };
        let body = serde_json::to_string(&payload).unwrap_or_default();
        let (tx, rx) = std::sync::mpsc::channel();
        let url = url.to_string();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(post(&url, body));
            ctx.request_repaint();
        });
        self.sending = Some(rx);
        self.error = None;
    }

    /// Draws the dialog. Returns a message to toast once a report was sent.
    pub fn show(&mut self, ctx: &egui::Context, palette: Palette, url: &str) -> Option<String> {
        if !self.open {
            return None;
        }
        let mut sent = None;
        if let Some(rx) = &self.sending {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    self.sending = None;
                    self.open = false;
                    self.title.clear();
                    self.description.clear();
                    sent = Some("Report sent, thank you!".to_string());
                }
                Ok(Err(e)) => {
                    self.sending = None;
                    self.error = Some(e);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.sending = None;
                    self.error = Some("The report could not be sent.".to_string());
                }
            }
        }
        let busy = self.sending.is_some();

        let modal = egui::Modal::new(egui::Id::new("report_issue")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading("Report an Issue");
            ui.add_space(6.0);
            if url.is_empty() {
                ui.colored_label(
                    palette.warning,
                    "Issue reporting isn't set up in this build (report_url in the config).",
                );
                ui.add_space(6.0);
            }
            // Picking the kind moves the focus off the fields: hand it back to
            // the title, where typing goes on (as when the dialog opens).
            let picked = ui
                .horizontal(|ui| {
                    let bug = ui.selectable_value(&mut self.kind, Kind::Bug, "🐛 Bug");
                    let idea = ui.selectable_value(&mut self.kind, Kind::Idea, "💡 Idea");
                    bug.clicked() || idea.clicked()
                })
                .inner;
            ui.add_space(6.0);
            ui.label("Title");
            let title = ui.add_enabled(
                !busy,
                egui::TextEdit::singleline(&mut self.title)
                    .char_limit(MAX_TITLE)
                    .desired_width(f32::INFINITY)
                    .hint_text(match self.kind {
                        Kind::Bug => "What goes wrong, in a few words",
                        Kind::Idea => "What you'd like, in a few words",
                    }),
            );
            if picked || std::mem::take(&mut self.focus_title) {
                title.request_focus();
            }
            ui.add_space(6.0);
            ui.label(match self.kind {
                Kind::Bug => "What did you do, what happened, what did you expect?",
                Kind::Idea => "Describe the idea and what it would help with",
            });
            ui.add_enabled(
                !busy,
                egui::TextEdit::multiline(&mut self.description)
                    .char_limit(MAX_DESCRIPTION)
                    .desired_rows(8)
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(6.0);
            ui.add_enabled(
                !busy,
                egui::Checkbox::new(&mut self.attach_log, "Attach the launch log"),
            )
            .on_hover_text("The last lines of the log written at startup: paths, modules, errors.");
            ui.label(
                egui::RichText::new(format!(
                    "Also sent: version {} · {}. Don't include passwords or other secrets.",
                    env!("CARGO_PKG_VERSION"),
                    os_label()
                ))
                .small()
                .color(palette.text_faint),
            );
            let blocker = problem(&self.title, &self.description);
            if let Some(e) = &self.error {
                ui.add_space(4.0);
                ui.colored_label(palette.error, e);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    self.open = false;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if busy { "Sending…" } else { "Send" };
                    let enabled = !busy && blocker.is_none() && !url.is_empty();
                    let send = ui
                        .add_enabled(
                            enabled,
                            egui::Button::new(egui::RichText::new(label).color(palette.on_accent))
                                .fill(palette.accent),
                        )
                        .on_disabled_hover_text(blocker.unwrap_or(""));
                    if send.clicked() {
                        self.send(url, ui.ctx());
                    }
                    if busy {
                        ui.spinner();
                    }
                });
            });
        });
        // Esc or a click outside closes, except while sending.
        if modal.should_close() && !busy {
            self.open = false;
        }
        sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_matches_what_the_relay_expects() {
        let p = Payload {
            kind: Kind::Idea.wire(),
            title: "t",
            description: "d",
            version: "1.2.3",
            os: "linux x86_64".into(),
            log: String::new(),
        };
        let v: serde_json::Value = serde_json::to_value(&p).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        for k in ["kind", "title", "description", "version", "os", "log"] {
            assert!(keys.contains(&k), "{k} missing");
        }
        assert_eq!(v["kind"], "idea");
    }

    #[test]
    fn a_report_needs_a_title_and_a_description() {
        assert!(problem("", "d").is_some());
        assert!(problem("t", "  ").is_some());
        assert!(problem(&"x".repeat(MAX_TITLE + 1), "d").is_some());
        assert!(problem("t", "d").is_none());
    }

    #[test]
    fn log_tail_keeps_the_end_on_a_line_boundary() {
        let log = "first line\nsecond line\nthird line";
        assert_eq!(tail_chars(log, 100), log);
        assert_eq!(tail_chars(log, 15), "third line");
        assert!(tail_chars(&"é".repeat(50), 10).chars().count() <= 10);
    }

    #[test]
    fn relay_errors_are_shown_as_sent() {
        assert_eq!(
            relay_error(r#"{"error":"Too many reports"}"#).as_deref(),
            Some("Too many reports")
        );
        assert_eq!(relay_error("<html>"), None);
    }
}
