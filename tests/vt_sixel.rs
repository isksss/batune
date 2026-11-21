use batune::vt::Term;

fn term(cols: usize, rows: usize, input: &str) -> Term {
    let mut term = Term::new(cols, rows);
    term.set_cell_pixel_size(4, 6);
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
fn sixel_image_covers_cells_by_pixel_size_and_leaves_the_cursor_below() {
    // 8 x 12 pixels on 4 x 6 pixel cells: 2 x 2 cells.
    let t = term(6, 5, "ab\x1bPq\"1;1;8;12#1;2;100;0;0!8~-!8~\x1b\\");
    assert_eq!(picture(&t), ["ab##", "  ##", "", "", ""]);
    assert_eq!(pos(&t), (2, 2));
}

#[test]
fn sixel_pixels_and_transparency_follow_p2() {
    let t = term(6, 4, "\x1bP0;1q#1;2;0;100;0@\x1b\\");
    let image = t.grid().cell(0, 0).image().unwrap().image().clone();
    assert_eq!((image.width(), image.height()), (1, 1));
    assert_eq!(&image.rgba()[..], [0, 255, 0, 255]);

    let t = term(6, 4, "\x1bPq\"1;1;2;1#1;2;0;100;0@\x1b\\");
    let image = t.grid().cell(0, 0).image().unwrap().image().clone();
    // The second pixel was never drawn and takes the background.
    assert_eq!(&image.rgba()[4..], [0, 0, 0, 255]);
}

#[test]
fn empty_sixel_leaves_the_screen_and_cursor_alone() {
    let t = term(6, 4, "ab\x1bPq#1;2;1;2;3\x1b\\cd");
    assert_eq!(picture(&t), ["abcd", "", "", ""]);
    assert_eq!(pos(&t), (0, 4));
}

#[test]
fn sixel_taller_than_the_screen_scrolls_it() {
    let t = term(4, 2, "x\r\n\x1bPq~-~-~\x1b\\");
    assert_eq!(picture(&t), ["#", ""]);
    assert_eq!(pos(&t), (1, 0));
}

#[test]
fn other_dcs_sequences_are_ignored() {
    let t = term(6, 2, "\x1bP$qm\x1b\\ok");
    assert_eq!(picture(&t), ["ok", ""]);
}

fn reply(t: &mut Term) -> String {
    String::from_utf8(t.take_responses()).unwrap()
}

#[test]
fn xtsmgraphics_reports_color_registers_and_sixel_geometry() {
    let mut t = term(10, 5, "\x1b[?1;1S\x1b[?1;4S\x1b[?1;3;16S");
    assert_eq!(reply(&mut t), "\x1b[?1;0;1024S".repeat(3));
    // 10 x 5 cells of 4 x 6 pixels.
    let mut t = term(10, 5, "\x1b[?2;1S\x1b[?2;4S");
    assert_eq!(reply(&mut t), "\x1b[?2;0;40;30S\x1b[?2;0;4096;4096S");
}

#[test]
fn xtsmgraphics_rejects_regis_and_unknown_actions() {
    let mut t = term(10, 5, "\x1b[?3;1S\x1b[?1;9S");
    assert_eq!(reply(&mut t), "\x1b[?3;1;0S\x1b[?1;2;0S");
}
