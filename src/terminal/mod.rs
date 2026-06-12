mod ansi;
mod colors;
mod screen_buffer;
mod shell;

use ansi::AnsiPerformer;
use screen_buffer::{Cell, DEFAULT_FG};
use shell::resolve_shell;
pub use shell::list_available_shells;

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
    /// The PTY master MUST be kept alive for the lifetime of the terminal. On
    /// Windows, dropping it closes the pseudoconsole, so the shell produces no
    /// output and never reaches a prompt. We never touch it after construction.
    _master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    /// Set to true when new output arrives — triggers a one-shot scroll to bottom.
    needs_scroll: bool,
    /// Whether this terminal has keyboard focus.
    focused: bool,
    /// Active mouse selection as (anchor_line, anchor_col, head_line, head_col) in
    /// rendered-line-index space (0 = first scrollback line, then visible rows).
    /// `None` when nothing is selected.
    selection: Option<(usize, usize, usize, usize)>,
    /// True while the primary button is held and dragging out a selection.
    selecting: bool,
}

impl Terminal {
    pub fn new(user_shell: &str) -> Self {
        let (rx, writer, child, master, shell_name, error) = Self::spawn_shell(user_shell);
        let mut parser = Parser::new();
        let mut performer = AnsiPerformer::new();
        let msg = if let Some(err) = error {
            format!("Failed to start shell: {err}\r\nTried: {shell_name}\r\n\r\nYou can configure a different shell in Settings > Terminal > Shell.\r\nExamples: cmd.exe, powershell.exe, bash\r\n")
        } else {
            format!("{shell_name} ready.\r\n")
        };
        for byte in msg.bytes() {
            parser.advance(&mut performer, byte);
        }
        Self {
            shell_name,
            performer,
            rx,
            writer,
            parser,
            _child: child,
            _master: master,
            needs_scroll: true,
            focused: false,
            selection: None,
            selecting: false,
        }
    }

    /// Spawn an explicit command (not a resolved shell) in its own PTY, optionally
    /// in `cwd`. Used to run the interactive `claude` CLI from the Claude panel so
    /// its built-in commands (/usage, /cost, …) work in a real terminal.
    pub fn new_command(command: &str, cwd: Option<&std::path::Path>) -> Self {
        let mut parser = Parser::new();
        let mut performer = AnsiPerformer::new();
        let (rx, writer, child, master, error) = match Self::try_spawn(command, &[], cwd) {
            Some((rx, w, c, m)) => (Some(rx), Some(w), Some(c), Some(m), None),
            None => (None, None, None, None, Some(command.to_string())),
        };
        let msg = if let Some(err) = error {
            format!("Failed to start `{err}`.\r\nIs it installed and on PATH?\r\n")
        } else {
            format!("{command} ready.\r\n")
        };
        for byte in msg.bytes() {
            parser.advance(&mut performer, byte);
        }
        Self {
            shell_name: command.to_string(),
            performer,
            rx,
            writer,
            parser,
            _child: child,
            _master: master,
            needs_scroll: true,
            focused: true,
            selection: None,
            selecting: false,
        }
    }

    #[allow(clippy::type_complexity)]
    fn spawn_shell(user_shell: &str) -> (
        Option<Receiver<Vec<u8>>>,
        Option<Box<dyn Write + Send>>,
        Option<Box<dyn portable_pty::Child + Send + Sync>>,
        Option<Box<dyn portable_pty::MasterPty + Send>>,
        String,
        Option<String>, // error message if all attempts failed
    ) {
        let (shell_path, shell_args) = resolve_shell(user_shell);
        let shell_name = std::path::Path::new(&shell_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("shell")
            .to_string();

        // Try spawning the resolved shell
        if let Some(result) = Self::try_spawn(&shell_path, &shell_args, None) {
            return (
                Some(result.0),
                Some(result.1),
                Some(result.2),
                Some(result.3),
                shell_name,
                None,
            );
        }

        let mut tried = shell_path.clone();

        // Fallback: try cmd.exe on Windows
        #[cfg(windows)]
        {
            if let Some(result) = Self::try_spawn("cmd.exe", &[], None) {
                return (
                    Some(result.0),
                    Some(result.1),
                    Some(result.2),
                    Some(result.3),
                    "cmd".to_string(),
                    None,
                );
            }
            tried.push_str(", cmd.exe");
        }

        // Fallback: try /bin/sh on Unix
        #[cfg(not(windows))]
        {
            if let Some(result) = Self::try_spawn("/bin/sh", &[], None) {
                return (
                    Some(result.0),
                    Some(result.1),
                    Some(result.2),
                    Some(result.3),
                    "sh".to_string(),
                    None,
                );
            }
            tried.push_str(", /bin/sh");
        }

        (None, None, None, None, shell_name, Some(tried))
    }

    #[allow(clippy::type_complexity)]
    fn try_spawn(
        shell_path: &str,
        shell_args: &[String],
        cwd: Option<&std::path::Path>,
    ) -> Option<(
        Receiver<Vec<u8>>,
        Box<dyn Write + Send>,
        Box<dyn portable_pty::Child + Send + Sync>,
        Box<dyn portable_pty::MasterPty + Send>,
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
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }

        let child = pair.slave.spawn_command(cmd).ok()?;
        let reader = pair.master.try_clone_reader().ok()?;
        let writer = pair.master.take_writer().ok()?;
        // Keep the master alive and hand it back to the caller. Dropping it (as the
        // old code did by letting `pair` fall out of scope) closes the Windows
        // pseudoconsole, so the shell emits no output and never prompts. The slave
        // is intentionally dropped now (recommended, so EOF is seen when the child exits).
        let master = pair.master;

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

        Some((rx, writer, child, master))
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
        const LINE_HEIGHT: f32 = 13.5;

        let term_rect = ui.available_rect_before_wrap();

        let mono = egui::FontId::monospace(13.5);
        let char_w = ui.fonts(|f| f.glyph_width(&mono, 'M')).max(1.0);

        let (pointer_pos, primary_pressed, primary_down, primary_released, any_click) = ui
            .ctx()
            .input(|i| {
                (
                    i.pointer.interact_pos(),
                    i.pointer.primary_pressed(),
                    i.pointer.primary_down(),
                    i.pointer.primary_released(),
                    i.pointer.any_click(),
                )
            });
        if any_click {
            if let Some(pos) = pointer_pos {
                self.focused = term_rect.contains(pos);
            }
        }
        // Pressing inside the terminal (including the start of a drag-select) focuses it.
        if primary_pressed {
            if let Some(pos) = pointer_pos {
                if term_rect.contains(pos) {
                    self.focused = true;
                }
            }
        }
        let focused = self.focused;

        // Normalised selection (start <= end), skipping zero-width selections.
        let sel_norm = self.selection.and_then(|(a_l, a_c, h_l, h_c)| {
            if (a_l, a_c) == (h_l, h_c) {
                None
            } else if (a_l, a_c) <= (h_l, h_c) {
                Some((a_l, a_c, h_l, h_c))
            } else {
                Some((h_l, h_c, a_l, a_c))
            }
        });
        let last_screen_row = self.last_screen_row();
        // Cell under the pointer this frame, in rendered-line-index space.
        let mut pointer_hit: Option<(usize, usize)> = None;

        let row_style = RowStyle {
            line_height: LINE_HEIGHT,
            default_fg,
            term_bg,
            clip_width: content_width,
            char_w,
        };

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
                    .drag_to_scroll(false)
                    .stick_to_bottom(scroll_to_bottom)
                    .show(ui, |ui| {
                        ui.style_mut().spacing.item_spacing.y = 0.0;

                        let cursor_row = self.performer.buf.cursor_row;
                        let cursor_col = self.performer.buf.cursor_col;
                        let num_rows = self.performer.buf.rows.len();
                        let cursor_row = cursor_row.min(num_rows.saturating_sub(1));

                        let mut li = 0usize;
                        let mut hit_test = |rect: egui::Rect, li: usize| {
                            if let Some(p) = pointer_pos {
                                if p.y >= rect.top() && p.y < rect.bottom() && p.x >= rect.left() {
                                    pointer_hit = Some((li, col_at(p.x, rect.left(), char_w)));
                                }
                            }
                        };

                        for row in &self.performer.buf.scrollback {
                            let rect = render_row(
                                ui,
                                row,
                                row_style,
                                None,
                                sel_cols(sel_norm, li, row.len()),
                            );
                            hit_test(rect, li);
                            li += 1;
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
                            let rect = render_row(
                                ui,
                                row,
                                row_style,
                                cur,
                                sel_cols(sel_norm, li, row.len()),
                            );
                            hit_test(rect, li);
                            li += 1;
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

        // ── Drag selection ─────────────────────────────────────────────────
        if primary_pressed {
            if let (Some(pos), Some(hit)) = (pointer_pos, pointer_hit) {
                if term_rect.contains(pos) {
                    // Start a fresh selection at the pressed cell.
                    self.selection = Some((hit.0, hit.1, hit.0, hit.1));
                    self.selecting = true;
                }
            }
        } else if self.selecting && primary_down {
            if let (Some((a_l, a_c, _, _)), Some(hit)) = (self.selection, pointer_hit) {
                self.selection = Some((a_l, a_c, hit.0, hit.1));
            }
        }
        if primary_released {
            self.selecting = false;
        }

        if focused {
            let mut to_send = String::new();
            let mut copy_requested = false;
            ui.ctx().input_mut(|i| {
                i.events.retain(|event| match event {
                    egui::Event::Text(text) => {
                        to_send.push_str(text);
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
                        if modifiers.ctrl && modifiers.shift && matches!(key, egui::Key::C) {
                            // Ctrl+Shift+C copies the selection; plain Ctrl+C stays SIGINT.
                            copy_requested = true;
                            false
                        } else if modifiers.ctrl && !modifiers.alt {
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
            if copy_requested {
                if let Some(text) = self.selected_text() {
                    if !text.is_empty() {
                        ui.ctx().copy_text(text);
                    }
                }
            }
            if !to_send.is_empty() {
                self.send_input(&to_send);
            }
        }
    }

    /// Number of visible rows worth rendering — through the cursor row or the last
    /// non-blank row, whichever is lower. Kept in sync with the render loop so
    /// selection line indices match what's drawn.
    fn last_screen_row(&self) -> usize {
        let buf = &self.performer.buf;
        let num_rows = buf.rows.len();
        let cursor_row = buf.cursor_row.min(num_rows.saturating_sub(1));
        buf.rows
            .iter()
            .rposition(|row| row.iter().any(|c| c.ch != ' ' || c.fg != DEFAULT_FG || c.bold))
            .map_or(0, |i| i + 1)
            .max(cursor_row + 1)
            .min(num_rows)
    }

    /// The currently selected text (scrollback + visible rows), or `None` if the
    /// selection is empty. Trailing whitespace is trimmed per line.
    fn selected_text(&self) -> Option<String> {
        let (s_l, s_c, e_l, e_c) = match self.selection? {
            (a_l, a_c, h_l, h_c) if (a_l, a_c) == (h_l, h_c) => return None,
            (a_l, a_c, h_l, h_c) if (a_l, a_c) <= (h_l, h_c) => (a_l, a_c, h_l, h_c),
            (a_l, a_c, h_l, h_c) => (h_l, h_c, a_l, a_c),
        };
        let buf = &self.performer.buf;
        let last = self.last_screen_row();
        let lines: Vec<&Vec<Cell>> = buf
            .scrollback
            .iter()
            .chain(buf.rows[..last].iter())
            .collect();
        let mut out = String::new();
        for li in s_l..=e_l {
            if let Some(row) = lines.get(li) {
                if let Some((c0, c1)) = sel_cols(Some((s_l, s_c, e_l, e_c)), li, row.len()) {
                    let text: String = row[c0..=c1].iter().map(|c| c.ch).collect();
                    out.push_str(text.trim_end());
                }
            }
            if li != e_l {
                out.push('\n');
            }
        }
        Some(out)
    }
}

// ─── Rendering helper ─────────────────────────────────────────────────────────

/// Per-frame constants shared by every rendered row.
#[derive(Clone, Copy)]
struct RowStyle {
    line_height: f32,
    default_fg: Color32,
    term_bg: Color32,
    clip_width: f32,
    char_w: f32,
}

fn render_row(
    ui: &mut egui::Ui,
    row: &[Cell],
    style: RowStyle,
    cursor_col: Option<usize>,
    sel_cols: Option<(usize, usize)>,
) -> egui::Rect {
    let RowStyle {
        line_height,
        default_fg,
        term_bg,
        clip_width,
        char_w,
    } = style;
    let font_id = egui::FontId::monospace(13.5);

    let last = row
        .iter()
        .rposition(|c| c.ch != ' ' || c.fg != DEFAULT_FG || c.bold)
        .map_or(0, |i| i + 1);

    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(clip_width, line_height), egui::Sense::hover());

    // Selection highlight, painted behind text and cursor.
    if let Some((c0, c1)) = sel_cols {
        let x0 = rect.left() + c0 as f32 * char_w;
        let x1 = rect.left() + (c1 as f32 + 1.0) * char_w;
        let sel_rect = egui::Rect::from_min_max(
            egui::pos2(x0.min(rect.right()), rect.top()),
            egui::pos2(x1.min(rect.right()), rect.bottom()),
        );
        ui.painter().rect_filled(
            sel_rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(70, 110, 180, 110),
        );
    }

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
        return rect;
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
    rect
}

/// Inclusive column range to highlight on rendered line `li` for a normalised
/// selection `(start_line, start_col, end_line, end_col)`, or `None` if the line
/// is outside the selection.
fn sel_cols(
    sel: Option<(usize, usize, usize, usize)>,
    li: usize,
    row_len: usize,
) -> Option<(usize, usize)> {
    let (s_l, s_c, e_l, e_c) = sel?;
    if li < s_l || li > e_l || row_len == 0 {
        return None;
    }
    let last = row_len - 1;
    let (c0, c1) = if s_l == e_l {
        (s_c, e_c)
    } else if li == s_l {
        (s_c, last)
    } else if li == e_l {
        (0, e_c)
    } else {
        (0, last)
    };
    let c1 = c1.min(last);
    Some((c0.min(c1), c1))
}

/// Column index under x-coordinate `x` for a row whose left edge is at `left`.
fn col_at(x: f32, left: f32, char_w: f32) -> usize {
    ((x - left) / char_w).floor().max(0.0) as usize
}
