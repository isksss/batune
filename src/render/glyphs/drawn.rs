//! Block elements and Powerline separators drawn to fill the cell.

/// Alpha mask of `width` x `height` for `ch`, or `None` to use the font.
///
/// Why not take them from the font: their outlines are sized to the
/// font's own idea of a line, not to our cell. HackGen's blocks overflow
/// a cell built from its ascent and descent by several pixels, while its
/// Powerline glyphs fit it, so segments of a prompt never line up with
/// the cell backgrounds next to them, and `line_height` breaks them all.
pub(super) fn mask(ch: char, width: u32, height: u32) -> Option<Vec<u8>> {
    let eighths = |n: u32| n as f32 / 8.0;
    let quadrants = |ul: bool, ur: bool, ll: bool, lr: bool| {
        [
            (ul, [0.0, 0.0, 0.5, 0.5]),
            (ur, [0.5, 0.0, 1.0, 0.5]),
            (ll, [0.0, 0.5, 0.5, 1.0]),
            (lr, [0.5, 0.5, 1.0, 1.0]),
        ]
        .into_iter()
        .filter_map(|(on, r)| on.then_some(r))
        .collect::<Vec<_>>()
    };
    let rects = match ch {
        '\u{2580}' => vec![[0.0, 0.0, 1.0, 0.5]],
        '\u{2581}'..='\u{2588}' => vec![[0.0, 1.0 - eighths(ch as u32 - 0x2580), 1.0, 1.0]],
        '\u{2589}'..='\u{258f}' => vec![[0.0, 0.0, eighths(0x2590 - ch as u32), 1.0]],
        '\u{2590}' => vec![[0.5, 0.0, 1.0, 1.0]],
        '\u{2591}'..='\u{2593}' => {
            let alpha = (ch as u32 - 0x2590) * 64;
            return Some(vec![alpha as u8; (width * height) as usize]);
        }
        '\u{2594}' => vec![[0.0, 0.0, 1.0, 0.125]],
        '\u{2595}' => vec![[0.875, 0.0, 1.0, 1.0]],
        '\u{2596}' => quadrants(false, false, true, false),
        '\u{2597}' => quadrants(false, false, false, true),
        '\u{2598}' => quadrants(true, false, false, false),
        '\u{2599}' => quadrants(true, false, true, true),
        '\u{259a}' => quadrants(true, false, false, true),
        '\u{259b}' => quadrants(true, true, true, false),
        '\u{259c}' => quadrants(true, true, false, true),
        '\u{259d}' => quadrants(false, true, false, false),
        '\u{259e}' => quadrants(false, true, true, false),
        '\u{259f}' => quadrants(false, true, true, true),
        _ => return shape(ch, width, height),
    };
    // Edges snap to whole pixels so neighbouring blocks meet without a
    // half-transparent seam.
    let snap = |v: f32, size: u32| (v * size as f32).round() as u32;
    let mut out = vec![0; (width * height) as usize];
    for [x0, y0, x1, y1] in rects {
        for y in snap(y0, height)..snap(y1, height) {
            for x in snap(x0, width)..snap(x1, width) {
                out[(y * width + x) as usize] = 255;
            }
        }
    }
    Some(out)
}

/// Powerline separators, antialiased by sampling each pixel 4x4 times.
fn shape(ch: char, width: u32, height: u32) -> Option<Vec<u8>> {
    let inside: fn(f32, f32) -> bool = match ch {
        '\u{e0b0}' => |x, y| x <= 1.0 - (2.0 * y - 1.0).abs(),
        '\u{e0b2}' => |x, y| x >= (2.0 * y - 1.0).abs(),
        '\u{e0b4}' => |x, y| x * x + (2.0 * y - 1.0).powi(2) <= 1.0,
        '\u{e0b6}' => |x, y| (1.0 - x).powi(2) + (2.0 * y - 1.0).powi(2) <= 1.0,
        '\u{e0b8}' => |x, y| y >= x,
        '\u{e0ba}' => |x, y| y >= 1.0 - x,
        '\u{e0bc}' => |x, y| y <= 1.0 - x,
        '\u{e0be}' => |x, y| y <= x,
        _ => return None,
    };
    const N: u32 = 4;
    let mut out = Vec::with_capacity((width * height) as usize);
    for py in 0..height {
        for px in 0..width {
            let hits = (0..N * N)
                .filter(|i| {
                    let x = (px as f32 + ((i % N) as f32 + 0.5) / N as f32) / width as f32;
                    let y = (py as f32 + ((i / N) as f32 + 0.5) / N as f32) / height as f32;
                    inside(x, y)
                })
                .count() as u32;
            out.push((hits * 255 / (N * N)) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(mask: &[u8], width: u32, x: u32, y: u32) -> u8 {
        mask[(y * width + x) as usize]
    }

    #[test]
    fn full_block_covers_the_whole_cell() {
        assert_eq!(mask('\u{2588}', 9, 21), Some(vec![255; 9 * 21]));
    }

    #[test]
    fn half_blocks_snap_to_whole_pixels() {
        let m = mask('\u{2584}', 2, 5).unwrap();
        let rows: Vec<u8> = (0..5).map(|y| at(&m, 2, 0, y)).collect();
        assert_eq!(rows, [0, 0, 0, 255, 255]);
        let upper = mask('\u{2580}', 2, 5).unwrap();
        let upper_rows: Vec<u8> = (0..5).map(|y| at(&upper, 2, 0, y)).collect();
        assert_eq!(upper_rows, [255, 255, 255, 0, 0]);
    }

    #[test]
    fn quadrants_fill_their_corners() {
        let m = mask('\u{259a}', 4, 4).unwrap();
        assert_eq!(
            [
                at(&m, 4, 0, 0),
                at(&m, 4, 3, 0),
                at(&m, 4, 0, 3),
                at(&m, 4, 3, 3)
            ],
            [255, 0, 0, 255]
        );
    }

    #[test]
    fn shades_are_uniform() {
        assert_eq!(mask('\u{2591}', 3, 4), Some(vec![64; 12]));
        assert_eq!(mask('\u{2593}', 3, 4), Some(vec![192; 12]));
    }

    #[test]
    fn powerline_caps_span_the_cell_height() {
        let (w, h) = (10, 20);
        let right = mask('\u{e0b4}', w, h).unwrap();
        assert_eq!(at(&right, w, 0, h / 2), 255);
        assert_eq!(at(&right, w, w - 1, h / 2), 255);
        assert_eq!(at(&right, w, w - 1, 0), 0);
        assert!(at(&right, w, 0, 0) > 0 && at(&right, w, 0, h - 1) > 0);
        let left = mask('\u{e0b6}', w, h).unwrap();
        assert_eq!(at(&left, w, w - 1, 0), at(&right, w, 0, 0));
        assert_eq!(at(&left, w, 0, 0), 0);
    }

    #[test]
    fn other_characters_use_the_font() {
        assert_eq!(mask('M', 9, 21), None);
        assert_eq!(mask('\u{e0b1}', 9, 21), None);
    }
}
