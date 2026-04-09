mod ansi;
mod colors;
mod screen_buffer;
mod shell;

use ansi::AnsiPerformer;
use screen_buffer::{Cell, DEFAULT_FG};
use shell::resolve_shell;

use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::Color32;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use vte::Parser;

pub struct Terminal {
    pub shell_name: String,
    performer: AnsiPerformer,
    rx: Option<Receiver<Vec<u8>>>,
    writer: Option<Box<dyn Write + Send>>,
    parser: Parser,
    _child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    /// Set to true when new output arrives — triggers a one-shot scroll to bottom.
    needs_scroll: bool,
    /// Whether this terminal has keyboard focus.
    focused: bool,
}

impl Terminal {
    pub fn new() -> Self {
        let (rx, writer, child, shell_name) = Self::spawn_shell();
        let mut parser = Parser::new();
        let mut performer = AnsiPerformer::new();
        for byte in "Terminal ready. Click to focus, then type.\r\n".bytes() {
            parser.advance(&mut performer, byte);
        }
        Self {
            shell_name,
            performer,
            rx,
            writer,
            parser,
            _child: child,
            needs_scroll: true,
            focused: false,
        }
    }

    #[allow(clippy::type_complexity)]
    fn spawn_shell() -> (
        Option<Receiver<Vec<u8>>>,
        Option<Box<dyn Write + Send>>,
        Option<Box<dyn portable_pty::Child + Send + Sync>>,
        String,
    ) {
        let (shell_path, shell_args) = resolve_shell();
        let shell_name = std::path::Path::new(&shell_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("shell")
            .to_string();

        // Try spawning the resolved shell; on failure, fall back to cmd.exe (Windows)
        if let Some(result) = Self::try_spawn(&shell_path, &shell_args) {
            return (Some(result.0), Some(result.1), Some(result.2), shell_name);
        }

        // Fallback: try cmd.exe on Windows
        #[cfg(windows)]
        {
            if let Some(result) = Self::try_spawn("cmd.exe", &[]) {
                return (Some(result.0), Some(result.1), Some(result.2), "cmd".to_string());
            }
        }

        (None, None, None, shell_name)
    }

    #[allow(clippy::type_complexity)]
    fn try_spawn(
        shell_path: &str,
        shell_args: &[String],
    ) -> Option<(
        Receiver<Vec<u8>>,
        Box<dyn Write + Send>,
        Box<dyn portable_pty::Child + Send + Sync>,
    )> {
        let pty_system = native_pty_system();
        let size = PtySize {
            rows: 50,
            cols: 200,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system.openpty(size).ok()?;

        let mut cmd = CommandBuilder::new(shell_path);
        for arg in shell_args {
            cmd.arg(arg);
        }
        cmd.env("TERM", "xterm-256color");

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

        Some((rx, writer, child))
    }

    fn update(&mut self) {
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
        const LINE_HEIGHT: f32 = 13.5;

        let term_rect = ui.available_rect_before_wrap();

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

                        let last_screen_row = self
                            .performer
                            .buf
                            .rows
                            .iter()
                            .rposition(|row| {
                                row.iter()
                                    .any(|c| c.ch != ' ' || c.fg != DEFAULT_FG || c.bold)
                            })
                            .map_or(0, |i| i + 1)
                            .max(cursor_row + 1);

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
                            1.0,
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
    let font_id = egui::FontId::monospace(13.5);
    let char_w = ui.fonts(|f| f.glyph_width(&font_id, 'M'));

    let last = row
        .iter()
        .rposition(|c| c.ch != ' ' || c.fg != DEFAULT_FG || c.bold)
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
        let bold = row[i].bold;
        let mut j = i + 1;
        while j < last && row[j].fg == fg && row[j].bold == bold {
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
                background: term_bg,
                ..Default::default()
            },
        );
        i = j;
    }

    let galley = ui.fonts(|f| f.layout_job(job));
    ui.painter().galley(rect.left_top(), galley, default_fg);
}
