//! Terminal sessions.
//!
//! [`Session`] abstracts over anything that behaves like a terminal: a local
//! shell on an OS pseudo terminal ([`LocalSession`], Linux PTY / Windows
//! ConPTY) today, and a native SSH channel later. Output is delivered from a
//! background thread through an [`Event`] callback, so the UI can forward it
//! to a channel or wake its event loop.

use std::io;

mod local;
mod shell;

pub use local::{Command, LocalSession};
pub use shell::{default_shell, with_terminal_env};

/// Terminal size in character cells, plus the pixel size of the text area
/// (0 when unknown) for programs that ask for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
}

impl Size {
    pub const fn new(rows: u16, cols: u16) -> Self {
        Self {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

impl Default for Size {
    fn default() -> Self {
        Self::new(24, 80)
    }
}

/// Output from a session, delivered on the reader thread.
#[derive(Debug)]
pub enum Event {
    /// Bytes the program wrote to the terminal.
    Output(Vec<u8>),
    /// The output stream ended; no more events follow. Call
    /// [`Session::wait`] to get the exit status.
    Closed,
}

/// How a session's program ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitStatus {
    /// Exit code; `None` when the program was killed by a signal.
    pub code: Option<u32>,
    /// Signal name (e.g. `"Terminated"`) when the program was killed by one.
    pub signal: Option<String>,
}

impl ExitStatus {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// A running terminal session.
pub trait Session: Send {
    /// Sends input (keystrokes, pasted text) to the program. It is called
    /// from the UI thread, so it must not wait for the program to read.
    fn write_all(&mut self, data: &[u8]) -> io::Result<()>;
    /// Tells the program the terminal size changed.
    fn resize(&mut self, size: Size) -> io::Result<()>;
    /// Returns the exit status if the program has ended, without blocking.
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>>;
    /// Blocks until the program ends.
    fn wait(&mut self) -> io::Result<ExitStatus>;
    /// Terminates the program.
    fn kill(&mut self) -> io::Result<()>;
}
