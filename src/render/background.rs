//! A picture behind the whole surface, under every cell.

use std::fmt;
use std::sync::Arc;

/// How the background image is fitted to the surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackgroundFit {
    /// Fills the surface keeping the aspect ratio; the overflow is cut off
    /// evenly on both sides.
    #[default]
    Cover,
    /// Fits inside the surface keeping the aspect ratio, centered; the rest
    /// shows the frame background.
    Contain,
    /// Fills the surface, ignoring the aspect ratio.
    Stretch,
}

/// Drawn over the frame background and under the cell backgrounds. Cells
/// whose background is the frame background are not filled, so the image
/// shows through them and the padding.
#[derive(Clone)]
pub struct BackgroundImage {
    pub width: u32,
    pub height: u32,
    /// 8-bit RGBA, not premultiplied, row by row.
    pub rgba: Arc<[u8]>,
    pub fit: BackgroundFit,
    /// 0 shows only the frame background, 1 the image as is.
    pub opacity: f32,
}

impl fmt::Debug for BackgroundImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackgroundImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("fit", &self.fit)
            .field("opacity", &self.opacity)
            .finish_non_exhaustive()
    }
}

/// Position and size on the surface, and the part of the image shown as
/// `[left, top, right, bottom]` fractions, for an `image`-sized picture on a
/// `surface`-sized target.
pub(crate) fn place(
    fit: BackgroundFit,
    image: (f32, f32),
    surface: (f32, f32),
) -> ([f32; 2], [f32; 2], [f32; 4]) {
    let full = [0.0, 0.0, 1.0, 1.0];
    let (iw, ih) = (image.0.max(1.0), image.1.max(1.0));
    let (sw, sh) = surface;
    match fit {
        BackgroundFit::Stretch => ([0.0, 0.0], [sw, sh], full),
        BackgroundFit::Contain => {
            let scale = (sw / iw).min(sh / ih);
            let (w, h) = (iw * scale, ih * scale);
            ([(sw - w) / 2.0, (sh - h) / 2.0], [w, h], full)
        }
        BackgroundFit::Cover => {
            let scale = (sw / iw).max(sh / ih);
            // Fractions of the image that fit on the surface.
            let (fx, fy) = (sw / (iw * scale), sh / (ih * scale));
            let (left, top) = ((1.0 - fx) / 2.0, (1.0 - fy) / 2.0);
            ([0.0, 0.0], [sw, sh], [left, top, left + fx, top + fy])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretch_fills_the_surface_with_the_whole_image() {
        let placed = place(BackgroundFit::Stretch, (100.0, 50.0), (300.0, 300.0));
        assert_eq!(placed, ([0.0, 0.0], [300.0, 300.0], [0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn contain_centers_the_whole_image_inside_the_surface() {
        // A wide image on a square surface: full width, bars above and below.
        let placed = place(BackgroundFit::Contain, (200.0, 100.0), (400.0, 400.0));
        assert_eq!(placed, ([0.0, 100.0], [400.0, 200.0], [0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn cover_fills_the_surface_and_cuts_the_overflow_evenly() {
        // A wide image on a square surface: the middle half of its width.
        let placed = place(BackgroundFit::Cover, (200.0, 100.0), (400.0, 400.0));
        assert_eq!(placed, ([0.0, 0.0], [400.0, 400.0], [0.25, 0.0, 0.75, 1.0]));
    }
}
