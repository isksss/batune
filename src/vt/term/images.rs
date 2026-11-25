//! Sixel and iTerm2 images: storing them and placing them on the grid.

use std::sync::Arc;

use super::Term;
use crate::vt::image::{Image, ImageCursor, PlacedCells, Placement};
use crate::vt::iterm;
use crate::vt::sixel;

impl Term {
    /// [`Term::place_image`], returning the placement's id. A `layered`
    /// image (kitty) leaves the text under it and outlives writes to it.
    pub(super) fn put_image(
        &mut self,
        image: Arc<Image>,
        placement: Placement,
        layered: bool,
    ) -> u64 {
        let (cols, rows) = placement.cells();
        let left = self.cursor.col;
        let span = cols.min(self.cols() - left);
        let top = self.cursor.row;
        let stay = placement.cursor == ImageCursor::Stay;
        let evict = self.images.add(&image);
        let placed = PlacedCells::new(image, placement, layered);
        let id = placed.id();
        for dy in 0..rows {
            let row = if stay {
                top + dy
            } else {
                if dy > 0 {
                    self.linefeed();
                }
                self.cursor.row
            };
            if row >= self.rows() {
                break;
            }
            if !layered {
                let bg = self.cursor.pen.bg;
                self.grid.row_mut(row).erase(left..left + span, bg);
            }
            for dx in 0..span {
                self.grid
                    .cell_mut(row, left + dx)
                    .set_image(Some(placed.cell(dx, dy)));
            }
        }
        self.cursor.pending_wrap = false;
        match placement.cursor {
            ImageCursor::Stay => {}
            ImageCursor::NextLine => {
                self.linefeed();
                self.cursor.col = left;
            }
            ImageCursor::AfterImage => {
                // Like a printed character in the last column: the next
                // character wraps instead of overwriting the image.
                self.goto_col(left + span);
                self.cursor.pending_wrap = left + span >= self.cols();
            }
        }
        self.remove_images(&evict);
        id
    }

    /// The end of a Sixel image: shown at the cursor, its size in cells
    /// taken from the cell size in pixels, with the cursor left at the
    /// start of the line below as in xterm and foot.
    pub(super) fn finish_sixel(&mut self, decoder: sixel::Decoder) {
        let (r, g, b) = self.reported_colors.background;
        let Some(image) = decoder.finish([r, g, b]) else {
            return;
        };
        let (cw, ch) = self.cell_pixels;
        let mut placement = Placement::new(
            image.width() as f32 / cw as f32,
            image.height() as f32 / ch as f32,
        );
        placement.cursor = ImageCursor::NextLine;
        self.place_image(Arc::new(image), placement);
    }

    /// OSC 1337 `File=`. Undecodable or non-inline files are dropped
    /// silently, as iTerm2 does.
    pub(super) fn iterm_file(&mut self, payload: &[u8]) {
        let Some(file) = iterm::parse(payload) else {
            return;
        };
        let Some(image) = iterm::decode(&file.data) else {
            return;
        };
        let (width, height) = iterm::size_in_cells(
            &file,
            (image.width(), image.height()),
            self.cell_pixels,
            (self.cols(), self.rows()),
        );
        self.place_image(Arc::new(image), Placement::new(width, height));
    }

    pub(super) fn remove_images(&mut self, ids: &[u64]) {
        if !ids.is_empty() {
            self.grid.remove_images(ids);
            self.inactive.remove_images(ids);
        }
    }
}
