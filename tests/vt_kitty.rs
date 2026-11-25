use batune::vt::Term;

/// A `w` x `h` RGBA image (f=32) as base64, all pixels 0x10.
fn pixels(w: usize, h: usize) -> String {
    // 0x10 0x10 0x10 encodes as "EBAQ".
    let n = w * h * 4;
    assert_eq!(n % 3, 0, "keep the test images padding-free");
    "EBAQ".repeat(n / 3)
}

/// A terminal whose cells are 10x20 px.
fn term(cols: usize, rows: usize) -> Term {
    let mut t = Term::new(cols, rows);
    t.set_cell_pixel_size(10, 20);
    t
}

/// `#` for cells showing an image, the text otherwise.
fn picture(term: &Term) -> Vec<String> {
    (0..term.rows())
        .map(|r| {
            let row = term.grid().row(r).cells().iter();
            let s: String = row
                .map(|c| if c.image().is_some() { '#' } else { c.c })
                .collect();
            s.trim_end().to_owned()
        })
        .collect()
}

fn pos(term: &Term) -> (usize, usize) {
    (term.cursor().row, term.cursor().col)
}

/// `ESC _ G <body> ESC \`
fn apc(body: &str) -> String {
    format!("\x1b_G{body}\x1b\\")
}

fn reply(term: &mut Term) -> String {
    String::from_utf8(term.take_responses()).unwrap()
}

#[test]
fn query_answers_ok_for_an_acceptable_image() {
    let mut t = Term::new(10, 3);
    // One RGB pixel.
    t.advance(apc("a=q,i=31,s=1,v=1,f=24;AAAA").as_bytes());
    assert_eq!(reply(&mut t), "\x1b_Gi=31;OK\x1b\\");
}

#[test]
fn query_reports_why_an_image_is_refused() {
    let mut t = Term::new(10, 3);
    t.advance(apc("a=q,i=1,t=f;L2V0Yy9wYXNzd2Q=").as_bytes());
    assert_eq!(
        reply(&mut t),
        "\x1b_Gi=1;EINVAL:only direct transmission (t=d) is supported\x1b\\"
    );
}

#[test]
fn graphics_commands_leave_no_text_and_split_writes_work() {
    let mut t = Term::new(20, 3);
    let input = format!(
        "ab{}cd",
        apc("a=q,i=2,s=1,v=1,f=24,m=1;AA") + &apc("m=0;AA")
    );
    for chunk in input.as_bytes().chunks(3) {
        t.advance(chunk);
    }
    assert_eq!(t.grid().row(0).text(), "abcd");
    assert_eq!(reply(&mut t), "\x1b_Gi=2;OK\x1b\\");
}

#[test]
fn commands_without_an_id_are_not_answered() {
    let mut t = Term::new(10, 3);
    t.advance(apc("a=q,s=1,v=1,f=24;AAAA").as_bytes());
    assert_eq!(reply(&mut t), "");
}

#[test]
fn transmit_and_display_covers_cells_by_pixel_size_and_moves_the_cursor_after_it() {
    let mut t = term(8, 4);
    t.advance(b"ab");
    // 30x30 px = 3 columns, 1.5 rows.
    t.advance(apc(&format!("a=T,s=30,v=30,i=1;{}", pixels(30, 30))).as_bytes());
    assert_eq!(picture(&t), ["ab###", "  ###", "", ""]);
    assert_eq!(pos(&t), (1, 5));
    assert_eq!(reply(&mut t), "\x1b_Gi=1;OK\x1b\\");
    let cell = t.cell(1, 2).image().unwrap();
    assert_eq!(cell.extent(), (1.0, 0.5));
    assert_eq!(cell.z(), 0);
}

#[test]
fn c_1_keeps_the_cursor_and_c_r_size_the_image_in_cells() {
    let mut t = term(8, 4);
    t.advance(apc(&format!("a=T,s=3,v=3,c=2,r=3,C=1,z=-1;{}", pixels(3, 3))).as_bytes());
    assert_eq!(picture(&t), ["##", "##", "##", ""]);
    assert_eq!(pos(&t), (0, 0));
    assert_eq!(t.cell(0, 0).image().unwrap().z(), -1);
}

#[test]
fn only_columns_given_keeps_the_aspect_ratio() {
    let mut t = term(8, 4);
    // 4 columns = 40 px wide, so 40 px = 2 rows high.
    t.advance(apc(&format!("a=T,s=3,v=3,c=4,C=1;{}", pixels(3, 3))).as_bytes());
    assert_eq!(picture(&t), ["####", "####", "", ""]);
}

#[test]
fn source_rectangle_selects_part_of_the_image() {
    let mut t = term(8, 4);
    t.advance(
        apc(&format!(
            "a=T,s=30,v=30,x=10,y=0,w=10,h=15,C=1;{}",
            pixels(30, 30)
        ))
        .as_bytes(),
    );
    let uv = t.cell(0, 0).image().unwrap().uv();
    assert_eq!(uv, [1.0 / 3.0, 0.0, 2.0 / 3.0, 0.5]);
}

#[test]
fn stored_images_are_placed_by_id_or_number() {
    let mut t = term(8, 4);
    t.advance(apc(&format!("a=t,s=3,v=3,i=5;{}", pixels(3, 3))).as_bytes());
    assert_eq!(picture(&t), ["", "", "", ""], "a=t only stores");
    t.advance(apc("a=p,i=5,c=1,r=1").as_bytes());
    assert_eq!(picture(&t), ["#", "", "", ""]);
    assert_eq!(reply(&mut t), "\x1b_Gi=5;OK\x1b\\\x1b_Gi=5;OK\x1b\\");

    t.advance(apc(&format!("a=t,s=3,v=3,I=9;{}", pixels(3, 3))).as_bytes());
    let answer = reply(&mut t);
    assert!(
        answer.starts_with("\x1b_Gi=") && answer.ends_with(",I=9;OK\x1b\\"),
        "{answer:?}"
    );
    t.advance(apc("a=p,I=9,c=1,r=1").as_bytes());
    assert_eq!(picture(&t), ["##", "", "", ""]);
}

#[test]
fn placing_an_unknown_image_is_an_error() {
    let mut t = term(8, 4);
    t.advance(apc("a=p,i=7").as_bytes());
    assert_eq!(
        reply(&mut t),
        "\x1b_Gi=7;ENOENT:no image with this id\x1b\\"
    );
}

#[test]
fn a_placement_id_replaces_the_earlier_placement() {
    let mut t = term(8, 4);
    t.advance(apc(&format!("a=t,s=3,v=3,i=1;{}", pixels(3, 3))).as_bytes());
    t.advance(apc("a=p,i=1,p=2,c=1,r=1").as_bytes());
    t.advance(b"\x1b[3;1H");
    t.advance(apc("a=p,i=1,p=2,c=1,r=1").as_bytes());
    assert_eq!(picture(&t), ["", "", "#", ""]);
}

#[test]
fn sending_an_id_again_replaces_the_image_and_its_placements() {
    let mut t = term(8, 4);
    t.advance(apc(&format!("a=T,s=3,v=3,i=1,c=1,r=1;{}", pixels(3, 3))).as_bytes());
    t.advance(apc(&format!("a=t,s=3,v=3,i=1;{}", pixels(3, 3))).as_bytes());
    assert_eq!(picture(&t), ["", "", "", ""]);
}

/// Stores image 1 and places it at row 1, col 1 (p=1) and row 3, col 3
/// (p=2, z=5), one cell each, without moving the cursor.
fn two_placements() -> Term {
    let mut t = term(6, 4);
    t.advance(apc(&format!("a=t,s=3,v=3,i=1,q=2;{}", pixels(3, 3))).as_bytes());
    t.advance(apc("a=p,i=1,p=1,c=1,r=1,C=1").as_bytes());
    t.advance(b"\x1b[3;3H");
    t.advance(apc("a=p,i=1,p=2,c=1,r=1,C=1,z=5").as_bytes());
    t.take_responses();
    t
}

fn deleted(command: &str) -> Vec<String> {
    let mut t = two_placements();
    t.advance(apc(command).as_bytes());
    assert_eq!(reply(&mut t), "", "deletions are not answered");
    picture(&t)
}

#[test]
fn delete_selects_placements_by_id_position_or_z() {
    let both = ["#", "", "  #", ""];
    let first = ["#", "", "", ""];
    let second = ["", "", "  #", ""];
    let none = ["", "", "", ""];
    assert_eq!(deleted("a=d"), none);
    assert_eq!(deleted("a=d,d=i,i=1"), none);
    assert_eq!(deleted("a=d,d=i,i=1,p=2"), first);
    assert_eq!(deleted("a=d,d=i,i=2"), both);
    assert_eq!(deleted("a=d,d=c"), first, "the cursor is on the second");
    assert_eq!(deleted("a=d,d=p,x=1,y=1"), second);
    assert_eq!(deleted("a=d,d=q,x=1,y=1,z=5"), both);
    assert_eq!(deleted("a=d,d=q,x=3,y=3,z=5"), first);
    assert_eq!(deleted("a=d,d=x,x=3"), first);
    assert_eq!(deleted("a=d,d=y,y=1"), second);
    assert_eq!(deleted("a=d,d=z,z=5"), first);
    assert_eq!(deleted("a=d,d=r,x=1,y=3"), none);
    assert_eq!(deleted("a=d,d=r,x=2,y=3"), both);
}

#[test]
fn lowercase_delete_keeps_the_image_and_uppercase_frees_it() {
    let mut t = two_placements();
    t.advance(apc("a=d,d=i,i=1").as_bytes());
    t.advance(apc("a=p,i=1,c=1,r=1").as_bytes());
    assert_eq!(reply(&mut t), "\x1b_Gi=1;OK\x1b\\");

    t.advance(apc("a=d,d=I,i=1").as_bytes());
    t.advance(apc("a=p,i=1").as_bytes());
    assert_eq!(
        reply(&mut t),
        "\x1b_Gi=1;ENOENT:no image with this id\x1b\\"
    );
}

#[test]
fn uppercase_delete_by_position_frees_images_left_without_placements() {
    let mut t = two_placements();
    t.advance(apc("a=d,d=P,x=1,y=1").as_bytes());
    t.advance(apc("a=p,i=1,c=1,r=1").as_bytes());
    assert_eq!(
        reply(&mut t),
        "\x1b_Gi=1;OK\x1b\\",
        "p=2 still shows image 1"
    );
    t.advance(apc("a=d,d=A").as_bytes());
    t.advance(apc("a=p,i=1").as_bytes());
    assert_eq!(
        reply(&mut t),
        "\x1b_Gi=1;ENOENT:no image with this id\x1b\\"
    );
}

#[test]
fn delete_by_position_leaves_other_kinds_of_images() {
    let mut t = term(6, 4);
    let image = batune::vt::Image::from_rgba(1, 1, vec![0; 4]).unwrap();
    t.place_image(
        std::sync::Arc::new(image),
        batune::vt::Placement::new(1.0, 1.0),
    );
    t.advance(apc("a=d,d=p,x=1,y=1").as_bytes());
    assert_eq!(picture(&t), ["#", "", "", ""]);
}

#[test]
fn stored_images_beyond_the_memory_limit_are_freed_oldest_first() {
    let mut t = term(6, 4);
    // Each 3x3 RGBA image is 36 bytes.
    t.set_image_memory_limit(80);
    for id in 1..=3 {
        t.advance(apc(&format!("a=t,s=3,v=3,i={id},q=2;{}", pixels(3, 3))).as_bytes());
    }
    for id in 1..=3 {
        t.advance(apc(&format!("a=p,i={id},q=1")).as_bytes());
    }
    assert_eq!(
        reply(&mut t),
        "\x1b_Gi=1;ENOENT:no image with this id\x1b\\"
    );
}

#[test]
fn a_placement_by_number_replaces_its_earlier_placement_too() {
    let mut t = term(8, 4);
    t.advance(apc(&format!("a=t,s=3,v=3,I=4,q=2;{}", pixels(3, 3))).as_bytes());
    t.advance(apc("a=p,I=4,p=1,c=1,r=1").as_bytes());
    t.advance(b"\x1b[3;1H");
    t.advance(apc("a=p,I=4,p=1,c=1,r=1").as_bytes());
    assert_eq!(picture(&t), ["", "", "#", ""]);
}

#[test]
fn a_huge_row_count_is_capped_at_1000_rows_and_returns_promptly() {
    let mut t = term(8, 4);
    let start = std::time::Instant::now();
    t.advance(apc("a=T,s=1,v=1,f=24,c=2,r=20000000;AAAA").as_bytes());
    assert!(start.elapsed() < std::time::Duration::from_secs(10));
    // 1000 rows from row 0 on a 4-row screen: 996 rows scrolled away and
    // the image's last row on the bottom line.
    assert_eq!(t.scrollback_len(), 996);
    assert_eq!(t.cell(3, 0).image().unwrap().offset(), (0, 999));
    assert_eq!(pos(&t), (3, 2));
}

#[test]
fn a_huge_column_count_is_capped_at_1000_columns() {
    let mut t = term(8, 4);
    // c alone would make the 1x1 image 20000000 columns and as many px tall.
    t.advance(apc("a=T,s=1,v=1,f=24,c=20000000,r=1,C=1;AAAA").as_bytes());
    assert_eq!(picture(&t), ["########", "", "", ""]);
    let cell = t.cell(0, 7).image().unwrap();
    assert_eq!(cell.uv()[2], 8.0 / 1000.0);
}

/// Places a stored 3x3 image over 2x2 cells at the top left, as herdr does:
/// between DECSC and DECRC, with the cursor kept (`C=1`).
fn placed_over_text() -> Term {
    let mut t = term(6, 4);
    t.advance(b"abcdef\r\nghijkl");
    t.advance(apc(&format!("a=t,s=3,v=3,i=1,q=2;{}", pixels(3, 3))).as_bytes());
    t.advance(format!("\x1b7\x1b[H{}\x1b8", apc("a=p,i=1,p=1,c=2,r=2,C=1,q=2")).as_bytes());
    t
}

#[test]
fn a_placement_keeps_the_text_under_it() {
    let t = placed_over_text();
    assert_eq!(t.grid().row(0).text(), "abcdef");
    assert_eq!(t.grid().row(1).text(), "ghijkl");
    assert!(t.cell(1, 1).image().is_some());
}

#[test]
fn printing_or_erasing_over_a_placement_does_not_remove_it() {
    let mut t = placed_over_text();
    // A redraw: home, print over the image, erase the rest of the line,
    // erase characters, then erase below.
    t.advance(b"\x1b[Hxy\x1b[K\r\n\x1b[2X\x1b[1K\x1b[J\x1b[2K");
    assert_eq!(picture(&t), ["##", "##", "", ""]);
    assert_eq!(t.grid().row(0).text(), "xy");
}

#[test]
fn inserting_or_deleting_characters_moves_the_text_under_a_placement() {
    let mut t = placed_over_text();
    t.advance(b"\x1b[H\x1b[2P\x1b[2;1H\x1b[1@");
    assert_eq!(picture(&t), ["##ef", "##hijk", "", ""]);
    assert_eq!(t.grid().row(0).text(), "cdef");
    assert_eq!(t.grid().row(1).text(), " ghijk");
}

#[test]
fn a_placement_goes_with_a_clear_screen_or_reset() {
    for clear in ["\x1b[2J", "\x1bc"] {
        let mut t = placed_over_text();
        t.advance(clear.as_bytes());
        assert_eq!(picture(&t), ["", "", "", ""], "{clear:?}");
    }
}

#[test]
fn a_placement_scrolls_with_the_text() {
    let mut t = placed_over_text();
    t.advance(b"\x1b[4H\n");
    assert_eq!(picture(&t), ["##ijkl", "", "", ""]);
}

#[test]
fn printing_over_other_images_still_removes_them() {
    let mut t = term(6, 4);
    let image = batune::vt::Image::from_rgba(1, 1, vec![0; 4]).unwrap();
    t.place_image(
        std::sync::Arc::new(image),
        batune::vt::Placement::new(2.0, 1.0),
    );
    t.advance(b"\x1b[Hx\x1b[K");
    assert_eq!(picture(&t), ["x", "", "", ""]);
}
