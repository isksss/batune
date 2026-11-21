//! Encodes key presses into the bytes a terminal sends to the application
//! (xterm conventions).

use winit::event::MouseScrollDelta;
use winit::keyboard::{Key, ModifiersState, NamedKey};

/// Terminal modes that change what keys send.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyModes {
    /// DECCKM: arrows, Home and End send `ESC O x` instead of `ESC [ x`.
    pub cursor_keys_app: bool,
    /// LNM: Return sends CR LF.
    pub linefeed_newline: bool,
    /// DECKPAM: the numeric keypad sends `ESC O x` instead of its text.
    pub keypad_app: bool,
    /// The kitty keyboard protocol's disambiguate flag: keys that legacy
    /// encoding cannot tell apart (Esc, Shift+Enter and Enter, Ctrl+I and
    /// Tab, ...) are sent as `CSI code ; mods u`.
    pub disambiguate: bool,
}

/// Default lines per notch of a wheel that reports whole lines, as in xterm
/// and most other terminals.
pub const WHEEL_LINES: u8 = 3;

/// Turns wheel events into whole lines.
#[derive(Debug, Default)]
pub struct Wheel {
    /// Touchpads report pixels in small steps; the part of a line not yet
    /// scrolled is kept for the next event instead of being dropped.
    pending: f64,
}

impl Wheel {
    /// Lines to scroll, `per_notch` for each notch of a wheel that reports
    /// whole lines: positive moves back into the scrollback.
    pub fn lines(&mut self, delta: MouseScrollDelta, cell_height: f64, per_notch: f64) -> isize {
        self.pending += match delta {
            MouseScrollDelta::LineDelta(_, y) => f64::from(y) * per_notch,
            MouseScrollDelta::PixelDelta(p) => p.y / cell_height.max(1.0),
        };
        let whole = self.pending.trunc();
        self.pending -= whole;
        whole as isize
    }
}

/// Arrow keys standing in for the wheel on the alternate screen, which has
/// no scrollback: pagers and editors scroll with them instead (xterm's
/// `alternateScroll`).
pub fn wheel_arrows(lines: isize, modes: KeyModes) -> Vec<u8> {
    let key = cursor_key(if lines > 0 { b'A' } else { b'B' }, 1, modes);
    key.repeat(lines.unsigned_abs())
}

/// Bytes to send for pasted text.
///
/// With bracketed paste the text is wrapped in `ESC [200~` / `ESC [201~`
/// and every ESC is dropped, so the text cannot end the bracket early and
/// run the rest as typed commands. Without it, line breaks become CR, the
/// byte the Return key sends; LF alone would not submit a line in programs
/// that read the terminal in raw mode.
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let mut out = b"\x1b[200~".to_vec();
        out.extend(text.bytes().filter(|&b| b != 0x1b));
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

/// Bytes for a key on the numeric keypad in application keypad mode, or
/// `None` when the key should be encoded like its main-keyboard twin
/// (normal mode, or a modifier is held).
pub fn encode_keypad(key: &Key, mods: ModifiersState, modes: KeyModes) -> Option<Vec<u8>> {
    if !modes.keypad_app || !mods.is_empty() {
        return None;
    }
    let final_byte = match key {
        Key::Named(NamedKey::Enter) => b'M',
        Key::Character(s) => match single_char(s)? {
            // `ESC O p` .. `ESC O y` for 0-9, as in xterm's VT220 keypad.
            c @ '0'..='9' => b'p' + (c as u8 - b'0'),
            '.' => b'n',
            '+' => b'k',
            '-' => b'm',
            '*' => b'j',
            '/' => b'o',
            _ => return None,
        },
        _ => return None,
    };
    Some(vec![0x1b, b'O', final_byte])
}

/// Bytes for a pressed key, or `None` when the key sends nothing (bare
/// modifiers, keys the terminal does not encode). `text` is the text the
/// platform produced for the key, if any. With `modes.disambiguate`, `key`
/// should be the key without modifiers (`a` for Ctrl+Shift+A, `1` for
/// Ctrl+!), which is the code the protocol reports.
pub fn encode(
    key: &Key,
    text: Option<&str>,
    mods: ModifiersState,
    modes: KeyModes,
) -> Option<Vec<u8>> {
    if modes.disambiguate
        && let Some(bytes) = disambiguated(key, mods)
    {
        return Some(bytes);
    }
    let alt = mods.alt_key();
    let ctrl = mods.control_key();
    // xterm's modifier parameter: 1 + Shift(1) + Alt(2) + Ctrl(4).
    let param = 1 + u8::from(mods.shift_key()) + 2 * u8::from(alt) + 4 * u8::from(ctrl);

    let bytes = match key {
        Key::Named(named) => match named_key(*named, ctrl, mods.shift_key(), param, modes) {
            Some(Encoded::Plain(bytes)) => with_alt(alt, bytes),
            Some(Encoded::Sequence(bytes)) => bytes,
            None => with_alt(alt, text?.as_bytes().to_vec()),
        },
        Key::Character(s) => {
            let control = ctrl
                .then(|| single_char(s).and_then(control_byte))
                .flatten();
            match control {
                Some(b) => with_alt(alt, vec![b]),
                // With Alt the platform text may already be altered (or
                // missing), while the logical key is the plain character.
                None if alt => with_alt(true, s.as_bytes().to_vec()),
                None => text.unwrap_or(s).as_bytes().to_vec(),
            }
        }
        _ => text?.as_bytes().to_vec(),
    };
    (!bytes.is_empty()).then_some(bytes)
}

/// `CSI code ; mods u` for the keys the disambiguate flag changes, or
/// `None` to keep the legacy encoding (arrows, function keys, plain text).
fn disambiguated(key: &Key, mods: ModifiersState) -> Option<Vec<u8>> {
    let chord = mods.control_key() || mods.alt_key() || mods.super_key();
    // The protocol's modifier parameter adds Super(8) to xterm's.
    let param = 1
        + u8::from(mods.shift_key())
        + 2 * u8::from(mods.alt_key())
        + 4 * u8::from(mods.control_key())
        + 8 * u8::from(mods.super_key());
    let code = match key {
        Key::Named(NamedKey::Escape) => 27,
        // Unmodified, these keep their legacy bytes so that a shell still
        // works after a program that left the flag on has crashed.
        Key::Named(NamedKey::Enter) if param > 1 => 13,
        Key::Named(NamedKey::Tab) if param > 1 => 9,
        Key::Named(NamedKey::Backspace) if param > 1 => 127,
        // Shift alone only changes the text typed, which is sent as is.
        Key::Named(NamedKey::Space) if chord => 32,
        Key::Character(s) if chord => {
            let c = single_char(s)?;
            let mut lower = c.to_lowercase();
            match (lower.next(), lower.next()) {
                (Some(l), None) => u32::from(l),
                _ => u32::from(c),
            }
        }
        _ => return None,
    };
    Some(if param > 1 {
        format!("\x1b[{code};{param}u").into_bytes()
    } else {
        format!("\x1b[{code}u").into_bytes()
    })
}

enum Encoded {
    /// Gets an ESC prefix when Alt is held.
    Plain(Vec<u8>),
    /// Already carries the modifiers in its parameters.
    Sequence(Vec<u8>),
}

fn named_key(
    key: NamedKey,
    ctrl: bool,
    shift: bool,
    param: u8,
    modes: KeyModes,
) -> Option<Encoded> {
    use Encoded::{Plain, Sequence};
    let plain = |b: &[u8]| Some(Plain(b.to_vec()));
    match key {
        NamedKey::Enter if modes.linefeed_newline => plain(b"\r\n"),
        NamedKey::Enter => plain(b"\r"),
        NamedKey::Backspace if ctrl => plain(b"\x08"),
        NamedKey::Backspace => plain(b"\x7f"),
        NamedKey::Tab if shift => Some(Sequence(b"\x1b[Z".to_vec())),
        NamedKey::Tab => plain(b"\t"),
        NamedKey::Escape => plain(b"\x1b"),
        NamedKey::Space if ctrl => plain(b"\0"),
        NamedKey::Space => plain(b" "),
        NamedKey::ArrowUp => Some(Sequence(cursor_key(b'A', param, modes))),
        NamedKey::ArrowDown => Some(Sequence(cursor_key(b'B', param, modes))),
        NamedKey::ArrowRight => Some(Sequence(cursor_key(b'C', param, modes))),
        NamedKey::ArrowLeft => Some(Sequence(cursor_key(b'D', param, modes))),
        NamedKey::Home => Some(Sequence(cursor_key(b'H', param, modes))),
        NamedKey::End => Some(Sequence(cursor_key(b'F', param, modes))),
        NamedKey::Insert => Some(Sequence(tilde_key(2, param))),
        NamedKey::Delete => Some(Sequence(tilde_key(3, param))),
        NamedKey::PageUp => Some(Sequence(tilde_key(5, param))),
        NamedKey::PageDown => Some(Sequence(tilde_key(6, param))),
        NamedKey::F1 => Some(Sequence(ss3_key(b'P', param))),
        NamedKey::F2 => Some(Sequence(ss3_key(b'Q', param))),
        NamedKey::F3 => Some(Sequence(ss3_key(b'R', param))),
        NamedKey::F4 => Some(Sequence(ss3_key(b'S', param))),
        NamedKey::F5 => Some(Sequence(tilde_key(15, param))),
        NamedKey::F6 => Some(Sequence(tilde_key(17, param))),
        NamedKey::F7 => Some(Sequence(tilde_key(18, param))),
        NamedKey::F8 => Some(Sequence(tilde_key(19, param))),
        NamedKey::F9 => Some(Sequence(tilde_key(20, param))),
        NamedKey::F10 => Some(Sequence(tilde_key(21, param))),
        NamedKey::F11 => Some(Sequence(tilde_key(23, param))),
        NamedKey::F12 => Some(Sequence(tilde_key(24, param))),
        _ => None,
    }
}

/// Arrows, Home, End: `CSI x`, `SS3 x` in application mode, and
/// `CSI 1 ; m x` with modifiers (which ignores the mode, like xterm).
fn cursor_key(final_byte: u8, param: u8, modes: KeyModes) -> Vec<u8> {
    if param > 1 {
        format!("\x1b[1;{param}{}", char::from(final_byte)).into_bytes()
    } else if modes.cursor_keys_app {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

/// F1-F4: `SS3 x`, or `CSI 1 ; m x` with modifiers.
fn ss3_key(final_byte: u8, param: u8) -> Vec<u8> {
    if param > 1 {
        format!("\x1b[1;{param}{}", char::from(final_byte)).into_bytes()
    } else {
        vec![0x1b, b'O', final_byte]
    }
}

/// `CSI n ~`, or `CSI n ; m ~` with modifiers.
fn tilde_key(n: u8, param: u8) -> Vec<u8> {
    if param > 1 {
        format!("\x1b[{n};{param}~").into_bytes()
    } else {
        format!("\x1b[{n}~").into_bytes()
    }
}

fn with_alt(alt: bool, mut bytes: Vec<u8>) -> Vec<u8> {
    if alt {
        bytes.insert(0, 0x1b);
    }
    bytes
}

fn single_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

/// The C0 control a Ctrl+key chord produces, including the xterm aliases
/// on the digit row (Ctrl+2 = NUL ... Ctrl+8 = DEL).
fn control_byte(c: char) -> Option<u8> {
    Some(match c {
        'a'..='z' | 'A'..='Z' => c.to_ascii_uppercase() as u8 & 0x1f,
        '@' | ' ' | '2' => 0x00,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '~' | '6' => 0x1e,
        '_' | '-' | '/' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: ModifiersState = ModifiersState::empty();
    const SHIFT: ModifiersState = ModifiersState::SHIFT;
    const CTRL: ModifiersState = ModifiersState::CONTROL;
    const ALT: ModifiersState = ModifiersState::ALT;

    fn named(key: NamedKey, mods: ModifiersState) -> Vec<u8> {
        encode(&Key::Named(key), None, mods, KeyModes::default()).unwrap()
    }

    fn named_in(key: NamedKey, modes: KeyModes) -> Vec<u8> {
        encode(&Key::Named(key), None, NONE, modes).unwrap()
    }

    fn chr(s: &str, text: Option<&str>, mods: ModifiersState) -> Option<Vec<u8>> {
        encode(&Key::Character(s.into()), text, mods, KeyModes::default())
    }

    const SUPER: ModifiersState = ModifiersState::SUPER;

    const KITTY: KeyModes = KeyModes {
        cursor_keys_app: false,
        linefeed_newline: false,
        keypad_app: false,
        disambiguate: true,
    };

    fn kitty_named(key: NamedKey, mods: ModifiersState) -> Vec<u8> {
        encode(&Key::Named(key), None, mods, KITTY).unwrap()
    }

    fn kitty_chr(s: &str, text: Option<&str>, mods: ModifiersState) -> Vec<u8> {
        encode(&Key::Character(s.into()), text, mods, KITTY).unwrap()
    }

    #[test]
    fn shift_enter_is_told_apart_from_enter_under_disambiguate() {
        assert_eq!(kitty_named(NamedKey::Enter, SHIFT), b"\x1b[13;2u");
        assert_eq!(kitty_named(NamedKey::Enter, CTRL), b"\x1b[13;5u");
        assert_eq!(kitty_named(NamedKey::Enter, ALT), b"\x1b[13;3u");
        assert_eq!(kitty_named(NamedKey::Enter, SUPER), b"\x1b[13;9u");
        // Without the flag Shift+Enter stays a plain CR, as in xterm.
        assert_eq!(named(NamedKey::Enter, SHIFT), b"\r");
    }

    #[test]
    fn unmodified_enter_tab_and_backspace_keep_their_legacy_bytes() {
        assert_eq!(kitty_named(NamedKey::Enter, NONE), b"\r");
        assert_eq!(kitty_named(NamedKey::Tab, NONE), b"\t");
        assert_eq!(kitty_named(NamedKey::Backspace, NONE), b"\x7f");
        assert_eq!(kitty_named(NamedKey::Tab, SHIFT), b"\x1b[9;2u");
        assert_eq!(kitty_named(NamedKey::Backspace, CTRL), b"\x1b[127;5u");
    }

    #[test]
    fn escape_is_always_a_csi_u_sequence_under_disambiguate() {
        assert_eq!(kitty_named(NamedKey::Escape, NONE), b"\x1b[27u");
        assert_eq!(kitty_named(NamedKey::Escape, SHIFT), b"\x1b[27;2u");
    }

    #[test]
    fn ctrl_and_alt_chords_report_the_unshifted_key_under_disambiguate() {
        assert_eq!(kitty_chr("i", None, CTRL), b"\x1b[105;5u");
        assert_eq!(kitty_chr("a", Some("å"), ALT), b"\x1b[97;3u");
        assert_eq!(kitty_chr("a", None, CTRL | SHIFT), b"\x1b[97;6u");
        // Caps Lock gives an upper-case key without Shift.
        assert_eq!(kitty_chr("A", None, CTRL), b"\x1b[97;5u");
        assert_eq!(kitty_named(NamedKey::Space, CTRL), b"\x1b[32;5u");
    }

    #[test]
    fn text_and_other_keys_are_unchanged_under_disambiguate() {
        assert_eq!(kitty_chr("a", Some("a"), NONE), b"a");
        assert_eq!(kitty_chr("a", Some("A"), SHIFT), b"A");
        assert_eq!(kitty_named(NamedKey::Space, SHIFT), b" ");
        assert_eq!(kitty_named(NamedKey::ArrowUp, NONE), b"\x1b[A");
        assert_eq!(kitty_named(NamedKey::ArrowUp, CTRL), b"\x1b[1;5A");
        assert_eq!(kitty_named(NamedKey::F5, NONE), b"\x1b[15~");
    }

    #[test]
    fn characters_send_the_platform_text() {
        assert_eq!(chr("a", Some("a"), NONE).unwrap(), b"a");
        assert_eq!(chr("A", Some("A"), SHIFT).unwrap(), b"A");
        assert_eq!(chr("a", Some("あ"), NONE).unwrap(), "あ".as_bytes());
        assert_eq!(chr("x", None, NONE).unwrap(), b"x");
    }

    #[test]
    fn ctrl_letters_and_symbols_send_c0_controls() {
        assert_eq!(chr("c", None, CTRL).unwrap(), [0x03]);
        assert_eq!(chr("C", None, CTRL | SHIFT).unwrap(), [0x03]);
        assert_eq!(chr("[", None, CTRL).unwrap(), [0x1b]);
        assert_eq!(chr("\\", None, CTRL).unwrap(), [0x1c]);
        assert_eq!(chr("]", None, CTRL).unwrap(), [0x1d]);
        assert_eq!(chr("@", None, CTRL).unwrap(), [0x00]);
        assert_eq!(chr("_", None, CTRL).unwrap(), [0x1f]);
        assert_eq!(chr("?", None, CTRL).unwrap(), [0x7f]);
        assert_eq!(named(NamedKey::Space, CTRL), [0x00]);
    }

    #[test]
    fn alt_prefixes_escape() {
        assert_eq!(chr("b", Some("∫"), ALT).unwrap(), b"\x1bb");
        assert_eq!(chr("c", None, CTRL | ALT).unwrap(), [0x1b, 0x03]);
        assert_eq!(named(NamedKey::Backspace, ALT), b"\x1b\x7f");
        assert_eq!(named(NamedKey::Enter, ALT), b"\x1b\r");
    }

    #[test]
    fn editing_keys() {
        assert_eq!(named(NamedKey::Enter, NONE), b"\r");
        assert_eq!(named(NamedKey::Backspace, NONE), b"\x7f");
        assert_eq!(named(NamedKey::Backspace, CTRL), b"\x08");
        assert_eq!(named(NamedKey::Tab, NONE), b"\t");
        assert_eq!(named(NamedKey::Tab, SHIFT), b"\x1b[Z");
        assert_eq!(named(NamedKey::Escape, NONE), b"\x1b");
        assert_eq!(named(NamedKey::Space, NONE), b" ");
    }

    #[test]
    fn return_sends_crlf_in_linefeed_mode() {
        let lnm = KeyModes {
            linefeed_newline: true,
            ..Default::default()
        };
        assert_eq!(named_in(NamedKey::Enter, lnm), b"\r\n");
    }

    #[test]
    fn arrows_follow_decckm() {
        assert_eq!(named(NamedKey::ArrowUp, NONE), b"\x1b[A");
        assert_eq!(named(NamedKey::ArrowDown, NONE), b"\x1b[B");
        assert_eq!(named(NamedKey::ArrowRight, NONE), b"\x1b[C");
        assert_eq!(named(NamedKey::ArrowLeft, NONE), b"\x1b[D");
        assert_eq!(named(NamedKey::Home, NONE), b"\x1b[H");
        assert_eq!(named(NamedKey::End, NONE), b"\x1b[F");
        let app = KeyModes {
            cursor_keys_app: true,
            ..Default::default()
        };
        assert_eq!(named_in(NamedKey::ArrowUp, app), b"\x1bOA");
        assert_eq!(named_in(NamedKey::Home, app), b"\x1bOH");
        assert_eq!(named_in(NamedKey::End, app), b"\x1bOF");
    }

    #[test]
    fn modified_cursor_keys_carry_the_modifier_parameter() {
        assert_eq!(named(NamedKey::ArrowRight, CTRL), b"\x1b[1;5C");
        assert_eq!(named(NamedKey::ArrowLeft, SHIFT), b"\x1b[1;2D");
        assert_eq!(named(NamedKey::ArrowUp, ALT), b"\x1b[1;3A");
        assert_eq!(named(NamedKey::Home, CTRL | SHIFT), b"\x1b[1;6H");
    }

    #[test]
    fn page_and_edit_keys_use_tilde_sequences() {
        assert_eq!(named(NamedKey::Insert, NONE), b"\x1b[2~");
        assert_eq!(named(NamedKey::Delete, NONE), b"\x1b[3~");
        assert_eq!(named(NamedKey::PageUp, NONE), b"\x1b[5~");
        assert_eq!(named(NamedKey::PageDown, NONE), b"\x1b[6~");
        assert_eq!(named(NamedKey::Delete, CTRL), b"\x1b[3;5~");
    }

    #[test]
    fn function_keys() {
        assert_eq!(named(NamedKey::F1, NONE), b"\x1bOP");
        assert_eq!(named(NamedKey::F4, NONE), b"\x1bOS");
        assert_eq!(named(NamedKey::F1, SHIFT), b"\x1b[1;2P");
        assert_eq!(named(NamedKey::F5, NONE), b"\x1b[15~");
        assert_eq!(named(NamedKey::F6, NONE), b"\x1b[17~");
        assert_eq!(named(NamedKey::F10, NONE), b"\x1b[21~");
        assert_eq!(named(NamedKey::F11, NONE), b"\x1b[23~");
        assert_eq!(named(NamedKey::F12, NONE), b"\x1b[24~");
        assert_eq!(named(NamedKey::F12, CTRL), b"\x1b[24;5~");
    }

    #[test]
    fn keypad_sends_ss3_only_in_application_mode() {
        let app = KeyModes {
            keypad_app: true,
            ..Default::default()
        };
        let digit = Key::Character("5".into());
        assert_eq!(encode_keypad(&digit, NONE, app).unwrap(), b"\x1bOu");
        let enter = Key::Named(NamedKey::Enter);
        assert_eq!(encode_keypad(&enter, NONE, app).unwrap(), b"\x1bOM");
        let plus = Key::Character("+".into());
        assert_eq!(encode_keypad(&plus, NONE, app).unwrap(), b"\x1bOk");
        assert_eq!(encode_keypad(&digit, NONE, KeyModes::default()), None);
        assert_eq!(encode_keypad(&digit, SHIFT, app), None);
    }

    #[test]
    fn bare_modifiers_and_unknown_keys_send_nothing() {
        let shift = Key::Named(NamedKey::Shift);
        assert_eq!(encode(&shift, None, SHIFT, KeyModes::default()), None);
        let unknown = Key::Unidentified(winit::keyboard::NativeKey::Unidentified);
        assert_eq!(encode(&unknown, None, NONE, KeyModes::default()), None);
    }

    #[test]
    fn a_wheel_notch_scrolls_the_given_lines_and_pixels_add_up_to_lines() {
        let mut wheel = Wheel::default();
        assert_eq!(
            wheel.lines(MouseScrollDelta::LineDelta(0.0, 2.0), 20.0, 5.0),
            10
        );
        assert_eq!(
            wheel.lines(MouseScrollDelta::LineDelta(0.0, 1.0), 20.0, 3.0),
            3
        );
        assert_eq!(
            wheel.lines(MouseScrollDelta::LineDelta(0.0, -1.0), 20.0, 3.0),
            -3
        );
        let px = |y| MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, y));
        assert_eq!(wheel.lines(px(15.0), 20.0, 3.0), 0);
        assert_eq!(wheel.lines(px(15.0), 20.0, 3.0), 1);
        assert_eq!(wheel.lines(px(-10.0), 20.0, 3.0), 0);
    }

    #[test]
    fn the_wheel_on_the_alternate_screen_sends_arrow_keys_in_the_cursor_mode() {
        assert_eq!(wheel_arrows(2, KeyModes::default()), b"\x1b[A\x1b[A");
        let app = KeyModes {
            cursor_keys_app: true,
            ..KeyModes::default()
        };
        assert_eq!(wheel_arrows(-1, app), b"\x1bOB");
    }

    #[test]
    fn pasted_line_breaks_become_carriage_returns() {
        assert_eq!(paste("a\r\nb\nc", false), b"a\rb\rc");
    }

    #[test]
    fn bracketed_paste_wraps_the_text_and_cannot_be_closed_from_inside() {
        assert_eq!(
            paste("ls\n\x1b[201~rm x\n", true),
            b"\x1b[200~ls\n[201~rm x\n\x1b[201~"
        );
    }
}
