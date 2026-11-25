//! Replies to queries (cursor, modes, colors, clipboard, graphics limits).

use super::{Event, MAX_CLIPBOARD_BASE64, MAX_CLIPBOARD_BYTES, Term};
use crate::vt::modes::{MouseEncoding, MouseTracking};
use crate::vt::sixel;

impl Term {
    pub(super) fn respond(&mut self, reply: &str) {
        self.responses.extend_from_slice(reply.as_bytes());
    }

    /// CPR: the cursor position, one-based and relative to the scroll
    /// region under DECOM.
    pub(super) fn report_cursor(&mut self, private: bool) {
        let row = if self.modes.origin {
            // Not a plain subtraction: a reply must never take the
            // terminal down, even if the cursor somehow left the region.
            self.cursor.row.saturating_sub(self.scroll_region.start)
        } else {
            self.cursor.row
        };
        let mark = if private { "?" } else { "" };
        self.respond(&format!("\x1b[{mark}{};{}R", row + 1, self.cursor.col + 1));
    }

    /// DECRQM: 1 set, 2 reset, 0 not recognized.
    pub(super) fn report_mode(&mut self, mode: u16, private: bool) {
        let m = &self.modes;
        let state = if private {
            match mode {
                1 => Some(m.cursor_keys_app),
                5 => Some(m.reverse_video),
                6 => Some(m.origin),
                7 => Some(m.autowrap),
                25 => Some(m.show_cursor),
                47 | 1047 | 1049 => Some(self.alt_screen),
                9 | 1000 | 1002 | 1003 => Some(
                    m.mouse_tracking
                        == match mode {
                            9 => MouseTracking::X10,
                            1000 => MouseTracking::Normal,
                            1002 => MouseTracking::ButtonEvent,
                            _ => MouseTracking::AnyEvent,
                        },
                ),
                1005 => Some(m.mouse_encoding == MouseEncoding::Utf8),
                1006 => Some(m.mouse_encoding == MouseEncoding::Sgr),
                1004 => Some(m.focus_reporting),
                2004 => Some(m.bracketed_paste),
                2026 => Some(m.synchronized_output),
                // Why not 3 (permanently set) like kitty or WezTerm: a shell
                // whose wcwidth counts each emoji of a sequence must be able
                // to turn clustering off to keep its cursor in step.
                2027 => Some(m.grapheme_cluster),
                _ => None,
            }
        } else {
            match mode {
                4 => Some(m.insert),
                20 => Some(m.linefeed_newline),
                _ => None,
            }
        };
        let value = match state {
            Some(true) => 1,
            Some(false) => 2,
            None => 0,
        };
        let mark = if private { "?" } else { "" };
        self.respond(&format!("\x1b[{mark}{mode};{value}$y"));
    }

    /// XTWINOPS 14 (text area in pixels), 16 (cell in pixels) and 18 (text
    /// area in cells), each answered as height then width like xterm.
    pub(super) fn report_size(&mut self, op: usize) {
        let (cw, ch) = (self.cell_pixels.0 as usize, self.cell_pixels.1 as usize);
        let (cols, rows) = (self.cols(), self.rows());
        let (height, width) = match op {
            14 => (rows * ch, cols * cw),
            16 => (ch, cw),
            18 => (rows, cols),
            _ => return,
        };
        self.respond(&format!("\x1b[{};{height};{width}t", op - 10));
    }

    /// XTSMGRAPHICS. The limits are fixed, so setting or resetting one
    /// reports the value in effect, as xterm does when asked for more than
    /// its maximum.
    pub(super) fn graphics_attribute(&mut self, item: u16, action: u16) {
        let reply = match (item, action) {
            (1, 1..=4) => format!("1;0;{}", sixel::COLOR_REGISTERS),
            (2, 1..=3) => {
                let (cw, ch) = self.cell_pixels;
                let width = (self.cols() * cw as usize).min(sixel::MAX_SIDE);
                let height = (self.rows() * ch as usize).min(sixel::MAX_SIDE);
                format!("2;0;{width};{height}")
            }
            (2, 4) => format!("2;0;{0};{0}", sixel::MAX_SIDE),
            (1 | 2, _) => format!("{item};2;0"),
            // ReGIS (3) and anything else.
            _ => format!("{item};1;0"),
        };
        self.respond(&format!("\x1b[?{reply}S"));
    }

    /// OSC 10/11/12 with `?`. Several queries may follow each other in one
    /// sequence (`OSC 10;?;? ST` asks for 10 and 11), as in xterm.
    pub(super) fn report_colors(&mut self, first: u16, args: &[&[u8]], bell_terminated: bool) {
        let terminator = if bell_terminated { "\x07" } else { "\x1b\\" };
        for (i, arg) in args.iter().enumerate() {
            let code = first + i as u16;
            let (r, g, b) = match (code, *arg) {
                (10, b"?") => self.reported_colors.foreground,
                (11, b"?") => self.reported_colors.background,
                (12, b"?") => self.reported_colors.cursor,
                _ => continue,
            };
            // X11 color syntax with 16 bits per channel, as xterm answers.
            self.respond(&format!(
                "\x1b]{code};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}{terminator}"
            ));
        }
    }

    /// OSC 52 `selection ; data`.
    pub(super) fn clipboard(&mut self, selection: &[u8], data: &[u8]) {
        // Not truncated: pasting the first part of what was copied could run
        // a cut-off command, so an over-long store is dropped as a whole.
        if data.len() > MAX_CLIPBOARD_BASE64 {
            return;
        }
        let selection = String::from_utf8_lossy(selection).into_owned();
        if data == b"?" {
            self.events.push_back(Event::ClipboardLoad { selection });
        } else if let Some(bytes) = crate::vt::base64::decode(data)
            && bytes.len() <= MAX_CLIPBOARD_BYTES
        {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            self.events
                .push_back(Event::ClipboardStore { selection, text });
        }
    }
}
