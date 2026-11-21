use batune::vt::Term;

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

const FILLED: &str = "a\r\nb\r\nc\r\nd\r\ne";

#[test]
fn decstbm_moves_the_cursor_home() {
    let t = term(5, 5, &format!("{FILLED}\x1b[2;4r"));
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn lf_at_the_bottom_margin_scrolls_only_the_region() {
    let t = term(5, 5, &format!("{FILLED}\x1b[2;4r\x1b[4;1H\nx"));
    assert_eq!(lines(&t), ["a", "c", "d", "x", "e"]);
}

#[test]
fn lf_below_the_region_moves_down_without_scrolling() {
    let t = term(5, 5, &format!("{FILLED}\x1b[1;3r\x1b[4;1H\n\n\nx"));
    assert_eq!(lines(&t), ["a", "b", "c", "d", "x"]);
}

#[test]
fn ri_at_the_top_margin_scrolls_the_region_down() {
    let t = term(5, 5, &format!("{FILLED}\x1b[2;4r\x1b[2;1H\x1bMx"));
    assert_eq!(lines(&t), ["a", "x", "b", "c", "e"]);
}

#[test]
fn ind_and_nel_move_down_and_nel_returns_to_the_first_column() {
    let t = term(5, 5, "ab\x1bDc\x1bEd");
    assert_eq!(lines(&t), ["ab", "  c", "d", "", ""]);
}

#[test]
fn su_and_sd_scroll_the_region_and_keep_the_cursor() {
    let t = term(5, 5, &format!("{FILLED}\x1b[2;4r\x1b[3;2H\x1b[2S"));
    assert_eq!(lines(&t), ["a", "d", "", "", "e"]);
    assert_eq!(pos(&t), (2, 1));

    let t = term(5, 5, &format!("{FILLED}\x1b[2;4r\x1b[T"));
    assert_eq!(lines(&t), ["a", "", "b", "c", "e"]);
}

#[test]
fn scrolled_in_rows_take_the_current_background() {
    let t = term(5, 3, "\x1b[44m\x1b[S");
    assert_eq!(t.cell(2, 0).bg, batune::vt::Color::Indexed(4));
}

#[test]
fn a_region_of_one_row_is_ignored() {
    let t = term(5, 5, &format!("{FILLED}\x1b[3;3r\x1b[5;1H\nx"));
    assert_eq!(lines(&t), ["b", "c", "d", "e", "x"]);
}

#[test]
fn decstbm_without_parameters_resets_to_the_whole_screen() {
    let t = term(5, 5, &format!("{FILLED}\x1b[2;3r\x1b[r\x1b[5;1H\nx"));
    assert_eq!(lines(&t), ["b", "c", "d", "e", "x"]);
}

#[test]
fn origin_mode_addresses_rows_from_the_top_margin_and_clamps_to_the_region() {
    let t = term(5, 5, "\x1b[2;4r\x1b[?6h");
    assert_eq!(pos(&t), (1, 0));
    let t = term(5, 5, "\x1b[2;4r\x1b[?6h\x1b[2;3H");
    assert_eq!(pos(&t), (2, 2));
    let t = term(5, 5, "\x1b[2;4r\x1b[?6h\x1b[9;1H");
    assert_eq!(pos(&t), (3, 0));
    let t = term(5, 5, "\x1b[2;4r\x1b[?6h\x1b[?6l");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn il_and_dl_stop_at_the_bottom_margin() {
    let t = term(5, 5, &format!("{FILLED}\x1b[1;3r\x1b[2;1H\x1b[L"));
    assert_eq!(lines(&t), ["a", "", "b", "d", "e"]);
    let t = term(5, 5, &format!("{FILLED}\x1b[1;3r\x1b[1;1H\x1b[M"));
    assert_eq!(lines(&t), ["b", "c", "", "d", "e"]);
}

#[test]
fn resize_resets_the_region() {
    let mut t = term(5, 5, "\x1b[2;3r");
    t.resize(5, 4);
    t.advance(b"\x1b[4;1H\nx");
    assert_eq!(lines(&t), ["", "", "", "x"]);
}
