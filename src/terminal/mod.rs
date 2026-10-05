mod ansi;
mod colors;
mod screen_buffer;
mod selection;
mod shell;

use ansi::AnsiPerformer;
use screen_buffer::{Cell, DEFAULT_FG};
use selection::{word_at, GridPos, Selection};
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
    /// Set to true when new output arrives — keeps the view pinned to the bottom
    /// if it already was there (the user may have scrolled up to read history).
    needs_scroll: bool,
    /// Force the view back to the bottom on the next frame, even when the user
    /// scrolled into the scrollback (set on keyboard input / paste).
    snap_to_bottom: bool,
    /// Whether this terminal has keyboard focus.
    focused: bool,
    /// Mouse text selection, in scrollback-then-screen line space.
    selection: Option<Selection>,
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
            snap_to_bottom: false,
            focused,
            selection: None,
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
        // Rows are re-laid out; a selection would no longer point at the same text.
        self.selection = None;
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
        self.snap_to_bottom = true;
    }

    /// Cells of `line` in scrollback-then-screen line space.
    fn line(&self, line: usize) -> Option<&[Cell]> {
        let buf = &self.performer.buf;
        match line.checked_sub(buf.scrollback.len()) {
            None => buf.scrollback.get(line).map(Vec::as_slice),
            Some(screen) => buf.rows.get(screen).map(Vec::as_slice),
        }
    }

    fn selection_text(&self) -> Option<String> {
        self.selection.map(|s| s.text(|l| self.line(l)))
    }

    /// Copy the selection (if any) to the clipboard and clear it. Returns whether
    /// anything was copied.
    fn copy_selection(&mut self, ctx: &egui::Context) -> bool {
        match self.selection_text() {
            Some(text) => {
                ctx.copy_text(text);
                self.selection = None;
                true
            }
            None => false,
        }
    }

    /// Mouse selection: drag selects, double-click a word, triple-click a line, a
    /// plain click clears, right-click copies. `origin` is the top-left of line 0;
    /// `lines` how many lines are laid out.
    fn handle_mouse(
        &mut self,
        ui: &mut egui::Ui,
        origin: egui::Pos2,
        lines: usize,
        style: &RowStyle,
    ) {
        if lines == 0 {
            return;
        }
        let rect = egui::Rect::from_min_size(
            origin,
            egui::vec2(style.clip_width, lines as f32 * LINE_HEIGHT),
        );
        let resp = ui.interact(
            rect,
            ui.id().with("term_select"),
            egui::Sense::click_and_drag(),
        );
        let char_w = style.char_w.max(1.0);
        let to_cell = |pos: egui::Pos2| {
            let line = ((pos.y - origin.y) / LINE_HEIGHT).floor().max(0.0) as usize;
            let col = ((pos.x - origin.x) / char_w).floor().max(0.0) as usize;
            GridPos::new(line.min(lines - 1), col)
        };

        if resp.drag_started_by(egui::PointerButton::Primary) {
            self.focused = true;
            let start = ui
                .ctx()
                .input(|i| i.pointer.press_origin())
                .or(resp.interact_pointer_pos());
            if let Some(pos) = start {
                let cell = to_cell(pos);
                self.selection = Some(Selection::new(cell, cell));
            }
        }
        if resp.dragged_by(egui::PointerButton::Primary) {
            if let (Some(pos), Some(sel)) = (resp.interact_pointer_pos(), &mut self.selection) {
                sel.head = to_cell(pos);
                // Auto-scroll when dragging past the visible edge.
                let clip = ui.clip_rect();
                if pos.y < clip.top() {
                    ui.scroll_with_delta(egui::vec2(0.0, LINE_HEIGHT));
                } else if pos.y > clip.bottom() {
                    ui.scroll_with_delta(egui::vec2(0.0, -LINE_HEIGHT));
                }
                ui.ctx().request_repaint();
            }
        }
        if resp.clicked() {
            let cell = resp.interact_pointer_pos().map(to_cell);
            self.selection = match cell {
                Some(cell) if resp.triple_clicked() => {
                    let width = self.line(cell.line).map_or(0, <[Cell]>::len);
                    Some(Selection::new(
                        GridPos::new(cell.line, 0),
                        GridPos::new(cell.line, width.saturating_sub(1)),
                    ))
                }
                Some(cell) if resp.double_clicked() => self.line(cell.line).map(|row| {
                    let (start, end) = word_at(row, cell.col);
                    Selection::new(
                        GridPos::new(cell.line, start),
                        GridPos::new(cell.line, end.saturating_sub(1).max(start)),
                    )
                }),
                _ => None,
            };
        }
        // Windows Terminal style: right-click copies the selection, or pastes.
        if resp.secondary_clicked() {
            self.focused = true;
            if !self.copy_selection(ui.ctx()) {
                // The integration answers with an `Event::Paste` next frame.
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            }
        }
    }

    /// Renders the terminal output (no header/tab bar).
    /// Keyboard input is forwarded directly to the PTY when the terminal has focus.
    /// Click anywhere in the terminal to focus it; drag to select text.
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

        let row_style = RowStyle {
            line_height: LINE_HEIGHT,
            char_w,
            default_fg,
            term_bg,
            clip_width: content_width,
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
                    // Dragging selects text instead of scrolling.
                    .drag_to_scroll(false)
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

                        let sb_len = self.performer.buf.scrollback.len();
                        let origin = ui.cursor().min;
                        self.handle_mouse(ui, origin, sb_len + last_screen_row, &row_style);

                        let selection = self.selection;
                        let sel_on = |line: usize, row: &[Cell]| {
                            selection.and_then(|s| s.cols_on_line(line, row.len()))
                        };
                        for (i, row) in self.performer.buf.scrollback.iter().enumerate() {
                            render_row(ui, row, &row_style, None, sel_on(i, row));
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
                            render_row(ui, row, &row_style, cur, sel_on(sb_len + i, row));
                        }
                        // Unlike `stick_to_bottom`, this also works after the user
                        // scrolled up into the scrollback with the mouse wheel.
                        if std::mem::take(&mut self.snap_to_bottom) {
                            ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                        }
                    });

                if self.focused {
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

        if self.focused {
            self.handle_keyboard(ui.ctx());
        }
    }

    /// Forward keyboard / clipboard events to the PTY.
    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        let has_selection = self.selection.is_some();
        let mut to_send = String::new();
        let mut copy = false;
        let bracketed = self.performer.bracketed_paste;
        let mut pasted = false;
        let mut paste_key = false;
        ctx.input_mut(|i| {
            // egui-winit reports Ctrl+Shift+C as `Event::Copy` as well; tell them
            // apart by the modifiers held this frame.
            let shift = i.modifiers.shift;
            i.events.retain(|event| match event {
                egui::Event::Text(text) => {
                    to_send.push_str(text);
                    false
                }
                // egui-winit turns Ctrl+C / Ctrl+X / Ctrl+V into these and emits no
                // Key event. Ctrl+C copies a selection; without one (and without
                // Shift) it is forwarded as the interrupt control code.
                egui::Event::Copy => {
                    if has_selection || shift {
                        copy = true;
                    } else {
                        to_send.push('\x03');
                    }
                    false
                }
                egui::Event::Cut => {
                    to_send.push('\x18');
                    false
                }
                // Ctrl+V and Ctrl+Shift+V both arrive as this from egui-winit.
                egui::Event::Paste(text) => {
                    to_send.push_str(&paste_payload(text, bracketed));
                    pasted = true;
                    false
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if modifiers.ctrl && !modifiers.alt {
                        if *key == egui::Key::C && (modifiers.shift || has_selection) {
                            copy = true;
                            return false;
                        }
                        if *key == egui::Key::V && modifiers.shift {
                            // A backend that reports Ctrl+Shift+V as a key; the
                            // clipboard is fetched below unless a Paste came too.
                            paste_key = true;
                            return false;
                        }
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
        if copy {
            self.copy_selection(ctx);
        }
        if paste_key && !pasted {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        }
        if !to_send.is_empty() {
            // Typing clears the selection highlight, like other terminals.
            self.selection = None;
            self.send_input(&to_send);
            // Any input jumps back to the prompt (applied on the next frame).
            self.snap_to_bottom = true;
            ctx.request_repaint();
        }
    }
}

/// Bytes to send to the PTY for pasted `text`. When the application enabled
/// bracketed paste (`CSI ?2004h`) the text is wrapped in `ESC[200~ … ESC[201~`
/// (with any embedded end marker removed so it cannot break out of the bracket);
/// otherwise newlines become carriage returns, as typing Enter would send.
fn paste_payload(text: &str, bracketed: bool) -> String {
    if bracketed {
        const END: &str = "\x1b[201~";
        let mut body = text.to_string();
        while body.contains(END) {
            body = body.replace(END, "");
        }
        format!("\x1b[200~{body}{END}")
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r")
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

/// Per-frame layout and colours shared by every rendered row.
#[derive(Clone, Copy)]
struct RowStyle {
    line_height: f32,
    char_w: f32,
    default_fg: Color32,
    term_bg: Color32,
    clip_width: f32,
}

const SELECTION_COLOR: Color32 = Color32::from_rgba_premultiplied(40, 70, 130, 110);

fn render_row(
    ui: &mut egui::Ui,
    row: &[Cell],
    style: &RowStyle,
    cursor_col: Option<usize>,
    selected: Option<(usize, usize)>,
) {
    let RowStyle {
        line_height,
        char_w,
        default_fg,
        term_bg,
        clip_width,
    } = *style;
    let font_id = egui::FontId::monospace(FONT_SIZE);

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

    if last > 0 {
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

    // Drawn over the text: glyph backgrounds would otherwise hide it.
    if let Some((start, end)) = selected {
        let left = (rect.left() + start as f32 * char_w).min(rect.right());
        let right = (rect.left() + end as f32 * char_w).min(rect.right());
        let sel_rect = egui::Rect::from_x_y_ranges(left..=right, rect.y_range());
        ui.painter().rect_filled(sel_rect, 0.0, SELECTION_COLOR);
    }
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
            snap_to_bottom: false,
            focused: false,
            selection: None,
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
        assert!(t.snap_to_bottom);
    }

    #[test]
    fn key_input_snaps_back_to_bottom_after_scrolling_up() {
        let ctx = egui::Context::default();
        let lines: String = (0..300)
            .map(|i| {
                format!(
                    "line {i}
"
                )
            })
            .collect();
        let (mut t, _out, cell) = shown_terminal(&ctx, &lines);
        t.focused = true;
        let none = egui::Modifiers::NONE;
        // Scroll far up with the mouse wheel over the terminal.
        let wheel = vec![
            egui::Event::PointerMoved(egui::pos2(300.0, 300.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 100_000.0),
                modifiers: none,
            },
        ];
        // Which line is at the top of the view: double-click there and look at
        // the selected line (after letting time pass so clicks don't chain).
        let top_line = |t: &mut Terminal| {
            for _ in 0..30 {
                run_frame_in(&ctx, t, vec![], none);
            }
            let primary = egui::PointerButton::Primary;
            run_frame_in(&ctx, t, click_at(cell(0, 0), primary), none);
            run_frame_in(&ctx, t, click_at(cell(0, 0), primary), none);
            t.selection
                .take()
                .expect("double-click selects")
                .anchor
                .line
        };
        let bottom_top = top_line(&mut t);
        assert!(bottom_top > 200, "starts at the bottom ({bottom_top})");

        run_frame_in(&ctx, &mut t, wheel, none);
        assert_eq!(top_line(&mut t), 0, "wheel scrolls into the scrollback");
        assert!(!t.snap_to_bottom, "wheel scrolling does not snap");

        run_frame_in(&ctx, &mut t, vec![egui::Event::Text("x".into())], none);
        assert!(t.snap_to_bottom, "typing requests a snap");
        assert_eq!(top_line(&mut t), bottom_top, "back at the bottom");
        assert!(!t.snap_to_bottom);
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
        run_frame_in(&egui::Context::default(), t, events, egui::Modifiers::NONE);
    }

    /// Run one frame on a persistent `ctx` (needed for multi-frame pointer
    /// gestures); returns the text copied to the clipboard during the frame.
    fn run_frame_full(
        ctx: &egui::Context,
        t: &mut Terminal,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> egui::FullOutput {
        let cfg = crate::config::Config::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            modifiers,
            ..Default::default()
        };
        ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| t.show_content(ui, &cfg));
        })
    }

    /// Whether the frame asked the integration to read the clipboard.
    fn requested_paste(out: &egui::FullOutput) -> bool {
        out.viewport_output.values().any(|v| {
            v.commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::RequestPaste))
        })
    }

    #[test]
    fn paste_payload_brackets_or_converts_newlines() {
        assert_eq!(paste_payload("a\nb\r\nc", false), "a\rb\rc");
        assert_eq!(
            paste_payload("a\nb", true),
            "\x1b[200~a\nb\x1b[201~",
            "bracketed text is sent verbatim"
        );
        // An embedded end marker cannot terminate the bracket early.
        assert_eq!(
            paste_payload("x\x1b[20\x1b[201~1~y", true),
            "\x1b[200~xy\x1b[201~",
        );
    }

    #[test]
    fn paste_honours_bracketed_paste_mode() {
        let (mut t, tx, out) = fake_terminal();
        t.focused = true;
        run_frame(&mut t, vec![egui::Event::Paste("ls\ncd ..".into())]);
        assert_eq!(out.take(), b"ls\rcd ..");

        tx.send(b"\x1b[?2004h".to_vec()).unwrap();
        run_frame(&mut t, vec![egui::Event::Paste("ls\ncd ..".into())]);
        assert_eq!(out.take(), b"\x1b[200~ls\ncd ..\x1b[201~");

        tx.send(b"\x1b[?2004l".to_vec()).unwrap();
        run_frame(&mut t, vec![egui::Event::Paste("x\n".into())]);
        assert_eq!(out.take(), b"x\r");
    }

    #[test]
    fn ctrl_shift_v_pastes_exactly_once() {
        let ctx = egui::Context::default();
        let (mut t, out, _cell) = shown_terminal(&ctx, "");
        t.focused = true;
        let ctrl_shift = egui::Modifiers::CTRL | egui::Modifiers::SHIFT;
        // egui-winit: a Paste event only.
        let o = run_frame_full(
            &ctx,
            &mut t,
            vec![egui::Event::Paste("p".into())],
            ctrl_shift,
        );
        assert!(!requested_paste(&o));
        assert_eq!(out.take(), b"p");
        // Both a key and a Paste event in the same frame: pasted once.
        let o = run_frame_full(
            &ctx,
            &mut t,
            vec![
                key(egui::Key::V, ctrl_shift),
                egui::Event::Paste("q".into()),
            ],
            ctrl_shift,
        );
        assert!(!requested_paste(&o));
        assert_eq!(out.take(), b"q");
        // Key only: the clipboard is requested (arrives as Paste next frame).
        let o = run_frame_full(
            &ctx,
            &mut t,
            vec![key(egui::Key::V, ctrl_shift)],
            ctrl_shift,
        );
        assert!(requested_paste(&o));
        assert!(out.take().is_empty(), "nothing typed for the key itself");
    }

    #[test]
    fn right_click_without_selection_requests_paste() {
        let ctx = egui::Context::default();
        let (mut t, _out, cell) = shown_terminal(&ctx, "abc");
        let o = run_frame_full(
            &ctx,
            &mut t,
            click_at(cell(0, 1), egui::PointerButton::Secondary),
            egui::Modifiers::NONE,
        );
        assert!(requested_paste(&o));
        assert!(t.focused);
    }

    fn run_frame_in(
        ctx: &egui::Context,
        t: &mut Terminal,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> Option<String> {
        run_frame_full(ctx, t, events, modifiers)
            .platform_output
            .commands
            .into_iter()
            .find_map(|c| match c {
                egui::OutputCommand::CopyText(text) => Some(text),
                _ => None,
            })
    }

    fn pointer(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn click_at(pos: egui::Pos2, button: egui::PointerButton) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            pointer(pos, button, true),
            pointer(pos, button, false),
        ]
    }

    /// Fake terminal showing `text`, laid out once on `ctx`. Returns the screen
    /// position of the centre of cell (line, col).
    fn shown_terminal(
        ctx: &egui::Context,
        text: &str,
    ) -> (Terminal, Shared, impl Fn(usize, usize) -> egui::Pos2) {
        let (mut t, tx, out) = fake_terminal();
        tx.send(text.as_bytes().to_vec()).unwrap();
        run_frame_in(ctx, &mut t, vec![], egui::Modifiers::NONE);
        let char_w = ctx.fonts(|f| f.glyph_width(&egui::FontId::monospace(FONT_SIZE), 'M'));
        // CentralPanel margin (8) + terminal frame margin (8 left, 4 top).
        let origin = egui::pos2(16.0, 12.0);
        let cell = move |line: usize, col: usize| {
            origin
                + egui::vec2(
                    (col as f32 + 0.5) * char_w,
                    (line as f32 + 0.5) * LINE_HEIGHT,
                )
        };
        (t, out, cell)
    }

    fn drag(ctx: &egui::Context, t: &mut Terminal, from: egui::Pos2, to: egui::Pos2) {
        let primary = egui::PointerButton::Primary;
        let none = egui::Modifiers::NONE;
        run_frame_in(
            ctx,
            t,
            vec![
                egui::Event::PointerMoved(from),
                pointer(from, primary, true),
            ],
            none,
        );
        run_frame_in(ctx, t, vec![egui::Event::PointerMoved(to)], none);
        run_frame_in(ctx, t, vec![pointer(to, primary, false)], none);
    }

    #[test]
    fn drag_selects_and_ctrl_c_copies_only_with_a_selection() {
        let ctx = egui::Context::default();
        let (mut t, out, cell) = shown_terminal(&ctx, "hello world   \r\nsecond line");
        drag(&ctx, &mut t, cell(0, 0), cell(1, 5));
        assert!(t.focused, "dragging focuses the terminal");
        assert_eq!(t.selection_text().as_deref(), Some("hello world\nsecond"));

        let ctrl = egui::Modifiers::CTRL;
        let copied = run_frame_in(&ctx, &mut t, vec![egui::Event::Copy], ctrl);
        assert_eq!(copied.as_deref(), Some("hello world\nsecond"));
        assert!(t.selection.is_none(), "copy clears the selection");
        assert!(out.take().is_empty(), "^C is not sent when copying");

        // No selection: Ctrl+C is an interrupt again.
        let copied = run_frame_in(&ctx, &mut t, vec![egui::Event::Copy], ctrl);
        assert_eq!(copied, None);
        assert_eq!(out.take(), b"\x03");
    }

    #[test]
    fn ctrl_shift_c_copies_and_never_interrupts() {
        let ctx = egui::Context::default();
        let (mut t, out, cell) = shown_terminal(&ctx, "abc def");
        t.focused = true;
        let ctrl_shift = egui::Modifiers::CTRL | egui::Modifiers::SHIFT;
        // egui-winit delivers Ctrl+Shift+C as Event::Copy.
        let copied = run_frame_in(&ctx, &mut t, vec![egui::Event::Copy], ctrl_shift);
        assert_eq!(copied, None);
        assert!(out.take().is_empty(), "no ^C without a selection either");

        drag(&ctx, &mut t, cell(0, 4), cell(0, 6));
        // Other backends may send a Key event instead.
        let copied = run_frame_in(
            &ctx,
            &mut t,
            vec![key(egui::Key::C, ctrl_shift)],
            ctrl_shift,
        );
        assert_eq!(copied.as_deref(), Some("def"));
        assert!(out.take().is_empty());
    }

    #[test]
    fn double_click_selects_word_and_triple_click_the_line() {
        let ctx = egui::Context::default();
        let (mut t, _out, cell) = shown_terminal(&ctx, "git log --oneline");
        let primary = egui::PointerButton::Primary;
        let none = egui::Modifiers::NONE;
        let at = cell(0, 5);
        run_frame_in(&ctx, &mut t, click_at(at, primary), none);
        assert!(t.selection.is_none(), "single click selects nothing");
        run_frame_in(&ctx, &mut t, click_at(at, primary), none);
        assert_eq!(t.selection_text().as_deref(), Some("log"));
        run_frame_in(&ctx, &mut t, click_at(at, primary), none);
        assert_eq!(t.selection_text().as_deref(), Some("git log --oneline"));
    }

    #[test]
    fn right_click_copies_the_selection() {
        let ctx = egui::Context::default();
        let (mut t, _out, cell) = shown_terminal(&ctx, "one two");
        drag(&ctx, &mut t, cell(0, 0), cell(0, 2));
        let copied = run_frame_in(
            &ctx,
            &mut t,
            click_at(cell(0, 0), egui::PointerButton::Secondary),
            egui::Modifiers::NONE,
        );
        assert_eq!(copied.as_deref(), Some("one"));
        assert!(t.selection.is_none());
    }

    #[test]
    fn typing_clears_the_selection_and_click_clears_it_too() {
        let ctx = egui::Context::default();
        let (mut t, out, cell) = shown_terminal(&ctx, "abc");
        drag(&ctx, &mut t, cell(0, 0), cell(0, 2));
        assert!(t.selection.is_some());
        run_frame_in(
            &ctx,
            &mut t,
            vec![egui::Event::Text("x".into())],
            egui::Modifiers::NONE,
        );
        assert!(t.selection.is_none());
        assert_eq!(out.take(), b"x");

        drag(&ctx, &mut t, cell(0, 0), cell(0, 2));
        run_frame_in(
            &ctx,
            &mut t,
            click_at(cell(0, 1), egui::PointerButton::Primary),
            egui::Modifiers::NONE,
        );
        assert!(t.selection.is_none());
    }

    #[test]
    fn selection_spans_scrollback_and_is_highlighted() {
        let ctx = egui::Context::default();
        let (mut t, tx, _out) = fake_terminal();
        run_frame_in(&ctx, &mut t, vec![], egui::Modifiers::NONE);
        let rows = t.grid_size.0 as usize;
        let mut data = String::new();
        for i in 0..rows + 3 {
            data.push_str(&format!("line {i}\r\n"));
        }
        tx.send(data.into_bytes()).unwrap();
        run_frame_in(&ctx, &mut t, vec![], egui::Modifiers::NONE);
        assert!(t.performer.buf.scrollback.len() >= 3);
        t.selection = Some(Selection::new(
            GridPos::new(1, 0),
            GridPos::new(rows + 1, 3),
        ));
        let text = t.selection_text().unwrap();
        assert!(text.starts_with("line 1\nline 2\n"), "{text}");
        assert!(text.ends_with("\nline"), "{text}");
        assert_eq!(text.lines().count(), rows + 1);
        // Rendering with a selection does not panic.
        run_frame_in(&ctx, &mut t, vec![], egui::Modifiers::NONE);
    }

    #[test]
    fn resizing_clears_the_selection() {
        let (mut t, _tx, _out) = fake_terminal();
        t.selection = Some(Selection::new(GridPos::new(0, 0), GridPos::new(0, 1)));
        t.apply_grid_size(INITIAL_ROWS, INITIAL_COLS);
        assert!(t.selection.is_some(), "same size is a no-op");
        t.apply_grid_size(10, 40);
        assert!(t.selection.is_none());
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
        assert_eq!(out.take(), b"\x03\x18echo hi");
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
