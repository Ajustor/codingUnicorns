mod ansi;
mod colors;
mod screen_buffer;
mod shell;

use ansi::AnsiPerformer;
use screen_buffer::{Cell, DEFAULT_FG};
pub use shell::list_available_shells;
use shell::resolve_shell;

use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::Color32;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use vte::Parser;

/// Initial PTY / screen size, used until the first frame measures the panel.
const INITIAL_ROWS: u16 = 50;
const INITIAL_COLS: u16 = 200;
/// Smallest grid the PTY is ever resized to.
const MIN_ROWS: u16 = 2;
const MIN_COLS: u16 = 10;
/// Upper bound guarding against absurd allocations on huge/odd viewports.
const MAX_GRID: u16 = 1000;

const FONT_SIZE: f32 = 13.5;
const LINE_HEIGHT: f32 = 13.5;

/// Handles to a spawned PTY session.
struct Pty {
    rx: Receiver<Vec<u8>>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
}

pub struct Terminal {
    pub shell_name: String,
    performer: AnsiPerformer,
    rx: Option<Receiver<Vec<u8>>>,
    writer: Option<Box<dyn Write + Send>>,
    parser: Parser,
    _child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    /// PTY master, kept to propagate size changes to the child process.
    master: Option<Box<dyn MasterPty + Send>>,
    /// Last (rows, cols) applied to the PTY and screen buffer.
    grid_size: (u16, u16),
    /// Set to true when new output arrives — triggers a one-shot scroll to bottom.
    needs_scroll: bool,
    /// Whether this terminal has keyboard focus.
    focused: bool,
}

impl Terminal {
    fn from_pty(shell_name: String, pty: Option<Pty>, banner: &str, focused: bool) -> Self {
        let mut parser = Parser::new();
        let mut performer = AnsiPerformer::new();
        for byte in banner.bytes() {
            parser.advance(&mut performer, byte);
        }
        let (rx, writer, child, master) = match pty {
            Some(p) => (Some(p.rx), Some(p.writer), Some(p.child), Some(p.master)),
            None => (None, None, None, None),
        };
        Self {
            shell_name,
            performer,
            rx,
            writer,
            parser,
            _child: child,
            master,
            grid_size: (INITIAL_ROWS, INITIAL_COLS),
            needs_scroll: true,
            focused,
        }
    }

    pub fn new(user_shell: &str) -> Self {
        let (pty, shell_name, error) = Self::spawn_shell(user_shell);
        let msg = if let Some(err) = error {
            format!(
                "Failed to start shell: {err}
Tried: {shell_name}

You can configure a different shell in Settings > Terminal > Shell.
Examples: cmd.exe, powershell.exe, bash
"
            )
        } else {
            format!(
                "{shell_name} ready.
"
            )
        };
        Self::from_pty(shell_name, pty, &msg, false)
    }

    /// Spawn an explicit command (not a resolved shell) in its own PTY, optionally
    /// in `cwd`. Used to run the interactive `claude` CLI from the Claude panel so
    /// its built-in commands (/usage, /cost, …) work in a real terminal.
    pub fn new_command(command: &str, cwd: Option<&std::path::Path>) -> Self {
        let pty = Self::try_spawn(command, &[], cwd);
        let msg = if pty.is_none() {
            format!(
                "Failed to start `{command}`.
Is it installed and on PATH?
"
            )
        } else {
            format!(
                "{command} ready.
"
            )
        };
        Self::from_pty(command.to_string(), pty, &msg, true)
    }

    /// Returns the PTY (if any spawn succeeded), the shell name, and an error
    /// message listing what was tried if every attempt failed.
    fn spawn_shell(user_shell: &str) -> (Option<Pty>, String, Option<String>) {
        let (shell_path, shell_args) = resolve_shell(user_shell);
        let shell_name = std::path::Path::new(&shell_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("shell")
            .to_string();

        // Try spawning the resolved shell
        if let Some(pty) = Self::try_spawn(&shell_path, &shell_args, None) {
            return (Some(pty), shell_name, None);
        }

        let mut tried = shell_path.clone();

        // Fallback: try cmd.exe on Windows
        #[cfg(windows)]
        {
            if let Some(pty) = Self::try_spawn("cmd.exe", &[], None) {
                return (Some(pty), "cmd".to_string(), None);
            }
            tried.push_str(", cmd.exe");
        }

        // Fallback: try /bin/sh on Unix
        #[cfg(not(windows))]
        {
            if let Some(pty) = Self::try_spawn("/bin/sh", &[], None) {
                return (Some(pty), "sh".to_string(), None);
            }
            tried.push_str(", /bin/sh");
        }

        (None, shell_name, Some(tried))
    }

    fn try_spawn(
        shell_path: &str,
        shell_args: &[String],
        cwd: Option<&std::path::Path>,
    ) -> Option<Pty> {
        let pty_system = native_pty_system();
        let size = PtySize {
            rows: INITIAL_ROWS,
            cols: INITIAL_COLS,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system.openpty(size).ok()?;

        let mut cmd = CommandBuilder::new(shell_path);
        for arg in shell_args {
            cmd.arg(arg);
        }
        cmd.env("TERM", "xterm-256color");
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }

        let child = pair.slave.spawn_command(cmd).ok()?;
        let reader = pair.master.try_clone_reader().ok()?;
        let writer = pair.master.take_writer().ok()?;

        let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = unbounded();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Some(Pty {
            rx,
            writer,
            child,
            master: pair.master,
        })
    }

    /// Resize the PTY and the screen buffer to `rows` x `cols` (clamped), but only
    /// when that differs from the size last applied.
    fn apply_grid_size(&mut self, rows: u16, cols: u16) {
        let rows = rows.clamp(MIN_ROWS, MAX_GRID);
        let cols = cols.clamp(MIN_COLS, MAX_GRID);
        if (rows, cols) == self.grid_size {
            return;
        }
        self.grid_size = (rows, cols);
        if let Some(master) = &self.master {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        self.performer.buf.resize(cols as usize, rows as usize);
    }

    pub fn update(&mut self) {
        let mut got_bytes = false;
        if let Some(rx) = &self.rx {
            while let Ok(chunk) = rx.try_recv() {
                for byte in chunk {
                    self.parser.advance(&mut self.performer, byte);
                }
                got_bytes = true;
            }
        }
        if got_bytes {
            self.needs_scroll = true;
        }
        // Reply to any terminal queries the shell/ConPTY emitted (e.g. the `ESC[6n`
        // cursor-position request). Without this the shell stalls and never prompts.
        if !self.performer.responses.is_empty() {
            if let Some(w) = &mut self.writer {
                let resp = std::mem::take(&mut self.performer.responses);
                let _ = w.write_all(&resp);
                let _ = w.flush();
            } else {
                self.performer.responses.clear();
            }
        }
    }

    pub fn send_input(&mut self, input: &str) {
        if let Some(w) = &mut self.writer {
            let _ = w.write_all(input.as_bytes());
        }
    }

    /// Signals the terminal to scroll to the bottom on the next render frame.
    pub fn scroll_to_bottom(&mut self) {
        self.needs_scroll = true;
    }

    /// Renders the terminal output (no header/tab bar).
    /// Keyboard input is forwarded directly to the PTY when the terminal has focus.
    /// Click anywhere in the terminal to focus it.
    pub fn show_content(&mut self, ui: &mut egui::Ui, config: &crate::config::Config) {
        self.update();

        let term_bg = egui::Color32::from_rgb(
            config.theme.background[0],
            config.theme.background[1],
            config.theme.background[2],
        );
        let default_fg = egui::Color32::from_rgb(
            config.theme.foreground[0],
            config.theme.foreground[1],
            config.theme.foreground[2],
        );

        let scroll_to_bottom = self.needs_scroll;
        self.needs_scroll = false;

        let content_width = (ui.available_width() - 16.0).max(1.0);

        let term_rect = ui.available_rect_before_wrap();

        // Fit the PTY grid to the panel: frame margins are 8px left/right, 4px top/bottom.
        let char_w = ui.fonts(|f| f.glyph_width(&egui::FontId::monospace(FONT_SIZE), 'M'));
        let grid_avail = egui::vec2(
            content_width - ui.spacing().scroll.allocated_width(),
            term_rect.height() - 8.0,
        );
        let (rows, cols) = grid_size(grid_avail, char_w, LINE_HEIGHT);
        self.apply_grid_size(rows, cols);

        let pointer_pos = ui.ctx().input(|i| i.pointer.interact_pos());
        let any_click = ui.ctx().input(|i| i.pointer.any_click());
        if any_click {
            if let Some(pos) = pointer_pos {
                self.focused = term_rect.contains(pos);
            }
        }

        let focused = self.focused;

        egui::Frame::new()
            .fill(term_bg)
            .inner_margin(egui::Margin {
                left: 8,
                right: 8,
                top: 4,
                bottom: 4,
            })
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;

                let scroll_out = egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .id_salt("term_scroll")
                    .stick_to_bottom(scroll_to_bottom)
                    .show(ui, |ui| {
                        ui.style_mut().spacing.item_spacing.y = 0.0;

                        let cursor_row = self.performer.buf.cursor_row;
                        let cursor_col = self.performer.buf.cursor_col;

                        let num_rows = self.performer.buf.rows.len();
                        let cursor_row = cursor_row.min(num_rows.saturating_sub(1));
                        let last_screen_row = self
                            .performer
                            .buf
                            .rows
                            .iter()
                            .rposition(|row| row.iter().any(Cell::is_styled_or_printed))
                            .map_or(0, |i| i + 1)
                            .max(cursor_row + 1)
                            .min(num_rows);

                        for row in &self.performer.buf.scrollback {
                            render_row(
                                ui,
                                row,
                                LINE_HEIGHT,
                                default_fg,
                                term_bg,
                                content_width,
                                None,
                            );
                        }
                        for (i, row) in self.performer.buf.rows[..last_screen_row]
                            .iter()
                            .enumerate()
                        {
                            let cur = if i == cursor_row {
                                Some(cursor_col)
                            } else {
                                None
                            };
                            render_row(
                                ui,
                                row,
                                LINE_HEIGHT,
                                default_fg,
                                term_bg,
                                content_width,
                                cur,
                            );
                        }
                    });

                if focused {
                    ui.painter().rect_stroke(
                        scroll_out.inner_rect,
                        0.0,
                        egui::Stroke::new(
                            1.0_f32,
                            egui::Color32::from_rgba_unmultiplied(80, 80, 200, 70),
                        ),
                        egui::StrokeKind::Inside,
                    );
                }
            });

        if focused {
            let mut to_send = String::new();
            ui.ctx().input_mut(|i| {
                i.events.retain(|event| match event {
                    egui::Event::Text(text) => {
                        to_send.push_str(text);
                        false
                    }
                    // egui-winit turns Ctrl+C / Ctrl+X / Ctrl+V into these and emits no
                    // Key event; the terminal has no selection, so forward the control codes.
                    egui::Event::Copy => {
                        to_send.push('');
                        false
                    }
                    egui::Event::Cut => {
                        to_send.push('');
                        false
                    }
                    egui::Event::Paste(text) => {
                        to_send.push_str(text);
                        false
                    }
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if modifiers.ctrl && !modifiers.alt {
                            let seq: Option<&str> = match key {
                                egui::Key::A => Some("\x01"),
                                egui::Key::B => Some("\x02"),
                                egui::Key::C => Some("\x03"),
                                egui::Key::D => Some("\x04"),
                                egui::Key::E => Some("\x05"),
                                egui::Key::F => Some("\x06"),
                                egui::Key::K => Some("\x0b"),
                                egui::Key::L => Some("\x0c"),
                                egui::Key::N => Some("\x1b[B"),
                                egui::Key::P => Some("\x1b[A"),
                                egui::Key::R => Some("\x12"),
                                egui::Key::U => Some("\x15"),
                                egui::Key::W => Some("\x17"),
                                egui::Key::Z => Some("\x1a"),
                                _ => None,
                            };
                            if let Some(s) = seq {
                                to_send.push_str(s);
                                false
                            } else {
                                true
                            }
                        } else if !modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd {
                            let seq: Option<&str> = match key {
                                egui::Key::Enter => Some("\r"),
                                egui::Key::Backspace => Some("\x7f"),
                                egui::Key::Tab => Some("\t"),
                                egui::Key::Escape => Some("\x1b"),
                                egui::Key::ArrowUp => Some("\x1b[A"),
                                egui::Key::ArrowDown => Some("\x1b[B"),
                                egui::Key::ArrowRight => Some("\x1b[C"),
                                egui::Key::ArrowLeft => Some("\x1b[D"),
                                egui::Key::Delete => Some("\x1b[3~"),
                                egui::Key::Home => Some("\x1b[H"),
                                egui::Key::End => Some("\x1b[F"),
                                egui::Key::PageUp => Some("\x1b[5~"),
                                egui::Key::PageDown => Some("\x1b[6~"),
                                _ => None,
                            };
                            if let Some(s) = seq {
                                to_send.push_str(s);
                                false
                            } else {
                                true
                            }
                        } else {
                            true
                        }
                    }
                    _ => true,
                });
            });
            if !to_send.is_empty() {
                self.send_input(&to_send);
            }
        }
    }
}

/// Number of whole (rows, cols) cells of `char_w` x `line_h` that fit in `avail`.
/// Clamping to sane bounds is done by [`Terminal::apply_grid_size`].
fn grid_size(avail: egui::Vec2, char_w: f32, line_h: f32) -> (u16, u16) {
    let fit = |len: f32, cell: f32| {
        if cell > 0.0 && len.is_finite() && len > 0.0 {
            (len / cell).floor().min(f32::from(u16::MAX)) as u16
        } else {
            0
        }
    };
    (fit(avail.y, line_h), fit(avail.x, char_w))
}

// ─── Rendering helper ─────────────────────────────────────────────────────────

fn render_row(
    ui: &mut egui::Ui,
    row: &[Cell],
    line_height: f32,
    default_fg: Color32,
    term_bg: Color32,
    clip_width: f32,
    cursor_col: Option<usize>,
) {
    let font_id = egui::FontId::monospace(FONT_SIZE);
    let char_w = ui.fonts(|f| f.glyph_width(&font_id, 'M'));

    let last = row
        .iter()
        .rposition(Cell::is_styled_or_printed)
        .map_or(0, |i| i + 1);

    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(clip_width, line_height), egui::Sense::hover());

    if let Some(col) = cursor_col {
        let prefix: String = row.iter().take(col).map(|c| c.ch).collect();
        let cx = rect.left()
            + if prefix.is_empty() {
                0.0
            } else {
                ui.fonts(|f| {
                    f.layout_no_wrap(prefix, font_id.clone(), egui::Color32::WHITE)
                        .size()
                        .x
                })
            };
        let cursor_rect = egui::Rect::from_min_size(
            egui::pos2(cx.min(rect.right() - char_w), rect.top()),
            egui::vec2(char_w, line_height),
        );
        ui.painter().rect_filled(
            cursor_rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(180, 180, 180, 180),
        );
    }

    if last == 0 {
        return;
    }

    let mut job = egui::text::LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: clip_width.max(1.0),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: None,
        },
        ..Default::default()
    };

    let mut i = 0;
    while i < last {
        let fg = row[i].fg;
        let bg = row[i].bg;
        let bold = row[i].bold;
        let mut j = i + 1;
        while j < last && row[j].fg == fg && row[j].bg == bg && row[j].bold == bold {
            j += 1;
        }
        let text: String = row[i..j].iter().map(|c| c.ch).collect();
        let color = if fg == DEFAULT_FG { default_fg } else { fg };
        job.append(
            &text,
            0.0,
            egui::TextFormat {
                font_id: font_id.clone(),
                color,
                background: bg.unwrap_or(term_bg),
                ..Default::default()
            },
        );
        i = j;
    }

    let galley = ui.fonts(|f| f.layout_job(job));
    ui.painter().galley(rect.left_top(), galley, default_fg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);
    impl Write for Shared {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Shared {
        fn take(&self) -> Vec<u8> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    /// A terminal wired to in-memory channels instead of a real PTY.
    fn fake_terminal() -> (Terminal, Sender<Vec<u8>>, Shared) {
        let (tx, rx) = unbounded();
        let out = Shared::default();
        let term = Terminal {
            shell_name: "fake".into(),
            performer: AnsiPerformer::new(),
            rx: Some(rx),
            writer: Some(Box::new(out.clone())),
            parser: Parser::new(),
            _child: None,
            master: None,
            grid_size: (INITIAL_ROWS, INITIAL_COLS),
            needs_scroll: false,
            focused: false,
        };
        (term, tx, out)
    }

    fn row_text(t: &Terminal, r: usize) -> String {
        t.performer.buf.rows[r]
            .iter()
            .map(|c| c.ch)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn update_parses_pending_chunks_and_requests_scroll() {
        let (mut t, tx, _out) = fake_terminal();
        tx.send(b"hel".to_vec()).unwrap();
        tx.send(b"lo\r\n\xc3".to_vec()).unwrap();
        tx.send(b"\xa9!".to_vec()).unwrap(); // UTF-8 'é' split across chunks
        t.update();
        assert_eq!(row_text(&t, 0), "hello");
        assert_eq!(row_text(&t, 1), "é!");
        assert!(t.needs_scroll);
    }

    #[test]
    fn update_without_data_does_not_request_scroll() {
        let (mut t, _tx, out) = fake_terminal();
        t.update();
        assert!(!t.needs_scroll);
        assert!(out.take().is_empty());
    }

    #[test]
    fn update_answers_cursor_position_queries() {
        let (mut t, tx, out) = fake_terminal();
        tx.send(b"ab\x1b[6n".to_vec()).unwrap();
        t.update();
        assert_eq!(out.take(), b"\x1b[1;3R");
        assert!(t.performer.responses.is_empty());
        // Responses are sent once.
        t.update();
        assert!(out.take().is_empty());
    }

    #[test]
    fn responses_are_dropped_without_writer() {
        let (mut t, tx, _out) = fake_terminal();
        t.writer = None;
        tx.send(b"\x1b[5n".to_vec()).unwrap();
        t.update();
        assert!(t.performer.responses.is_empty());
    }

    #[test]
    fn update_without_pty_is_noop() {
        let (mut t, _tx, _out) = fake_terminal();
        t.rx = None;
        t.update();
        assert!(!t.needs_scroll);
    }

    #[test]
    fn send_input_writes_to_pty() {
        let (mut t, _tx, out) = fake_terminal();
        t.send_input("ls -la\r");
        assert_eq!(out.take(), b"ls -la\r");
        t.writer = None;
        t.send_input("ignored"); // no writer → silently dropped
    }

    #[test]
    fn scroll_to_bottom_sets_flag() {
        let (mut t, _tx, _out) = fake_terminal();
        t.scroll_to_bottom();
        assert!(t.needs_scroll);
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn run_frame(t: &mut Terminal, events: Vec<egui::Event>) {
        let ctx = egui::Context::default();
        let cfg = crate::config::Config::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| t.show_content(ui, &cfg));
        });
    }

    #[test]
    fn focused_terminal_forwards_clipboard_events() {
        // egui-winit sends these (no Key event) for Ctrl+C / Ctrl+X / Ctrl+V.
        let (mut t, _tx, out) = fake_terminal();
        t.focused = true;
        run_frame(
            &mut t,
            vec![
                egui::Event::Copy,
                egui::Event::Cut,
                egui::Event::Paste("echo hi".into()),
            ],
        );
        assert_eq!(out.take(), b"echo hi");
    }

    #[test]
    fn focused_terminal_forwards_text_and_special_keys() {
        let (mut t, _tx, out) = fake_terminal();
        t.focused = true;
        let none = egui::Modifiers::NONE;
        let ctrl = egui::Modifiers::CTRL;
        run_frame(
            &mut t,
            vec![
                egui::Event::Text("ls".into()),
                key(egui::Key::Enter, none),
                key(egui::Key::Backspace, none),
                key(egui::Key::Tab, none),
                key(egui::Key::Escape, none),
                key(egui::Key::ArrowUp, none),
                key(egui::Key::ArrowDown, none),
                key(egui::Key::ArrowRight, none),
                key(egui::Key::ArrowLeft, none),
                key(egui::Key::Delete, none),
                key(egui::Key::Home, none),
                key(egui::Key::End, none),
                key(egui::Key::PageUp, none),
                key(egui::Key::PageDown, none),
                key(egui::Key::F1, none), // unmapped → not sent
                key(egui::Key::C, ctrl),
                key(egui::Key::A, ctrl),
                key(egui::Key::B, ctrl),
                key(egui::Key::D, ctrl),
                key(egui::Key::E, ctrl),
                key(egui::Key::F, ctrl),
                key(egui::Key::K, ctrl),
                key(egui::Key::L, ctrl),
                key(egui::Key::N, ctrl),
                key(egui::Key::P, ctrl),
                key(egui::Key::R, ctrl),
                key(egui::Key::U, ctrl),
                key(egui::Key::W, ctrl),
                key(egui::Key::Z, ctrl),
                key(egui::Key::Q, ctrl), // unmapped ctrl combo → not sent
                key(egui::Key::A, egui::Modifiers::ALT), // alt combos are ignored
            ],
        );
        let expected = "ls\r\x7f\t\x1b\x1b[A\x1b[B\x1b[C\x1b[D\x1b[3~\x1b[H\x1b[F\x1b[5~\x1b[6~\
                        \x03\x01\x02\x04\x05\x06\x0b\x0c\x1b[B\x1b[A\x12\x15\x17\x1a";
        assert_eq!(String::from_utf8(out.take()).unwrap(), expected);
    }

    #[test]
    fn unfocused_terminal_does_not_consume_input() {
        let (mut t, _tx, out) = fake_terminal();
        run_frame(
            &mut t,
            vec![
                egui::Event::Text("x".into()),
                key(egui::Key::Enter, egui::Modifiers::NONE),
            ],
        );
        assert!(out.take().is_empty());
    }

    #[test]
    fn clicking_focuses_and_clicking_outside_blurs() {
        let (mut t, _tx, _out) = fake_terminal();
        let click = |x: f32, y: f32| {
            let pos = egui::pos2(x, y);
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        run_frame(&mut t, click(100.0, 100.0));
        assert!(t.focused);
        run_frame(&mut t, click(5000.0, 5000.0));
        assert!(!t.focused);
    }

    #[test]
    fn grid_size_fits_whole_cells() {
        assert_eq!(grid_size(egui::vec2(100.0, 50.0), 10.0, 12.0), (4, 10));
        assert_eq!(grid_size(egui::vec2(-5.0, 50.0), 10.0, 12.0), (4, 0));
        assert_eq!(grid_size(egui::vec2(100.0, f32::NAN), 10.0, 12.0), (0, 10));
        assert_eq!(grid_size(egui::vec2(100.0, 50.0), 0.0, 12.0), (4, 0));
        assert_eq!(
            grid_size(egui::vec2(1e9, 1e9), 1.0, 1.0),
            (u16::MAX, u16::MAX)
        );
    }

    #[test]
    fn apply_grid_size_clamps_and_resizes_buffer() {
        let (mut t, _tx, _out) = fake_terminal();
        t.apply_grid_size(0, 0);
        assert_eq!(t.grid_size, (MIN_ROWS, MIN_COLS));
        assert_eq!(t.performer.buf.rows.len(), MIN_ROWS as usize);
        assert_eq!(t.performer.buf.cols, MIN_COLS as usize);
        t.apply_grid_size(u16::MAX, u16::MAX);
        assert_eq!(t.grid_size, (MAX_GRID, MAX_GRID));
        t.apply_grid_size(24, 80);
        assert_eq!(t.grid_size, (24, 80));
        assert_eq!(t.performer.buf.rows.len(), 24);
        assert!(t.performer.buf.rows.iter().all(|r| r.len() == 80));
    }

    #[test]
    fn frame_fits_grid_to_panel_and_output_wraps_there() {
        let (mut t, tx, _out) = fake_terminal();
        run_frame(&mut t, vec![]);
        let (rows, cols) = t.grid_size;
        assert!((MIN_COLS..INITIAL_COLS).contains(&cols), "cols = {cols}");
        assert!((MIN_ROWS..INITIAL_ROWS).contains(&rows), "rows = {rows}");
        assert_eq!(t.performer.buf.cols, cols as usize);
        assert_eq!(t.performer.buf.rows.len(), rows as usize);
        // Same panel size → no change.
        run_frame(&mut t, vec![]);
        assert_eq!(t.grid_size, (rows, cols));
        tx.send("x".repeat(cols as usize + 3).into_bytes()).unwrap();
        t.update();
        assert_eq!(row_text(&t, 1), "xxx");
    }

    #[test]
    fn rendering_consumes_scroll_flag_and_handles_rich_content() {
        let (mut t, tx, _out) = fake_terminal();
        let mut data = Vec::new();
        for i in 0..60 {
            data.extend_from_slice(
                format!("\x1b[1;3{}mline {i}\x1b[0m plain\r\n", i % 8).as_bytes(),
            );
        }
        data.extend_from_slice(b"prompt> ");
        tx.send(data).unwrap();
        t.focused = true;
        run_frame(&mut t, vec![]);
        assert!(!t.needs_scroll, "scroll request consumed by the frame");
        assert!(!t.performer.buf.scrollback.is_empty());
        // Render again with the cursor at column 0 on a blank screen.
        t.performer.buf.erase_display(2);
        run_frame(&mut t, vec![]);
    }
}
