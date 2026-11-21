use batune::vt::{KeyboardFlags, Term};

fn term(input: &str) -> Term {
    let mut term = Term::new(10, 5);
    term.advance(input.as_bytes());
    term
}

fn flags(input: &str) -> u8 {
    term(input).keyboard_flags().bits()
}

fn reply(input: &str) -> String {
    String::from_utf8(term(input).take_responses()).unwrap()
}

#[test]
fn flags_start_off_and_a_query_reports_them() {
    assert_eq!(flags(""), 0);
    assert_eq!(reply("\x1b[?u"), "\x1b[?0u");
    assert_eq!(reply("\x1b[>1u\x1b[?u"), "\x1b[?1u");
}

#[test]
fn only_supported_flags_are_kept() {
    // Claude Code pushes 5 (disambiguate + alternate keys).
    assert_eq!(flags("\x1b[>5u"), 1);
    assert_eq!(reply("\x1b[>5u\x1b[?u"), "\x1b[?1u");
    assert_eq!(flags("\x1b[>8u"), 0);
    assert!(
        term("\x1b[>31u")
            .keyboard_flags()
            .contains(KeyboardFlags::DISAMBIGUATE)
    );
}

#[test]
fn pop_returns_to_the_flags_before_the_push() {
    assert_eq!(flags("\x1b[>1u\x1b[>0u"), 0);
    assert_eq!(flags("\x1b[>1u\x1b[>0u\x1b[<u"), 1);
    assert_eq!(flags("\x1b[>1u\x1b[>0u\x1b[<2u"), 0);
}

#[test]
fn popping_more_than_was_pushed_resets_the_flags() {
    assert_eq!(flags("\x1b[>1u\x1b[<5u"), 0);
    assert_eq!(flags("\x1b[<u"), 0);
}

#[test]
fn the_oldest_entries_are_dropped_past_the_stack_limit() {
    // The first push saves "off"; once it falls off the bottom, popping
    // everything still ends at "off".
    let input = format!("{}{}", "\x1b[>1u".repeat(40), "\x1b[<u".repeat(39));
    assert_eq!(flags(&input), 0);
}

#[test]
fn set_replaces_adds_or_removes_flags() {
    assert_eq!(flags("\x1b[=1u"), 1);
    assert_eq!(flags("\x1b[=1;1u"), 1);
    assert_eq!(flags("\x1b[=1;2u"), 1);
    assert_eq!(flags("\x1b[=1u\x1b[=1;3u"), 0);
    assert_eq!(flags("\x1b[=1u\x1b[=0;1u"), 0);
}

#[test]
fn each_screen_has_its_own_flags() {
    assert_eq!(flags("\x1b[>1u\x1b[?1049h"), 0);
    assert_eq!(flags("\x1b[?1049h\x1b[>1u"), 1);
    assert_eq!(flags("\x1b[?1049h\x1b[>1u\x1b[?1049l"), 0);
    assert_eq!(flags("\x1b[>1u\x1b[?1049h\x1b[?1049l"), 1);
}

#[test]
fn full_reset_turns_the_flags_off() {
    assert_eq!(flags("\x1b[>1u\x1bc"), 0);
}

#[test]
fn csi_u_without_a_prefix_still_restores_the_cursor() {
    assert_eq!(reply("\x1b[2;3H\x1b[s\x1b[H\x1b[u\x1b[6n"), "\x1b[2;3R");
}
