//! Mouse reports for programs that turned on mouse tracking (`?9`,
//! `?1000`, `?1002`, `?1003`) in the encodings of `?1005` and `?1006`.

use winit::keyboard::ModifiersState;

use crate::vt::{MouseEncoding, MouseTracking};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

impl Button {
    fn code(self) -> u32 {
        match self {
            Button::Left => 0,
            Button::Middle => 1,
            Button::Right => 2,
            Button::WheelUp => 64,
            Button::WheelDown => 65,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Press(Button),
    Release(Button),
    /// The pointer moved to another cell, with the button held (if any).
    Motion(Option<Button>),
}

/// Bytes reporting `action` at cell (`col`, `row`) (0-based), or `None`
/// when the tracking mode does not ask for it or the encoding cannot carry
/// the position.
pub fn report(
    action: Action,
    col: usize,
    row: usize,
    mods: ModifiersState,
    tracking: MouseTracking,
    encoding: MouseEncoding,
) -> Option<Vec<u8>> {
    let wanted = match (tracking, action) {
        (MouseTracking::Off, _) => false,
        // X10 predates the wheel; its presses are buttons 1-3 only.
        (MouseTracking::X10, action) => matches!(
            action,
            Action::Press(Button::Left | Button::Middle | Button::Right)
        ),
        (_, Action::Press(_) | Action::Release(_)) => true,
        (MouseTracking::Normal, Action::Motion(_)) => false,
        (MouseTracking::ButtonEvent, Action::Motion(held)) => held.is_some(),
        (MouseTracking::AnyEvent, Action::Motion(_)) => true,
    };
    // A wheel "release" is not a thing: the wheel only sends presses.
    if !wanted || matches!(action, Action::Release(Button::WheelUp | Button::WheelDown)) {
        return None;
    }
    let mut code = match action {
        Action::Press(b) => b.code(),
        // SGR names the released button; the older encodings cannot and
        // send 3 ("some button was released").
        Action::Release(b) if encoding == MouseEncoding::Sgr => b.code(),
        Action::Release(_) => 3,
        Action::Motion(held) => 32 + held.map_or(3, Button::code),
    };
    // X10 mode predates modifier reporting.
    if tracking != MouseTracking::X10 {
        if mods.shift_key() {
            code += 4;
        }
        if mods.alt_key() {
            code += 8;
        }
        if mods.control_key() {
            code += 16;
        }
    }
    let (x, y) = (col + 1, row + 1);
    match encoding {
        MouseEncoding::Sgr => {
            let end = if matches!(action, Action::Release(_)) {
                'm'
            } else {
                'M'
            };
            Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes())
        }
        MouseEncoding::Default => {
            let byte = |v: usize| u8::try_from(v + 32).ok();
            Some(vec![
                0x1b,
                b'[',
                b'M',
                byte(code as usize)?,
                byte(x)?,
                byte(y)?,
            ])
        }
        MouseEncoding::Utf8 => {
            let mut out = b"\x1b[M".to_vec();
            for v in [code as usize, x, y] {
                // xterm stops at 2015 so every value fits in two UTF-8 bytes.
                let v = u32::try_from(v + 32).ok().filter(|&v| v < 2048)?;
                let c = char::from_u32(v)?;
                let mut buf = [0; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            Some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: ModifiersState = ModifiersState::empty();

    fn sgr(action: Action, tracking: MouseTracking) -> Option<String> {
        report(action, 4, 9, NONE, tracking, MouseEncoding::Sgr)
            .map(|b| String::from_utf8(b).unwrap())
    }

    #[test]
    fn nothing_is_reported_while_tracking_is_off() {
        let press = Action::Press(Button::Left);
        assert_eq!(sgr(press, MouseTracking::Off), None);
    }

    #[test]
    fn sgr_reports_presses_and_releases_with_one_based_cells() {
        let t = MouseTracking::Normal;
        assert_eq!(
            sgr(Action::Press(Button::Left), t).as_deref(),
            Some("\x1b[<0;5;10M")
        );
        assert_eq!(
            sgr(Action::Release(Button::Right), t).as_deref(),
            Some("\x1b[<2;5;10m")
        );
        assert_eq!(
            sgr(Action::Press(Button::WheelDown), t).as_deref(),
            Some("\x1b[<65;5;10M")
        );
    }

    #[test]
    fn motion_is_reported_only_in_the_modes_that_ask_for_it() {
        let drag = Action::Motion(Some(Button::Left));
        let hover = Action::Motion(None);
        assert_eq!(sgr(drag, MouseTracking::Normal), None);
        assert_eq!(
            sgr(drag, MouseTracking::ButtonEvent).as_deref(),
            Some("\x1b[<32;5;10M")
        );
        assert_eq!(sgr(hover, MouseTracking::ButtonEvent), None);
        assert_eq!(
            sgr(hover, MouseTracking::AnyEvent).as_deref(),
            Some("\x1b[<35;5;10M")
        );
    }

    #[test]
    fn x10_reports_presses_only_and_without_modifiers() {
        let t = MouseTracking::X10;
        let enc = MouseEncoding::Default;
        let ctrl = ModifiersState::CONTROL;
        assert_eq!(
            report(Action::Press(Button::Left), 0, 0, ctrl, t, enc),
            Some(b"\x1b[M !!".to_vec())
        );
        assert_eq!(
            report(Action::Release(Button::Left), 0, 0, NONE, t, enc),
            None
        );
        for wheel in [Button::WheelUp, Button::WheelDown] {
            assert_eq!(report(Action::Press(wheel), 0, 0, NONE, t, enc), None);
        }
    }

    #[test]
    fn modifiers_add_to_the_button_code() {
        let mods = ModifiersState::SHIFT | ModifiersState::ALT | ModifiersState::CONTROL;
        let bytes = report(
            Action::Press(Button::Middle),
            0,
            0,
            mods,
            MouseTracking::Normal,
            MouseEncoding::Sgr,
        );
        assert_eq!(bytes, Some(b"\x1b[<29;1;1M".to_vec()));
    }

    #[test]
    fn the_default_encoding_sends_3_for_a_release_and_gives_up_past_column_223() {
        let t = MouseTracking::Normal;
        let enc = MouseEncoding::Default;
        assert_eq!(
            report(Action::Release(Button::Left), 1, 2, NONE, t, enc),
            Some(b"\x1b[M#\"#".to_vec())
        );
        assert!(report(Action::Press(Button::Left), 222, 0, NONE, t, enc).is_some());
        assert_eq!(
            report(Action::Press(Button::Left), 223, 0, NONE, t, enc),
            None
        );
    }

    #[test]
    fn utf8_encoding_carries_columns_past_223_in_two_bytes() {
        let bytes = report(
            Action::Press(Button::Left),
            299,
            0,
            NONE,
            MouseTracking::Normal,
            MouseEncoding::Utf8,
        )
        .unwrap();
        // 300 + 32 = 332 = U+014C.
        assert_eq!(bytes, "\x1b[M \u{14c}!".as_bytes());
    }
}
