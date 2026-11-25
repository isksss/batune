use std::sync::Arc;

use bitflags::bitflags;

use crate::vt::image::ImageCell;

/// A color as the application requested it. Mapping to actual RGB values
/// (palette, theme, bold-as-bright) is left to the renderer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Color {
    /// The terminal's default foreground or background.
    #[default]
    Default,
    /// An entry of the 256-color palette; 0-15 are the ANSI colors.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Flags: u16 {
        const BOLD = 1 << 0;
        const DIM = 1 << 1;
        const ITALIC = 1 << 2;
        const INVERSE = 1 << 3;
        const HIDDEN = 1 << 4;
        const STRIKEOUT = 1 << 5;
        const BLINK = 1 << 6;
        /// The first cell of a double-width character.
        const WIDE = 1 << 7;
        /// The second cell of a double-width character; holds no glyph.
        const WIDE_SPACER = 1 << 8;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// Most zero-width characters kept on one cell. Real scripts stack a few
/// (Vietnamese, Indic, emoji ZWJ sequences); without a bound, a million
/// U+0301 after one letter would all land in that cell.
pub const MAX_ZEROWIDTH: usize = 16;

/// Rarely used per-cell data, boxed so that ordinary cells stay small.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CellExtra {
    zerowidth: Vec<char>,
    hyperlink: Option<Arc<Hyperlink>>,
    image: Option<ImageCell>,
}

/// A link set by OSC 8. Shared by every cell printed while it was active.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hyperlink {
    /// The `id=` parameter: cells with the same id belong to one link even
    /// when they are not adjacent (e.g. wrapped by an application).
    pub id: Option<String>,
    pub uri: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub c: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: Flags,
    pub underline: Underline,
    /// `Color::Default` means "same as the foreground".
    pub underline_color: Color,
    extra: Option<Box<CellExtra>>,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            c: ' ',
            fg: Color::Default,
            bg: Color::Default,
            flags: Flags::empty(),
            underline: Underline::None,
            underline_color: Color::Default,
            extra: None,
        }
    }
}

impl Cell {
    /// An empty cell as left behind by erase operations: only the background
    /// survives (background color erase), like xterm and most of its successors.
    pub(crate) fn blank(bg: Color) -> Self {
        Self {
            bg,
            ..Self::default()
        }
    }

    /// Combining marks and other zero-width characters drawn on top of `c`.
    pub fn zerowidth(&self) -> &[char] {
        self.extra.as_ref().map_or(&[], |e| &e.zerowidth)
    }

    /// The OSC 8 link this cell belongs to.
    pub fn hyperlink(&self) -> Option<&Hyperlink> {
        self.extra.as_ref()?.hyperlink.as_deref()
    }

    /// The piece of an image shown in this cell.
    pub fn image(&self) -> Option<&ImageCell> {
        self.extra.as_ref()?.image.as_ref()
    }

    pub(crate) fn set_image(&mut self, image: Option<ImageCell>) {
        if image.is_none() && self.extra.is_none() {
            return;
        }
        self.extra.get_or_insert_default().image = image;
    }

    /// On the pen: SGR 0. The OSC 8 link is not an SGR attribute and stays.
    pub(crate) fn reset_attributes(&mut self) {
        *self = Self {
            extra: self.extra.take(),
            ..Self::default()
        };
    }

    /// On the pen: the link given to newly printed cells.
    pub(crate) fn set_hyperlink(&mut self, link: Option<Arc<Hyperlink>>) {
        if link.is_none() && self.extra.is_none() {
            return;
        }
        self.extra.get_or_insert_default().hyperlink = link;
    }

    pub(crate) fn push_zerowidth(&mut self, c: char) {
        let zerowidth = &mut self.extra.get_or_insert_default().zerowidth;
        // Marks past the limit are dropped instead of replacing earlier ones:
        // the first marks are the ones a real text needs (base + accents),
        // the rest of an over-long run is only stacking for its own sake.
        if zerowidth.len() < MAX_ZEROWIDTH {
            zerowidth.push(c);
        }
    }

    /// Replaces the glyph while keeping attributes from `pen`.
    pub(crate) fn write(&mut self, c: char, pen: &Cell) {
        self.c = c;
        self.fg = pen.fg;
        self.bg = pen.bg;
        self.flags = pen.flags;
        self.underline = pen.underline;
        self.underline_color = pen.underline_color;
        let image = self.layered_image();
        self.extra = pen
            .extra
            .as_ref()
            .and_then(|e| e.hyperlink.clone())
            .map(|link| {
                Box::new(CellExtra {
                    hyperlink: Some(link),
                    ..CellExtra::default()
                })
            });
        self.set_image(image);
    }

    /// Makes this a [`Cell::blank`], keeping a kitty placement over it.
    pub(crate) fn erase(&mut self, bg: Color) {
        let image = self.layered_image();
        *self = Self::blank(bg);
        self.set_image(image);
    }

    pub(crate) fn layered_image(&self) -> Option<ImageCell> {
        self.image().filter(|i| i.layered()).cloned()
    }
}
