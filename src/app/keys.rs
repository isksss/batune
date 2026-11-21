//! Keyboard and IME input: key bindings, their actions and the search bar.

use std::time::Instant;

use winit::event::{ElementState, Ime};
use winit::keyboard::KeyLocation;
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::Fullscreen;

use crate::bindings::{self, Bound};
use crate::config::{self, Config};
use crate::ime::Preedit;
use crate::input;
use crate::search::{self, Search};
use crate::tab_bar::Clicked;

use super::Active;

impl Active {
    /// Handles a key; a tab action is returned for the app to carry out,
    /// since opening a tab needs more than the window has.
    pub(super) fn on_key(
        &mut self,
        event: &winit::event::KeyEvent,
        config: &Config,
    ) -> Option<Clicked> {
        // While composing, keys belong to the IME; some platforms still
        // report them (Backspace, Enter, arrows) and they must not reach the
        // shell as well.
        if event.state != ElementState::Pressed || self.preedit.is_some() {
            return None;
        }
        self.blink.reset(Instant::now(), self.blink_timing(config));
        if self.tab().search.is_some() {
            self.on_search_key(event);
            return None;
        }
        let selected = self.tab().selected_text(config);
        let bound = bindings::lookup(
            &config.keyboard.bindings,
            &event.logical_key,
            self.modifiers,
            selected.is_some(),
        );
        match bound {
            Some(Bound::Action(action)) => {
                if let Some(clicked) = self.tab_action(*action, config) {
                    return Some(clicked);
                }
                if self.run(*action, selected, config) {
                    return None;
                }
            }
            Some(Bound::Chars(chars)) => {
                self.hide_pointer(config);
                self.send_input(chars.as_bytes());
                return None;
            }
            None => {}
        }
        let modes = self.tab().key_modes();
        let numpad = event.location == KeyLocation::Numpad;
        let keypad = numpad
            .then(|| input::encode_keypad(&event.logical_key, self.modifiers, modes))
            .flatten();
        // The protocol reports Ctrl+Shift+A as `a`; the logical key would
        // be `A` (or `!` for Shift+1).
        let key = if modes.disambiguate {
            event.key_without_modifiers()
        } else {
            event.logical_key.clone()
        };
        let bytes =
            keypad.or_else(|| input::encode(&key, event.text.as_deref(), self.modifiers, modes))?;
        self.hide_pointer(config);
        self.send_input(&bytes);
        None
    }

    /// The tab bar request a tab action stands for; `None` for the others.
    pub(super) fn tab_action(&self, action: bindings::Action, config: &Config) -> Option<Clicked> {
        use bindings::Action;
        let (len, current) = (self.tabs.len(), self.current);
        Some(match action {
            Action::NewTab => Clicked::New(config.startup_profile(None).ok().flatten()),
            Action::CloseTab => Clicked::Close(current),
            Action::NextTab => Clicked::Select((current + 1) % len),
            Action::PreviousTab => Clicked::Select((current + len - 1) % len),
            Action::OpenSettings => Clicked::Settings,
            _ => return None,
        })
    }

    /// Carries out a bound action. `false` when it does not apply here and
    /// the key should go to the program instead.
    pub(super) fn run(
        &mut self,
        action: bindings::Action,
        selected: Option<String>,
        config: &Config,
    ) -> bool {
        use bindings::Action;
        match action {
            Action::Copy => {
                if let Some(text) = selected {
                    self.clipboard.copy(&text);
                }
            }
            Action::Paste => {
                if let Some(text) = self.clipboard.paste() {
                    let bytes = input::paste(&text, self.tab().term.modes().bracketed_paste);
                    self.send_input(&bytes);
                }
            }
            Action::Search => {
                let tab = self.tab_mut();
                let mut search = Search::default();
                search.refresh(&tab.term);
                tab.search = Some(search);
                self.window.request_redraw();
            }
            // The alternate screen has no scrollback, so the keys go to the
            // program there.
            Action::ScrollPageUp
            | Action::ScrollPageDown
            | Action::ScrollLineUp
            | Action::ScrollLineDown
            | Action::ScrollToTop
            | Action::ScrollToBottom
                if self.tab().term.is_alt_screen() =>
            {
                return false;
            }
            Action::ScrollPageUp | Action::ScrollPageDown => {
                let rows = isize::try_from(self.tab().term.rows()).unwrap_or(isize::MAX);
                self.scroll_view(if action == Action::ScrollPageUp {
                    rows
                } else {
                    -rows
                });
            }
            Action::ScrollLineUp => self.scroll_view(1),
            Action::ScrollLineDown => self.scroll_view(-1),
            // The terminal clamps the offset to the scrollback.
            Action::ScrollToTop => self.scroll_view(isize::MAX),
            Action::ScrollToBottom => self.scroll_view(isize::MIN),
            Action::IncreaseFontSize => {
                self.set_font_zoom(config, config.zoom_by(self.font_zoom, config::FONT_STEP));
            }
            Action::DecreaseFontSize => {
                self.set_font_zoom(config, config.zoom_by(self.font_zoom, -config::FONT_STEP));
            }
            Action::ResetFontSize => self.set_font_zoom(config, 0.0),
            Action::ToggleFullscreen => {
                let fullscreen = self
                    .window
                    .fullscreen()
                    .is_none()
                    .then_some(Fullscreen::Borderless(None));
                self.window.set_fullscreen(fullscreen);
            }
            // With nothing selected the key keeps its usual meaning, so
            // Escape can be bound without taking it from the program.
            Action::ClearSelection if selected.is_none() => return false,
            Action::ClearSelection => self.clear_selection(),
            // `resolve` never returns `None`; `tab_action` takes the rest.
            Action::None
            | Action::NewTab
            | Action::CloseTab
            | Action::NextTab
            | Action::PreviousTab
            | Action::OpenSettings => return false,
        }
        true
    }

    /// Keeps the window size and fits the grid to the new cells.
    pub(super) fn set_font_zoom(&mut self, config: &Config, zoom: f32) {
        if zoom == self.font_zoom {
            return;
        }
        self.font_zoom = zoom;
        self.gpu.set_font(config.font_zoomed(zoom));
        self.fit_grid();
    }

    /// Keys while the search bar is open. Keys it does not use are dropped
    /// rather than sent to the shell, which the user is not looking at.
    pub(super) fn on_search_key(&mut self, event: &winit::event::KeyEvent) {
        use winit::keyboard::{Key, NamedKey};
        let plain = !self.modifiers.control_key() && !self.modifiers.alt_key();
        let shift = self.modifiers.shift_key();
        let tab = &mut self.tabs[self.current];
        let Some(search) = tab.search.as_mut() else {
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => tab.search = None,
            Key::Named(NamedKey::Enter) => search.step(&tab.term, shift),
            Key::Named(NamedKey::Backspace) => search.pop(&tab.term),
            _ => match event.text.as_deref() {
                Some(text) if plain && !text.chars().any(char::is_control) => {
                    search.push(text, &tab.term)
                }
                _ => return,
            },
        }
        self.show_search_match();
        self.window.request_redraw();
    }

    pub(super) fn show_search_match(&mut self) {
        let tab = self.tab_mut();
        let Some(line) = tab
            .search
            .as_ref()
            .and_then(Search::current)
            .map(|r| r.start.line)
        else {
            return;
        };
        if let Some(offset) = search::offset_to_show(&tab.term, line) {
            tab.term.set_display_offset(offset);
        }
    }

    /// Sends typed or pasted input. It jumps back to the live screen, as in
    /// most terminals.
    pub(super) fn send_input(&mut self, bytes: &[u8]) {
        self.scroll_to_bottom();
        self.tab_mut().write(bytes);
    }

    /// Puts the IME's candidate window under the cell at `cell`: the start
    /// of the composition, or the cursor before one begins. The caret is
    /// not used because the window would then follow every keystroke.
    pub(super) fn place_ime(&mut self, cell: (u16, u16)) {
        let area = self.gpu.cell_rect(cell.0, cell.1);
        if self.ime_area != Some(area) {
            self.ime_area = Some(area);
            self.window.set_ime_cursor_area(area.0, area.1);
        }
    }

    pub(super) fn on_ime(&mut self, event: Ime, config: &Config) {
        match event {
            Ime::Preedit(text, cursor) => {
                let preedit = (!text.is_empty()).then(|| Preedit::new(text, cursor));
                if preedit.is_some() && self.tab().search.is_none() {
                    self.scroll_to_bottom();
                }
                if preedit != self.preedit {
                    self.preedit = preedit;
                    self.window.request_redraw();
                }
            }
            Ime::Commit(text) => {
                let tab = self.tab_mut();
                match tab.search.as_mut() {
                    Some(search) => {
                        search.push(&text, &tab.term);
                        self.show_search_match();
                        self.window.request_redraw();
                    }
                    None => {
                        self.blink.reset(Instant::now(), self.blink_timing(config));
                        self.hide_pointer(config);
                        self.send_input(text.as_bytes());
                    }
                }
            }
            Ime::Disabled => {
                if self.preedit.take().is_some() {
                    self.window.request_redraw();
                }
            }
            Ime::Enabled => {}
        }
    }
}
