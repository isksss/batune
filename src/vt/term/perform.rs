//! The parser's callbacks: what each control sequence does to the terminal.

use unicode_width::UnicodeWidthChar;
use vte::{Params, Perform};

use super::{Charset, Event, MAX_TITLE_BYTES, Term, arg};
use crate::vt::cell::{Cell, Flags};
use crate::vt::sgr;
use crate::vt::sixel;

impl Perform for Term {
    fn print(&mut self, c: char) {
        let c = self.cursor.charsets[self.cursor.gl].map(c);
        let Some(width) = c.width() else {
            return;
        };
        // Why not ASCII too: only a Prepend character (GB9b) could hold an
        // ASCII character in its cluster, and skipping the lookup keeps
        // plain text as cheap as before.
        if self.modes.grapheme_cluster && !c.is_ascii() && self.extend_cluster(c) {
            return;
        }
        if width == 0 {
            return self.print_zerowidth(c);
        }
        let cols = self.cols();
        if width > cols {
            return;
        }

        if self.cursor.pending_wrap {
            self.cursor.pending_wrap = false;
            if self.modes.autowrap {
                self.wrap();
            }
        }
        if self.cursor.col + width > cols {
            if self.modes.autowrap {
                // A wide character never straddles rows: the last column is
                // left blank and the character starts the next row.
                let (row, col) = (self.cursor.row, self.cursor.col);
                self.clear_wide_halves(row, col, 1);
                *self.grid.cell_mut(row, col) = Cell::blank(self.cursor.pen.bg);
                self.wrap();
            } else {
                self.cursor.col = cols - width;
            }
        }

        let (row, col) = (self.cursor.row, self.cursor.col);
        if self.modes.insert {
            let bg = self.cursor.pen.bg;
            self.grid.row_mut(row).insert_blank(col, width, bg);
        }
        self.clear_wide_halves(row, col, width);
        let pen = self.cursor.pen.clone();
        let cell = self.grid.cell_mut(row, col);
        cell.write(c, &pen);
        if width == 2 {
            cell.flags.insert(Flags::WIDE);
            let spacer = self.grid.cell_mut(row, col + 1);
            spacer.write(' ', &pen);
            spacer.flags.insert(Flags::WIDE_SPACER);
        }

        if col + width < cols {
            self.cursor.col = col + width;
        } else {
            self.cursor.col = cols - 1;
            self.cursor.pending_wrap = true;
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => self.events.push_back(Event::Bell),
            0x08 => self.goto_col(self.cursor.col.saturating_sub(1)),
            b'\t' => self.tab_forward(1),
            // LF, VT and FF all act as LF, as in xterm.
            0x0a..=0x0c => {
                self.cursor.pending_wrap = false;
                self.linefeed();
                if self.modes.linefeed_newline {
                    self.goto_col(0);
                }
            }
            b'\r' => self.goto_col(0),
            0x0e => self.cursor.gl = 1,
            0x0f => self.cursor.gl = 0,
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        if let [g @ (b'(' | b')')] = intermediates {
            let charset = if byte == b'0' {
                Charset::DecSpecialGraphics
            } else {
                Charset::Ascii
            };
            self.cursor.charsets[usize::from(*g == b')')] = charset;
            return;
        }
        if !intermediates.is_empty() {
            return;
        }
        match byte {
            b'D' => self.linefeed(),
            b'E' => {
                self.linefeed();
                self.goto_col(0);
            }
            b'M' => self.reverse_index(),
            b'H' => self.tabstops[self.cursor.col] = true,
            b'7' => self.save_cursor(),
            b'8' => self.restore_cursor(),
            b'c' => self.full_reset(),
            b'=' => self.modes.keypad_app = true,
            b'>' => self.modes.keypad_app = false,
            _ => {}
        }
    }

    fn hook(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if action == 'q' && intermediates.is_empty() && !ignore {
            // P2 = 1 keeps undrawn pixels transparent; 0 and 2 fill them.
            let transparent = params.iter().nth(1).map(|p| p[0]) == Some(1);
            self.sixel = Some(Box::new(sixel::Decoder::new(transparent)));
        }
    }

    fn put(&mut self, byte: u8) {
        if let Some(decoder) = &mut self.sixel {
            decoder.put(byte);
        }
    }

    fn unhook(&mut self) {
        if let Some(decoder) = self.sixel.take() {
            self.finish_sixel(*decoder);
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        // vte splits the payload at every ';', but titles and URIs may
        // contain ';' themselves, so the rest is joined back.
        match params {
            [b"0" | b"2", rest @ ..] => {
                // An over-long title is dropped rather than cut: a cut can
                // split a UTF-8 sequence and shows a title the application
                // never sent, and no real program sends one this long.
                let joined = rest.iter().map(|p| p.len() + 1).sum::<usize>();
                if joined.saturating_sub(1) > MAX_TITLE_BYTES {
                    return;
                }
                let title = String::from_utf8_lossy(&rest.join(&b';')).into_owned();
                self.title.clone_from(&title);
                self.events.push_back(Event::Title(title));
            }
            [b"8", link_params, uri @ ..] => self.set_hyperlink(link_params, &uri.join(&b';')),
            [b"52", selection, data] => self.clipboard(selection, data),
            [b"1337", rest @ ..] => self.iterm_file(&rest.join(&b';')),
            [code @ (b"10" | b"11" | b"12"), args @ ..] => {
                let first = if *code == b"10" {
                    10
                } else if *code == b"11" {
                    11
                } else {
                    12
                };
                self.report_colors(first, args, bell_terminated);
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, c: char) {
        if ignore {
            return;
        }
        let col = self.cursor.col;
        match (intermediates, c) {
            ([], 'A') => self.move_up(arg(params, 0, 1)),
            ([], 'B' | 'e') => self.move_down(arg(params, 0, 1)),
            ([], 'C' | 'a') => self.goto_col(col + arg(params, 0, 1)),
            ([], 'D') => self.goto_col(col.saturating_sub(arg(params, 0, 1))),
            ([], 'E') => {
                self.move_down(arg(params, 0, 1));
                self.goto_col(0);
            }
            ([], 'F') => {
                self.move_up(arg(params, 0, 1));
                self.goto_col(0);
            }
            ([], 'G' | '`') => self.goto_col(arg(params, 0, 1) - 1),
            ([], 'H' | 'f') => {
                self.goto_addressed(arg(params, 0, 1) - 1, arg(params, 1, 1) - 1);
            }
            ([], 'd') => self.goto_addressed(arg(params, 0, 1) - 1, col),
            ([], 'r') => {
                let rows = self.rows();
                self.set_scroll_region(arg(params, 0, 1) - 1, arg(params, 1, rows));
            }
            ([], 'S') => self.scroll_region_by(arg(params, 0, 1), true),
            // xterm's mouse highlight tracking also uses `CSI ... T`, with
            // five parameters; it is never sent by applications we target.
            ([], 'T') if params.len() <= 1 => self.scroll_region_by(arg(params, 0, 1), false),
            ([], 'I') => self.tab_forward(arg(params, 0, 1)),
            ([], 'Z') => self.tab_backward(arg(params, 0, 1)),
            ([], 'g') => match params.iter().next().map_or(0, |p| p[0]) {
                0 => self.tabstops[col] = false,
                3 => self.tabstops.fill(false),
                _ => {}
            },
            ([], 'J') => self.erase_display(params.iter().next().map_or(0, |p| p[0])),
            ([], 'K') => self.erase_line(params.iter().next().map_or(0, |p| p[0])),
            ([], 'X') => {
                let end = (col + arg(params, 0, 1)).min(self.cols());
                let bg = self.cursor.pen.bg;
                self.cursor.pending_wrap = false;
                self.grid.row_mut(self.cursor.row).erase(col..end, bg);
            }
            ([], '@') => {
                let bg = self.cursor.pen.bg;
                self.cursor.pending_wrap = false;
                let n = arg(params, 0, 1);
                self.grid.row_mut(self.cursor.row).insert_blank(col, n, bg);
            }
            ([], 'P') => {
                let bg = self.cursor.pen.bg;
                self.cursor.pending_wrap = false;
                let n = arg(params, 0, 1);
                self.grid.row_mut(self.cursor.row).delete(col, n, bg);
            }
            ([], 'm') => sgr::apply(&mut self.cursor.pen, params),
            ([], 'L') => self.insert_or_delete_lines(arg(params, 0, 1), true),
            ([], 'M') => self.insert_or_delete_lines(arg(params, 0, 1), false),
            // Also DECSLRM while left/right margins are enabled, which is
            // not supported, so it is always a save.
            ([], 's') => self.save_cursor(),
            ([], 'u') => self.restore_cursor(),
            ([b'?'], 'u') => {
                let bits = self.keyboard.current().bits();
                self.respond(&format!("\x1b[?{bits}u"));
            }
            ([b'>'], 'u') => self.keyboard.push(arg(params, 0, 0)),
            ([b'<'], 'u') => self.keyboard.pop(arg(params, 0, 1)),
            ([b'='], 'u') => self.keyboard.set(arg(params, 0, 0), arg(params, 1, 1)),
            ([b'!'], 'p') => self.soft_reset(),
            ([], 'c') if arg(params, 0, 0) == 0 => {
                // VT220 with Sixel (4) and ANSI color, the class most
                // terminals claim.
                self.respond("\x1b[?62;4;22c");
            }
            ([b'>'], 'c') if arg(params, 0, 0) == 0 => {
                // Version 10 is low enough that vim does not assume xterm
                // extensions it would query further.
                self.respond("\x1b[>0;10;1c");
            }
            ([], 'n') => match arg(params, 0, 0) {
                5 => self.respond("\x1b[0n"),
                6 => self.report_cursor(false),
                _ => {}
            },
            ([b'?'], 'n') if arg(params, 0, 0) == 6 => self.report_cursor(true),
            ([b'?'], 'S') => {
                let mut values = params.iter().map(|p| p[0]);
                let item = values.next().unwrap_or(0);
                self.graphics_attribute(item, values.next().unwrap_or(0));
            }
            ([b'$'], 'p') => self.report_mode(arg(params, 0, 0) as u16, false),
            ([b'?', b'$'], 'p') => self.report_mode(arg(params, 0, 0) as u16, true),
            ([], 'h' | 'l') => {
                for p in params.iter() {
                    self.set_mode(p[0], c == 'h');
                }
            }
            ([b' '], 'q') => self.set_cursor_style(params.iter().next().map_or(0, |p| p[0])),
            ([b'?'], 'h' | 'l') => {
                for p in params.iter() {
                    self.set_private_mode(p[0], c == 'h');
                }
            }
            _ => {}
        }
    }
}
