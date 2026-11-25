use batune::vt::{Event, Hyperlink, MAX_CLIPBOARD_BYTES, ReportedColors, Term};

fn term(input: &str) -> Term {
    let mut term = Term::new(10, 5);
    term.advance(input.as_bytes());
    term
}

fn reply(input: &str) -> String {
    String::from_utf8(term(input).take_responses()).unwrap()
}

fn events(input: &str) -> Vec<Event> {
    term(input).drain_events().collect()
}

#[test]
fn da1_and_da2_identify_the_terminal() {
    assert_eq!(reply("\x1b[c"), "\x1b[?62;4;22c");
    assert_eq!(reply("\x1b[0c"), "\x1b[?62;4;22c");
    assert_eq!(reply("\x1b[>c"), "\x1b[>0;10;1c");
}

#[test]
fn dsr_5_reports_ok_and_dsr_6_reports_the_cursor_position() {
    assert_eq!(reply("\x1b[5n"), "\x1b[0n");
    assert_eq!(reply("\x1b[3;4H\x1b[6n"), "\x1b[3;4R");
    assert_eq!(reply("\x1b[3;4H\x1b[?6n"), "\x1b[?3;4R");
}

#[test]
fn cursor_position_reports_are_relative_to_the_region_under_origin_mode() {
    assert_eq!(reply("\x1b[2;4r\x1b[?6h\x1b[2;1H\x1b[6n"), "\x1b[2;1R");
}

#[test]
fn decrc_under_origin_mode_puts_the_cursor_inside_the_region_set_since_the_save() {
    // Saved above the region, then restored: the top margin.
    assert_eq!(reply("\x1b[?6h\x1b7\x1b[3;5r\x1b8\x1b[6n"), "\x1b[1;1R");
    // Saved below the region: the bottom margin.
    assert_eq!(
        reply("\x1b[?6h\x1b[5;1H\x1b7\x1b[1;3r\x1b8\x1b[6n"),
        "\x1b[3;1R"
    );
}

#[test]
fn responses_are_taken_once_in_order() {
    let mut t = term("\x1b[5n\x1b[6n");
    assert_eq!(t.take_responses(), b"\x1b[0n\x1b[1;1R");
    assert!(t.take_responses().is_empty());
}

#[test]
fn decrqm_reports_whether_a_mode_is_set() {
    assert_eq!(reply("\x1b[?2026$p"), "\x1b[?2026;2$y");
    assert_eq!(reply("\x1b[?2026h\x1b[?2026$p"), "\x1b[?2026;1$y");
    assert_eq!(reply("\x1b[?1006h\x1b[?1006$p"), "\x1b[?1006;1$y");
    assert_eq!(reply("\x1b[?2027$p"), "\x1b[?2027;1$y");
    assert_eq!(reply("\x1b[?2027l\x1b[?2027$p"), "\x1b[?2027;2$y");
    assert_eq!(reply("\x1b[?9999$p"), "\x1b[?9999;0$y");
    assert_eq!(reply("\x1b[4h\x1b[4$p"), "\x1b[4;1$y");
}

#[test]
fn osc_8_links_the_cells_printed_until_it_is_closed() {
    let t = term("a\x1b]8;id=x;http://e.com/?a=1;b\x1b\\bc\x1b]8;;\x1b\\d");
    let link = Hyperlink {
        id: Some("x".into()),
        uri: "http://e.com/?a=1;b".into(),
    };
    assert_eq!(t.cell(0, 0).hyperlink(), None);
    assert_eq!(t.cell(0, 1).hyperlink(), Some(&link));
    assert_eq!(t.cell(0, 2).hyperlink(), Some(&link));
    assert_eq!(t.cell(0, 3).hyperlink(), None);
}

#[test]
fn a_link_survives_sgr_reset_but_not_erasure() {
    let t = term("\x1b]8;;http://e.com\x07\x1b[1ma\x1b[0mb\x1b[1;1H\x1b[X");
    assert_eq!(t.cell(0, 0).hyperlink(), None);
    assert_eq!(t.cell(0, 1).hyperlink().unwrap().uri, "http://e.com");
}

#[test]
fn osc_52_store_is_decoded_into_an_event() {
    assert_eq!(
        events("\x1b]52;c;5pel5pys6Kqe\x07"),
        [Event::ClipboardStore {
            selection: "c".into(),
            text: "日本語".into()
        }]
    );
}

#[test]
fn osc_52_with_invalid_base64_is_ignored() {
    assert_eq!(events("\x1b]52;c;!!\x07"), []);
}

#[test]
fn osc_52_query_is_reported_but_never_answered() {
    let mut t = term("\x1b]52;c;?\x07");
    assert_eq!(
        t.drain_events().collect::<Vec<_>>(),
        [Event::ClipboardLoad {
            selection: "c".into()
        }]
    );
    assert_eq!(t.take_responses(), b"");
}

#[test]
fn osc_10_11_12_queries_report_the_configured_colors() {
    let mut t = Term::new(10, 5);
    t.set_reported_colors(ReportedColors {
        foreground: (0x11, 0x22, 0x33),
        background: (0xff, 0xee, 0xdd),
        cursor: (1, 2, 3),
    });
    t.advance(b"\x1b]11;?\x07\x1b]10;?;?\x1b\\\x1b]12;?\x07\x1b]10;red\x07");
    assert_eq!(
        String::from_utf8(t.take_responses()).unwrap(),
        "\x1b]11;rgb:ffff/eeee/dddd\x07\
         \x1b]10;rgb:1111/2222/3333\x1b\\\
         \x1b]11;rgb:ffff/eeee/dddd\x1b\\\
         \x1b]12;rgb:0101/0202/0303\x07"
    );
}

#[test]
fn osc_52_store_over_1_mib_is_dropped_whole() {
    let data = "QUFB".repeat((2 << 20) / 4);
    assert_eq!(events(&format!("\x1b]52;c;{data}\x07")), []);
}

#[test]
fn osc_52_store_of_1_mib_is_kept() {
    let data = "QUFB".repeat((1 << 20) / 3);
    let events = events(&format!("\x1b]52;c;{data}\x07"));
    let [Event::ClipboardStore { text, .. }] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(text.len(), (1 << 20) / 3 * 3);
}

#[test]
fn osc_52_store_is_limited_by_its_decoded_length() {
    let full = "QUFB".repeat(MAX_CLIPBOARD_BYTES / 3);
    let kept = events(&format!("\x1b]52;c;{full}QQ==\x07"));
    let [Event::ClipboardStore { text, .. }] = kept.as_slice() else {
        panic!("{kept:?}");
    };
    assert_eq!(text.len(), MAX_CLIPBOARD_BYTES);
    assert_eq!(events(&format!("\x1b]52;c;{full}QUE=\x07")), []);
}
