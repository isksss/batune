use std::sync::Arc;

use batune::vt::{Image, ImageCursor, Placement, Term};

fn image(width: u32, height: u32) -> Arc<Image> {
    let rgba = vec![0xff; (width * height * 4) as usize];
    Arc::new(Image::from_rgba(width, height, rgba).unwrap())
}

fn term(cols: usize, rows: usize, input: &str) -> Term {
    let mut term = Term::new(cols, rows);
    term.advance(input.as_bytes());
    term
}

/// `#` for cells showing an image, the text otherwise.
fn picture(term: &Term) -> Vec<String> {
    (0..term.rows())
        .map(|r| {
            let s: String = term
                .grid()
                .row(r)
                .cells()
                .iter()
                .map(|c| if c.image().is_some() { '#' } else { c.c })
                .collect();
            s.trim_end().to_owned()
        })
        .collect()
}

fn pos(term: &Term) -> (usize, usize) {
    (term.cursor().row, term.cursor().col)
}

#[test]
fn image_covers_cells_from_the_cursor_and_leaves_it_after_the_image() {
    let mut t = term(6, 4, "ab");
    t.place_image(image(4, 4), Placement::new(2.0, 2.0));
    assert_eq!(picture(&t), ["ab##", "  ##", "", ""]);
    assert_eq!(pos(&t), (1, 4));
}

#[test]
fn next_line_cursor_goes_below_the_left_corner() {
    let mut t = term(6, 4, "ab");
    let p = Placement {
        cursor: ImageCursor::NextLine,
        ..Placement::new(2.0, 2.0)
    };
    t.place_image(image(4, 4), p);
    assert_eq!(pos(&t), (2, 2));
}

#[test]
fn image_taller_than_the_space_below_scrolls_the_screen() {
    let mut t = term(4, 3, "a\r\nb\r\nc");
    t.place_image(image(4, 4), Placement::new(1.0, 2.0));
    assert_eq!(picture(&t), ["b", "c#", " #"]);
    assert_eq!(t.grid().scrollback_row(0).text(), "a");
}

#[test]
fn stay_cuts_the_image_at_the_bottom_instead_of_scrolling() {
    let mut t = term(4, 3, "\r\n\r\nc");
    let p = Placement {
        cursor: ImageCursor::Stay,
        ..Placement::new(1.0, 2.0)
    };
    t.place_image(image(4, 4), p);
    assert_eq!(picture(&t), ["", "", "c#"]);
    assert_eq!(pos(&t), (2, 1));
    assert_eq!(t.scrollback_len(), 0);
}

#[test]
fn image_is_cut_at_the_right_edge() {
    let mut t = term(4, 2, "abc");
    t.place_image(image(4, 4), Placement::new(3.0, 1.0));
    assert_eq!(picture(&t), ["abc#", ""]);
    t.advance(b"x");
    assert_eq!(picture(&t), ["abc#", "x"]);
}

#[test]
fn image_scrolls_with_its_cells_into_the_scrollback() {
    let mut t = term(4, 2, "");
    t.place_image(image(4, 4), Placement::new(1.0, 1.0));
    t.advance(b"\r\n\r\n");
    assert!(t.grid().scrollback_row(0)[0].image().is_some());
    assert!(t.cell(0, 0).image().is_none());
}

#[test]
fn printing_or_erasing_removes_the_piece_of_the_image() {
    let mut t = term(4, 2, "");
    t.place_image(image(4, 4), Placement::new(3.0, 1.0));
    t.advance(b"\x1b[1;2Hx\x1b[K");
    assert_eq!(picture(&t), ["#x", ""]);
}

#[test]
fn each_cell_knows_its_part_of_the_image() {
    let mut t = term(4, 2, "");
    t.place_image(image(4, 4), Placement::new(1.5, 2.0));
    let left = t.cell(1, 0).image().unwrap();
    let right = t.cell(1, 1).image().unwrap();
    assert_eq!(left.offset(), (0, 1));
    assert_eq!(right.extent(), (0.5, 1.0));
    assert!(left.same_placement(right));
    let uv = right.uv();
    assert!((uv[0] - 2.0 / 3.0).abs() < 1e-6 && uv[2] == 1.0);
    assert_eq!((uv[1], uv[3]), (0.5, 1.0));
}

#[test]
fn source_rectangle_maps_onto_the_cells() {
    let mut t = term(4, 2, "");
    let p = Placement {
        source: [0.5, 0.0, 1.0, 0.5],
        ..Placement::new(1.0, 1.0)
    };
    t.place_image(image(4, 4), p);
    assert_eq!(t.cell(0, 0).image().unwrap().uv(), [0.5, 0.0, 1.0, 0.5]);
}

#[test]
fn oldest_images_are_removed_beyond_the_memory_limit() {
    let mut t = term(4, 2, "");
    // Two 4x4 RGBA images are 128 bytes.
    t.set_image_memory_limit(100);
    let old = image(4, 4);
    t.place_image(old.clone(), Placement::new(1.0, 1.0));
    t.place_image(image(4, 4), Placement::new(1.0, 1.0));
    assert!(t.cell(0, 0).image().is_none());
    assert!(t.cell(0, 1).image().is_some());
}

#[test]
fn image_size_is_checked() {
    assert!(Image::from_rgba(2, 2, vec![0; 15]).is_none());
    assert!(Image::from_rgba(0, 2, Vec::new()).is_none());
}

#[test]
fn image_ids_are_unique() {
    assert_ne!(image(1, 1).id(), image(1, 1).id());
}

/// 16x32 solid red PNG.
const RED_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAABAAAAAgCAIAAACU62+bAAAAHUlEQVR4nGP4z8BAEiJN9aiGUQ2jGkY1jGoYohoAP8T+EE3PqSIAAAAASUVORK5CYII=";

#[test]
fn osc_1337_inline_png_is_decoded_and_placed_at_its_pixel_size() {
    let mut t = term(10, 4, "a");
    t.set_cell_pixel_size(8, 16);
    t.advance(format!("\x1b]1337;File=name=eC5wbmc=;inline=1:{RED_PNG}\x07").as_bytes());
    assert_eq!(picture(&t), ["a##", " ##", "", ""]);
    assert_eq!(pos(&t), (1, 3));
    let img = t.cell(0, 1).image().unwrap().image();
    assert_eq!((img.width(), img.height()), (16, 32));
    assert_eq!(img.rgba()[..4], [0xff, 0, 0, 0xff]);
}

#[test]
fn osc_1337_width_in_cells_scales_the_image() {
    let mut t = term(10, 6, "");
    t.set_cell_pixel_size(8, 16);
    t.advance(format!("\x1b]1337;File=inline=1;width=4:{RED_PNG}\x1b\\").as_bytes());
    assert_eq!(picture(&t), ["####", "####", "####", "####", "", ""]);
}

#[test]
fn osc_1337_without_inline_or_with_bad_data_is_ignored() {
    let mut t = term(10, 4, "");
    t.advance(format!("\x1b]1337;File=inline=0:{RED_PNG}\x07").as_bytes());
    t.advance(b"\x1b]1337;File=inline=1:bm90IGFuIGltYWdl\x07");
    assert_eq!(picture(&t), ["", "", "", ""]);
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn cell_pixel_size_survives_a_full_reset() {
    let mut t = Term::new(4, 2);
    t.set_cell_pixel_size(9, 18);
    t.advance(b"\x1bc");
    assert_eq!(t.cell_pixel_size(), (9, 18));
}
