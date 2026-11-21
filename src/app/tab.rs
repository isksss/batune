//! One tab: a terminal, its session and what is drawn of it.

use std::ffi::OsStr;
use std::io;
use std::path::Path;

use winit::event_loop::EventLoopProxy;

use crate::config::Config;
use crate::gpu::Gpu;
use crate::input::KeyModes;
use crate::output;
use crate::palette::Palette;
use crate::pty::{LocalSession, Session, Size};
use crate::selection;
use crate::ssh::SshSession;
use crate::vt::{KeyboardFlags, Term};

use super::{Launch, Mouse, Tab, UserEvent};

/// The program's file name without its directory or `.exe`, to name a tab
/// running `[shell]`.
fn program_name(program: &OsStr) -> String {
    let path = Path::new(program);
    path.file_stem()
        .unwrap_or(program)
        .to_string_lossy()
        .into_owned()
}

impl Tab {
    /// Starts `launch` in a terminal of `cols` x `rows`.
    pub(super) fn open(
        id: u64,
        launch: &Launch,
        config: &Config,
        palette: &Palette,
        gpu: &Gpu,
        (cols, rows): (u16, u16),
        proxy: &EventLoopProxy<UserEvent>,
    ) -> io::Result<Self> {
        let proxy = proxy.clone();
        let (sender, output) = output::channel(output::LIMIT, move || {
            // Fails only after the event loop has exited, when nobody is
            // left to show the output.
            let _ = proxy.send_event(UserEvent::Output(id));
        });
        // Waiting in `push` holds up the reader thread, or for SSH the whole
        // connection task; either way the program is slowed to what the
        // window can show.
        let on_event = move |event| match event {
            crate::pty::Event::Output(bytes) => sender.push(&bytes),
            crate::pty::Event::Closed => sender.close(),
        };
        let (pixel_width, pixel_height) = gpu.grid_pixels(cols, rows);
        let size = Size {
            rows,
            cols,
            pixel_width,
            pixel_height,
        };
        let (name, session): (String, Box<dyn Session>) = match launch {
            Launch::Local(profile) => {
                let command = config.command(profile.as_ref());
                let name = match profile {
                    Some(p) => p.name.clone(),
                    None => program_name(&command.program),
                };
                (
                    name,
                    Box::new(LocalSession::spawn(&command, size, on_event)?),
                )
            }
            Launch::Ssh(target) => (
                target.to_string(),
                Box::new(SshSession::connect(target.clone(), size, on_event)?),
            ),
        };

        let mut term = Term::new(cols.into(), rows.into());
        term.set_scrollback_limit(config.scrolling.history);
        term.set_reported_colors(palette.reported_colors());
        let (cell_width, cell_height) = gpu.cell_pixels();
        term.set_cell_pixel_size(cell_width, cell_height);
        Ok(Self {
            id,
            name,
            term,
            session,
            output,
            mouse: Mouse::default(),
            drawn: None,
            search: None,
            program_title: None,
        })
    }

    pub(super) fn title(&self) -> &str {
        self.program_title.as_deref().unwrap_or(&self.name)
    }

    pub(super) fn write(&mut self, bytes: &[u8]) {
        if let Err(e) = self.session.write_all(bytes) {
            eprintln!("batune: write to session: {e}");
        }
    }

    pub(super) fn key_modes(&self) -> KeyModes {
        let modes = self.term.modes();
        KeyModes {
            cursor_keys_app: modes.cursor_keys_app,
            linefeed_newline: modes.linefeed_newline,
            keypad_app: modes.keypad_app,
            disambiguate: self
                .term
                .keyboard_flags()
                .contains(KeyboardFlags::DISAMBIGUATE),
        }
    }

    pub(super) fn selected_text(&self, config: &Config) -> Option<String> {
        let range = self
            .mouse
            .selection?
            .range(&self.term, &config.selection.semantic_escape_chars)?;
        Some(selection::text(&self.term, range))
    }

    /// `true` when there was a selection to clear.
    pub(super) fn clear_selection(&mut self) -> bool {
        self.mouse.selecting = false;
        self.mouse.selection.take().is_some()
    }

    /// Fits the terminal and the session to `cols` x `rows`.
    pub(super) fn resize(&mut self, gpu: &Gpu, (cols, rows): (u16, u16)) {
        // Before the early return: a new scale factor changes the cell size
        // even when the grid keeps its size.
        let (cell_width, cell_height) = gpu.cell_pixels();
        self.term.set_cell_pixel_size(cell_width, cell_height);
        if usize::from(cols) == self.term.cols() && usize::from(rows) == self.term.rows() {
            return;
        }
        self.term.resize(cols.into(), rows.into());
        // Without reflow the old positions no longer match the text.
        self.clear_selection();
        if let Some(search) = self.search.as_mut() {
            search.refresh(&self.term);
        }
        let (pixel_width, pixel_height) = gpu.grid_pixels(cols, rows);
        let size = Size {
            rows,
            cols,
            pixel_width,
            pixel_height,
        };
        if let Err(e) = self.session.resize(size) {
            eprintln!("batune: resize session: {e}");
        }
    }

    /// Whether the shell has exited; a failure to ask counts as exited.
    pub(super) fn exited(&mut self) -> bool {
        match self.session.try_wait() {
            Ok(None) => false,
            Ok(Some(_)) => true,
            Err(e) => {
                eprintln!("batune: wait for shell: {e}");
                true
            }
        }
    }
}
