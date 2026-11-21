//! Mode switches: ANSI/DEC private modes, the cursor style and the alternate screen.

use super::Term;
use crate::vt::modes::{CursorShape, CursorStyle, MouseEncoding, MouseTracking};

impl Term {
    pub(super) fn set_private_mode(&mut self, mode: u16, on: bool) {
        match mode {
            6 => {
                self.modes.origin = on;
                self.goto_addressed(0, 0);
            }
            47 | 1047 | 1049 => self.set_alt_screen(mode, on),
            1048 if on => self.save_cursor(),
            1048 => self.restore_cursor(),
            7 => {
                self.modes.autowrap = on;
                if !on {
                    self.cursor.pending_wrap = false;
                }
            }
            1 => self.modes.cursor_keys_app = on,
            5 => {
                self.modes.reverse_video = on;
                self.grid.mark_all_dirty();
            }
            12 => self.cursor_style.blinking = on,
            25 => self.modes.show_cursor = on,
            1004 => self.modes.focus_reporting = on,
            2004 => self.modes.bracketed_paste = on,
            2026 => self.modes.synchronized_output = on,
            2027 => self.modes.grapheme_cluster = on,
            9 | 1000 | 1002 | 1003 => {
                let tracking = match mode {
                    9 => MouseTracking::X10,
                    1000 => MouseTracking::Normal,
                    1002 => MouseTracking::ButtonEvent,
                    _ => MouseTracking::AnyEvent,
                };
                if on {
                    self.modes.mouse_tracking = tracking;
                } else if self.modes.mouse_tracking == tracking {
                    self.modes.mouse_tracking = MouseTracking::Off;
                }
            }
            1005 | 1006 => {
                let encoding = if mode == 1005 {
                    MouseEncoding::Utf8
                } else {
                    MouseEncoding::Sgr
                };
                if on {
                    self.modes.mouse_encoding = encoding;
                } else if self.modes.mouse_encoding == encoding {
                    self.modes.mouse_encoding = MouseEncoding::Default;
                }
            }
            _ => {}
        }
    }

    /// ANSI modes (`CSI Pm h` / `CSI Pm l`).
    pub(super) fn set_mode(&mut self, mode: u16, on: bool) {
        match mode {
            4 => self.modes.insert = on,
            20 => self.modes.linefeed_newline = on,
            _ => {}
        }
    }

    /// DECSCUSR. 0 is the terminal's default, a blinking block in xterm.
    pub(super) fn set_cursor_style(&mut self, ps: u16) {
        let shape = match ps {
            0..=2 => CursorShape::Block,
            3 | 4 => CursorShape::Underline,
            5 | 6 => CursorShape::Bar,
            _ => return,
        };
        self.cursor_style = CursorStyle {
            shape,
            blinking: ps == 0 || ps % 2 == 1,
        };
    }

    pub(super) fn swap_screens(&mut self) {
        std::mem::swap(&mut self.grid, &mut self.inactive);
        std::mem::swap(&mut self.saved_cursor, &mut self.inactive_saved_cursor);
        std::mem::swap(&mut self.keyboard, &mut self.inactive_keyboard);
        self.alt_screen = !self.alt_screen;
        self.grid.mark_all_dirty();
    }

    /// Modes 47, 1047 and 1049. They differ only in when the alternate screen
    /// is cleared and whether the cursor is saved around the switch.
    pub(super) fn set_alt_screen(&mut self, mode: u16, on: bool) {
        if on == self.alt_screen {
            return;
        }
        let bg = self.cursor.pen.bg;
        if on {
            if mode == 1049 {
                self.save_cursor();
            }
            self.swap_screens();
            if mode == 1049 {
                self.clear_screen(bg);
            }
        } else {
            if mode == 1047 {
                self.clear_screen(bg);
            }
            self.swap_screens();
            if mode == 1049 {
                self.restore_cursor();
            }
        }
    }
}
