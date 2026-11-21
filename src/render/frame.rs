//! Renderer input: a snapshot of the visible grid, independent of any
//! terminal emulator state machine.

use std::fmt;
use std::sync::Arc;

use bitflags::bitflags;

/// An opaque sRGB color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(0xff, 0xff, 0xff);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Mixes `self` toward `other`; `t = 0` keeps `self`, `t = 1` yields `other`.
    pub fn mix(self, other: Color, t: f32) -> Color {
        let lerp = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Color::rgb(
            lerp(self.r, other.r),
            lerp(self.g, other.g),
            lerp(self.b, other.b),
        )
    }
}

bitflags! {
    /// Per-cell attributes.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct CellFlags: u16 {
        const BOLD = 1 << 0;
        const ITALIC = 1 << 1;
        const UNDERLINE = 1 << 2;
        const STRIKETHROUGH = 1 << 3;
        /// Swap foreground and background.
        const INVERSE = 1 << 4;
        /// Draw the foreground faded toward the background.
        const DIM = 1 << 5;
        /// Draw the background only.
        const HIDDEN = 1 << 6;
        /// The character occupies this cell and the next one.
        const WIDE = 1 << 7;
        /// Right half of a `WIDE` cell; its own `ch` is ignored.
        const WIDE_SPACER = 1 << 8;
        /// Underline styles (SGR 4:2..4:5). Each replaces `UNDERLINE`; when
        /// several are set the first in this order wins.
        const DOUBLE_UNDERLINE = 1 << 9;
        const CURLY_UNDERLINE = 1 << 10;
        const DOTTED_UNDERLINE = 1 << 11;
        const DASHED_UNDERLINE = 1 << 12;
        const ANY_UNDERLINE = Self::UNDERLINE.bits()
            | Self::DOUBLE_UNDERLINE.bits()
            | Self::CURLY_UNDERLINE.bits()
            | Self::DOTTED_UNDERLINE.bits()
            | Self::DASHED_UNDERLINE.bits();
    }
}

/// One grid cell as the renderer sees it. Colors are already resolved
/// (palette lookups, bold-as-bright, etc. are the caller's job).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellView {
    pub ch: char,
    /// Combining marks, variation selectors and ZWJ sequences that follow
    /// `ch` in the same cell; they are shaped together with it. `None` for
    /// the common case so plain cells stay allocation-free.
    pub zerowidth: Option<Arc<[char]>>,
    pub fg: Color,
    pub bg: Color,
    pub flags: CellFlags,
    /// Color of the underline (SGR 58); `None` uses the drawn foreground.
    pub underline_color: Option<Color>,
}

impl CellView {
    pub const fn blank(fg: Color, bg: Color) -> Self {
        Self {
            ch: ' ',
            zerowidth: None,
            fg,
            bg,
            flags: CellFlags::empty(),
            underline_color: None,
        }
    }

    /// Foreground and background after applying `INVERSE` and `DIM`.
    pub fn effective_colors(&self) -> (Color, Color) {
        let (mut fg, bg) = if self.flags.contains(CellFlags::INVERSE) {
            (self.bg, self.fg)
        } else {
            (self.fg, self.bg)
        };
        if self.flags.contains(CellFlags::DIM) {
            fg = fg.mix(bg, 0.4);
        }
        (fg, bg)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    /// Outline only; terminals use this when the window is unfocused.
    HollowBlock,
    Underline,
    Bar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub col: u16,
    pub row: u16,
    pub shape: CursorShape,
    pub color: Color,
}

/// Decoded image pixels: 8-bit RGBA, not premultiplied, row by row.
#[derive(Clone)]
pub struct ImageData {
    /// Identifies the pixels: the renderer keeps one texture per id, so
    /// different pixels must never share an id.
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

impl PartialEq for ImageData {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl fmt::Debug for ImageData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageData")
            .field("id", &self.id)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// A rectangle of the grid showing part of an image.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageQuad {
    pub image: ImageData,
    /// Top-left cell.
    pub col: u16,
    pub row: u16,
    /// Size in cells; fractions end the quad inside a cell.
    pub width: f32,
    pub height: f32,
    /// Part of the image shown, `[left, top, right, bottom]` in fractions
    /// of its size.
    pub uv: [f32; 4],
    /// Negative: drawn over cell backgrounds and under text. Otherwise over
    /// text (but under the cursor and decorations). Higher is drawn later.
    pub z: i32,
}

/// Everything needed to draw one frame. `cells` is row-major and holds
/// exactly `cols * rows` entries.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<CellView>,
    /// `None` hides the cursor.
    pub cursor: Option<Cursor>,
    /// Clear color for the padding around the grid.
    pub background: Color,
    pub images: Vec<ImageQuad>,
}

impl Frame {
    /// A frame filled with blank cells.
    pub fn new(cols: u16, rows: u16, fg: Color, bg: Color) -> Self {
        Self {
            cols,
            rows,
            cells: vec![CellView::blank(fg, bg); usize::from(cols) * usize::from(rows)],
            cursor: None,
            background: bg,
            images: Vec::new(),
        }
    }

    pub fn cell(&self, col: u16, row: u16) -> Option<&CellView> {
        self.index(col, row).and_then(|i| self.cells.get(i))
    }

    pub fn cell_mut(&mut self, col: u16, row: u16) -> Option<&mut CellView> {
        self.index(col, row).and_then(|i| self.cells.get_mut(i))
    }

    fn index(&self, col: u16, row: u16) -> Option<usize> {
        (col < self.cols && row < self.rows)
            .then(|| usize::from(row) * usize::from(self.cols) + usize::from(col))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_swaps_colors() {
        let mut cell = CellView::blank(Color::WHITE, Color::BLACK);
        cell.flags = CellFlags::INVERSE;
        assert_eq!(cell.effective_colors(), (Color::BLACK, Color::WHITE));
    }

    #[test]
    fn dim_moves_foreground_toward_background() {
        let mut cell = CellView::blank(Color::rgb(200, 200, 200), Color::rgb(0, 0, 0));
        cell.flags = CellFlags::DIM;
        assert_eq!(cell.effective_colors().0, Color::rgb(120, 120, 120));
    }

    #[test]
    fn cells_are_row_major_and_bounds_checked() {
        let mut frame = Frame::new(3, 2, Color::WHITE, Color::BLACK);
        frame.cell_mut(2, 1).unwrap().ch = 'x';
        assert_eq!(frame.cells[5].ch, 'x');
        assert!(frame.cell(3, 0).is_none());
        assert!(frame.cell(0, 2).is_none());
    }
}
