use batune::vt::{Cell, Color, Event, Flags, Term, Underline};

fn term(cols: usize, rows: usize, input: &str) -> Term {
    let mut term = Term::new(cols, rows);
    term.advance(input.as_bytes());
    term
}

/// The cell holding the first printed character after `sgr`.
fn styled(sgr: &str) -> Cell {
    term(10, 1, &format!("\x1b[{sgr}mx")).cell(0, 0).clone()
}

#[test]
fn attributes_are_set_and_combined() {
    let c = styled("1;3;7;9");
    assert_eq!(
        c.flags,
        Flags::BOLD | Flags::ITALIC | Flags::INVERSE | Flags::STRIKEOUT
    );
    assert_eq!(
        styled("2;5;8").flags,
        Flags::DIM | Flags::BLINK | Flags::HIDDEN
    );
}

#[test]
fn attributes_are_cleared_individually() {
    assert_eq!(styled("1;2;22").flags, Flags::empty());
    assert_eq!(styled("3;23;7;27;8;28;9;29;5;25").flags, Flags::empty());
    assert_eq!(styled("4;24").underline, Underline::None);
}

#[test]
fn reset_clears_everything_including_an_empty_sgr() {
    let blank = Cell::default();
    let reset = styled("1;4;31;42;58;5;1;0");
    assert_eq!(
        (reset.flags, reset.fg, reset.bg),
        (blank.flags, blank.fg, blank.bg)
    );
    assert_eq!(reset.underline_color, Color::Default);
    let t = term(10, 1, "\x1b[1;31m\x1b[mx");
    assert_eq!(t.cell(0, 0).flags, Flags::empty());
    assert_eq!(t.cell(0, 0).fg, Color::Default);
}

#[test]
fn ansi_and_bright_colors_map_to_palette_indices() {
    assert_eq!(styled("31").fg, Color::Indexed(1));
    assert_eq!(styled("47").bg, Color::Indexed(7));
    assert_eq!(styled("92").fg, Color::Indexed(10));
    assert_eq!(styled("107").bg, Color::Indexed(15));
    assert_eq!(styled("31;39").fg, Color::Default);
    assert_eq!(styled("41;49").bg, Color::Default);
}

#[test]
fn indexed_colors_in_semicolon_and_colon_forms() {
    assert_eq!(styled("38;5;208").fg, Color::Indexed(208));
    assert_eq!(styled("48:5:17").bg, Color::Indexed(17));
}

#[test]
fn truecolor_in_semicolon_and_colon_forms() {
    assert_eq!(styled("38;2;1;2;3").fg, Color::Rgb(1, 2, 3));
    assert_eq!(styled("48:2::10:20:30").bg, Color::Rgb(10, 20, 30));
    assert_eq!(styled("38:2:10:20:30").fg, Color::Rgb(10, 20, 30));
}

#[test]
fn parameters_after_an_extended_color_still_apply() {
    let c = styled("38;2;1;2;3;1;48;5;4");
    assert_eq!(c.fg, Color::Rgb(1, 2, 3));
    assert!(c.flags.contains(Flags::BOLD));
    assert_eq!(c.bg, Color::Indexed(4));
}

#[test]
fn out_of_range_color_components_are_ignored() {
    assert_eq!(styled("38;5;300").fg, Color::Default);
    assert_eq!(styled("38;2;1;2;999").fg, Color::Default);
}

#[test]
fn underline_styles_and_color() {
    assert_eq!(styled("4").underline, Underline::Single);
    assert_eq!(styled("21").underline, Underline::Double);
    assert_eq!(styled("4:2").underline, Underline::Double);
    assert_eq!(styled("4:3").underline, Underline::Curly);
    assert_eq!(styled("4:4").underline, Underline::Dotted);
    assert_eq!(styled("4:5").underline, Underline::Dashed);
    assert_eq!(styled("4;4:0").underline, Underline::None);
    assert_eq!(styled("58:2::1:2:3").underline_color, Color::Rgb(1, 2, 3));
    assert_eq!(styled("58;5;9;59").underline_color, Color::Default);
}

#[test]
fn the_pen_applies_to_both_halves_of_a_wide_character() {
    let t = term(10, 1, "\x1b[31mあ");
    assert_eq!(t.cell(0, 0).fg, Color::Indexed(1));
    assert_eq!(t.cell(0, 1).fg, Color::Indexed(1));
}

#[test]
fn osc_0_and_2_set_the_title_and_emit_an_event() {
    let mut t = term(10, 1, "\x1b]0;one\x07\x1b]2;a;b\x1b\\");
    assert_eq!(t.title(), "a;b");
    assert_eq!(
        t.drain_events().collect::<Vec<_>>(),
        [Event::Title("one".into()), Event::Title("a;b".into())]
    );
}

#[test]
fn erased_cells_take_the_current_background() {
    let t = term(5, 2, "\x1b[41m\x1b[2J");
    assert_eq!(t.cell(1, 4).bg, Color::Indexed(1));
    assert_eq!(t.cell(1, 4).c, ' ');
}

#[test]
fn an_over_long_title_is_dropped_and_the_previous_one_kept() {
    let long = "x".repeat(5000);
    let mut t = term(10, 1, &format!("\x1b]0;old\x07\x1b]0;{long}\x07"));
    assert_eq!(t.title(), "old");
    assert_eq!(
        t.drain_events().collect::<Vec<_>>(),
        [Event::Title("old".into())]
    );
}

#[test]
fn a_title_of_4096_bytes_is_kept() {
    let title = format!("{};{}", "a".repeat(2000), "b".repeat(2095));
    let t = term(10, 1, &format!("\x1b]2;{title}\x07"));
    assert_eq!(t.title(), title);
}
