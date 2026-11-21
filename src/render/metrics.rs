//! Cell geometry derived from the primary font.

/// Font-wide measurements at the rasterization size, in physical pixels.
/// Offsets are relative to the baseline, positive upward (OpenType convention).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontMetrics {
    pub ascent: f32,
    /// Positive distance below the baseline.
    pub descent: f32,
    pub leading: f32,
    /// Advance width of a single-width glyph.
    pub advance: f32,
    /// Top of the underline stroke.
    pub underline_offset: f32,
    /// Top of the strikeout stroke.
    pub strikeout_offset: f32,
    pub stroke_size: f32,
}

/// Integer cell geometry in physical pixels; every y position is measured
/// from the top of the cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellMetrics {
    pub width: u32,
    pub height: u32,
    pub baseline: u32,
    pub underline_y: u32,
    pub strikeout_y: u32,
    pub stroke: u32,
    /// Added to every glyph's x so glyphs stay centered in a cell made
    /// wider or narrower than the font's advance.
    pub glyph_x: i32,
}

impl CellMetrics {
    /// Rounds up so glyphs never overlap their neighbours, and centers the
    /// font's leading so the text sits in the middle of the cell.
    pub fn from_font(m: &FontMetrics) -> Self {
        let width = m.advance.ceil().max(1.0) as u32;
        let height = (m.ascent + m.descent + m.leading.max(0.0)).ceil().max(1.0) as u32;
        let baseline = ((m.leading.max(0.0) / 2.0 + m.ascent).round() as u32).min(height);
        let stroke = m.stroke_size.round().max(1.0) as u32;
        let below = |offset: f32| {
            let y = (baseline as f32 - offset).round().max(0.0) as u32;
            y.min(height - stroke.min(height))
        };
        Self {
            width,
            height,
            baseline,
            underline_y: below(m.underline_offset),
            strikeout_y: below(m.strikeout_offset),
            stroke,
            glyph_x: 0,
        }
    }

    /// Scales the cell by `width` and `height` (1.0 keeps it), keeping the
    /// text centered. Glyphs may overflow a cell made smaller than the
    /// font; they are not clipped.
    pub fn scaled(self, width: f32, height: f32) -> Self {
        let scale = |v: u32, by: f32| (v as f32 * by).round().max(1.0) as u32;
        let (new_width, new_height) = (scale(self.width, width), scale(self.height, height));
        let half = |new: u32, old: u32| (new as i32 - old as i32) / 2;
        let dy = half(new_height, self.height);
        let stroke = self.stroke.min(new_height);
        let shift = |y: u32, max: u32| (y as i32 + dy).clamp(0, max as i32) as u32;
        Self {
            width: new_width,
            height: new_height,
            baseline: shift(self.baseline, new_height),
            underline_y: shift(self.underline_y, new_height - stroke),
            strikeout_y: shift(self.strikeout_y, new_height - stroke),
            stroke,
            glyph_x: self.glyph_x + half(new_width, self.width),
        }
    }

    /// How many whole cells fit in a `width` x `height` surface after
    /// removing `padding` on every side. Never returns zero, because a
    /// terminal with no cells cannot be resized back meaningfully.
    pub fn grid_size(&self, width: u32, height: u32, padding: u32) -> (u16, u16) {
        let fit = |avail: u32, cell: u32| {
            let n = avail.saturating_sub(padding * 2) / cell;
            n.clamp(1, u32::from(u16::MAX)) as u16
        };
        (fit(width, self.width), fit(height, self.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dejavu_like_14px() -> FontMetrics {
        FontMetrics {
            ascent: 13.36,
            descent: 3.4,
            leading: 0.0,
            advance: 8.43,
            underline_offset: -1.23,
            strikeout_offset: 4.4,
            stroke_size: 0.92,
        }
    }

    #[test]
    fn cell_size_rounds_up_to_whole_pixels() {
        let m = CellMetrics::from_font(&dejavu_like_14px());
        assert_eq!((m.width, m.height), (9, 17));
        assert_eq!(m.baseline, 13);
    }

    #[test]
    fn decorations_are_placed_relative_to_baseline() {
        let m = CellMetrics::from_font(&dejavu_like_14px());
        assert_eq!(m.stroke, 1);
        assert_eq!(m.underline_y, 14);
        assert_eq!(m.strikeout_y, 9);
    }

    #[test]
    fn leading_is_split_above_and_below() {
        let m = CellMetrics::from_font(&FontMetrics {
            leading: 4.0,
            ..dejavu_like_14px()
        });
        assert_eq!(m.height, 21);
        assert_eq!(m.baseline, 15);
    }

    #[test]
    fn underline_stays_inside_the_cell() {
        let m = CellMetrics::from_font(&FontMetrics {
            underline_offset: -20.0,
            stroke_size: 2.0,
            ..dejavu_like_14px()
        });
        assert_eq!(m.underline_y + m.stroke, m.height);
    }

    #[test]
    fn scaling_by_one_keeps_the_cell() {
        let m = CellMetrics::from_font(&dejavu_like_14px());
        assert_eq!(m.scaled(1.0, 1.0), m);
    }

    #[test]
    fn a_taller_cell_keeps_the_text_in_the_middle() {
        let m = CellMetrics::from_font(&dejavu_like_14px()).scaled(1.0, 1.5);
        // 17 * 1.5 = 25.5 -> 26: 9 more pixels, 4 of them above.
        assert_eq!((m.width, m.height), (9, 26));
        assert_eq!((m.baseline, m.underline_y, m.strikeout_y), (17, 18, 13));
        assert_eq!(m.glyph_x, 0);
    }

    #[test]
    fn a_wider_cell_moves_glyphs_to_the_middle() {
        let m = CellMetrics::from_font(&dejavu_like_14px()).scaled(1.5, 1.0);
        assert_eq!((m.width, m.glyph_x), (14, 2));
        let narrow = CellMetrics::from_font(&dejavu_like_14px()).scaled(0.5, 1.0);
        assert_eq!((narrow.width, narrow.glyph_x), (5, -2));
    }

    #[test]
    fn decorations_stay_inside_a_shorter_cell() {
        let m = CellMetrics::from_font(&dejavu_like_14px()).scaled(1.0, 0.5);
        assert_eq!(m.height, 9);
        assert!(m.baseline <= m.height);
        assert!(m.underline_y + m.stroke <= m.height);
    }

    #[test]
    fn grid_size_excludes_padding_and_partial_cells() {
        let m = CellMetrics::from_font(&dejavu_like_14px());
        assert_eq!(m.grid_size(800, 600, 4), (88, 34));
        assert_eq!(m.grid_size(9 * 80 + 8, 17 * 24 + 8, 4), (80, 24));
    }

    #[test]
    fn grid_size_is_at_least_one_cell() {
        let m = CellMetrics::from_font(&dejavu_like_14px());
        assert_eq!(m.grid_size(0, 0, 4), (1, 1));
    }

    #[test]
    fn hidpi_doubles_the_cell() {
        let base = dejavu_like_14px();
        let scaled = FontMetrics {
            ascent: base.ascent * 2.0,
            descent: base.descent * 2.0,
            advance: base.advance * 2.0,
            ..base
        };
        let m = CellMetrics::from_font(&scaled);
        assert_eq!((m.width, m.height), (17, 34));
    }
}
