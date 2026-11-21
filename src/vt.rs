//! Terminal emulation core: turns the byte stream written by an application
//! into a grid of cells. Independent of any OS facility or renderer.
//!
//! A host drives a [`Term`] like this:
//!
//! - feed the application's output to [`Term::advance`];
//! - redraw the display lines returned by [`Term::take_dirty`], reading them
//!   through [`Term::display_row`] and the cursor through [`Term::cursor`]
//!   (shifted by [`Term::display_offset`] while the view is scrolled back);
//! - write [`Term::take_responses`] back to the application (PTY or SSH
//!   channel);
//! - handle [`Term::drain_events`] (bell, title, clipboard, ...);
//! - call [`Term::resize`] when the window size changes.
//!
//! ```
//! use batune::vt::Term;
//!
//! let mut term = Term::new(80, 24);
//! term.advance("hello".as_bytes());
//! assert_eq!(term.grid().row(0).text(), "hello");
//! ```

mod base64;
mod cell;
mod grid;
mod image;
mod iterm;
mod keyboard;
mod kitty;
mod modes;
mod sgr;
mod sixel;
mod term;

pub use cell::{Cell, Color, Flags, Hyperlink, MAX_ZEROWIDTH, Underline};
pub use grid::{DEFAULT_SCROLLBACK, Grid, Row};
pub use image::{
    DEFAULT_IMAGE_MEMORY_LIMIT, Image, ImageCell, ImageCursor, MAX_IMAGE_PIXELS, Placement,
};
pub use keyboard::KeyboardFlags;
pub use modes::{CursorShape, CursorStyle, Modes, MouseEncoding, MouseTracking};
pub use term::{Cursor, Event, MAX_CLIPBOARD_BYTES, ReportedColors, Term};
