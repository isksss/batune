//! The winit application: owns the window and its tabs, each with a
//! terminal and a session, and moves data between them.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, Fullscreen, Window};

use crate::background;
use crate::blink::{Blink, Timing};
use crate::chrome;
use crate::clipboard::{self, Clipboard};
use crate::config::{self, Changes, Config, Profile, StartupMode};
use crate::frame::{self, View};
use crate::gpu::{self, Gpu};
use crate::icon;
use crate::ime::{self, Preedit};
use crate::input::Wheel;
use crate::links::Link;
use crate::mouse;
use crate::output;
use crate::palette::Palette;
use crate::pty::Session;
use crate::search::{self, Search};
use crate::selection::{Clicks, Selection, clamp_u16};
use crate::settings::Settings;
use crate::ssh::Target;
use crate::tab_bar::{self, Clicked};
use crate::ui::Ui;
use crate::vt::{CursorStyle, Event as VtEvent, Modes, Term};

mod event_loop;
mod keys;
mod pointer;
mod tab;

/// How often to ask whether the shell has exited. ConPTY may keep the
/// output pipe open after the child exits, so `Closed` alone is not enough.
const EXIT_POLL: Duration = Duration::from_millis(500);
/// How long the screen stays inverted for a visual bell.
const BELL_FLASH: Duration = Duration::from_millis(150);

/// Sent from other threads to wake the UI thread.
#[derive(Debug)]
pub enum UserEvent {
    /// The tab with this id has output (or has ended); take it from
    /// `Tab::output`.
    Output(u64),
    ConfigChanged,
}

/// The window and what all its tabs share.
struct Active {
    window: Arc<Window>,
    gpu: Gpu,
    ui: Ui,
    /// Never empty: the window closes with its last tab.
    tabs: Vec<Tab>,
    /// Index of the tab shown.
    current: usize,
    /// Id for the next tab opened.
    next_id: u64,
    focused: bool,
    modifiers: ModifiersState,
    /// Last pointer position in physical pixels.
    pointer: PhysicalPosition<f64>,
    /// The pointer is over the tab bar.
    over_bar: bool,
    clipboard: Clipboard,
    /// Composition in progress; `None` when the IME is idle.
    preedit: Option<Preedit>,
    /// Area last given to `set_ime_cursor_area`, so it is sent only when it
    /// moves (the cell, or the cell size after a scale change).
    ime_area: Option<(PhysicalPosition<u32>, PhysicalSize<u32>)>,
    /// When the visual bell's flash ends.
    bell: Option<Instant>,
    /// The last frame could not be drawn; try again at the next timer.
    redraw_later: bool,
    blink: Blink,
    /// When to next ask whether the shells have exited.
    exit_poll: Instant,
    /// The mouse pointer was hidden by typing.
    pointer_hidden: bool,
    /// Points the font size is off `font.size`; see [`Config::font_zoomed`].
    font_zoom: f32,
    /// Open settings screen; the mouse goes only to egui while it is.
    settings: Option<Settings>,
}

/// One session and the terminal state that belongs to it.
struct Tab {
    /// Names the tab in [`UserEvent::Output`]; indices shift as tabs close.
    id: u64,
    /// Shown while the program has set no title: the profile, the shell or
    /// the SSH target.
    name: String,
    term: Term,
    session: Box<dyn Session>,
    output: output::Receiver,
    mouse: Mouse,
    /// Screen state outside the lines as of the last drawn frame. Cursor
    /// movement and mode changes do not mark lines dirty, so they are
    /// compared separately.
    drawn: Option<Drawn>,
    /// Open scrollback search; keys go to it instead of the shell.
    search: Option<Search>,
    /// Title the program set with OSC 0/2.
    program_title: Option<String>,
}

#[derive(Default)]
struct Mouse {
    wheel: Wheel,
    /// Button held down while reporting, for motion reports.
    held: Option<mouse::Button>,
    /// Screen cell of the last report; motion within a cell is not sent.
    reported_cell: Option<(u16, u16)>,
    clicks: Clicks,
    selection: Option<Selection>,
    /// The left button is down and moving it extends the selection.
    selecting: bool,
    /// Whether the alternate screen was shown when the selection was made;
    /// switching screens leaves the selection pointing at other text.
    alt_screen: bool,
    /// Link under the pointer while Ctrl is held.
    link: Option<Link>,
    /// The left press opened a link; its release is not passed on.
    link_click: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Drawn {
    cursor: (usize, usize),
    modes: Modes,
    cursor_style: CursorStyle,
}

impl Drawn {
    fn of(term: &Term) -> Self {
        Self {
            cursor: (term.cursor().row, term.cursor().col),
            modes: *term.modes(),
            cursor_style: term.cursor_style(),
        }
    }
}

/// What a tab connects to.
pub enum Launch {
    /// The profile, or `[shell]` when `None`.
    Local(Option<Profile>),
    Ssh(Target),
}

pub struct App {
    proxy: EventLoopProxy<UserEvent>,
    config_path: Option<PathBuf>,
    config: Config,
    launch: Launch,
    palette: Palette,
    active: Option<Active>,
    /// Kept only so the watch lasts as long as the app.
    _watcher: Option<notify::RecommendedWatcher>,
}

impl App {
    /// `config` was read from `config_path`, which is watched for changes.
    pub fn new(
        proxy: EventLoopProxy<UserEvent>,
        config_path: Option<PathBuf>,
        config: Config,
        launch: Launch,
    ) -> Self {
        let watcher = config_path.as_deref().and_then(|path| {
            let proxy = proxy.clone();
            crate::watch::watch(path, move || {
                let _ = proxy.send_event(UserEvent::ConfigChanged);
            })
            // A missing config directory is the common case and needs no
            // warning; there is just nothing to reload until the next launch.
            .inspect_err(|e| {
                if path.parent().is_some_and(|dir| dir.exists()) {
                    eprintln!("batune: watch {}: {e}", path.display());
                }
            })
            .ok()
        });
        Self {
            proxy,
            config_path,
            palette: config.palette(),
            config,
            launch,
            active: None,
            _watcher: watcher,
        }
    }

    /// Re-reads the file and applies what changed. A broken file keeps the
    /// current settings, unlike at startup, so a half-edited file does not
    /// reset the window.
    fn reload_config(&mut self) {
        let Some(path) = self.config_path.as_deref() else {
            return;
        };
        // Renaming the file away reaches here as a change to it, and load
        // would read the missing file as the defaults.
        if !path.exists() {
            return;
        }
        let new = match config::load(path) {
            Ok(new) => new,
            Err(e) => {
                eprintln!(
                    "batune: {}: {e}; keeping the current settings",
                    path.display()
                );
                return;
            }
        };
        let changes = Changes::between(&self.config, &new);
        // Settings read when used (the bell) need nothing redone, but the
        // new values must still be kept.
        self.palette = new.palette();
        self.config = new;
        if changes == Changes::default() {
            return;
        }
        if changes.next_launch {
            eprintln!(
                "batune: [shell], profiles and default_profile take effect in new tabs, window.columns/rows/startup_mode at the next launch"
            );
        }
        if let Some(active) = self.active.as_mut() {
            active.apply(&self.config, &self.palette, changes);
        }
    }

    fn open(&self, event_loop: &ActiveEventLoop) -> Active {
        let window = Arc::new(
            event_loop
                .create_window(chrome::attributes(
                    icon::attributes(match self.config.window.startup_mode {
                        StartupMode::Windowed => Window::default_attributes(),
                        StartupMode::Maximized => Window::default_attributes().with_maximized(true),
                        StartupMode::Fullscreen => Window::default_attributes()
                            .with_fullscreen(Some(Fullscreen::Borderless(None))),
                    })
                    .with_title(self.config.window.title_for(None))
                    .with_inner_size(LogicalSize::new(800.0, 480.0)),
                    &self.palette,
                    self.config.window.backdrop,
                ))
                .expect("create window"),
        );
        chrome::apply(&window, &self.palette, self.config.window.backdrop);
        let ui = Ui::new(&window, &self.palette);
        let mut gpu = Gpu::new(
            window.clone(),
            self.config.font(),
            self.config.window.padding,
        );
        gpu.set_background_image(background::load(&self.config.background_image).as_ref());
        let grid = (self.config.window.columns, self.config.window.rows);
        window.set_ime_allowed(true);
        // The cell size is only known once the font is loaded, so the window
        // is created with a guess and then fitted to the configured grid. The
        // window manager may ignore the request; `Resized` handles either
        // outcome.
        if self.config.window.startup_mode == StartupMode::Windowed {
            let _ = window.request_inner_size(gpu.surface_size_for(grid.0, grid.1));
        }
        let tab = Tab::open(
            0,
            &self.launch,
            &self.config,
            &self.palette,
            &gpu,
            grid,
            &self.proxy,
        )
        .expect(match self.launch {
            Launch::Local(_) => "start shell",
            Launch::Ssh(_) => "start ssh",
        });

        let now = Instant::now();
        let mut blink = Blink::new(now);
        blink.reset(now, self.config.blink());
        Active {
            window,
            gpu,
            ui,
            tabs: vec![tab],
            current: 0,
            next_id: 1,
            focused: true,
            modifiers: ModifiersState::empty(),
            pointer: PhysicalPosition::default(),
            over_bar: false,
            clipboard: Clipboard::default(),
            preedit: None,
            ime_area: None,
            bell: None,
            redraw_later: false,
            blink,
            exit_poll: now + EXIT_POLL,
            pointer_hidden: false,
            font_zoom: 0.0,
            settings: None,
        }
    }

    fn on_clicked(&mut self, clicked: Clicked) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        match clicked {
            Clicked::Select(index) => active.select(index, &self.config),
            Clicked::Close(index) => active.close_tab(index, &self.config),
            Clicked::New(profile) => self.new_tab(Launch::Local(profile)),
            Clicked::Settings => {
                active.settings = Some(Settings::new(
                    &self.config,
                    self.config_path.clone(),
                    self.proxy.clone(),
                ));
                // Keys and the IME now go to egui; a composition begun in
                // the terminal would otherwise stay drawn under the screen.
                active.preedit = None;
                active.window.request_redraw();
            }
        }
    }

    /// Opens `launch` in a new tab after the others and shows it. A failure
    /// is only logged: the window still has its other tabs.
    fn new_tab(&mut self, launch: Launch) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let tab = Tab::open(
            active.next_id,
            &launch,
            &self.config,
            &self.palette,
            &active.gpu,
            active.gpu.grid_size(),
            &self.proxy,
        );
        match tab {
            Ok(tab) => {
                active.next_id += 1;
                active.tabs.push(tab);
                active.select(active.tabs.len() - 1, &self.config);
            }
            Err(e) => eprintln!("batune: open tab: {e}"),
        }
    }

    /// Closes the window once its last tab is gone.
    fn exit_if_empty(&mut self, event_loop: &ActiveEventLoop) {
        if self.active.as_ref().is_some_and(|a| a.tabs.is_empty()) {
            self.active = None;
            event_loop.exit();
        }
    }
}

impl Active {
    fn tab(&self) -> &Tab {
        &self.tabs[self.current]
    }

    fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.current]
    }

    /// Feeds output to the tab at `index`, shown or not.
    fn on_output(&mut self, index: usize, bytes: &[u8], config: &Config) {
        let shown = index == self.current;
        let tab = &mut self.tabs[index];
        tab.term.advance(bytes);
        let responses = tab.term.take_responses();
        if !responses.is_empty() {
            tab.write(&responses);
        }
        let mut retitled = false;
        let events: Vec<_> = tab.term.drain_events().collect();
        for event in events {
            match event {
                VtEvent::Title(title) => {
                    tab.program_title = Some(title);
                    retitled = true;
                }
                VtEvent::Bell if config.bell.visual && shown => {
                    self.bell = Some(Instant::now() + BELL_FLASH);
                    self.window.request_redraw();
                }
                VtEvent::ClipboardStore { selection, text } if config.clipboard.accepts(&text) => {
                    let targets = clipboard::Targets::of(&selection);
                    if targets.clipboard {
                        self.clipboard.copy(&text);
                    }
                    if targets.primary {
                        self.clipboard.copy_primary(&text);
                    }
                }
                // Reading is refused (no reply): any program, including one
                // on a remote host or in a `cat`ed file, could otherwise read
                // whatever the user last copied.
                VtEvent::ClipboardLoad { .. } => {}
                _ => {}
            }
        }
        if tab.mouse.selection.is_some() && tab.term.is_alt_screen() != tab.mouse.alt_screen {
            tab.clear_selection();
        }
        if let Some(search) = tab.search.as_mut() {
            search.follow(&tab.term);
        }
        let dirty = !tab.term.take_dirty().is_empty() || tab.drawn != Some(Drawn::of(&tab.term));
        if retitled {
            if shown {
                self.update_title(config);
            }
            // The tab bar shows every tab's title.
            self.window.request_redraw();
        }
        if shown {
            self.update_link();
            if dirty {
                self.window.request_redraw();
            }
        }
    }

    /// Fits every tab to the window's current size.
    fn fit_grid(&mut self) {
        let grid = self.gpu.grid_size();
        for tab in &mut self.tabs {
            tab.resize(&self.gpu, grid);
        }
        self.window.request_redraw();
    }

    /// Ends the session of the tab at `index` and removes the tab. The
    /// window is closed by the caller once no tab is left.
    fn close_tab(&mut self, index: usize, config: &Config) {
        // Dropping the session only closes the PTY; a shell that ignores
        // SIGHUP (or any program on Windows) would outlive the tab.
        let _ = self.tabs[index].session.kill();
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            return;
        }
        self.current = current_after_close(self.current, index, self.tabs.len());
        self.show_current(config);
    }

    /// Closes the tabs whose shell has exited.
    fn close_exited(&mut self, config: &Config) {
        for index in (0..self.tabs.len()).rev() {
            if self.tabs[index].exited() {
                self.close_tab(index, config);
            }
        }
    }

    fn select(&mut self, index: usize, config: &Config) {
        if index >= self.tabs.len() || index == self.current {
            return;
        }
        // `update_link` compares with the new tab's link only, so the hand
        // cursor of the old one would stay.
        if self.tab_mut().mouse.link.take().is_some() {
            self.window.set_cursor(CursorIcon::Default);
        }
        self.current = index;
        self.show_current(config);
    }

    /// Brings the window up to date after another tab became the current
    /// one.
    fn show_current(&mut self, config: &Config) {
        self.update_title(config);
        self.update_link();
        self.window.request_redraw();
    }

    fn apply(&mut self, config: &Config, palette: &Palette, changes: Changes) {
        if changes.font {
            self.gpu.set_font(config.font_zoomed(self.font_zoom));
        }
        if changes.padding {
            self.gpu.set_padding(config.window.padding);
        }
        if changes.font || changes.padding {
            self.fit_grid();
        }
        for tab in &mut self.tabs {
            if changes.colors {
                tab.term.set_reported_colors(palette.reported_colors());
            }
            if changes.scrollback {
                tab.term.set_scrollback_limit(config.scrolling.history);
                if let Some(search) = tab.search.as_mut() {
                    search.follow(&tab.term);
                }
            }
        }
        if changes.background_image {
            self.gpu
                .set_background_image(background::load(&config.background_image).as_ref());
        }
        if changes.colors {
            self.ui.set_palette(palette);
        }
        if changes.colors || changes.backdrop {
            chrome::apply(&self.window, palette, config.window.backdrop);
        }
        if changes.title {
            self.update_title(config);
        }
        self.window.request_redraw();
    }

    fn update_title(&self, config: &Config) {
        self.window
            .set_title(config.window.title_for(self.tab().program_title.as_deref()));
    }

    /// Blinking applies while the window has focus and the application
    /// has not asked for a steady cursor; the search bar has no cursor.
    fn blink_timing(&self, config: &Config) -> Option<Timing> {
        let tab = self.tab();
        let blinks = self.focused
            && tab.search.is_none()
            && tab.term.modes().show_cursor
            && tab.term.cursor_style().blinking;
        config.blink().filter(|_| blinks)
    }

    /// Typing jumps back to the live screen, as in most terminals.
    fn scroll_to_bottom(&mut self) {
        let term = &mut self.tab_mut().term;
        if term.display_offset() != 0 {
            term.set_display_offset(0);
            self.window.request_redraw();
        }
    }

    /// Draws the window; returns what was clicked on the tab bar since the
    /// last frame.
    fn redraw(&mut self, palette: &Palette, config: &Config) -> Option<Clicked> {
        let titles: Vec<String> = self.tabs.iter().map(|t| t.title().to_owned()).collect();
        let mut clicked = None;
        let mut closed = false;
        let typing = self.settings.is_some();
        let (mut painted, again) = self.ui.run(&self.window, typing, |ui| {
            clicked = tab_bar::show(ui, &titles, self.current, config);
            if let Some(settings) = self.settings.as_mut() {
                closed = !settings.show(ui.ctx());
            }
        });
        if closed {
            self.settings = None;
            // The IME was placed at egui's fields; back to the cursor.
            self.ime_area = None;
            self.window.request_redraw();
        }
        if again {
            self.window.request_redraw();
        }
        let tab = self.tab();
        let view = View {
            focused: self.focused,
            cursor_shape: config.cursor_shape(),
            selection: tab
                .mouse
                .selection
                .and_then(|sel| sel.range(&tab.term, &config.selection.semantic_escape_chars)),
            link: tab.mouse.link.as_ref().map(|l| l.range),
            search: tab.search.as_ref(),
            flash: self.bell.is_some(),
            cursor_blinked_off: !self.blink.visible(),
        };
        let mut frame = frame::build(&tab.term, palette, view);
        let mut anchor = (
            clamp_u16(tab.term.cursor().col),
            clamp_u16(tab.term.cursor().row + tab.term.display_offset()),
        );
        if let Some(search) = &tab.search {
            anchor = (
                search::bar(&mut frame, search, palette),
                frame.rows.saturating_sub(1),
            );
        }
        if let Some(preedit) = &self.preedit {
            anchor.0 = ime::overlay(&mut frame, anchor, preedit, palette);
        }
        let drawn = Drawn::of(&tab.term);
        if self.settings.is_none() {
            self.place_ime(anchor);
        }
        self.tab_mut().drawn = Some(drawn);
        match self.gpu.render(&self.window, &frame, &mut painted) {
            gpu::Rendered::RetryNow => self.window.request_redraw(),
            gpu::Rendered::RetryLater => self.redraw_later = true,
            gpu::Rendered::Presented | gpu::Rendered::Hidden => {}
        }
        clicked
    }
}

/// The selected tab once the tab at `closed` is removed, leaving `len` (> 0)
/// tabs: the same tab if it is still there, else the one that took its place,
/// else the new last one.
fn current_after_close(current: usize, closed: usize, len: usize) -> usize {
    if closed < current || current == len {
        current - 1
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::current_after_close;

    #[test]
    fn closing_a_tab_before_the_current_one_keeps_the_same_tab_selected() {
        // [a b C d] -> [b C d]
        assert_eq!(current_after_close(2, 0, 3), 1);
    }

    #[test]
    fn closing_a_tab_after_the_current_one_keeps_the_index() {
        // [a B c d] -> [a B c]
        assert_eq!(current_after_close(1, 3, 3), 1);
    }

    #[test]
    fn closing_the_current_tab_selects_the_next_one() {
        // [a B c d] -> [a c d], c selected
        assert_eq!(current_after_close(1, 1, 3), 1);
    }

    #[test]
    fn closing_the_current_last_tab_selects_the_new_last_one() {
        // [a b c D] -> [a b c]
        assert_eq!(current_after_close(3, 3, 3), 2);
    }

    #[test]
    fn closing_several_tabs_from_the_back_like_close_exited_stays_in_range() {
        // [a b C d e], b, C and e exited; close_exited removes from the back.
        let mut len = 5;
        let mut current = 2;
        for closed in [4, 2, 1] {
            len -= 1;
            current = current_after_close(current, closed, len);
            assert!(current < len);
        }
        // [a d]: C's place went to d.
        assert_eq!(current, 1);
    }
}
