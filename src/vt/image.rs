//! Images shown in the grid (iTerm2 inline images, sixel, kitty graphics).
//!
//! An image is attached to the cells it covers: each cell holds an
//! [`ImageCell`] naming the placement and the cell's offset inside it. The
//! image therefore scrolls, moves into the scrollback and disappears with
//! the cells, and printing or erasing over a cell removes that piece.
//! kitty placements are the exception: kitty draws them on a layer of their
//! own, so printing and erasing keep them (see `ImageCell::layered`).

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

/// Largest image accepted, in pixels (64 MiB of RGBA).
pub const MAX_IMAGE_PIXELS: u64 = 4096 * 4096;

/// Largest side of a placement, in cells. Each row is a linefeed and a row
/// of cell writes, so a bogus `r=20000000` would otherwise keep the terminal
/// busy for minutes and flush the whole scrollback.
pub(crate) const MAX_IMAGE_CELLS: f32 = 1000.0;

/// Image memory kept by default, in bytes of decoded RGBA.
pub const DEFAULT_IMAGE_MEMORY_LIMIT: usize = 256 << 20;

/// Decoded pixels, shared by every cell that shows them.
pub struct Image {
    id: u64,
    width: u32,
    height: u32,
    rgba: Arc<[u8]>,
}

impl Image {
    /// `rgba` is 8-bit RGBA, not premultiplied, row by row. `None` when the
    /// length does not match, a side is zero or the image is larger than
    /// [`MAX_IMAGE_PIXELS`].
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let pixels = u64::from(width) * u64::from(height);
        if pixels == 0 || pixels > MAX_IMAGE_PIXELS || rgba.len() as u64 != pixels * 4 {
            return None;
        }
        Some(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            width,
            height,
            rgba: rgba.into(),
        })
    }

    /// Unique within the process, so a renderer can cache textures by it.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Shared so that a renderer can keep the pixels without copying them.
    pub fn rgba(&self) -> &Arc<[u8]> {
        &self.rgba
    }

    fn byte_len(&self) -> usize {
        self.rgba.len()
    }
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Image")
            .field("id", &self.id)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Where the cursor goes after [`Term::place_image`](crate::vt::Term::place_image).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageCursor {
    /// The cursor does not move and the image is cut at the bottom of the
    /// screen instead of scrolling it (kitty `C=1`).
    Stay,
    /// The first column of the line below the image (sixel).
    NextLine,
    /// Just right of the image on its last row (iTerm2).
    #[default]
    AfterImage,
}

/// How an image is laid over the grid, starting at the cursor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Size of the drawn image in cells. Fractions leave the rest of the
    /// last column / row empty; the image covers `ceil` of each.
    pub width: f32,
    pub height: f32,
    /// The part of the image shown, as `[left, top, right, bottom]` in
    /// fractions of its size: `[0.0, 0.0, 1.0, 1.0]` is all of it.
    pub source: [f32; 4],
    /// Stacking order for renderers: below text when negative, above
    /// otherwise (kitty's convention).
    pub z: i32,
    pub cursor: ImageCursor,
}

impl Placement {
    /// The whole image, `width` x `height` cells, below text, cursor after
    /// the image.
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            source: [0.0, 0.0, 1.0, 1.0],
            z: -1,
            cursor: ImageCursor::default(),
        }
    }

    pub(crate) fn cells(&self) -> (usize, usize) {
        let cells = |v: f32| (v.ceil() as usize).max(1);
        (cells(self.width), cells(self.height))
    }
}

#[derive(Debug)]
struct Placed {
    id: u64,
    image: Arc<Image>,
    placement: Placement,
    layered: bool,
}

/// The piece of a placed image that one cell shows.
#[derive(Clone, Debug)]
pub struct ImageCell {
    placed: Arc<Placed>,
    col: u32,
    row: u32,
}

impl PartialEq for ImageCell {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.placed, &other.placed) && self.col == other.col && self.row == other.row
    }
}

impl Eq for ImageCell {}

impl ImageCell {
    pub fn image(&self) -> &Arc<Image> {
        &self.placed.image
    }

    pub fn z(&self) -> i32 {
        self.placed.placement.z
    }

    /// Unique within the process for each [`Term::place_image`](crate::vt::Term::place_image) call.
    pub(crate) fn placement_id(&self) -> u64 {
        self.placed.id
    }

    /// Whether printing and erasing leave this piece in place (kitty
    /// graphics). Other images belong to the text they replaced.
    pub(crate) fn layered(&self) -> bool {
        self.placed.layered
    }

    /// The cell's column and row inside the placement.
    pub fn offset(&self) -> (usize, usize) {
        (self.col as usize, self.row as usize)
    }

    /// Whether both cells come from the same placement, i.e. a renderer can
    /// merge neighbors into one quad.
    pub fn same_placement(&self, other: &ImageCell) -> bool {
        Arc::ptr_eq(&self.placed, &other.placed)
    }

    /// The covered part of the cell from its top-left corner, as fractions
    /// of the cell's width and height (1.0 except at the image's right and
    /// bottom edges).
    pub fn extent(&self) -> (f32, f32) {
        let p = &self.placed.placement;
        let cover = |size: f32, i: u32| (size - i as f32).clamp(0.0, 1.0);
        (cover(p.width, self.col), cover(p.height, self.row))
    }

    /// Texture coordinates `[left, top, right, bottom]` of the covered part,
    /// in fractions of the image size.
    pub fn uv(&self) -> [f32; 4] {
        let p = &self.placed.placement;
        let (w, h) = self.extent();
        let [l, t, r, b] = p.source;
        let u = |i: f32| l + (r - l) * i / p.width;
        let v = |i: f32| t + (b - t) * i / p.height;
        let (x, y) = (self.col as f32, self.row as f32);
        [u(x), v(y), u(x + w), v(y + h)]
    }
}

/// Builds the [`ImageCell`]s of one placement.
pub(crate) struct PlacedCells(Arc<Placed>);

impl PlacedCells {
    pub(crate) fn new(image: Arc<Image>, placement: Placement, layered: bool) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self(Arc::new(Placed {
            id,
            image,
            placement,
            layered,
        }))
    }

    /// Identifies the placement for [`ImageCell::placement_id`].
    pub(crate) fn id(&self) -> u64 {
        self.0.id
    }

    pub(crate) fn cell(&self, col: usize, row: usize) -> ImageCell {
        ImageCell {
            placed: self.0.clone(),
            col: col as u32,
            row: row as u32,
        }
    }
}

/// Tracks the images still shown somewhere, oldest first, to enforce the
/// memory limit. Holding [`Weak`] references means images whose cells were
/// all overwritten are freed without the store being told.
#[derive(Debug)]
pub(crate) struct ImageStore {
    images: VecDeque<Weak<Image>>,
    limit: usize,
}

impl Default for ImageStore {
    fn default() -> Self {
        Self {
            images: VecDeque::new(),
            limit: DEFAULT_IMAGE_MEMORY_LIMIT,
        }
    }
}

impl ImageStore {
    pub(crate) fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    /// Records `image` and returns the ids of the oldest images that must be
    /// removed from the grid to get back under the limit.
    pub(crate) fn add(&mut self, image: &Arc<Image>) -> Vec<u64> {
        if !self.images.iter().any(|w| w.as_ptr() == Arc::as_ptr(image)) {
            self.images.push_back(Arc::downgrade(image));
        }
        self.over_limit()
    }

    pub(crate) fn over_limit(&mut self) -> Vec<u64> {
        self.images.retain(|w| w.strong_count() > 0);
        let mut total: usize = self.live().map(|i| i.byte_len()).sum();
        let mut evict = Vec::new();
        while total > self.limit {
            let Some(image) = self.images.pop_front().and_then(|w| w.upgrade()) else {
                break;
            };
            total -= image.byte_len();
            evict.push(image.id());
        }
        evict
    }

    fn live(&self) -> impl Iterator<Item = Arc<Image>> + '_ {
        self.images.iter().filter_map(Weak::upgrade)
    }
}
