//! What kitty graphics commands do to the terminal. Parsing and decoding are
//! in `crate::vt::kitty`.

use std::collections::HashSet;
use std::sync::Arc;

use super::Term;
use crate::vt::image::{Image, ImageCell, ImageCursor, MAX_IMAGE_CELLS, Placement};
use crate::vt::kitty::{self, Assembler, Command, Keys};

/// Most transmitted images kept. Lookups by id scan the list, so a stream
/// of tiny images must not grow it without bound; kitty's own limit is by
/// bytes only, but no real program keeps this many.
pub(super) const MAX_STORED_IMAGES: usize = 4096;

/// kitty graphics state kept by the terminal.
#[derive(Debug, Default)]
pub(super) struct Kitty {
    assembler: Assembler,
    /// Transmitted images that can be placed (again), oldest first.
    images: Vec<Stored>,
    /// Placements possibly still on the grid. Entries whose cells were all
    /// overwritten are pruned once the list doubles.
    placements: Vec<Placed>,
    prune_at: usize,
    next_id: u32,
}

#[derive(Debug)]
struct Stored {
    id: u32,
    number: Option<u32>,
    image: Arc<Image>,
}

#[derive(Debug)]
struct Placed {
    /// `None` for an image sent without an id.
    image: Option<u32>,
    placement: Option<u32>,
    /// [`crate::vt::image::ImageCell::placement_id`] of its cells.
    cells: u64,
}

impl Kitty {
    fn find(&self, id: Option<u32>, number: Option<u32>) -> Option<&Stored> {
        match (id, number) {
            (Some(id), _) => self.images.iter().find(|s| s.id == id),
            // The newest image with that number.
            (None, Some(n)) => self.images.iter().rev().find(|s| s.number == Some(n)),
            (None, None) => None,
        }
    }

    /// An id for an image sent with only a number (`I`). kitty answers with
    /// it, so it must not clash with ids the application chose; those are
    /// usually small, so these count down from the top.
    fn fresh_id(&mut self) -> u32 {
        loop {
            self.next_id = self.next_id.wrapping_sub(1).max(1);
            let id = self.next_id;
            if !self.images.iter().any(|s| s.id == id) {
                return id;
            }
        }
    }
}

impl Term {
    pub(super) fn kitty_command(&mut self, body: &[u8]) {
        let Some(Command { mut keys, data }) = self.kitty.assembler.push(body) else {
            return;
        };
        let result = match keys.action {
            b't' | b'T' | b'q' => data.and_then(|data| self.kitty_transmit(&mut keys, &data)),
            b'p' => self.kitty_place_stored(&keys),
            // kitty does not answer deletions.
            b'd' => return self.kitty_delete(&keys),
            _ => Err("EINVAL:unsupported action"),
        };
        let message = result.err().unwrap_or("OK");
        if let Some(reply) = kitty::reply(&keys, message) {
            self.responses.extend_from_slice(reply.as_bytes());
        }
    }

    /// `a=t`, `a=T` and `a=q`. Fills in the id given to an image sent with
    /// only a number, for the reply.
    fn kitty_transmit(&mut self, keys: &mut Keys, data: &[u8]) -> Result<(), &'static str> {
        let image = Arc::new(kitty::decode_image(keys, data)?);
        if keys.action == b'q' {
            return Ok(());
        }
        if keys.id.is_none() && keys.number.is_some() {
            keys.id = Some(self.kitty.fresh_id());
        }
        if let Some(id) = keys.id {
            // Sending an id again replaces the image and its placements.
            self.kitty_delete_images(|s| s.id == id);
            self.kitty.images.push(Stored {
                id,
                number: keys.number,
                image: image.clone(),
            });
            self.kitty_enforce_limit();
        }
        // Without an id the image cannot be placed again, so it lives only
        // in the cells of this placement.
        if keys.action == b'T' {
            self.kitty_place(image, keys.id, keys)?;
        }
        Ok(())
    }

    /// `a=p`.
    fn kitty_place_stored(&mut self, keys: &Keys) -> Result<(), &'static str> {
        let (id, image) = self
            .kitty
            .find(keys.id, keys.number)
            .map(|s| (s.id, s.image.clone()))
            .ok_or("ENOENT:no image with this id")?;
        self.kitty_place(image, Some(id), keys)
    }

    /// `id` is the stored image's, also when `keys` named it by number.
    fn kitty_place(
        &mut self,
        image: Arc<Image>,
        id: Option<u32>,
        keys: &Keys,
    ) -> Result<(), &'static str> {
        let placement = placement(keys, &image, self.cell_pixels)?;
        if let (Some(i), Some(p)) = (id, keys.placement) {
            // A placement id names one placement of the image: the new one
            // replaces it.
            self.kitty_delete_placements(|q| q.image == Some(i) && q.placement == Some(p));
        }
        let cells = self.put_image(image, placement, true);
        self.kitty.placements.push(Placed {
            image: id,
            placement: keys.placement,
            cells,
        });
        if self.kitty.placements.len() > self.kitty.prune_at {
            let mut live = HashSet::new();
            self.grid.placement_ids(&mut live);
            self.inactive.placement_ids(&mut live);
            self.kitty.placements.retain(|p| live.contains(&p.cells));
            self.kitty.prune_at = (self.kitty.placements.len() * 2).max(256);
        }
        Ok(())
    }

    /// `a=d`. Lowercase `d` removes placements; uppercase also frees the
    /// images left without any. Animation frames (`d=f`) do not exist here.
    fn kitty_delete(&mut self, keys: &Keys) {
        let free = keys.delete.is_ascii_uppercase();
        let [x, y, ..] = keys.source;
        // Cell positions in the keys are 1-based.
        let (col, row) = (x.saturating_sub(1) as usize, y.saturating_sub(1) as usize);
        let cursor = (self.cursor.row, self.cursor.col);
        let z = keys.z;
        let mut free_ids = Vec::new();
        let gone: HashSet<u64> = match keys.delete.to_ascii_lowercase() {
            b'a' => {
                if free {
                    free_ids.extend(self.kitty.images.iter().map(|s| s.id));
                }
                self.kitty.placements.iter().map(|p| p.cells).collect()
            }
            b'i' | b'n' => {
                let Some(id) = self.kitty.find(keys.id, keys.number).map(|s| s.id) else {
                    return;
                };
                if free && keys.placement.is_none() {
                    free_ids.push(id);
                }
                let placement = keys.placement;
                self.kitty
                    .placements
                    .iter()
                    .filter(|p| {
                        p.image == Some(id) && placement.is_none_or(|q| p.placement == Some(q))
                    })
                    .map(|p| p.cells)
                    .collect()
            }
            b'r' => self
                .kitty
                .placements
                .iter()
                .filter(|p| ids_of(p).is_some_and(|i| (x..=y).contains(&i)))
                .map(|p| p.cells)
                .collect(),
            b'c' => self.placements_on_screen(|r, c, _| (r, c) == cursor),
            b'p' => self.placements_on_screen(|r, c, _| (r, c) == (row, col)),
            b'q' => self.placements_on_screen(|r, c, i| (r, c) == (row, col) && i.z() == z),
            b'x' => self.placements_on_screen(|_, c, _| c == col),
            b'y' => self.placements_on_screen(|r, _, _| r == row),
            b'z' => self.placements_on_screen(|_, _, i| i.z() == z),
            _ => return,
        };
        let touched: HashSet<u32> = self
            .kitty
            .placements
            .iter()
            .filter(|p| gone.contains(&p.cells))
            .flat_map(ids_of)
            .collect();
        // Only kitty's own placements: position-based deletion must not
        // remove sixel or iTerm2 images in the same cells.
        self.kitty_delete_placements(|p| gone.contains(&p.cells));
        if !free {
            return;
        }
        let placed: HashSet<u32> = self.kitty.placements.iter().flat_map(ids_of).collect();
        self.kitty_delete_images(|s| {
            free_ids.contains(&s.id) || (touched.contains(&s.id) && !placed.contains(&s.id))
        });
    }

    /// Placements with a cell on the screen (not the scrollback) that
    /// `matches` its position and piece.
    fn placements_on_screen(
        &self,
        matches: impl Fn(usize, usize, &ImageCell) -> bool,
    ) -> HashSet<u64> {
        let mut found = HashSet::new();
        for r in 0..self.rows() {
            for (c, cell) in self.grid.row(r).cells().iter().enumerate() {
                if let Some(i) = cell.image().filter(|i| matches(r, c, i)) {
                    found.insert(i.placement_id());
                }
            }
        }
        found
    }

    fn kitty_delete_placements(&mut self, matches: impl Fn(&Placed) -> bool) {
        let (gone, kept) = std::mem::take(&mut self.kitty.placements)
            .into_iter()
            .partition::<Vec<_>, _>(|p| matches(p));
        self.kitty.placements = kept;
        let ids: Vec<u64> = gone.iter().map(|p| p.cells).collect();
        if !ids.is_empty() {
            self.grid.remove_placements(&ids);
            self.inactive.remove_placements(&ids);
        }
    }

    /// Frees images and removes their placements.
    fn kitty_delete_images(&mut self, matches: impl Fn(&Stored) -> bool) {
        let (gone, kept) = std::mem::take(&mut self.kitty.images)
            .into_iter()
            .partition::<Vec<_>, _>(|s| matches(s));
        self.kitty.images = kept;
        let ids: Vec<u32> = gone.iter().map(|s| s.id).collect();
        self.kitty_delete_placements(|p| p.image.is_some_and(|i| ids.contains(&i)));
    }

    /// Stored images count against the image memory limit on their own,
    /// since they are kept even when shown nowhere, and against
    /// [`MAX_STORED_IMAGES`]. The oldest go first.
    fn kitty_enforce_limit(&mut self) {
        let limit = self.images.limit();
        let mut total: usize = self.kitty.images.iter().map(|s| s.image.rgba().len()).sum();
        let mut count = self.kitty.images.len();
        let mut evict = Vec::new();
        for s in &self.kitty.images {
            if total <= limit && count <= MAX_STORED_IMAGES {
                break;
            }
            total -= s.image.rgba().len();
            count -= 1;
            evict.push(s.id);
        }
        self.kitty_delete_images(|s| evict.contains(&s.id));
    }
}

fn ids_of(p: &Placed) -> Option<u32> {
    p.image
}

/// Size in cells and shown part of a placement. Without `c`/`r` the image
/// keeps its pixel size; with one of them the other follows the aspect ratio.
/// Each side is capped at [`MAX_IMAGE_CELLS`] like iTerm2 images.
fn placement(keys: &Keys, image: &Image, cell: (u32, u32)) -> Result<Placement, &'static str> {
    let (w, h) = (image.width(), image.height());
    let [x, y, sw, sh] = keys.source;
    let (x, y) = (x.min(w), y.min(h));
    let part = |size: u32, start: u32, len: u32| match len {
        0 => size - start,
        len => len.min(size - start),
    };
    let (sw, sh) = (part(w, x, sw), part(h, y, sh));
    if sw == 0 || sh == 0 {
        return Err("EINVAL:the source rectangle is empty");
    }
    let (cw, ch) = (cell.0 as f32, cell.1 as f32);
    let (sw_f, sh_f) = (sw as f32, sh as f32);
    let (width, height) = match (keys.cols, keys.rows) {
        (0, 0) => (sw_f / cw, sh_f / ch),
        (c, 0) => (c as f32, c as f32 * cw * sh_f / sw_f / ch),
        (0, r) => (r as f32 * ch * sw_f / sh_f / cw, r as f32),
        (c, r) => (c as f32, r as f32),
    };
    let (width, height) = (width.min(MAX_IMAGE_CELLS), height.min(MAX_IMAGE_CELLS));
    let (wf, hf) = (w as f32, h as f32);
    Ok(Placement {
        width,
        height,
        source: [
            x as f32 / wf,
            y as f32 / hf,
            (x + sw) as f32 / wf,
            (y + sh) as f32 / hf,
        ],
        z: keys.z,
        cursor: if keys.cursor_stay {
            ImageCursor::Stay
        } else {
            ImageCursor::AfterImage
        },
    })
}
