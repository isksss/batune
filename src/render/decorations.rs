//! Underline shapes as rectangles, so they go through the same pipeline as
//! the straight underline and the cursor instead of needing a shader or
//! glyphs of their own.

use crate::render::frame::CellFlags;
use crate::render::metrics::CellMetrics;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Style {
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

impl Style {
    pub(crate) fn of(flags: CellFlags) -> Option<Self> {
        [
            (CellFlags::UNDERLINE, Style::Single),
            (CellFlags::DOUBLE_UNDERLINE, Style::Double),
            (CellFlags::CURLY_UNDERLINE, Style::Curly),
            (CellFlags::DOTTED_UNDERLINE, Style::Dotted),
            (CellFlags::DASHED_UNDERLINE, Style::Dashed),
        ]
        .into_iter()
        .find_map(|(flag, style)| flags.contains(flag).then_some(style))
    }
}

/// `[x, y, width, height]` relative to the cell's top-left corner.
pub(crate) type Rect = [f32; 4];

/// Rectangles for an underline across `width` pixels of a cell whose left
/// edge is `grid_x` pixels from the grid's left edge. Patterns are phased on
/// `grid_x`, so neighbouring cells continue the same dots, dashes and waves.
pub(crate) fn underline(style: Style, m: &CellMetrics, grid_x: u32, width: u32) -> Vec<Rect> {
    let s = m.stroke;
    let y = m.underline_y;
    let stroke = s as f32;
    match style {
        Style::Single => vec![[0.0, y as f32, width as f32, stroke]],
        Style::Double => {
            // Two strokes with a stroke-wide gap; moved up when the font's
            // underline sits too low for the second one to fit in the cell.
            let top = y.min(m.height.saturating_sub(3 * s)) as f32;
            vec![
                [0.0, top, width as f32, stroke],
                [0.0, top + 2.0 * stroke, width as f32, stroke],
            ]
        }
        Style::Dotted => pattern(grid_x, width, s, s + s.max(2), [y, s]),
        Style::Dashed => {
            let period = (m.width / 2).max(4);
            pattern(grid_x, width, period - period / 3, period, [y, s])
        }
        Style::Curly => curly(m, grid_x, width),
    }
}

/// Segments `on` pixels long every `period` pixels of the grid row, at
/// `[top, height]`.
fn pattern(grid_x: u32, width: u32, on: u32, period: u32, [y, h]: [u32; 2]) -> Vec<Rect> {
    let end = grid_x + width;
    let mut out = Vec::new();
    let mut start = grid_x - grid_x % period;
    while start < end {
        let (left, right) = (start.max(grid_x), (start + on).min(end));
        if left < right {
            out.push([
                (left - grid_x) as f32,
                y as f32,
                (right - left) as f32,
                h as f32,
            ]);
        }
        start += period;
    }
    out
}

/// One sine period per cell width, drawn as one column of pixels at a time
/// tall enough to join the neighbouring column.
fn curly(m: &CellMetrics, grid_x: u32, width: u32) -> Vec<Rect> {
    let stroke = m.stroke as f32;
    let amplitude = (stroke * 1.5).max(1.5);
    let band = 2.0 * amplitude + stroke;
    let height = m.height as f32;
    let top = (m.underline_y as f32 - amplitude).clamp(0.0, (height - band).max(0.0));
    let center = top + amplitude;
    let period = m.width.max(1) as f32;
    let wave = |x: f32| {
        let phase = std::f32::consts::TAU * (grid_x as f32 + x) / period;
        (center - amplitude * phase.sin()).round()
    };
    (0..width)
        .map(|px| {
            let x = px as f32;
            let (a, b) = (wave(x), wave(x + 1.0));
            [x, a.min(b), 1.0, (a - b).abs().max(0.0) + stroke]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: CellMetrics = CellMetrics {
        width: 10,
        height: 20,
        baseline: 15,
        underline_y: 17,
        strikeout_y: 10,
        stroke: 1,
        glyph_x: 0,
    };

    fn within_cell(rects: &[Rect], width: u32) {
        for &[x, y, w, h] in rects {
            assert!(x >= 0.0 && x + w <= width as f32, "{rects:?}");
            assert!(y >= 0.0 && y + h <= M.height as f32, "{rects:?}");
        }
    }

    #[test]
    fn style_follows_the_flags() {
        assert_eq!(Style::of(CellFlags::BOLD), None);
        assert_eq!(Style::of(CellFlags::UNDERLINE), Some(Style::Single));
        assert_eq!(Style::of(CellFlags::CURLY_UNDERLINE), Some(Style::Curly));
    }

    #[test]
    fn single_is_one_stroke_at_the_font_underline() {
        assert_eq!(
            underline(Style::Single, &M, 0, 10),
            vec![[0.0, 17.0, 10.0, 1.0]]
        );
    }

    #[test]
    fn double_fits_two_strokes_in_the_cell() {
        let rects = underline(Style::Double, &M, 0, 10);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[1][1] - rects[0][1], 2.0);
        within_cell(&rects, 10);
        let low = CellMetrics {
            underline_y: 19,
            ..M
        };
        let rects = underline(Style::Double, &low, 0, 10);
        assert!(rects[1][1] + 1.0 <= 20.0, "{rects:?}");
    }

    #[test]
    fn dots_leave_gaps_and_continue_into_the_next_cell() {
        let first = pattern(0, 10, 1, 3, [17, 1]);
        let starts: Vec<f32> = first.iter().map(|r| r[0]).collect();
        assert_eq!(starts, [0.0, 3.0, 6.0, 9.0]);
        // The next cell starts at grid x 10: the dot at 12 is its x 2.
        let second = pattern(10, 10, 1, 3, [17, 1]);
        assert_eq!(second[0][0], 2.0);
    }

    #[test]
    fn a_segment_crossing_the_cell_edge_is_cut() {
        let rects = pattern(10, 10, 4, 6, [17, 1]);
        // The dash at 6..10 ends at the edge; the next starts at 12.
        assert_eq!(rects[0][0], 2.0);
        within_cell(&rects, 10);
        // The dash at 0..4 started in the previous cell, which begins at 2.
        assert_eq!(pattern(2, 10, 4, 6, [17, 1])[0], [0.0, 17.0, 2.0, 1.0]);
    }

    #[test]
    fn dashes_and_dots_stay_on_the_underline() {
        for style in [Style::Dotted, Style::Dashed] {
            let rects = underline(style, &M, 30, 10);
            assert!(!rects.is_empty());
            assert!(rects.iter().all(|r| r[1] == 17.0), "{style:?}");
            within_cell(&rects, 10);
        }
    }

    #[test]
    fn curly_covers_every_column_and_waves_inside_the_cell() {
        let rects = underline(Style::Curly, &M, 0, 20);
        assert_eq!(rects.len(), 20);
        within_cell(&rects, 20);
        let tops: Vec<f32> = rects.iter().map(|r| r[1]).collect();
        let (min, max) = tops
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), &t| (a.min(t), b.max(t)));
        assert!(max - min >= 2.0, "flat: {tops:?}");
    }
}
