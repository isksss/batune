//! Text being composed by an input method (preedit), and how it is drawn
//! over the grid.

use unicode_width::UnicodeWidthChar;

use crate::palette::Palette;
use crate::render::{CellFlags, CellView, Cursor, CursorShape, Frame};

/// The composition string from `Ime::Preedit`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preedit {
    pub text: String,
    /// Byte offset of the caret in `text`; `None` hides it.
    pub caret: Option<usize>,
}

impl Preedit {
    pub fn new(text: String, cursor: Option<(usize, usize)>) -> Self {
        Self {
            caret: cursor.map(|(start, _)| start.min(text.len())),
            text,
        }
    }
}

/// Draws `preedit` underlined over `frame`, starting at `anchor`
/// (column, row) — the terminal cursor — and moves the frame's cursor to
/// the caret. Returns the column where the text starts, which the caller
/// uses to place the candidate window.
///
/// The cells under the text are replaced rather than blended so the text
/// stays readable over anything. Text that would run past the right edge is
/// shifted left to keep its end, where the caret usually is, in view.
pub fn overlay(frame: &mut Frame, anchor: (u16, u16), preedit: &Preedit, palette: &Palette) -> u16 {
    let (col, row) = anchor;
    if row >= frame.rows || frame.cols == 0 {
        return col;
    }
    // Zero-width characters cannot have a cell of their own (the renderer
    // draws one char per cell), so they are left out.
    let glyphs: Vec<(usize, char, u16)> = preedit
        .text
        .char_indices()
        .filter_map(|(i, c)| {
            let w = c.width().unwrap_or(0);
            (w > 0).then(|| (i, c, w.min(2) as u16))
        })
        .collect();
    let width: u16 = glyphs.iter().map(|g| g.2).sum();
    let start = col.min(frame.cols.saturating_sub(width));

    let mut x = start;
    let mut caret_col = None;
    for &(offset, c, w) in &glyphs {
        if preedit.caret == Some(offset) {
            caret_col = Some(x);
        }
        if x + w > frame.cols {
            break;
        }
        let mut flags = CellFlags::UNDERLINE;
        if w == 2 {
            flags |= CellFlags::WIDE;
        }
        if let Some(cell) = frame.cell_mut(x, row) {
            *cell = CellView {
                ch: c,
                flags,
                ..CellView::blank(palette.foreground, palette.background)
            };
        }
        if w == 2
            && let Some(cell) = frame.cell_mut(x + 1, row)
        {
            *cell = CellView {
                flags: CellFlags::UNDERLINE | CellFlags::WIDE_SPACER,
                ..CellView::blank(palette.foreground, palette.background)
            };
        }
        x += w;
    }
    if preedit.caret.is_some() && caret_col.is_none() {
        caret_col = Some(x);
    }
    frame.cursor = caret_col.filter(|&c| c < frame.cols).map(|c| Cursor {
        col: c,
        row,
        shape: CursorShape::Bar,
        color: palette.cursor,
    });
    start
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank(cols: u16, rows: u16, p: &Palette) -> Frame {
        Frame::new(cols, rows, p.foreground, p.background)
    }

    fn text_of(frame: &Frame, row: u16) -> String {
        (0..frame.cols)
            .map(|c| frame.cell(c, row).unwrap())
            .filter(|c| !c.flags.contains(CellFlags::WIDE_SPACER))
            .map(|c| c.ch)
            .collect()
    }

    #[test]
    fn preedit_is_drawn_underlined_from_the_cursor() {
        let p = Palette::default();
        let mut frame = blank(10, 2, &p);
        let preedit = Preedit::new("かな".into(), Some((6, 6)));
        let start = overlay(&mut frame, (2, 1), &preedit, &p);
        assert_eq!(start, 2);
        assert_eq!(text_of(&frame, 1), "  かな    ");
        let first = frame.cell(2, 1).unwrap();
        assert_eq!(first.flags, CellFlags::UNDERLINE | CellFlags::WIDE);
        assert_eq!(
            frame.cell(3, 1).unwrap().flags,
            CellFlags::UNDERLINE | CellFlags::WIDE_SPACER
        );
        assert!(
            !frame
                .cell(6, 1)
                .unwrap()
                .flags
                .contains(CellFlags::UNDERLINE)
        );
        assert_eq!(text_of(&frame, 0), " ".repeat(10));
    }

    #[test]
    fn preedit_hides_the_cells_beneath_it() {
        let p = Palette::default();
        let mut frame = blank(5, 1, &p);
        frame.cell_mut(0, 0).unwrap().ch = 'x';
        frame.cell_mut(0, 0).unwrap().flags = CellFlags::INVERSE | CellFlags::BOLD;
        overlay(&mut frame, (0, 0), &Preedit::new("a".into(), None), &p);
        let cell = frame.cell(0, 0).unwrap();
        assert_eq!((cell.ch, cell.flags), ('a', CellFlags::UNDERLINE));
    }

    #[test]
    fn caret_becomes_a_bar_at_its_byte_offset() {
        let p = Palette::default();
        let mut frame = blank(10, 1, &p);
        // Caret after "あ" (3 bytes), i.e. before "い".
        overlay(
            &mut frame,
            (1, 0),
            &Preedit::new("あい".into(), Some((3, 3))),
            &p,
        );
        let cursor = frame.cursor.unwrap();
        assert_eq!(
            (cursor.col, cursor.row, cursor.shape),
            (3, 0, CursorShape::Bar)
        );

        let mut frame = blank(10, 1, &p);
        overlay(
            &mut frame,
            (1, 0),
            &Preedit::new("あい".into(), Some((6, 6))),
            &p,
        );
        assert_eq!(frame.cursor.unwrap().col, 5);
    }

    #[test]
    fn no_caret_hides_the_cursor() {
        let p = Palette::default();
        let mut frame = blank(10, 1, &p);
        frame.cursor = Some(Cursor {
            col: 0,
            row: 0,
            shape: CursorShape::Block,
            color: p.cursor,
        });
        overlay(&mut frame, (0, 0), &Preedit::new("あ".into(), None), &p);
        assert!(frame.cursor.is_none());
    }

    #[test]
    fn text_near_the_right_edge_is_shifted_left_to_fit() {
        let p = Palette::default();
        let mut frame = blank(6, 1, &p);
        let start = overlay(
            &mut frame,
            (4, 0),
            &Preedit::new("日本".into(), Some((6, 6))),
            &p,
        );
        assert_eq!(start, 2);
        assert_eq!(text_of(&frame, 0), "  日本");
        // The caret at the very end has no cell left; it is not drawn
        // outside the grid.
        assert!(frame.cursor.is_none());
    }

    #[test]
    fn text_wider_than_the_grid_is_cut_at_the_right_edge() {
        let p = Palette::default();
        let mut frame = blank(3, 1, &p);
        let start = overlay(&mut frame, (1, 0), &Preedit::new("日本語".into(), None), &p);
        assert_eq!(start, 0);
        // "本" would straddle the edge, so it is dropped rather than halved.
        assert_eq!(text_of(&frame, 0), "日 ");
        assert!(
            !frame
                .cell(2, 0)
                .unwrap()
                .flags
                .contains(CellFlags::UNDERLINE)
        );
    }

    #[test]
    fn zero_width_characters_are_skipped() {
        let p = Palette::default();
        let mut frame = blank(5, 1, &p);
        overlay(
            &mut frame,
            (0, 0),
            &Preedit::new("e\u{301}x".into(), None),
            &p,
        );
        assert_eq!(text_of(&frame, 0), "ex   ");
    }

    #[test]
    fn anchor_outside_the_grid_draws_nothing() {
        let p = Palette::default();
        let mut frame = blank(5, 1, &p);
        let before = frame.clone();
        overlay(&mut frame, (0, 3), &Preedit::new("a".into(), None), &p);
        assert_eq!(frame, before);
    }

    #[test]
    fn caret_offset_past_the_end_is_clamped() {
        assert_eq!(Preedit::new("ab".into(), Some((9, 9))).caret, Some(2));
    }
}
