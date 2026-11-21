use batune::vt::{CursorShape, CursorStyle, Modes, MouseEncoding, MouseTracking, Term};

fn term(input: &str) -> Term {
    let mut term = Term::new(10, 3);
    term.advance(input.as_bytes());
    term
}

fn modes(input: &str) -> Modes {
    *term(input).modes()
}

#[test]
fn defaults_match_a_freshly_started_xterm() {
    let m = modes("");
    assert!(m.autowrap && m.show_cursor);
    assert!(!m.cursor_keys_app && !m.keypad_app && !m.bracketed_paste);
    assert!(!m.focus_reporting && !m.synchronized_output && !m.insert);
    assert_eq!(m.mouse_tracking, MouseTracking::Off);
    assert_eq!(m.mouse_encoding, MouseEncoding::Default);
}

#[test]
fn private_modes_are_set_and_reset() {
    let m = modes("\x1b[?1;25;2004;1004;2026;5h");
    assert!(m.cursor_keys_app && m.bracketed_paste && m.focus_reporting);
    assert!(m.synchronized_output && m.reverse_video);
    let m = modes("\x1b[?1;25;2004;1004;2026;5h\x1b[?1;25;2004;1004;2026;5l");
    assert!(!m.cursor_keys_app && !m.show_cursor && !m.bracketed_paste);
    assert!(!m.focus_reporting && !m.synchronized_output && !m.reverse_video);
}

#[test]
fn keypad_application_mode_follows_deckpam_and_deckpnm() {
    assert!(modes("\x1b=").keypad_app);
    assert!(!modes("\x1b=\x1b>").keypad_app);
}

#[test]
fn the_latest_mouse_tracking_mode_wins() {
    assert_eq!(modes("\x1b[?9h").mouse_tracking, MouseTracking::X10);
    assert_eq!(modes("\x1b[?1000h").mouse_tracking, MouseTracking::Normal);
    assert_eq!(
        modes("\x1b[?1000h\x1b[?1002h").mouse_tracking,
        MouseTracking::ButtonEvent
    );
    assert_eq!(modes("\x1b[?1003h").mouse_tracking, MouseTracking::AnyEvent);
}

#[test]
fn resetting_a_mouse_mode_turns_tracking_off_only_if_it_is_the_active_one() {
    assert_eq!(
        modes("\x1b[?1002h\x1b[?1002l").mouse_tracking,
        MouseTracking::Off
    );
    assert_eq!(
        modes("\x1b[?1002h\x1b[?1000l").mouse_tracking,
        MouseTracking::ButtonEvent
    );
}

#[test]
fn mouse_encoding_follows_1005_and_1006() {
    assert_eq!(modes("\x1b[?1006h").mouse_encoding, MouseEncoding::Sgr);
    assert_eq!(modes("\x1b[?1005h").mouse_encoding, MouseEncoding::Utf8);
    assert_eq!(
        modes("\x1b[?1006h\x1b[?1006l").mouse_encoding,
        MouseEncoding::Default
    );
}

#[test]
fn insert_mode_pushes_the_rest_of_the_row_right() {
    let t = term("abcd\x1b[1;2H\x1b[4hXY\x1b[4lZ");
    assert_eq!(t.grid().row(0).text(), "aXYZcd");
    assert!(!t.modes().insert);
}

#[test]
fn linefeed_newline_mode_returns_to_the_first_column() {
    let t = term("\x1b[20hab\ncd");
    assert_eq!(t.grid().row(1).text(), "cd");
    assert!(t.modes().linefeed_newline);
}

#[test]
fn decscusr_sets_shape_and_blinking() {
    let style = |ps: &str| term(&format!("\x1b[{ps} q")).cursor_style();
    let s = |shape, blinking| CursorStyle { shape, blinking };
    assert_eq!(style("0"), s(CursorShape::Block, true));
    assert_eq!(style("2"), s(CursorShape::Block, false));
    assert_eq!(style("3"), s(CursorShape::Underline, true));
    assert_eq!(style("4"), s(CursorShape::Underline, false));
    assert_eq!(style("5"), s(CursorShape::Bar, true));
    assert_eq!(style("6"), s(CursorShape::Bar, false));
    assert_eq!(
        term("\x1b[6 q\x1b[?12h").cursor_style(),
        s(CursorShape::Bar, true)
    );
}

#[test]
fn decstr_resets_keyboard_modes_but_keeps_mouse_and_paste_modes() {
    let m = modes("\x1b[?1;1000;1006;2004h\x1b[?25l\x1b=\x1b[4h\x1b[!p");
    assert!(!m.cursor_keys_app && !m.keypad_app && !m.insert && m.show_cursor);
    assert_eq!(m.mouse_tracking, MouseTracking::Normal);
    assert_eq!(m.mouse_encoding, MouseEncoding::Sgr);
    assert!(m.bracketed_paste);
}

#[test]
fn ris_resets_every_mode() {
    assert_eq!(
        modes("\x1b[?1;1000;1006;2004h\x1b[?25l\x1b[5 q\x1bc"),
        Modes::default()
    );
    assert_eq!(term("\x1b[5 q\x1bc").cursor_style(), CursorStyle::default());
}

#[test]
fn grapheme_cluster_mode_is_on_by_default_and_can_be_turned_off() {
    assert!(modes("").grapheme_cluster);
    assert!(!modes("\x1b[?2027l").grapheme_cluster);
    assert!(modes("\x1b[?2027l\x1b[?2027h").grapheme_cluster);
    assert!(modes("\x1b[?2027l\x1b[!p").grapheme_cluster);
    assert!(modes("\x1b[?2027l\x1bc").grapheme_cluster);
}
