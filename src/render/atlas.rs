//! Shelf packing for the glyph atlas.
//!
//! Terminal glyphs come in very few heights (one per font size), so a shelf
//! packer wastes little space and stays trivial. A general rectangle packer
//! (skyline, guillotine) would only pay off for the image layer, which gets
//! its own textures instead of sharing the atlas.

/// Gap left between allocations so linear sampling (and rounding in the
/// shader) never bleeds a neighbour into a glyph.
const GAP: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Debug)]
struct Shelf {
    y: u32,
    height: u32,
    next_x: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct ShelfPacker {
    width: u32,
    height: u32,
    shelves: Vec<Shelf>,
    next_y: u32,
}

impl ShelfPacker {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            shelves: Vec::new(),
            next_y: 0,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Returns `None` when the rectangle does not fit; the caller then grows
    /// or clears the atlas.
    pub fn allocate(&mut self, w: u32, h: u32) -> Option<Rect> {
        let (pw, ph) = (w + GAP, h + GAP);
        if pw > self.width {
            return None;
        }
        // Best fit: the lowest shelf that is tall enough, so small glyphs do
        // not claim the rows meant for tall emoji. A much taller shelf is
        // only used once no new shelf can be opened.
        let fits = |s: &Shelf| s.height >= ph && s.next_x + pw <= self.width;
        let snug = self
            .shelves
            .iter()
            .position(|s| fits(s) && s.height <= ph + ph / 2);
        let index = match snug {
            Some(i) => i,
            None if self.next_y + ph <= self.height => {
                self.shelves.push(Shelf {
                    y: self.next_y,
                    height: ph,
                    next_x: 0,
                });
                self.next_y += ph;
                self.shelves.len() - 1
            }
            None => self
                .shelves
                .iter()
                .enumerate()
                .filter(|(_, s)| fits(s))
                .min_by_key(|(_, s)| s.height)
                .map(|(i, _)| i)?,
        };
        let shelf = &mut self.shelves[index];
        let rect = Rect {
            x: shelf.next_x,
            y: shelf.y,
            w,
            h,
        };
        shelf.next_x += pw;
        Some(rect)
    }

    /// Allocates every size or none of them, so a caller that needs several
    /// rectangles together (a cell with a color and a mask layer) does not
    /// leave the first ones unused in the atlas when a later one fails.
    /// Why not undo the earlier allocations one by one: an allocation may
    /// have opened a shelf, and taking it back would need that recorded;
    /// restoring a copy is simpler and the shelf list stays short.
    pub fn allocate_all(&mut self, sizes: &[(u32, u32)]) -> Option<Vec<Rect>> {
        let saved = self.clone();
        let rects = sizes
            .iter()
            .map(|&(w, h)| self.allocate(w, h))
            .collect::<Option<Vec<_>>>();
        if rects.is_none() {
            *self = saved;
        }
        rects
    }

    /// Extends the packable area downward; existing allocations stay valid.
    pub fn grow(&mut self, height: u32) {
        self.height = self.height.max(height);
    }

    pub fn clear(&mut self) {
        self.shelves.clear();
        self.next_y = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlaps(a: &Rect, b: &Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn allocations_stay_in_bounds_and_never_overlap() {
        let mut p = ShelfPacker::new(128, 128);
        let mut rects = Vec::new();
        for i in 0.. {
            let (w, h) = (5 + i % 7, 8 + i % 3 * 6);
            match p.allocate(w, h) {
                Some(r) => rects.push(r),
                None => break,
            }
        }
        assert!(rects.len() > 50, "packed only {}", rects.len());
        for (i, a) in rects.iter().enumerate() {
            assert!(a.x + a.w <= 128 && a.y + a.h <= 128);
            for b in &rects[i + 1..] {
                assert!(!overlaps(a, b), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn same_height_glyphs_share_a_shelf() {
        let mut p = ShelfPacker::new(64, 64);
        let a = p.allocate(10, 16).unwrap();
        let b = p.allocate(10, 16).unwrap();
        assert_eq!(a.y, b.y);
        assert_eq!(b.x, a.x + 10 + GAP);
    }

    #[test]
    fn short_glyph_prefers_the_tightest_shelf() {
        let mut p = ShelfPacker::new(64, 64);
        let tall = p.allocate(10, 30).unwrap();
        let short = p.allocate(10, 10).unwrap();
        let short2 = p.allocate(10, 9).unwrap();
        assert_eq!(tall.y, 0);
        // The 10px row was opened below the tall one; the 9px glyph joins it
        // rather than wasting the 30px row.
        assert_eq!(short.y, 31);
        assert_eq!(short2.y, 31);
    }

    #[test]
    fn short_glyph_falls_back_to_a_tall_shelf_when_out_of_rows() {
        let mut p = ShelfPacker::new(64, 32);
        let tall = p.allocate(10, 30).unwrap();
        let short = p.allocate(10, 10).unwrap();
        assert_eq!((short.x, short.y), (tall.x + 10 + GAP, 0));
    }

    #[test]
    fn too_wide_or_full_returns_none() {
        let mut p = ShelfPacker::new(32, 20);
        assert!(p.allocate(32, 4).is_none());
        assert!(p.allocate(10, 18).is_some());
        assert!(p.allocate(10, 18).is_some());
        assert!(p.allocate(31, 5).is_none());
    }

    #[test]
    fn grow_makes_room_without_moving_existing_glyphs() {
        let mut p = ShelfPacker::new(16, 16);
        let first = p.allocate(15, 15).unwrap();
        assert!(p.allocate(15, 15).is_none());
        p.grow(32);
        let second = p.allocate(15, 15).unwrap();
        assert_eq!(
            first,
            Rect {
                x: 0,
                y: 0,
                w: 15,
                h: 15
            }
        );
        assert_eq!(second.y, 16);
    }

    #[test]
    fn allocate_all_takes_nothing_when_one_size_does_not_fit() {
        let mut p = ShelfPacker::new(16, 16);
        assert!(p.allocate_all(&[(15, 7), (15, 15)]).is_none());
        assert_eq!(p.allocate(15, 15).unwrap().y, 0);
    }

    #[test]
    fn allocate_all_returns_one_rect_per_size_in_order() {
        let mut p = ShelfPacker::new(32, 32);
        let rects = p.allocate_all(&[(4, 8), (6, 8)]).unwrap();
        assert_eq!(
            rects.iter().map(|r| (r.w, r.h)).collect::<Vec<_>>(),
            [(4, 8), (6, 8)]
        );
    }

    #[test]
    fn clear_starts_over_from_the_top() {
        let mut p = ShelfPacker::new(16, 16);
        p.allocate(15, 15).unwrap();
        p.clear();
        assert_eq!(p.allocate(15, 15).unwrap().y, 0);
    }
}
