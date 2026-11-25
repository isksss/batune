use super::{MAX_OSC_BYTES, MAX_OSC_PARAMS, OscScan, Term};

fn screen(t: &Term) -> Vec<String> {
    (0..t.rows()).map(|r| t.grid().row(r).text()).collect()
}

#[test]
fn an_unterminated_osc_is_cut_off_at_the_limit_and_its_rest_dropped() {
    let mut t = Term::new(10, 2);
    t.advance(b"\x1b]0;");
    t.advance(&vec![b'a'; MAX_OSC_BYTES + 100]);
    // vte was handed CAN and no longer buffers; the rest is dropped here.
    assert!(matches!(t.osc, OscScan::Discard));
    t.advance(b"\x07ok\x1b]2;title\x07");
    assert_eq!(screen(&t)[0], "ok");
    assert_eq!(t.title(), "title");
}

#[test]
fn an_osc_with_more_params_than_vte_keeps_is_dropped_not_cut() {
    let mut t = Term::new(10, 2);
    let at_limit = format!("\x1b]2;{}x\x07", "a;".repeat(MAX_OSC_PARAMS - 2));
    t.advance(at_limit.as_bytes());
    assert_eq!(t.title(), format!("{}x", "a;".repeat(MAX_OSC_PARAMS - 2)));
    let over = format!("\x1b]2;{}x\x07", "b;".repeat(MAX_OSC_PARAMS - 1));
    t.advance(over.as_bytes());
    assert_eq!(t.title(), format!("{}x", "a;".repeat(MAX_OSC_PARAMS - 2)));
    // The next OSC is not affected.
    t.advance(b"\x1b]2;next\x07");
    assert_eq!(t.title(), "next");
}

#[test]
fn the_scrollback_limit_set_on_the_alternate_screen_goes_to_the_primary_one() {
    let mut t = Term::new(10, 2);
    t.advance(b"\x1b[?1049h");
    t.set_scrollback_limit(1);
    t.advance(b"1\r\n2\r\n3\r\n4");
    assert_eq!(t.scrollback_len(), 0);
    t.advance(b"\x1b[?1049l1\r\n2\r\n3\r\n4");
    assert_eq!(t.scrollback_len(), 1);
}

#[test]
fn the_oldest_kitty_images_are_freed_past_the_count_limit() {
    let mut t = Term::new(10, 2);
    for id in 1..=super::graphics::MAX_STORED_IMAGES + 1 {
        t.advance(format!("\x1b_Ga=t,q=2,f=24,s=1,v=1,i={id};AAAA\x1b\\").as_bytes());
    }
    t.advance(b"\x1b_Ga=p,i=1\x1b\\\x1b_Ga=p,i=2\x1b\\");
    let reply = String::from_utf8(t.take_responses()).unwrap();
    assert!(reply.starts_with("\x1b_Gi=1;ENOENT"), "{reply:?}");
    assert!(reply.ends_with("\x1b_Gi=2;OK\x1b\\"), "{reply:?}");
}
