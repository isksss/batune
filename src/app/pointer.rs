//! The mouse: wheel, buttons, selection, links and reporting to the application.

use std::time::Instant;

use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, MouseScrollDelta};
use winit::keyboard::ModifiersState;
use winit::window::CursorIcon;

use crate::config::Config;
use crate::input;
use crate::links;
use crate::mouse::{self, Action};
use crate::selection::{self, Point, Selection};
use crate::vt::MouseTracking;

use super::Active;

impl Active {
    /// Only typed input hides the pointer: Ctrl alone is pressed to find
    /// links with it.
    pub(super) fn hide_pointer(&mut self, config: &Config) {
        if config.mouse.hide_when_typing && !self.pointer_hidden {
            self.window.set_cursor_visible(false);
            self.pointer_hidden = true;
        }
    }

    pub(super) fn show_pointer(&mut self) {
        if std::mem::take(&mut self.pointer_hidden) {
            self.window.set_cursor_visible(true);
        }
    }

    /// Moves the view `lines` back into the scrollback (negative: towards
    /// the live screen). A drag in progress follows the text under the
    /// pointer.
    pub(super) fn scroll_view(&mut self, lines: isize) {
        let term = &mut self.tab_mut().term;
        let before = term.display_offset();
        term.scroll_display(lines);
        if term.display_offset() == before {
            return;
        }
        if self.tab().mouse.selecting {
            let at = self.pointer_cell();
            if let Some(sel) = self.tab_mut().mouse.selection.as_mut() {
                sel.extend(at);
            }
        }
        self.update_link();
        self.window.request_redraw();
    }

    pub(super) fn on_wheel(&mut self, delta: MouseScrollDelta, config: &Config) {
        let reporting = self.reporting();
        // A program that reads the wheel scrolls its own amount per event,
        // so it gets one event per notch.
        let per_notch = if reporting {
            1.0
        } else {
            f64::from(config.scrolling.multiplier)
        };
        let cell_height = f64::from(self.gpu.cell_pixels().1);
        let lines = self
            .tab_mut()
            .mouse
            .wheel
            .lines(delta, cell_height, per_notch);
        if lines == 0 {
            return;
        }
        if reporting {
            let button = if lines > 0 {
                mouse::Button::WheelUp
            } else {
                mouse::Button::WheelDown
            };
            for _ in 0..lines.unsigned_abs() {
                self.send_mouse(Action::Press(button));
            }
        } else if self.tab().term.is_alt_screen() {
            let tab = self.tab_mut();
            let bytes = input::wheel_arrows(lines, tab.key_modes());
            tab.write(&bytes);
        } else {
            self.scroll_view(lines);
        }
    }

    /// Screen cell (column, row) under the pointer.
    pub(super) fn pointer_screen_cell(&self) -> (u16, u16) {
        let term = &self.tab().term;
        self.gpu.cell_at(
            self.pointer.x,
            self.pointer.y,
            selection::clamp_u16(term.cols()),
            selection::clamp_u16(term.rows()),
        )
    }

    /// Absolute cell under the pointer.
    pub(super) fn pointer_cell(&self) -> Point {
        let (col, row) = self.pointer_screen_cell();
        Point::new(
            selection::absolute_line(&self.tab().term, row.into()),
            col.into(),
        )
    }

    /// The program asked for mouse events. Holding Shift selects text
    /// instead, as in xterm, so text stays copyable in such programs.
    pub(super) fn reporting(&self) -> bool {
        self.tab().term.modes().mouse_tracking != MouseTracking::Off && !self.modifiers.shift_key()
    }

    pub(super) fn send_mouse(&mut self, action: Action) {
        let cell = self.pointer_screen_cell();
        let modifiers = self.modifiers;
        let tab = self.tab_mut();
        tab.mouse.reported_cell = Some(cell);
        let modes = tab.term.modes();
        if let Some(bytes) = mouse::report(
            action,
            cell.0.into(),
            cell.1.into(),
            modifiers,
            modes.mouse_tracking,
            modes.mouse_encoding,
        ) {
            tab.write(&bytes);
        }
    }

    pub(super) fn clear_selection(&mut self) {
        if self.tab_mut().clear_selection() {
            self.window.request_redraw();
        }
    }

    /// Finds the link under the pointer while Ctrl is held, and shows it
    /// with an underline and a hand cursor.
    pub(super) fn update_link(&mut self) {
        let link = self
            .modifiers
            .control_key()
            .then(|| links::at(&self.tab().term, self.pointer_cell()))
            .flatten();
        if link != self.tab().mouse.link {
            self.window.set_cursor(if link.is_some() {
                CursorIcon::Pointer
            } else {
                CursorIcon::Default
            });
            self.tab_mut().mouse.link = link;
            self.window.request_redraw();
        }
    }

    pub(super) fn set_modifiers(&mut self, modifiers: ModifiersState) {
        self.modifiers = modifiers;
        self.update_link();
    }

    pub(super) fn on_mouse_button(
        &mut self,
        state: ElementState,
        button: MouseButton,
        config: &Config,
    ) {
        let button = match button {
            MouseButton::Left => mouse::Button::Left,
            MouseButton::Middle => mouse::Button::Middle,
            MouseButton::Right => mouse::Button::Right,
            _ => return,
        };
        let pressed = state == ElementState::Pressed;
        // Checked before mouse reporting so that Ctrl+click opens links in
        // programs that read the mouse too; passing the click on as well
        // would let the program act on a click the user meant for the link.
        if button == mouse::Button::Left {
            if pressed && let Some(link) = &self.tab().mouse.link {
                if let Err(e) = open::that_detached(&link.uri) {
                    eprintln!("batune: open {}: {e}", link.uri);
                }
                self.tab_mut().mouse.link_click = true;
                return;
            }
            if !pressed && std::mem::take(&mut self.tab_mut().mouse.link_click) {
                return;
            }
        }
        if self.reporting() && !self.tab().mouse.selecting {
            // A release whose press went to the tab bar was never reported.
            if !pressed && self.tab().mouse.held.is_none() {
                return;
            }
            self.tab_mut().mouse.held = pressed.then_some(button);
            self.send_mouse(if pressed {
                Action::Press(button)
            } else {
                Action::Release(button)
            });
            return;
        }
        if !pressed {
            self.tab_mut().mouse.held = None;
            if std::mem::take(&mut self.tab_mut().mouse.selecting)
                && config.selection.save_to_clipboard
                && let Some(text) = self.tab().selected_text(config)
            {
                self.clipboard.copy(&text);
            }
            return;
        }
        if button != mouse::Button::Left {
            return;
        }
        let at = self.pointer_cell();
        let shift = self.modifiers.shift_key();
        let tab = self.tab_mut();
        let unit = tab.mouse.clicks.click(at, Instant::now());
        match tab.mouse.selection.as_mut() {
            // Shift+click moves the far end of the current selection.
            Some(sel) if shift => sel.extend(at),
            _ => {
                tab.mouse.selection = Some(Selection::new(at, unit));
                tab.mouse.alt_screen = tab.term.is_alt_screen();
            }
        }
        tab.mouse.selecting = true;
        self.window.request_redraw();
    }

    pub(super) fn on_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        self.pointer = position;
        self.show_pointer();
        self.update_link();
        if !self.tab().mouse.selecting {
            if self.reporting()
                && !self.over_bar
                && self.tab().mouse.reported_cell != Some(self.pointer_screen_cell())
            {
                let held = self.tab().mouse.held;
                self.send_mouse(Action::Motion(held));
            }
            return;
        }
        let at = self.pointer_cell();
        if let Some(sel) = self.tab_mut().mouse.selection.as_mut() {
            sel.extend(at);
            self.window.request_redraw();
        }
    }
}
