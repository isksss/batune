//! Maps the colors an application asks for to the RGB values drawn.

use crate::render::Color;
use crate::vt::{Color as VtColor, ReportedColors};

use crate::theme::{self, Theme};

/// Theme colors plus the 256-color table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub foreground: Color,
    pub background: Color,
    pub cursor: Color,
    /// 0-15 ANSI colors, 16-231 the 6x6x6 cube, 232-255 the gray ramp.
    pub colors: [Color; 256],
    /// Bold text in one of the eight normal ANSI colors is drawn in its
    /// bright variant.
    pub bold_is_bright: bool,
}

impl Default for Palette {
    fn default() -> Self {
        Self::new(theme::find(theme::DEFAULT).expect("default theme"))
    }
}

impl Palette {
    /// The theme's colors with the xterm layout for colors 16-255.
    pub fn new(theme: &Theme) -> Self {
        let mut colors = [Color::BLACK; 256];
        colors[..16].copy_from_slice(&theme.ansi);
        // xterm's cube levels: 0 then 95 + 40 * (n - 1).
        let level = |n: usize| if n == 0 { 0 } else { (55 + 40 * n) as u8 };
        for i in 0..216 {
            colors[16 + i] = Color::rgb(level(i / 36), level(i / 6 % 6), level(i % 6));
        }
        for i in 0..24 {
            let v = (8 + 10 * i) as u8;
            colors[232 + i] = Color::rgb(v, v, v);
        }
        Self {
            foreground: theme.foreground,
            background: theme.background,
            cursor: theme.cursor,
            colors,
            bold_is_bright: true,
        }
    }

    /// Foreground color. Bold brightens by default, as in most terminals.
    pub fn foreground(&self, color: VtColor, bold: bool) -> Color {
        match color {
            VtColor::Default => self.foreground,
            VtColor::Indexed(i) if bold && self.bold_is_bright && i < 8 => {
                self.colors[usize::from(i) + 8]
            }
            VtColor::Indexed(i) => self.colors[usize::from(i)],
            VtColor::Rgb(r, g, b) => Color::rgb(r, g, b),
        }
    }

    /// The theme colors as the terminal answers OSC 10/11/12 queries.
    pub fn reported_colors(&self) -> ReportedColors {
        let rgb = |c: Color| (c.r, c.g, c.b);
        ReportedColors {
            foreground: rgb(self.foreground),
            background: rgb(self.background),
            cursor: rgb(self.cursor),
        }
    }

    pub fn background(&self, color: VtColor) -> Color {
        match color {
            VtColor::Default => self.background,
            VtColor::Indexed(i) => self.colors[usize::from(i)],
            VtColor::Rgb(r, g, b) => Color::rgb(r, g, b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_colors_follow_the_xterm_layout() {
        let p = Palette::default();
        assert_eq!(p.colors[16], Color::rgb(0, 0, 0));
        assert_eq!(p.colors[21], Color::rgb(0, 0, 0xff));
        assert_eq!(p.colors[196], Color::rgb(0xff, 0, 0));
        assert_eq!(p.colors[231], Color::rgb(0xff, 0xff, 0xff));
        assert_eq!(p.colors[232], Color::rgb(8, 8, 8));
        assert_eq!(p.colors[255], Color::rgb(238, 238, 238));
    }

    #[test]
    fn default_colors_come_from_the_theme() {
        let p = Palette::default();
        assert_eq!(p.foreground(VtColor::Default, false), p.foreground);
        assert_eq!(p.background(VtColor::Default), p.background);
    }

    #[test]
    fn bold_brightens_only_the_normal_ansi_colors() {
        let p = Palette::default();
        assert_eq!(p.foreground(VtColor::Indexed(1), true), p.colors[9]);
        assert_eq!(p.foreground(VtColor::Indexed(1), false), p.colors[1]);
        assert_eq!(p.foreground(VtColor::Indexed(9), true), p.colors[9]);
        assert_eq!(p.foreground(VtColor::Indexed(100), true), p.colors[100]);
        assert_eq!(p.background(VtColor::Indexed(1)), p.colors[1]);
    }

    #[test]
    fn bold_keeps_the_normal_color_when_brightening_is_off() {
        let p = Palette {
            bold_is_bright: false,
            ..Palette::default()
        };
        assert_eq!(p.foreground(VtColor::Indexed(1), true), p.colors[1]);
        assert_eq!(p.foreground(VtColor::Indexed(9), true), p.colors[9]);
    }

    #[test]
    fn rgb_passes_through() {
        let p = Palette::default();
        assert_eq!(
            p.foreground(VtColor::Rgb(1, 2, 3), true),
            Color::rgb(1, 2, 3)
        );
    }

    #[test]
    fn osc_11_query_is_answered_with_the_palette_background() {
        let palette = Palette::default();
        let mut term = crate::vt::Term::new(10, 1);
        term.set_reported_colors(palette.reported_colors());
        term.advance(b"\x1b]11;?\x07");
        let reply = String::from_utf8(term.take_responses()).unwrap();
        assert!(reply.contains("rgb:1c1c/1c1c/2424"), "{reply:?}");
    }
}
