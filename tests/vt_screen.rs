use batune::vt::{Color, Event, Term};

fn term(cols: usize, rows: usize, input: &str) -> Term {
    let mut term = Term::new(cols, rows);
    term.advance(input.as_bytes());
    term
}

fn lines(term: &Term) -> Vec<String> {
    (0..term.rows())
        .map(|r| term.grid().row(r).text())
        .collect()
}

fn pos(term: &Term) -> (usize, usize) {
    (term.cursor().row, term.cursor().col)
}

#[test]
fn mode_1049_saves_the_cursor_and_shows_a_cleared_alternate_screen() {
    let t = term(5, 3, "ab\r\ncd\x1b[?1049h");
    assert!(t.is_alt_screen());
    assert_eq!(lines(&t), ["", "", ""]);
    assert_eq!(pos(&t), (1, 2));
}

#[test]
fn leaving_1049_restores_the_primary_screen_and_the_cursor() {
    let t = term(5, 3, "ab\r\ncd\x1b[?1049h\x1b[3;1Hxyz\x1b[?1049l");
    assert!(!t.is_alt_screen());
    assert_eq!(lines(&t), ["ab", "cd", ""]);
    assert_eq!(pos(&t), (1, 2));
}

#[test]
fn the_alternate_screen_does_not_add_to_the_scrollback() {
    let t = term(5, 2, "\x1b[?1049ha\r\nb\r\nc\r\nd");
    assert_eq!(t.scrollback_len(), 0);
}

#[test]
fn the_primary_scrollback_survives_the_alternate_screen() {
    let t = term(5, 1, "a\r\nb\x1b[?1049hx\x1b[?1049l");
    assert_eq!(t.scrollback_len(), 1);
    assert_eq!(t.grid().scrollback_row(0).text(), "a");
}

#[test]
fn mode_47_keeps_the_alternate_screen_contents_between_visits() {
    let t = term(5, 2, "\x1b[?47hx\x1b[?47l\x1b[?47h");
    assert_eq!(lines(&t), ["x", ""]);
}

#[test]
fn mode_1047_clears_the_alternate_screen_on_leaving() {
    let t = term(5, 2, "\x1b[?1047hx\x1b[?1047l\x1b[?1047h");
    assert_eq!(lines(&t), ["", ""]);
}

#[test]
fn switching_screens_marks_every_line_dirty() {
    let mut t = term(5, 3, "");
    t.take_dirty();
    t.advance(b"\x1b[?1049h");
    assert_eq!(t.take_dirty(), [0, 1, 2]);
}

#[test]
fn resizing_on_the_alternate_screen_also_resizes_the_primary_one() {
    let mut t = term(5, 3, "a\r\nb\r\nc\x1b[?1049h");
    t.resize(4, 2);
    t.advance(b"\x1b[?1049l");
    assert_eq!(t.grid().cols(), 4);
    assert_eq!(lines(&t), ["b", "c"]);
    assert_eq!(pos(&t), (1, 1));
}

#[test]
fn a_cursor_saved_on_the_alternate_screen_survives_resizes_after_leaving_it() {
    let mut t = term(80, 24, "\x1b[?1049h\x1b[24;1H\x1b7");
    t.resize(80, 10);
    t.advance(b"\x1b[?1049l");
    t.resize(80, 12);
    t.advance(b"\x1b[?47h\x1b8");
    assert_eq!(pos(&t), (9, 0));
}

#[test]
fn a_cursor_saved_on_the_primary_screen_survives_resizes_on_the_alternate_one() {
    let mut t = term(80, 24, "\x1b[24;1H\x1b7");
    t.resize(80, 10);
    t.advance(b"\x1b[?47h");
    t.resize(80, 12);
    t.advance(b"\x1b[?47l\x1b8");
    assert_eq!(pos(&t), (9, 0));
}

#[test]
fn the_saved_cursor_moves_with_the_text_when_rows_are_dropped_from_the_top() {
    let mut t = term(5, 3, "a\r\nb\r\nc\x1b[2;1H\x1b7\x1b[3;2H");
    t.resize(5, 2);
    t.advance(b"\x1b8");
    assert_eq!(lines(&t), ["b", "c"]);
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn a_saved_cursor_beyond_a_narrower_screen_is_restored_to_the_last_column() {
    let mut t = term(10, 3, "\x1b[1;10H\x1b7");
    t.resize(4, 3);
    t.advance(b"\x1b8");
    assert_eq!(pos(&t), (0, 3));
}

#[test]
fn decrc_restores_position_attributes_wrap_state_and_origin_mode() {
    let t = term(
        5,
        5,
        "\x1b[2;4r\x1b[?6h\x1b[31m\x1b[2;5Ha\x1b7\x1b[?6l\x1b[m\x1b[H\x1b8c",
    );
    // Pending wrap was saved after "a" filled the last column, so "c"
    // wraps to the next row.
    assert_eq!(t.grid().row(3).text(), "c");
    assert_eq!(t.cell(3, 0).fg, Color::Indexed(1));
    // Origin mode came back: CUP 1;1 is the top margin.
    let mut t = t;
    t.advance(b"\x1b[1;1H");
    assert_eq!(pos(&t), (1, 0));
}

#[test]
fn decrc_without_decsc_goes_home_with_default_attributes() {
    let t = term(5, 3, "\x1b[31m\x1b[2;3H\x1b8x");
    assert_eq!(pos(&t), (0, 1));
    assert_eq!(t.cell(0, 0).fg, Color::Default);
}

#[test]
fn csi_s_and_csi_u_save_and_restore_the_cursor() {
    let t = term(5, 3, "\x1b[2;3H\x1b[s\x1b[H\x1b[u");
    assert_eq!(pos(&t), (1, 2));
}

#[test]
fn the_primary_and_alternate_screens_have_their_own_saved_cursor() {
    let t = term(5, 3, "\x1b[3;3H\x1b7\x1b[?47h\x1b[2;2H\x1b7\x1b[?47l\x1b8");
    assert_eq!(pos(&t), (2, 2));
}

#[test]
fn dec_line_drawing_is_used_while_designated_to_gl() {
    let t = term(10, 2, "\x1b(0lqk\x1b(Bq");
    assert_eq!(t.grid().row(0).text(), "┌─┐q");
    let t = term(10, 2, "\x1b)0a\x0eq\x0fq");
    assert_eq!(t.grid().row(0).text(), "a─q");
}

#[test]
fn ris_resets_the_screen_modes_and_scrollback_but_not_the_size() {
    let mut t = Term::new(5, 2);
    t.set_scrollback_limit(1);
    t.advance(b"\x1b[31ma\r\nb\r\nc\x1b[?7l\x1b(0\x1b[?1049h\x07\x1bc");
    assert!(!t.is_alt_screen());
    assert_eq!(lines(&t), ["", ""]);
    assert_eq!(pos(&t), (0, 0));
    assert_eq!(t.scrollback_len(), 0);
    assert_eq!((t.cols(), t.rows()), (5, 2));
    assert_eq!(t.drain_events().collect::<Vec<_>>(), [Event::Bell]);
    t.advance(b"q\r\nx\r\ny\r\nz");
    assert_eq!(t.grid().row(0).text(), "y");
    assert_eq!(t.cell(0, 0).fg, Color::Default);
    assert_eq!(t.scrollback_len(), 1);
}

#[test]
fn decstr_resets_attributes_and_modes_but_keeps_the_text() {
    let t = term(5, 3, "\x1b[2;3r\x1b[31mab\x1b[!pc");
    assert_eq!(t.grid().row(0).text(), "abc");
    assert_eq!(t.cell(0, 2).fg, Color::Default);
}
