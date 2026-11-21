use vte::{Params, ParamsIter};

use crate::vt::cell::{Cell, Color, Flags, Underline};

/// Applies SGR (`CSI ... m`) to the pen. Both the colon (ITU T.416) and
/// the semicolon (xterm) forms of extended colors are accepted; unknown
/// parameters are skipped.
pub(crate) fn apply(pen: &mut Cell, params: &Params) {
    if params.is_empty() {
        pen.reset_attributes();
        return;
    }
    let mut iter = params.iter();
    while let Some(param) = iter.next() {
        match param {
            [0] => pen.reset_attributes(),
            [1] => pen.flags.insert(Flags::BOLD),
            [2] => pen.flags.insert(Flags::DIM),
            [3] => pen.flags.insert(Flags::ITALIC),
            [4] => pen.underline = Underline::Single,
            [4, style, ..] => {
                pen.underline = match style {
                    0 => Underline::None,
                    2 => Underline::Double,
                    3 => Underline::Curly,
                    4 => Underline::Dotted,
                    5 => Underline::Dashed,
                    _ => Underline::Single,
                }
            }
            [5] | [6] => pen.flags.insert(Flags::BLINK),
            [7] => pen.flags.insert(Flags::INVERSE),
            [8] => pen.flags.insert(Flags::HIDDEN),
            [9] => pen.flags.insert(Flags::STRIKEOUT),
            [21] => pen.underline = Underline::Double,
            [22] => pen.flags.remove(Flags::BOLD | Flags::DIM),
            [23] => pen.flags.remove(Flags::ITALIC),
            [24] => pen.underline = Underline::None,
            [25] => pen.flags.remove(Flags::BLINK),
            [27] => pen.flags.remove(Flags::INVERSE),
            [28] => pen.flags.remove(Flags::HIDDEN),
            [29] => pen.flags.remove(Flags::STRIKEOUT),
            [n @ 30..=37] => pen.fg = Color::Indexed((n - 30) as u8),
            [38, ..] => {
                if let Some(c) = extended_color(param, &mut iter) {
                    pen.fg = c;
                }
            }
            [39] => pen.fg = Color::Default,
            [n @ 40..=47] => pen.bg = Color::Indexed((n - 40) as u8),
            [48, ..] => {
                if let Some(c) = extended_color(param, &mut iter) {
                    pen.bg = c;
                }
            }
            [49] => pen.bg = Color::Default,
            [58, ..] => {
                if let Some(c) = extended_color(param, &mut iter) {
                    pen.underline_color = c;
                }
            }
            [59] => pen.underline_color = Color::Default,
            [n @ 90..=97] => pen.fg = Color::Indexed((n - 90 + 8) as u8),
            [n @ 100..=107] => pen.bg = Color::Indexed((n - 100 + 8) as u8),
            _ => {}
        }
    }
}

/// Parses the color after 38/48/58. In the semicolon form the components
/// are separate parameters, so they are taken from `rest`.
fn extended_color(param: &[u16], rest: &mut ParamsIter) -> Option<Color> {
    if param.len() > 1 {
        return match param[1..] {
            [5, n, ..] => byte(n).map(Color::Indexed),
            // 38:2:<colorspace>:r:g:b as in T.416, and 38:2:r:g:b as many
            // programs emit it.
            [2, _, r, g, b, ..] | [2, r, g, b] => rgb(r, g, b),
            _ => None,
        };
    }
    match rest.next()? {
        [5] => byte(rest.next()?[0]).map(Color::Indexed),
        [2] => {
            let r = rest.next()?[0];
            let g = rest.next()?[0];
            let b = rest.next()?[0];
            rgb(r, g, b)
        }
        _ => None,
    }
}

fn byte(v: u16) -> Option<u8> {
    u8::try_from(v).ok()
}

fn rgb(r: u16, g: u16, b: u16) -> Option<Color> {
    Some(Color::Rgb(byte(r)?, byte(g)?, byte(b)?))
}
