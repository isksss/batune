use batune::vt::{Event, Flags, Term};

fn term(cols: usize, rows: usize, input: &str) -> Term {
    let mut term = Term::new(cols, rows);
    term.advance(input.as_bytes());
    term
}

fn text(term: &Term, row: usize) -> String {
    term.grid().row(row).text()
}

fn pos(term: &Term) -> (usize, usize) {
    (term.cursor().row, term.cursor().col)
}

#[test]
fn cr_lf_starts_a_new_line() {
    let t = term(10, 3, "ab\r\ncd");
    assert_eq!(text(&t, 0), "ab");
    assert_eq!(text(&t, 1), "cd");
    assert!(!t.grid().row(0).wrapped());
}

#[test]
fn lf_alone_keeps_the_column() {
    let t = term(10, 3, "ab\nc");
    assert_eq!(text(&t, 1), "  c");
}

#[test]
fn lf_on_the_last_row_scrolls_up() {
    let t = term(10, 2, "a\r\nb\r\nc");
    assert_eq!(text(&t, 0), "b");
    assert_eq!(text(&t, 1), "c");
}

#[test]
fn backspace_moves_left_and_stops_at_column_zero() {
    let t = term(10, 2, "ab\x08\x08\x08x");
    assert_eq!(text(&t, 0), "xb");
}

#[test]
fn cr_cancels_the_pending_wrap() {
    let t = term(3, 2, "abc\rx");
    assert_eq!(text(&t, 0), "xbc");
    assert_eq!(text(&t, 1), "");
}

#[test]
fn cup_is_one_based_and_clamped() {
    assert_eq!(pos(&term(10, 5, "\x1b[2;3H")), (1, 2));
    assert_eq!(pos(&term(10, 5, "\x1b[H")), (0, 0));
    assert_eq!(pos(&term(10, 5, "\x1b[99;99f")), (4, 9));
    assert_eq!(pos(&term(10, 5, "\x1b[0;0H")), (0, 0));
}

#[test]
fn relative_moves_are_clamped_to_the_screen() {
    let t = term(10, 5, "\x1b[3;3H\x1b[A\x1b[2B\x1b[3C\x1b[D");
    assert_eq!(pos(&t), (3, 4));
    assert_eq!(pos(&term(10, 5, "\x1b[99A")), (0, 0));
    assert_eq!(pos(&term(10, 5, "\x1b[99B\x1b[99C")), (4, 9));
    assert_eq!(pos(&term(10, 5, "\x1b[99D")), (0, 0));
}

#[test]
fn cnl_and_cpl_also_return_to_column_zero() {
    assert_eq!(pos(&term(10, 5, "abc\x1b[2E")), (2, 0));
    assert_eq!(pos(&term(10, 5, "\x1b[4;5H\x1b[2F")), (1, 0));
}

#[test]
fn cha_hpa_and_vpa_set_one_coordinate() {
    assert_eq!(pos(&term(10, 5, "\x1b[3;3H\x1b[7G")), (2, 6));
    assert_eq!(pos(&term(10, 5, "\x1b[3;3H\x1b[5`")), (2, 4));
    assert_eq!(pos(&term(10, 5, "\x1b[3;3H\x1b[4d")), (3, 2));
}

#[test]
fn hpr_and_vpr_move_relatively() {
    assert_eq!(pos(&term(10, 5, "\x1b[2;2H\x1b[3a\x1b[2e")), (3, 4));
}

#[test]
fn ht_advances_to_every_eighth_column_then_the_last() {
    let mut t = term(20, 2, "a\t");
    assert_eq!(t.cursor().col, 8);
    t.advance(b"\t");
    assert_eq!(t.cursor().col, 16);
    t.advance(b"\t");
    assert_eq!(t.cursor().col, 19);
}

#[test]
fn hts_and_tbc_edit_tab_stops() {
    // Set a stop at column 3, clear the one at 8.
    let mut t = term(20, 2, "\x1b[4G\x1bH\x1b[9G\x1b[g\r\t");
    assert_eq!(t.cursor().col, 3);
    t.advance(b"\t");
    assert_eq!(t.cursor().col, 16);
    t.advance(b"\x1b[3g\r\t");
    assert_eq!(t.cursor().col, 19);
}

#[test]
fn cht_and_cbt_move_by_tab_stops() {
    assert_eq!(pos(&term(40, 2, "\x1b[2I")).1, 16);
    assert_eq!(pos(&term(40, 2, "\x1b[20G\x1b[Z")).1, 16);
    assert_eq!(pos(&term(40, 2, "\x1b[20G\x1b[9Z")).1, 0);
}

#[test]
fn resize_wider_adds_default_tab_stops() {
    let mut t = Term::new(10, 2);
    t.resize(30, 2);
    t.advance(b"\x1b[10G\t\t");
    assert_eq!(t.cursor().col, 24);
}

#[test]
fn reverse_index_at_the_top_scrolls_down() {
    let t = term(10, 3, "a\r\nb\x1b[H\x1bM");
    assert_eq!(text(&t, 0), "");
    assert_eq!(text(&t, 1), "a");
    assert_eq!(text(&t, 2), "b");
}

#[test]
fn index_and_next_line() {
    assert_eq!(pos(&term(10, 3, "ab\x1bD")), (1, 2));
    assert_eq!(pos(&term(10, 3, "ab\x1bE")), (1, 0));
}

#[test]
fn decsc_and_decrc_save_and_restore_the_position() {
    let t = term(10, 5, "\x1b[2;3H\x1b7\x1b[5;5H\x1b8x");
    assert_eq!(text(&t, 1), "  x");
}

#[test]
fn disabling_autowrap_overwrites_the_last_column() {
    let t = term(3, 2, "\x1b[?7labcde");
    assert_eq!(text(&t, 0), "abe");
    assert_eq!(text(&t, 1), "");
    let t = term(3, 2, "\x1b[?7l\x1b[?7habcd");
    assert_eq!(text(&t, 1), "d");
}

#[test]
fn bel_is_reported_as_an_event() {
    let mut t = term(10, 2, "a\x07");
    assert_eq!(t.drain_events().collect::<Vec<_>>(), vec![Event::Bell]);
    assert_eq!(t.drain_events().count(), 0);
}

#[test]
fn overwriting_half_of_a_wide_character_erases_the_other_half() {
    let mut t = term(6, 2, "あい");
    // Overwrite the spacer of 'あ'.
    t.advance(b"\x1b[1;2Hx");
    assert_eq!(text(&t, 0), " xい");
    assert!(!t.cell(0, 0).flags.contains(Flags::WIDE));
    // Overwrite the head of 'い'.
    let t2 = term(6, 2, "あい\x1b[1;3Hx");
    assert_eq!(text(&t2, 0), "あx");
    assert!(!t2.cell(0, 3).flags.contains(Flags::WIDE_SPACER));
}
