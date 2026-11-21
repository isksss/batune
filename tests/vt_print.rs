use batune::vt::{Flags, MAX_ZEROWIDTH, Term};

fn term(cols: usize, rows: usize, input: &str) -> Term {
    let mut term = Term::new(cols, rows);
    term.advance(input.as_bytes());
    term
}

fn text(term: &Term, row: usize) -> String {
    term.grid().row(row).text()
}

#[test]
fn ascii_is_printed_left_to_right_and_moves_the_cursor() {
    let t = term(10, 3, "abc");
    assert_eq!(text(&t, 0), "abc");
    assert_eq!((t.cursor().row, t.cursor().col), (0, 3));
}

#[test]
fn wide_character_occupies_two_cells_with_a_spacer() {
    let t = term(10, 3, "あい");
    assert_eq!(text(&t, 0), "あい");
    assert_eq!(t.cell(0, 0).c, 'あ');
    assert!(t.cell(0, 0).flags.contains(Flags::WIDE));
    assert!(t.cell(0, 1).flags.contains(Flags::WIDE_SPACER));
    assert_eq!(t.cell(0, 2).c, 'い');
    assert_eq!(t.cursor().col, 4);
}

#[test]
fn reaching_the_last_column_defers_the_wrap_until_the_next_character() {
    let mut t = term(4, 3, "abcd");
    assert_eq!((t.cursor().row, t.cursor().col), (0, 3));
    assert_eq!(text(&t, 1), "");
    t.advance(b"e");
    assert_eq!(text(&t, 0), "abcd");
    assert_eq!(text(&t, 1), "e");
    assert!(t.grid().row(0).wrapped());
    assert!(!t.grid().row(1).wrapped());
}

#[test]
fn wide_character_that_does_not_fit_moves_to_the_next_row() {
    let t = term(5, 3, "abcdあ");
    assert_eq!(text(&t, 0), "abcd");
    assert_eq!(text(&t, 1), "あ");
    assert!(t.grid().row(0).wrapped());
}

#[test]
fn wrapping_on_the_last_row_scrolls_the_screen() {
    let t = term(3, 2, "abcdefg");
    assert_eq!(text(&t, 0), "def");
    assert_eq!(text(&t, 1), "g");
}

#[test]
fn combining_marks_attach_to_the_previous_cell() {
    let t = term(10, 2, "e\u{301}x");
    assert_eq!(t.cell(0, 0).c, 'e');
    assert_eq!(t.cell(0, 0).zerowidth(), &['\u{301}']);
    assert_eq!(t.cell(0, 1).c, 'x');
    assert_eq!(text(&t, 0), "e\u{301}x");
}

#[test]
fn combining_marks_past_the_limit_are_dropped() {
    let marks = "\u{301}".repeat(100_000);
    let t = term(10, 2, &format!("e{marks}x"));
    assert_eq!(t.cell(0, 0).zerowidth(), vec!['\u{301}'; MAX_ZEROWIDTH]);
    assert_eq!(t.cell(0, 1).c, 'x');
}

#[test]
fn split_utf8_sequences_across_calls_are_reassembled() {
    let mut t = Term::new(10, 2);
    let bytes = "あ".as_bytes();
    t.advance(&bytes[..1]);
    t.advance(&bytes[1..]);
    assert_eq!(t.cell(0, 0).c, 'あ');
}

#[test]
fn only_modified_rows_are_reported_dirty() {
    let mut t = Term::new(10, 3);
    t.take_dirty();
    t.advance(b"abcdefghijk");
    assert_eq!(t.take_dirty(), vec![0, 1]);
    assert_eq!(t.take_dirty(), Vec::<usize>::new());
}

#[test]
fn resize_keeps_content_and_the_cursor_row_visible() {
    let mut t = term(4, 3, "abcdefghij");
    assert_eq!(t.cursor().row, 2);
    t.resize(6, 2);
    assert_eq!((t.cols(), t.rows()), (6, 2));
    assert_eq!(text(&t, 0), "efgh");
    assert_eq!(text(&t, 1), "ij");
    assert_eq!(t.cursor().row, 1);
}

#[test]
fn zwj_sequence_shares_one_wide_cell() {
    let t = term(10, 2, "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}x");
    assert_eq!(t.cell(0, 0).c, '\u{1F468}');
    assert_eq!(
        t.cell(0, 0).zerowidth(),
        &['\u{200D}', '\u{1F469}', '\u{200D}', '\u{1F467}']
    );
    assert!(t.cell(0, 0).flags.contains(Flags::WIDE));
    assert!(t.cell(0, 1).flags.contains(Flags::WIDE_SPACER));
    assert_eq!(t.cell(0, 2).c, 'x');
    assert_eq!(t.cursor().col, 3);
}

#[test]
fn skin_tone_modifier_joins_its_base() {
    let t = term(10, 2, "\u{1F44D}\u{1F3FD}x");
    assert_eq!(t.cell(0, 0).zerowidth(), &['\u{1F3FD}']);
    assert_eq!(t.cell(0, 2).c, 'x');
    assert_eq!(t.cursor().col, 3);
}

#[test]
fn cluster_continues_across_a_pending_wrap() {
    let t = term(4, 2, "ab\u{1F44D}\u{1F3FD}");
    assert_eq!(t.cell(0, 2).zerowidth(), &['\u{1F3FD}']);
    assert_eq!(text(&t, 1), "");
}

#[test]
fn separate_emoji_stay_in_separate_cells() {
    let t = term(10, 2, "\u{1F468}\u{1F469}");
    assert_eq!(t.cell(0, 0).zerowidth(), &[] as &[char]);
    assert_eq!(t.cell(0, 2).c, '\u{1F469}');
}

#[test]
fn lone_skin_tone_modifier_after_a_blank_takes_its_own_cell() {
    let t = term(10, 2, " \u{1F3FD}");
    assert_eq!(t.cell(0, 1).c, '\u{1F3FD}');
    assert_eq!(t.cursor().col, 3);
}

#[test]
fn without_mode_2027_each_emoji_takes_its_own_cells() {
    let t = term(10, 2, "\x1b[?2027l\u{1F468}\u{200D}\u{1F469}");
    assert_eq!(t.cell(0, 0).zerowidth(), &['\u{200D}']);
    assert_eq!(t.cell(0, 2).c, '\u{1F469}');
    assert_eq!(t.cursor().col, 4);
}

#[test]
fn vs16_widens_a_narrow_base_to_two_cells() {
    let t = term(10, 2, "\u{2764}\u{FE0F}x");
    assert_eq!(t.cell(0, 0).c, '\u{2764}');
    assert_eq!(t.cell(0, 0).zerowidth(), &['\u{FE0F}']);
    assert!(t.cell(0, 0).flags.contains(Flags::WIDE));
    assert!(t.cell(0, 1).flags.contains(Flags::WIDE_SPACER));
    assert_eq!(t.cell(0, 2).c, 'x');
    assert_eq!(t.cursor().col, 3);
}

#[test]
fn widening_overwrites_the_cell_after_the_base() {
    let t = term(10, 2, "\u{2764}ab\x1b[1;2H\u{FE0F}");
    assert!(t.cell(0, 1).flags.contains(Flags::WIDE_SPACER));
    assert_eq!(t.cell(0, 2).c, 'b');
    assert_eq!(t.cursor().col, 2);
}

#[test]
fn flags_and_keycaps_are_two_cells_wide() {
    let t = term(10, 2, "\u{1F1EF}\u{1F1F5}1\u{FE0F}\u{20E3}|");
    assert!(t.cell(0, 0).flags.contains(Flags::WIDE));
    assert_eq!(t.cell(0, 0).zerowidth(), &['\u{1F1F5}']);
    assert!(t.cell(0, 2).flags.contains(Flags::WIDE));
    assert_eq!(t.cell(0, 2).zerowidth(), &['\u{FE0F}', '\u{20E3}']);
    assert_eq!(t.cell(0, 4).c, '|');
}

#[test]
fn widening_in_the_last_column_wraps_the_cluster_to_the_next_row() {
    let t = term(4, 3, "abc\u{2764}\u{FE0F}x");
    assert_eq!(text(&t, 0), "abc");
    assert!(t.grid().row(0).wrapped());
    assert_eq!(t.cell(1, 0).c, '\u{2764}');
    assert!(t.cell(1, 0).flags.contains(Flags::WIDE));
    assert_eq!(t.cell(1, 2).c, 'x');
}

#[test]
fn widening_up_to_the_last_column_defers_the_wrap() {
    let t = term(4, 3, "ab\u{2764}\u{FE0F}");
    assert!(t.cell(0, 2).flags.contains(Flags::WIDE));
    assert_eq!((t.cursor().row, t.cursor().col), (0, 3));
    assert_eq!(text(&t, 1), "");
}

#[test]
fn without_autowrap_a_cluster_in_the_last_column_stays_narrow() {
    let t = term(4, 2, "\x1b[?7labc\u{2764}\u{FE0F}");
    assert_eq!(t.cell(0, 3).c, '\u{2764}');
    assert!(!t.cell(0, 3).flags.contains(Flags::WIDE));
    assert_eq!(text(&t, 1), "");
}

#[test]
fn vs15_keeps_a_wide_emoji_wide() {
    let t = term(10, 2, "\u{231A}\u{FE0E}x");
    assert!(t.cell(0, 0).flags.contains(Flags::WIDE));
    assert_eq!(t.cell(0, 2).c, 'x');
}

#[test]
fn without_mode_2027_vs16_leaves_the_width_alone() {
    let t = term(10, 2, "\x1b[?2027l\u{2764}\u{FE0F}x");
    assert!(!t.cell(0, 0).flags.contains(Flags::WIDE));
    assert_eq!(t.cell(0, 1).c, 'x');
}
