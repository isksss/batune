use batune::vt::{Flags, Term};

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

const FILLED: &str = "abcde\r\nfghij\r\nklmno";

#[test]
fn ed_0_erases_from_the_cursor_to_the_end_of_the_screen() {
    let t = term(5, 3, &format!("{FILLED}\x1b[2;3H\x1b[J"));
    assert_eq!(lines(&t), ["abcde", "fg", ""]);
}

#[test]
fn ed_1_erases_from_the_start_of_the_screen_to_the_cursor() {
    let t = term(5, 3, &format!("{FILLED}\x1b[2;3H\x1b[1J"));
    assert_eq!(lines(&t), ["", "   ij", "klmno"]);
}

#[test]
fn ed_2_erases_the_whole_screen_without_moving_the_cursor() {
    let t = term(5, 3, &format!("{FILLED}\x1b[2;3H\x1b[2J"));
    assert_eq!(lines(&t), ["", "", ""]);
    assert_eq!((t.cursor().row, t.cursor().col), (1, 2));
}

#[test]
fn el_erases_parts_of_the_cursor_row() {
    assert_eq!(
        lines(&term(5, 3, &format!("{FILLED}\x1b[2;3H\x1b[K")))[1],
        "fg"
    );
    assert_eq!(
        lines(&term(5, 3, &format!("{FILLED}\x1b[2;3H\x1b[1K")))[1],
        "   ij"
    );
    assert_eq!(
        lines(&term(5, 3, &format!("{FILLED}\x1b[2;3H\x1b[2K")))[1],
        ""
    );
}

#[test]
fn ech_blanks_cells_without_shifting() {
    let t = term(5, 1, "abcde\x1b[2G\x1b[2X");
    assert_eq!(lines(&t), ["a  de"]);
    let t = term(5, 1, "abcde\x1b[4G\x1b[9X");
    assert_eq!(lines(&t), ["abc"]);
}

#[test]
fn ich_shifts_the_rest_of_the_row_right() {
    let t = term(5, 1, "abcde\x1b[2G\x1b[2@");
    assert_eq!(lines(&t), ["a  bc"]);
}

#[test]
fn dch_pulls_the_rest_of_the_row_left() {
    let t = term(5, 1, "abcde\x1b[2G\x1b[2P");
    assert_eq!(lines(&t), ["ade"]);
    let t = term(5, 1, "abcde\x1b[2G\x1b[99P");
    assert_eq!(lines(&t), ["a"]);
}

#[test]
fn il_inserts_blank_rows_and_returns_to_column_zero() {
    let t = term(5, 3, &format!("{FILLED}\x1b[2;3H\x1b[L"));
    assert_eq!(lines(&t), ["abcde", "", "fghij"]);
    assert_eq!(t.cursor().col, 0);
}

#[test]
fn dl_removes_rows_and_pulls_the_rest_up() {
    let t = term(5, 3, &format!("{FILLED}\x1b[1;3H\x1b[2M"));
    assert_eq!(lines(&t), ["klmno", "", ""]);
    assert_eq!(t.cursor().col, 0);
}

#[test]
fn editing_never_leaves_half_of_a_wide_character() {
    // ECH on the spacer of 'い'.
    let t = term(6, 1, "あいう\x1b[4G\x1b[X");
    assert_eq!(lines(&t), ["あ  う"]);
    // ICH splits 'あ' and pushes half of 'う' past the edge.
    let t = term(6, 1, "あいう\x1b[2G\x1b[@");
    assert_eq!(lines(&t), ["   い"]);
    assert!(!t.cell(0, 5).flags.contains(Flags::WIDE));
    // DCH removes the head of 'あ'.
    let t = term(6, 1, "あいう\x1b[1G\x1b[P");
    assert_eq!(lines(&t), [" いう"]);
}

#[test]
fn el_to_the_end_clears_the_wrapped_flag() {
    let mut t = term(3, 2, "abcd");
    assert!(t.grid().row(0).wrapped());
    t.advance(b"\x1b[1;2H\x1b[K");
    assert!(!t.grid().row(0).wrapped());
}
