//! iTerm2 inline images: `OSC 1337 ; File=[args] : <base64> BEL`.
//! <https://iterm2.com/documentation-images.html>

use std::io::Cursor;

use crate::vt::image::{Image, MAX_IMAGE_CELLS, MAX_IMAGE_PIXELS};

/// `width=` / `height=`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Dimension {
    Auto,
    Cells(f32),
    Pixels(f32),
    Percent(f32),
}

impl Dimension {
    fn parse(s: &str) -> Self {
        let (num, unit): (&str, fn(f32) -> Self) = if let Some(n) = s.strip_suffix("px") {
            (n, Self::Pixels)
        } else if let Some(n) = s.strip_suffix('%') {
            (n, Self::Percent)
        } else {
            (s, Self::Cells)
        };
        match num.parse::<f32>() {
            Ok(v) if v.is_finite() && v > 0.0 => unit(v),
            _ => Self::Auto,
        }
    }

    /// In pixels; `whole` is the terminal's size in pixels along this axis.
    fn pixels(self, cell: f32, whole: f32) -> Option<f32> {
        match self {
            Self::Auto => None,
            Self::Cells(n) => Some(n * cell),
            Self::Pixels(n) => Some(n),
            Self::Percent(p) => Some(p / 100.0 * whole),
        }
    }
}

#[derive(Debug)]
pub(crate) struct File {
    width: Dimension,
    height: Dimension,
    preserve_aspect_ratio: bool,
    pub(crate) data: Vec<u8>,
}

/// Parses the payload after `1337;`. `None` unless it is an inline file
/// (`inline=0` asks for a download, which a terminal emulator library has
/// no business doing).
pub(crate) fn parse(payload: &[u8]) -> Option<File> {
    let rest = payload.strip_prefix(b"File=")?;
    let colon = rest.iter().position(|&b| b == b':')?;
    let args = std::str::from_utf8(&rest[..colon]).ok()?;
    let mut file = File {
        width: Dimension::Auto,
        height: Dimension::Auto,
        preserve_aspect_ratio: true,
        data: Vec::new(),
    };
    let mut inline = false;
    for arg in args.split(';') {
        let (key, value) = arg.split_once('=').unwrap_or((arg, ""));
        match key {
            "inline" => inline = value == "1",
            "width" => file.width = Dimension::parse(value),
            "height" => file.height = Dimension::parse(value),
            "preserveAspectRatio" => file.preserve_aspect_ratio = value != "0",
            _ => {}
        }
    }
    if !inline {
        return None;
    }
    file.data = crate::vt::base64::decode(&rest[colon + 1..])?;
    Some(file)
}

/// Longest side of an image kept in memory when the source is larger;
/// still finer than any screen the terminal would be shown on.
const MAX_SIDE: u32 = 4096;

/// Decodes PNG, JPEG or GIF (first frame) into RGBA. Images over
/// [`MAX_IMAGE_PIXELS`] are scaled down instead of rejected, since photos
/// from cameras are often bigger.
pub(crate) fn decode(data: &[u8]) -> Option<Image> {
    let mut reader = image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1 << 15);
    limits.max_image_height = Some(1 << 15);
    reader.limits(limits);
    let mut img = reader.decode().ok()?;
    if u64::from(img.width()) * u64::from(img.height()) > MAX_IMAGE_PIXELS {
        img = img.thumbnail(MAX_SIDE, MAX_SIDE);
    }
    let rgba = img.into_rgba8();
    Image::from_rgba(rgba.width(), rgba.height(), rgba.into_raw())
}

/// Size in cells for an image of `image` pixels, given the cell size and
/// the terminal size in cells. Never wider than the terminal; height is
/// capped so a bogus request cannot scroll forever.
pub(crate) fn size_in_cells(
    file: &File,
    image: (u32, u32),
    cell: (u32, u32),
    term: (usize, usize),
) -> (f32, f32) {
    let (cw, ch) = (cell.0 as f32, cell.1 as f32);
    let (iw, ih) = (image.0 as f32, image.1 as f32);
    let (tw, th) = (term.0 as f32 * cw, term.1 as f32 * ch);
    let (w, h) = match (file.width.pixels(cw, tw), file.height.pixels(ch, th)) {
        (None, None) => (iw, ih),
        (Some(w), None) => (w, ih * w / iw),
        (None, Some(h)) => (iw * h / ih, h),
        (Some(w), Some(h)) if file.preserve_aspect_ratio => {
            let scale = (w / iw).min(h / ih);
            (iw * scale, ih * scale)
        }
        (Some(w), Some(h)) => (w, h),
    };
    let shrink = (tw / w).min(1.0);
    let (w, h) = (w * shrink, h * shrink);
    (
        (w / cw).max(1.0 / cw),
        (h / ch).clamp(1.0 / ch, MAX_IMAGE_CELLS),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(args: &str) -> File {
        parse(format!("File={args}:AAAA").as_bytes()).unwrap()
    }

    #[test]
    fn dimensions_take_cells_pixels_percent_or_auto() {
        assert_eq!(Dimension::parse("3"), Dimension::Cells(3.0));
        assert_eq!(Dimension::parse("10px"), Dimension::Pixels(10.0));
        assert_eq!(Dimension::parse("50%"), Dimension::Percent(50.0));
        assert_eq!(Dimension::parse("auto"), Dimension::Auto);
        assert_eq!(Dimension::parse("-1"), Dimension::Auto);
    }

    #[test]
    fn only_inline_files_are_accepted() {
        assert!(parse(b"File=inline=1:AAAA").is_some());
        assert!(parse(b"File=name=eA==;inline=0:AAAA").is_none());
        assert!(parse(b"File=size=3:AAAA").is_none());
        assert!(parse(b"File=inline=1").is_none());
    }

    #[test]
    fn auto_size_is_the_native_size() {
        let f = file("inline=1");
        assert_eq!(size_in_cells(&f, (40, 32), (8, 16), (80, 24)), (5.0, 2.0));
    }

    #[test]
    fn one_given_side_scales_the_other() {
        let f = file("inline=1;width=10");
        assert_eq!(size_in_cells(&f, (40, 32), (8, 16), (80, 24)), (10.0, 4.0));
    }

    #[test]
    fn both_sides_fit_inside_when_preserving_the_aspect_ratio() {
        let f = file("inline=1;width=10;height=100px");
        assert_eq!(size_in_cells(&f, (40, 32), (8, 16), (80, 24)), (10.0, 4.0));
        let f = file("inline=1;width=10;height=100px;preserveAspectRatio=0");
        assert_eq!(size_in_cells(&f, (40, 32), (8, 16), (80, 24)), (10.0, 6.25));
    }

    #[test]
    fn images_wider_than_the_terminal_are_shrunk() {
        let f = file("inline=1;width=200%");
        assert_eq!(size_in_cells(&f, (40, 32), (8, 16), (80, 24)), (80.0, 32.0));
    }
}
