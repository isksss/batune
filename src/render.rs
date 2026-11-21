//! GPU cell-grid renderer.
//!
//! The module does not know about escape sequences: callers convert their
//! terminal state into a [`Frame`] and hand it to the renderer.

mod atlas;
mod background;
mod decorations;
mod frame;
mod glyphs;
mod images;
mod metrics;
mod renderer;

pub use background::{BackgroundFit, BackgroundImage};
pub use frame::{CellFlags, CellView, Color, Cursor, CursorShape, Frame, ImageData, ImageQuad};
pub use glyphs::{DEFAULT_FAMILY, FontConfig, builtin_font};
pub use metrics::{CellMetrics, FontMetrics};
pub use renderer::Renderer;
