// Shared by the `demo` example and the offscreen test so the screenshot and
// the assertions look at the same frame.

use batune::render::{CellFlags, Color, Cursor, CursorShape, Frame};
use unicode_width::UnicodeWidthChar;

pub const FG: Color = Color::rgb(0xd8, 0xd8, 0xd8);
pub const BG: Color = Color::rgb(0x1c, 0x1c, 0x24);

pub const PALETTE: [Color; 8] = [
    Color::rgb(0x45, 0x47, 0x5a),
    Color::rgb(0xf3, 0x8b, 0xa8),
    Color::rgb(0xa6, 0xe3, 0xa1),
    Color::rgb(0xf9, 0xe2, 0xaf),
    Color::rgb(0x89, 0xb4, 0xfa),
    Color::rgb(0xf5, 0xc2, 0xe7),
    Color::rgb(0x94, 0xe2, 0xd5),
    Color::rgb(0xba, 0xc2, 0xde),
];

/// Writes `text` starting at (`col`, `row`), marking wide characters, and
/// returns the column after the last one.
pub fn put(
    frame: &mut Frame,
    mut col: u16,
    row: u16,
    text: &str,
    fg: Color,
    bg: Color,
    flags: CellFlags,
) -> u16 {
    for ch in text.chars() {
        let wide = ch.width() == Some(2);
        if let Some(cell) = frame.cell_mut(col, row) {
            cell.ch = ch;
            cell.fg = fg;
            cell.bg = bg;
            cell.flags = flags
                | if wide {
                    CellFlags::WIDE
                } else {
                    CellFlags::empty()
                };
        }
        col += 1;
        if wide {
            if let Some(cell) = frame.cell_mut(col, row) {
                cell.ch = ' ';
                cell.fg = fg;
                cell.bg = bg;
                cell.flags = flags | CellFlags::WIDE_SPACER;
            }
            col += 1;
        }
    }
    col
}

pub fn demo_frame(cols: u16, rows: u16, cursor: CursorShape) -> Frame {
    let mut f = Frame::new(cols, rows, FG, BG);
    let none = CellFlags::empty();
    put(
        &mut f,
        0,
        0,
        "$ batune-render demo",
        FG,
        BG,
        CellFlags::BOLD,
    );
    put(
        &mut f,
        0,
        1,
        "ASCII: The quick brown fox jumps over the lazy dog 0123456789",
        FG,
        BG,
        none,
    );
    put(
        &mut f,
        0,
        2,
        "日本語: こんにちは、世界。漢字とカナの表示",
        FG,
        BG,
        none,
    );
    put(&mut f, 0, 3, "Emoji: 🍣 🚀 ✨ 👍 🐧", FG, BG, none);

    let mut col = put(&mut f, 0, 4, "Colors:", FG, BG, none);
    for (i, &c) in PALETTE.iter().enumerate() {
        col = put(&mut f, col + 1, 4, &format!("fg{i}"), c, BG, none);
    }
    let mut col = put(&mut f, 0, 5, "Backgrounds:", FG, BG, none);
    for (i, &c) in PALETTE.iter().enumerate() {
        col = put(&mut f, col + 1, 5, &format!(" bg{i} "), BG, c, none);
    }

    let mut col = put(&mut f, 0, 6, "Attrs:", FG, BG, none);
    for (label, flags) in [
        ("bold", CellFlags::BOLD),
        ("italic", CellFlags::ITALIC),
        ("bold-italic", CellFlags::BOLD | CellFlags::ITALIC),
        ("dim", CellFlags::DIM),
        ("inverse", CellFlags::INVERSE),
        ("underline", CellFlags::UNDERLINE),
        ("strike", CellFlags::STRIKETHROUGH),
    ] {
        col = put(&mut f, col + 1, 6, label, FG, BG, flags);
    }
    put(
        &mut f,
        0,
        7,
        "太字の日本語",
        PALETTE[3],
        BG,
        CellFlags::BOLD | CellFlags::UNDERLINE,
    );

    let mut col = put(&mut f, 0, 8, "Underlines:", FG, BG, none);
    for (label, flags) in [
        ("double", CellFlags::DOUBLE_UNDERLINE),
        ("curly", CellFlags::CURLY_UNDERLINE),
        ("dotted", CellFlags::DOTTED_UNDERLINE),
        ("dashed", CellFlags::DASHED_UNDERLINE),
    ] {
        col = put(&mut f, col + 1, 8, label, FG, BG, flags);
    }

    let prompt_end = put(&mut f, 0, 9, "$ echo hello", FG, BG, none);
    f.cursor = Some(Cursor {
        col: prompt_end,
        row: 9,
        shape: cursor,
        color: Color::rgb(0xf5, 0xe0, 0xdc),
    });
    f
}
