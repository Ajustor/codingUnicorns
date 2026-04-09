use super::auto_close;
use super::Editor;

impl Editor {
    pub fn insert_char(&mut self, ch: char, auto_close_enabled: bool) {
        self.buffer.checkpoint();

        // Skip-close: if typing a closing char that's already under the cursor, just move right.
        if auto_close_enabled && auto_close::is_closing(ch) {
            let (row, col) = self.cursor.position();
            let line = self.buffer.line(row);
            let chars: Vec<char> = line.chars().collect();
            if col < chars.len() && chars[col] == ch {
                self.cursor.move_right(&self.buffer);
                self.cursor.clear_selection();
                self.cursor_blink_epoch = std::time::Instant::now();
                return;
            }
        }

        // Surround: if there's a selection and typing an opening char, wrap the selection.
        if auto_close_enabled && self.cursor.has_selection() {
            if let Some(close) = auto_close::closing_pair(ch) {
                if let Some(((sr, sc), (er, ec))) = self.cursor.selection_range() {
                    self.buffer.insert_char(er, ec, close);
                    self.buffer.insert_char(sr, sc, ch);
                    if sr == er {
                        self.cursor.set_position(er, ec + 2);
                    } else {
                        self.cursor.set_position(er, ec + 1);
                    }
                    self.cursor.clear_selection();
                    self.is_modified = true;
                    self.content_version = self.content_version.wrapping_add(1);
                    self.cursor_blink_epoch = std::time::Instant::now();
                    return;
                }
            }
        }

        if self.cursor.has_selection() {
            self.delete_selection();
        }
        let (row, col) = self.cursor.position();
        self.buffer.insert_char(row, col, ch);
        self.cursor.move_right(&self.buffer);
        self.cursor.clear_selection();

        // Auto-insert closing bracket/quote if appropriate.
        if auto_close_enabled {
            if let Some(close) = auto_close::closing_pair(ch) {
                let line = self.buffer.line(row);
                let chars: Vec<char> = line.chars().collect();
                let cursor_col = col + 1; // cursor moved right already
                let next_char = chars.get(cursor_col).copied();
                let prev_char = if col > 0 {
                    chars.get(col.saturating_sub(1)).copied()
                } else {
                    None
                };
                if !auto_close::should_skip_quote_auto_close(ch, prev_char)
                    && auto_close::should_auto_close(ch, next_char)
                {
                    let (cur_row, cur_col) = self.cursor.position();
                    self.buffer.insert_char(cur_row, cur_col, close);
                }
            }
        }

        // Adjust extra cursors on the same row that are after the insertion point.
        for ec in &mut self.extra_cursors {
            if ec.row == row && ec.col > col {
                ec.col += 1;
                ec.desired_col = ec.col;
            }
            if let Some((ar, ac)) = ec.sel_anchor {
                if ar == row && ac > col {
                    ec.sel_anchor = Some((ar, ac + 1));
                }
            }
        }

        // Process each extra cursor in order: delete its selection first, then insert.
        let n = self.extra_cursors.len();
        for i in 0..n {
            if self.extra_cursors[i].has_selection() {
                if let Some(((sr, sc), (er, ec))) = self.extra_cursors[i].selection_range() {
                    if sr == er {
                        // Same-line selection: delete the selected characters.
                        let count = ec - sc;
                        for _ in 0..count {
                            self.buffer.delete_char(sr, sc);
                        }
                        self.extra_cursors[i].set_position(sr, sc);
                        self.extra_cursors[i].clear_selection();

                        // Adjust all subsequent extra cursors on the same row.
                        for j in (i + 1)..n {
                            if self.extra_cursors[j].row == sr && self.extra_cursors[j].col > sc {
                                self.extra_cursors[j].col = self.extra_cursors[j]
                                    .col
                                    .saturating_sub(ec)
                                    .saturating_add(sc);
                                self.extra_cursors[j].desired_col = self.extra_cursors[j].col;
                            }
                            if let Some((ar, ac)) = self.extra_cursors[j].sel_anchor {
                                if ar == sr && ac > sc {
                                    let adj = ac.saturating_sub(ec).saturating_add(sc);
                                    self.extra_cursors[j].sel_anchor = Some((ar, adj));
                                }
                            }
                        }
                    } else {
                        // Multi-line selection: move cursor to start and clear selection.
                        self.extra_cursors[i].set_position(sr, sc);
                        self.extra_cursors[i].clear_selection();
                    }
                }
            }

            // Insert the character at the (possibly adjusted) cursor position.
            let (er, ec) = self.extra_cursors[i].position();
            self.buffer.insert_char(er, ec, ch);
            self.extra_cursors[i].move_right(&self.buffer);

            // Adjust all subsequent extra cursors on the same row.
            for j in (i + 1)..n {
                if self.extra_cursors[j].row == er && self.extra_cursors[j].col >= ec {
                    self.extra_cursors[j].col += 1;
                    self.extra_cursors[j].desired_col = self.extra_cursors[j].col;
                }
                if let Some((ar, ac)) = self.extra_cursors[j].sel_anchor {
                    if ar == er && ac >= ec {
                        self.extra_cursors[j].sel_anchor = Some((ar, ac + 1));
                    }
                }
            }
        }

        self.is_modified = true;
        self.content_version = self.content_version.wrapping_add(1);
        self.cursor_blink_epoch = std::time::Instant::now();
    }

    pub fn delete_char_before(&mut self) {
        self.buffer.checkpoint();
        if self.cursor.has_selection() {
            self.delete_selection();
            return;
        }
        let (row, col) = self.cursor.position();

        // Pair-delete: if the char before cursor and the char under cursor form a pair, delete both.
        if col > 0 {
            let line = self.buffer.line(row);
            let chars: Vec<char> = line.chars().collect();
            let prev = chars[col - 1];
            if let Some(expected_close) = auto_close::closing_pair(prev) {
                if col < chars.len() && chars[col] == expected_close {
                    self.buffer.delete_char(row, col);
                    self.buffer.delete_char(row, col - 1);
                    self.cursor.move_left(&self.buffer);
                    self.is_modified = true;
                    self.content_version = self.content_version.wrapping_add(1);
                    self.cursor_blink_epoch = std::time::Instant::now();
                    return;
                }
            }
        }

        if col > 0 {
            self.buffer.delete_char(row, col - 1);
            self.cursor.move_left(&self.buffer);

            // Adjust extra cursors on the same row that are after the deleted column.
            for ec in &mut self.extra_cursors {
                if ec.row == row && ec.col >= col {
                    ec.col -= 1;
                    ec.desired_col = ec.col;
                }
                if let Some((ar, ac)) = ec.sel_anchor {
                    if ar == row && ac >= col {
                        ec.sel_anchor = Some((ar, ac - 1));
                    }
                }
            }
        } else if row > 0 {
            let prev_len = self.buffer.line_len(row - 1);
            self.buffer.join_lines(row);
            self.cursor.set_position(row - 1, prev_len);
        }

        // Process each extra cursor in order: delete its selection if any, else delete char before.
        let n = self.extra_cursors.len();
        for i in 0..n {
            if self.extra_cursors[i].has_selection() {
                if let Some(((sr, sc), (er, ec))) = self.extra_cursors[i].selection_range() {
                    if sr == er {
                        let count = ec - sc;
                        for _ in 0..count {
                            self.buffer.delete_char(sr, sc);
                        }
                        self.extra_cursors[i].set_position(sr, sc);
                        self.extra_cursors[i].clear_selection();

                        // Adjust all subsequent extra cursors on the same row.
                        for j in (i + 1)..n {
                            if self.extra_cursors[j].row == sr && self.extra_cursors[j].col > sc {
                                self.extra_cursors[j].col = self.extra_cursors[j]
                                    .col
                                    .saturating_sub(ec)
                                    .saturating_add(sc);
                                self.extra_cursors[j].desired_col = self.extra_cursors[j].col;
                            }
                            if let Some((ar, ac)) = self.extra_cursors[j].sel_anchor {
                                if ar == sr && ac > sc {
                                    let adj = ac.saturating_sub(ec).saturating_add(sc);
                                    self.extra_cursors[j].sel_anchor = Some((ar, adj));
                                }
                            }
                        }
                    } else {
                        self.extra_cursors[i].set_position(sr, sc);
                        self.extra_cursors[i].clear_selection();
                    }
                }
            } else {
                let (er, ec) = self.extra_cursors[i].position();
                if ec > 0 {
                    self.buffer.delete_char(er, ec - 1);
                    self.extra_cursors[i].move_left(&self.buffer);

                    // Adjust all subsequent extra cursors on the same row.
                    for j in (i + 1)..n {
                        if self.extra_cursors[j].row == er && self.extra_cursors[j].col >= ec {
                            self.extra_cursors[j].col -= 1;
                            self.extra_cursors[j].desired_col = self.extra_cursors[j].col;
                        }
                        if let Some((ar, ac)) = self.extra_cursors[j].sel_anchor {
                            if ar == er && ac >= ec {
                                self.extra_cursors[j].sel_anchor = Some((ar, ac - 1));
                            }
                        }
                    }
                } else if er > 0 {
                    let prev_len = self.buffer.line_len(er - 1);
                    self.buffer.join_lines(er);
                    self.extra_cursors[i].set_position(er - 1, prev_len);
                }
            }
        }

        self.is_modified = true;
        self.content_version = self.content_version.wrapping_add(1);
    }

    pub fn insert_newline(&mut self) {
        self.buffer.checkpoint();
        if self.cursor.has_selection() {
            self.delete_selection();
        }
        let (row, col) = self.cursor.position();
        self.buffer.split_line(row, col);
        self.cursor.set_position(row + 1, 0);

        // Process each extra cursor in order: delete its selection if any, then insert newline.
        let n = self.extra_cursors.len();
        for i in 0..n {
            if self.extra_cursors[i].has_selection() {
                if let Some(((sr, sc), (er, ec))) = self.extra_cursors[i].selection_range() {
                    if sr == er {
                        let count = ec - sc;
                        for _ in 0..count {
                            self.buffer.delete_char(sr, sc);
                        }
                        self.extra_cursors[i].set_position(sr, sc);
                        self.extra_cursors[i].clear_selection();

                        // Adjust subsequent extra cursors on the same row.
                        for j in (i + 1)..n {
                            if self.extra_cursors[j].row == sr && self.extra_cursors[j].col > sc {
                                self.extra_cursors[j].col = self.extra_cursors[j]
                                    .col
                                    .saturating_sub(ec)
                                    .saturating_add(sc);
                                self.extra_cursors[j].desired_col = self.extra_cursors[j].col;
                            }
                            if let Some((ar, ac)) = self.extra_cursors[j].sel_anchor {
                                if ar == sr && ac > sc {
                                    let adj = ac.saturating_sub(ec).saturating_add(sc);
                                    self.extra_cursors[j].sel_anchor = Some((ar, adj));
                                }
                            }
                        }
                    } else {
                        self.extra_cursors[i].set_position(sr, sc);
                        self.extra_cursors[i].clear_selection();
                    }
                }
            }

            let (er, ec) = self.extra_cursors[i].position();
            self.buffer.split_line(er, ec);
            self.extra_cursors[i].set_position(er + 1, 0);
        }

        self.is_modified = true;
        self.content_version = self.content_version.wrapping_add(1);
    }

    pub fn selected_text(&self) -> Option<String> {
        let ((sr, sc), (er, ec)) = self.cursor.selection_range()?;
        let start = self.buffer.char_index(sr, sc);
        let end = self.buffer.char_index(er, ec).min(self.buffer.rope_len());
        Some(self.buffer.rope_slice(start, end))
    }

    pub fn delete_selection(&mut self) {
        if let Some(((sr, sc), (er, ec))) = self.cursor.selection_range() {
            let start = self.buffer.char_index(sr, sc);
            let end = self.buffer.char_index(er, ec).min(self.buffer.rope_len());
            self.buffer.delete_range(start, end);
            self.cursor.set_position(sr, sc);
            self.cursor.clear_selection();
            self.is_modified = true;
            self.content_version = self.content_version.wrapping_add(1);
        }
    }
}
