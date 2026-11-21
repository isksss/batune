//! Converts the terminal state into the renderer's input.

use crate::render::{CellFlags, CellView, Cursor, CursorShape, Frame, ImageData, ImageQuad};
use crate::vt::{
    Cell, Color as VtColor, CursorShape as VtCursorShape, CursorStyle, Flags, ImageCell, Term,
    Underline,
};

use crate::palette::Palette;
use crate::search::Search;
use crate::selection::{self, Point, Range};

/// Search highlights take the theme's yellow and red rather than colours
/// of their own, so they suit every theme without new settings.
const SEARCH_MATCH: usize = 3;
const SEARCH_CURRENT: usize = 1;

/// Window state that affects how the grid is drawn.
#[derive(Clone, Copy, Debug)]
pub struct View<'a> {
    pub focused: bool,
    /// Shape used until an application picks one with DECSCUSR.
    pub cursor_shape: VtCursorShape,
    /// Cells to highlight, in absolute lines (see [`selection`]).
    pub selection: Option<Range>,
    /// Link under the pointer while Ctrl is held, drawn underlined.
    pub link: Option<Range>,
    /// Open search, whose matches are highlighted.
    pub search: Option<&'a Search>,
    /// Visual bell: the screen is drawn inverted, as with DECSCNM.
    pub flash: bool,
    /// The cursor is in the hidden phase of a blink.
    pub cursor_blinked_off: bool,
}

/// Builds the frame for the lines currently in view (scrollback included
/// while scrolled back).
pub fn build(term: &Term, palette: &Palette, view: View<'_>) -> Frame {
    let cols = clamp_u16(term.cols());
    let rows = clamp_u16(term.rows());
    let reverse = term.modes().reverse_video != view.flash;
    let mut frame = Frame::new(cols, rows, palette.foreground, palette.background);
    if reverse {
        frame.background = palette.foreground;
    }
    for (line, out) in frame.cells.chunks_mut(usize::from(cols)).enumerate() {
        let row = term.display_row(line).cells();
        push_images(&mut frame.images, line, row);
        let abs = selection::absolute_line(term, line);
        for (col, (dst, src)) in out.iter_mut().zip(row).enumerate() {
            *dst = cell_view(src, palette);
            let point = Point::new(abs, col);
            if view.link.is_some_and(|r| r.contains(point)) {
                dst.flags.insert(CellFlags::UNDERLINE);
            }
            if let Some(current) = view.search.and_then(|s| s.hit(point)) {
                dst.fg = palette.background;
                dst.bg = palette.colors[if current {
                    SEARCH_CURRENT
                } else {
                    SEARCH_MATCH
                }];
                dst.flags.remove(CellFlags::INVERSE);
            }
            let selected = view.selection.is_some_and(|r| r.contains(point));
            // DECSCNM inverts every cell, so a cell that is already INVERSE
            // goes back to normal, as in xterm. The selection is shown the
            // same way rather than in a palette colour, so it stays readable
            // whatever colours the application chose.
            if reverse != selected {
                dst.flags.toggle(CellFlags::INVERSE);
            }
            // A wide character cut off at the right edge has no room for its
            // right half; drawing it would spill into the padding.
            if col + 1 == usize::from(cols) {
                dst.flags.remove(CellFlags::WIDE);
            }
        }
    }
    let cursor_line = term.cursor().row + term.display_offset();
    if term.modes().show_cursor && !view.cursor_blinked_off && cursor_line < term.rows() {
        // vt has no notion of a configured default, so its own default style
        // stands for "not chosen by the application". DECSCUSR 1 (blinking
        // block) is indistinguishable from that and also gets the configured
        // shape.
        let shape = if term.cursor_style() == CursorStyle::default() {
            view.cursor_shape
        } else {
            term.cursor_style().shape
        };
        frame.cursor = Some(Cursor {
            col: clamp_u16(term.cursor().col),
            row: clamp_u16(cursor_line),
            shape: match shape {
                _ if !view.focused => CursorShape::HollowBlock,
                VtCursorShape::Block => CursorShape::Block,
                VtCursorShape::Underline => CursorShape::Underline,
                VtCursorShape::Bar => CursorShape::Bar,
            },
            color: palette.cursor,
        });
    }
    frame
}

/// Adds one quad per horizontal run of cells showing the same strip of one
/// placement, instead of one per cell, so a picture is a handful of quads
/// and the texture is sampled across cell borders without seams.
fn push_images(out: &mut Vec<ImageQuad>, line: usize, row: &[Cell]) {
    let mut run: Option<(usize, &ImageCell, &ImageCell)> = None;
    let flush =
        |out: &mut Vec<ImageQuad>, run: Option<(usize, &ImageCell, &ImageCell)>, end: usize| {
            let Some((start, first, last)) = run else {
                return;
            };
            let (first_uv, last_uv) = (first.uv(), last.uv());
            let image = first.image();
            out.push(ImageQuad {
                image: ImageData {
                    id: image.id(),
                    width: image.width(),
                    height: image.height(),
                    rgba: image.rgba().clone(),
                },
                col: clamp_u16(start),
                row: clamp_u16(line),
                // Every cell but the last of a run is fully covered.
                width: (end - start - 1) as f32 + last.extent().0,
                height: first.extent().1,
                uv: [first_uv[0], first_uv[1], last_uv[2], last_uv[3]],
                z: first.z(),
            });
        };
    for (col, cell) in row.iter().enumerate() {
        let image = cell.image();
        let continues = match (run, image) {
            (Some((_, _, last)), Some(img)) => {
                img.same_placement(last)
                    && img.offset() == (last.offset().0 + 1, last.offset().1)
                    && last.extent().0 >= 1.0
            }
            _ => false,
        };
        if continues {
            if let (Some(r), Some(img)) = (run.as_mut(), image) {
                r.2 = img;
            }
            continue;
        }
        flush(out, run, col);
        run = image.map(|img| (col, img, img));
    }
    flush(out, run, row.len());
}

fn cell_view(cell: &Cell, palette: &Palette) -> CellView {
    let bold = cell.flags.contains(Flags::BOLD);
    let mut flags = CellFlags::empty();
    for (from, to) in [
        (Flags::BOLD, CellFlags::BOLD),
        (Flags::DIM, CellFlags::DIM),
        (Flags::ITALIC, CellFlags::ITALIC),
        (Flags::INVERSE, CellFlags::INVERSE),
        (Flags::HIDDEN, CellFlags::HIDDEN),
        (Flags::STRIKEOUT, CellFlags::STRIKETHROUGH),
        (Flags::WIDE, CellFlags::WIDE),
        (Flags::WIDE_SPACER, CellFlags::WIDE_SPACER),
    ] {
        flags.set(to, cell.flags.contains(from));
    }
    flags |= match cell.underline {
        Underline::None => CellFlags::empty(),
        Underline::Single => CellFlags::UNDERLINE,
        Underline::Double => CellFlags::DOUBLE_UNDERLINE,
        Underline::Curly => CellFlags::CURLY_UNDERLINE,
        Underline::Dotted => CellFlags::DOTTED_UNDERLINE,
        Underline::Dashed => CellFlags::DASHED_UNDERLINE,
    };
    CellView {
        ch: if cell.flags.contains(Flags::WIDE_SPACER) {
            ' '
        } else {
            cell.c
        },
        // vt keeps at most MAX_ZEROWIDTH marks per cell, so this copy is
        // bounded; most cells have none and skip the allocation.
        zerowidth: (!cell.zerowidth().is_empty() && !cell.flags.contains(Flags::WIDE_SPACER))
            .then(|| cell.zerowidth().into()),
        fg: palette.foreground(cell.fg, bold),
        bg: palette.background(cell.bg),
        flags,
        // Not bright for bold: SGR 58 colors only the line, as in kitty.
        underline_color: (cell.underline_color != VtColor::Default)
            .then(|| palette.foreground(cell.underline_color, false)),
    }
}

fn clamp_u16(v: usize) -> u16 {
    u16::try_from(v).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use crate::render::Color;

    use super::*;

    const FOCUSED: View<'static> = View {
        focused: true,
        cursor_shape: VtCursorShape::Block,
        selection: None,
        link: None,
        search: None,
        flash: false,
        cursor_blinked_off: false,
    };

    #[test]
    fn combining_marks_travel_with_their_base_cell() {
        let (frame, _) = frame_of(3, 1, "e\u{301}x");
        let e = frame.cell(0, 0).unwrap();
        assert_eq!(
            (e.ch, e.zerowidth.as_deref()),
            ('e', Some(&['\u{301}'][..]))
        );
        assert_eq!(frame.cell(1, 0).unwrap().zerowidth, None);
    }

    #[test]
    fn plain_cells_carry_no_marks() {
        let (frame, _) = frame_of(2, 1, "ab");
        assert!(frame.cells.iter().all(|c| c.zerowidth.is_none()));
    }

    fn frame_of(cols: usize, rows: usize, input: &str) -> (Frame, Palette) {
        let mut term = Term::new(cols, rows);
        term.advance(input.as_bytes());
        let palette = Palette::default();
        (build(&term, &palette, FOCUSED), palette)
    }

    #[test]
    fn frame_has_the_terminal_size_and_text() {
        let (frame, _) = frame_of(10, 3, "ab\r\ncd");
        assert_eq!((frame.cols, frame.rows), (10, 3));
        assert_eq!(frame.cells.len(), 30);
        assert_eq!(frame.cell(0, 0).unwrap().ch, 'a');
        assert_eq!(frame.cell(1, 1).unwrap().ch, 'd');
        assert_eq!(frame.cell(2, 1).unwrap().ch, ' ');
    }

    #[test]
    fn sgr_colors_are_resolved_through_the_palette() {
        let (frame, p) = frame_of(
            10,
            1,
            "\x1b[31;42mA\x1b[38;5;196mB\x1b[38;2;1;2;3mC\x1b[0mD",
        );
        let a = frame.cell(0, 0).unwrap();
        assert_eq!((a.fg, a.bg), (p.colors[1], p.colors[2]));
        assert_eq!(frame.cell(1, 0).unwrap().fg, p.colors[196]);
        assert_eq!(frame.cell(2, 0).unwrap().fg, Color::rgb(1, 2, 3));
        let d = frame.cell(3, 0).unwrap();
        assert_eq!((d.fg, d.bg), (p.foreground, p.background));
    }

    #[test]
    fn bold_ansi_color_is_drawn_bright() {
        let (frame, p) = frame_of(10, 1, "\x1b[1;34mX");
        let x = frame.cell(0, 0).unwrap();
        assert_eq!(x.fg, p.colors[12]);
        assert!(x.flags.contains(CellFlags::BOLD));
    }

    #[test]
    fn attributes_are_carried_over() {
        let (frame, _) = frame_of(10, 1, "\x1b[2;3;7;8;9;4mX\x1b[0m\x1b[4:3mY");
        let x = frame.cell(0, 0).unwrap().flags;
        assert_eq!(
            x,
            CellFlags::DIM
                | CellFlags::ITALIC
                | CellFlags::INVERSE
                | CellFlags::HIDDEN
                | CellFlags::STRIKETHROUGH
                | CellFlags::UNDERLINE
        );
        assert_eq!(frame.cell(1, 0).unwrap().flags, CellFlags::CURLY_UNDERLINE);
    }

    #[test]
    fn each_underline_style_maps_to_its_own_flag() {
        let (frame, _) = frame_of(10, 1, "\x1b[4:1ma\x1b[4:2mb\x1b[4:3mc\x1b[4:4md\x1b[4:5me");
        let flags: Vec<CellFlags> = (0..5).map(|c| frame.cell(c, 0).unwrap().flags).collect();
        assert_eq!(
            flags,
            [
                CellFlags::UNDERLINE,
                CellFlags::DOUBLE_UNDERLINE,
                CellFlags::CURLY_UNDERLINE,
                CellFlags::DOTTED_UNDERLINE,
                CellFlags::DASHED_UNDERLINE,
            ]
        );
    }

    #[test]
    fn underline_color_is_resolved_only_when_set() {
        let (frame, p) = frame_of(10, 1, "\x1b[4;58;5;1ma\x1b[59mb");
        assert_eq!(frame.cell(0, 0).unwrap().underline_color, Some(p.colors[1]));
        assert_eq!(frame.cell(1, 0).unwrap().underline_color, None);
    }

    #[test]
    fn wide_characters_take_two_cells() {
        let (frame, _) = frame_of(10, 1, "日本");
        let first = frame.cell(0, 0).unwrap();
        assert_eq!(first.ch, '日');
        assert!(first.flags.contains(CellFlags::WIDE));
        let spacer = frame.cell(1, 0).unwrap();
        assert_eq!(spacer.ch, ' ');
        assert!(spacer.flags.contains(CellFlags::WIDE_SPACER));
        assert_eq!(frame.cell(2, 0).unwrap().ch, '本');
    }

    #[test]
    fn cursor_follows_the_terminal_and_focus() {
        let mut term = Term::new(10, 3);
        term.advance(b"\x1b[2;4H");
        let palette = Palette::default();
        let cursor = build(&term, &palette, FOCUSED).cursor.unwrap();
        assert_eq!((cursor.col, cursor.row), (3, 1));
        assert_eq!(cursor.shape, CursorShape::Block);
        assert_eq!(cursor.color, palette.cursor);
        let unfocused = View {
            focused: false,
            ..FOCUSED
        };
        let unfocused = build(&term, &palette, unfocused);
        assert_eq!(unfocused.cursor.unwrap().shape, CursorShape::HollowBlock);
    }

    #[test]
    fn cursor_is_not_drawn_in_the_off_phase_of_a_blink() {
        let (frame, _) = frame_of(10, 3, "a");
        assert!(frame.cursor.is_some());
        let term = Term::new(10, 3);
        let off = View {
            cursor_blinked_off: true,
            ..FOCUSED
        };
        assert!(build(&term, &Palette::default(), off).cursor.is_none());
    }

    #[test]
    fn configured_cursor_shape_applies_until_the_application_picks_one() {
        let mut term = Term::new(10, 3);
        let palette = Palette::default();
        let bar = View {
            cursor_shape: VtCursorShape::Bar,
            ..FOCUSED
        };
        let shape = |term: &Term| build(term, &palette, bar).cursor.unwrap().shape;
        assert_eq!(shape(&term), CursorShape::Bar);
        term.advance(b"\x1b[4 q");
        assert_eq!(shape(&term), CursorShape::Underline);
        term.advance(b"\x1b[0 q");
        assert_eq!(shape(&term), CursorShape::Bar);
    }

    #[test]
    fn cursor_shape_and_visibility_follow_the_application() {
        let palette = Palette::default();
        let mut term = Term::new(10, 3);
        term.advance(b"\x1b[6 q");
        let cursor = build(&term, &palette, FOCUSED).cursor.unwrap();
        assert_eq!(cursor.shape, CursorShape::Bar);
        term.advance(b"\x1b[4 q");
        let cursor = build(&term, &palette, FOCUSED).cursor.unwrap();
        assert_eq!(cursor.shape, CursorShape::Underline);
        term.advance(b"\x1b[?25l");
        assert!(build(&term, &palette, FOCUSED).cursor.is_none());
    }

    #[test]
    fn reverse_video_inverts_every_cell_and_the_padding() {
        let (frame, p) = frame_of(10, 1, "\x1b[?5ha\x1b[7mb");
        assert!(frame.cell(0, 0).unwrap().flags.contains(CellFlags::INVERSE));
        assert!(!frame.cell(1, 0).unwrap().flags.contains(CellFlags::INVERSE));
        assert_eq!(frame.background, p.foreground);
    }

    #[test]
    fn bell_flash_inverts_the_screen_and_undoes_reverse_video() {
        let palette = Palette::default();
        let flash = View {
            flash: true,
            ..FOCUSED
        };
        let mut term = Term::new(10, 1);
        term.advance(b"a");
        let frame = build(&term, &palette, flash);
        assert!(frame.cell(0, 0).unwrap().flags.contains(CellFlags::INVERSE));
        assert_eq!(frame.background, palette.foreground);
        term.advance(b"\x1b[?5h");
        let frame = build(&term, &palette, flash);
        assert!(!frame.cell(0, 0).unwrap().flags.contains(CellFlags::INVERSE));
        assert_eq!(frame.background, palette.background);
    }

    #[test]
    fn search_matches_are_highlighted_and_the_current_one_stands_out() {
        let mut term = Term::new(10, 1);
        term.advance(b"ab ab x");
        let mut search = Search::default();
        search.push("ab", &term);
        let palette = Palette::default();
        let view = View {
            search: Some(&search),
            ..FOCUSED
        };
        let frame = build(&term, &palette, view);
        let bg = |c| frame.cell(c, 0).unwrap().bg;
        assert_eq!(bg(0), palette.colors[SEARCH_MATCH]);
        assert_eq!(bg(3), palette.colors[SEARCH_CURRENT]);
        assert_ne!(palette.colors[SEARCH_MATCH], palette.colors[SEARCH_CURRENT]);
        assert_eq!(bg(2), palette.background);
        assert_eq!(frame.cell(0, 0).unwrap().fg, palette.background);
    }

    #[test]
    fn the_hovered_link_is_underlined() {
        let mut term = Term::new(10, 1);
        term.advance(b"abcdef");
        let view = View {
            link: Some(Range {
                start: Point::new(0, 1),
                end: Point::new(0, 3),
            }),
            ..FOCUSED
        };
        let frame = build(&term, &Palette::default(), view);
        let underlined: Vec<bool> = (0..5)
            .map(|c| {
                frame
                    .cell(c, 0)
                    .unwrap()
                    .flags
                    .contains(CellFlags::UNDERLINE)
            })
            .collect();
        assert_eq!(underlined, [false, true, true, true, false]);
    }

    /// 16x32 solid red PNG.
    const RED_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAABAAAAAgCAIAAACU62+bAAAAHUlEQVR4nGP4z8BAEiJN9aiGUQ2jGkY1jGoYohoAP8T+EE3PqSIAAAAASUVORK5CYII=";

    #[test]
    fn an_image_becomes_one_quad_per_row() {
        let mut term = Term::new(10, 5);
        term.set_cell_pixel_size(8, 16);
        term.advance(format!("\x1b[2Cab\x1b]1337;File=inline=1;width=2:{RED_PNG}\x07").as_bytes());
        let frame = build(&term, &Palette::default(), FOCUSED);
        assert_eq!(frame.images.len(), 2, "{:?}", frame.images);
        let (top, bottom) = (&frame.images[0], &frame.images[1]);
        assert_eq!((top.col, top.row, bottom.col, bottom.row), (4, 0, 4, 1));
        assert_eq!((top.width, top.height), (2.0, 1.0));
        assert_eq!((top.image.width, top.image.height), (16, 32));
        assert_eq!(top.uv, [0.0, 0.0, 1.0, 0.5]);
        assert_eq!(bottom.uv, [0.0, 0.5, 1.0, 1.0]);
        assert_eq!(top.image, bottom.image);
    }

    #[test]
    fn overwritten_image_cells_split_the_quad() {
        let mut term = Term::new(10, 5);
        term.set_cell_pixel_size(8, 16);
        term.advance(
            format!("\x1b]1337;File=inline=1;width=3;height=1;preserveAspectRatio=0:{RED_PNG}\x07")
                .as_bytes(),
        );
        term.advance(b"\x1b[1;2Hx");
        let frame = build(&term, &Palette::default(), FOCUSED);
        assert_eq!(frame.images.len(), 2, "{:?}", frame.images);
        assert_eq!(frame.images[0].col, 0);
        assert_eq!(frame.images[1].col, 2);
    }

    #[test]
    fn scrolled_back_view_shows_history_and_moves_the_cursor_down() {
        let mut term = Term::new(5, 2);
        term.advance(b"1\r\n2\r\n3\r\n4");
        term.scroll_display(1);
        let palette = Palette::default();
        let frame = build(&term, &palette, FOCUSED);
        assert_eq!(frame.cell(0, 0).unwrap().ch, '2');
        assert_eq!(frame.cell(0, 1).unwrap().ch, '3');
        // The cursor sits on the live screen's last row, now below the view.
        assert!(frame.cursor.is_none());
    }

    #[test]
    fn selected_cells_are_inverted_and_follow_the_scroll_position() {
        let mut term = Term::new(5, 2);
        term.advance(b"ab\r\ncd\r\nef");
        let palette = Palette::default();
        // Absolute line 1 is "cd": the top row of the live screen.
        let view = View {
            selection: Some(Range {
                start: Point::new(1, 0),
                end: Point::new(1, 0),
            }),
            ..FOCUSED
        };
        let inverted = |frame: &Frame, col, row| {
            frame
                .cell(col, row)
                .unwrap()
                .flags
                .contains(CellFlags::INVERSE)
        };
        let frame = build(&term, &palette, view);
        assert!(inverted(&frame, 0, 0));
        assert!(!inverted(&frame, 1, 0));
        term.scroll_display(1);
        let frame = build(&term, &palette, view);
        assert!(!inverted(&frame, 0, 0));
        assert!(inverted(&frame, 0, 1));
    }
}
