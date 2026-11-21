use batune::vt::Term;

fn term(cols: usize, rows: usize, input: &str) -> Term {
    let mut term = Term::new(cols, rows);
    term.advance(input.as_bytes());
    term
}

fn history(term: &Term) -> Vec<String> {
    (0..term.scrollback_len())
        .map(|i| term.grid().scrollback_row(i).text())
        .collect()
}

fn view(term: &Term) -> Vec<String> {
    (0..term.rows())
        .map(|l| term.display_row(l).text())
        .collect()
}

#[test]
fn rows_scrolled_off_the_top_go_to_the_scrollback() {
    let t = term(5, 2, "a\r\nb\r\nc\r\nd");
    assert_eq!(history(&t), ["a", "b"]);
    assert_eq!(view(&t), ["c", "d"]);
}

#[test]
fn rows_scrolled_out_of_a_lower_region_are_lost() {
    let t = term(5, 3, "\x1b[2;3r\x1b[3;1Ha\nb\nc");
    assert_eq!(t.scrollback_len(), 0);
}

#[test]
fn su_at_the_top_keeps_rows_in_the_scrollback() {
    let t = term(5, 2, "a\x1b[S");
    assert_eq!(history(&t), ["a"]);
}

#[test]
fn the_oldest_rows_are_dropped_beyond_the_limit() {
    let mut t = Term::new(5, 1);
    t.set_scrollback_limit(2);
    t.advance(b"a\r\nb\r\nc\r\nd");
    assert_eq!(history(&t), ["b", "c"]);
    t.set_scrollback_limit(1);
    assert_eq!(history(&t), ["c"]);
    t.set_scrollback_limit(0);
    t.advance(b"\r\ne");
    assert_eq!(t.scrollback_len(), 0);
}

#[test]
fn the_view_shows_scrollback_above_the_screen_when_scrolled_back() {
    let mut t = term(5, 2, "a\r\nb\r\nc\r\nd");
    t.scroll_display(1);
    assert_eq!(t.display_offset(), 1);
    assert_eq!(view(&t), ["b", "c"]);
    t.scroll_display(10);
    assert_eq!(view(&t), ["a", "b"]);
    t.scroll_display(-1);
    assert_eq!(view(&t), ["b", "c"]);
    t.set_display_offset(0);
    assert_eq!(view(&t), ["c", "d"]);
}

#[test]
fn moving_the_view_marks_every_line_dirty() {
    let mut t = term(5, 2, "a\r\nb\r\nc");
    t.take_dirty();
    t.scroll_display(1);
    assert_eq!(t.take_dirty(), [0, 1]);
    t.scroll_display(1);
    assert_eq!(t.take_dirty(), Vec::<usize>::new());
}

#[test]
fn dirty_screen_rows_are_reported_as_display_lines() {
    let mut t = term(5, 3, "a\r\nb\r\nc\r\nd");
    t.scroll_display(1);
    t.take_dirty();
    t.advance(b"\x1b[1;1Hx\x1b[3;1Hy");
    assert_eq!(t.take_dirty(), [1]);
}

#[test]
fn the_scrolled_back_view_stays_on_the_same_text_while_output_arrives() {
    let mut t = term(5, 2, "a\r\nb\r\nc");
    t.scroll_display(1);
    assert_eq!(view(&t), ["a", "b"]);
    t.advance(b"\r\nd\r\ne");
    assert_eq!(t.display_offset(), 3);
    assert_eq!(view(&t), ["a", "b"]);
}

#[test]
fn ed_3_clears_the_scrollback_and_returns_to_the_live_screen() {
    let mut t = term(5, 2, "a\r\nb\r\nc");
    t.scroll_display(1);
    t.advance(b"\x1b[3J");
    assert_eq!(t.scrollback_len(), 0);
    assert_eq!(t.display_offset(), 0);
    assert_eq!(view(&t), ["b", "c"]);
}

#[test]
fn shrinking_moves_rows_above_the_cursor_to_the_scrollback() {
    let mut t = term(5, 3, "a\r\nb\r\nc");
    t.resize(5, 2);
    assert_eq!(history(&t), ["a"]);
    assert_eq!(view(&t), ["b", "c"]);
}

#[test]
fn scrollback_rows_are_resized_with_the_screen() {
    let mut t = term(5, 1, "abcde\r\n");
    t.resize(3, 1);
    assert_eq!(t.grid().scrollback_row(0).cells().len(), 3);
    assert_eq!(history(&t), ["abc"]);
    t.resize(8, 1);
    assert_eq!(t.grid().scrollback_row(0).cells().len(), 8);
}

#[test]
fn rows_reused_from_a_full_scrollback_come_back_blank() {
    let mut t = Term::new(3, 2);
    t.set_scrollback_limit(1);
    t.advance(b"\x1b[44mabc\r\ndef\r\nghi\r\n");
    assert_eq!(view(&t), ["ghi", ""]);
    assert_eq!(t.cell(1, 0).bg, batune::vt::Color::Indexed(4));
}

#[test]
fn rows_dropped_from_a_full_scrollback_are_counted() {
    let mut t = Term::new(3, 1);
    t.set_scrollback_limit(2);
    t.advance(b"a\r\nb\r\nc\r\nd\r\ne");
    assert_eq!(history(&t), ["c", "d"]);
    assert_eq!(t.scrollback_dropped(), 2);
}

#[test]
fn rows_dropped_by_a_lower_limit_ed_3_and_ris_are_counted() {
    let mut t = term(3, 1, "a\r\nb\r\nc\r\nd");
    t.set_scrollback_limit(2);
    assert_eq!(t.scrollback_dropped(), 1);
    t.advance(b"\x1b[3J");
    assert_eq!(t.scrollback_dropped(), 3);
    t.advance(b"\r\ne\r\nf\x1bc");
    assert_eq!(t.scrollback_dropped(), 5);
}

#[test]
fn rows_dropped_when_shrinking_into_a_full_scrollback_are_counted() {
    let mut t = Term::new(3, 3);
    t.set_scrollback_limit(1);
    t.advance(b"a\r\nb\r\nc");
    t.resize(3, 1);
    assert_eq!(history(&t), ["b"]);
    assert_eq!(t.scrollback_dropped(), 1);
}
