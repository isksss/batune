//! Built-in color themes.

use crate::render::Color;

/// The colors a theme sets; the rest of the 256-color table is fixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    pub foreground: Color,
    pub background: Color,
    pub cursor: Color,
    /// ANSI colors 0-15 (normal then bright).
    pub ansi: [Color; 16],
}

const fn hex(v: u32) -> Color {
    Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

const fn ansi(v: [u32; 16]) -> [Color; 16] {
    let mut out = [Color::BLACK; 16];
    let mut i = 0;
    while i < 16 {
        out[i] = hex(v[i]);
        i += 1;
    }
    out
}

pub const DEFAULT: &str = "batune-dark";

pub const THEMES: &[Theme] = &[
    Theme {
        name: "batune-dark",
        foreground: hex(0xd8d8d8),
        background: hex(0x1c1c24),
        cursor: hex(0xd8d8d8),
        ansi: ansi([
            0x1c1c24, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xc8ccd4,
            0x5c6370, 0xf08c95, 0xb5e096, 0xf5d59b, 0x81c8ff, 0xdc98f0, 0x76d0dc, 0xffffff,
        ]),
    },
    Theme {
        name: "batune-light",
        foreground: hex(0x383a42),
        background: hex(0xfafafa),
        cursor: hex(0x526fff),
        ansi: ansi([
            0x383a42, 0xe45649, 0x50a14f, 0xc18401, 0x0184bc, 0xa626a4, 0x0997b3, 0xa0a1a7,
            0x696c77, 0xca1243, 0x3e953a, 0x986801, 0x4078f2, 0x9a2ea8, 0x0b7a8f, 0x202227,
        ]),
    },
    Theme {
        name: "solarized-dark",
        foreground: hex(0x839496),
        background: hex(0x002b36),
        cursor: hex(0x93a1a1),
        ansi: ansi([
            0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
            0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
        ]),
    },
    Theme {
        name: "solarized-light",
        foreground: hex(0x657b83),
        background: hex(0xfdf6e3),
        cursor: hex(0x586e75),
        ansi: ansi([
            0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
            0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
        ]),
    },
    Theme {
        name: "gruvbox-dark",
        foreground: hex(0xebdbb2),
        background: hex(0x282828),
        cursor: hex(0xebdbb2),
        ansi: ansi([
            0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984,
            0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
        ]),
    },
];

pub fn find(name: &str) -> Option<&'static Theme> {
    THEMES.iter().find(|t| t.name == name)
}

pub fn names() -> impl Iterator<Item = &'static str> {
    THEMES.iter().map(|t| t.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_theme_exists() {
        assert_eq!(find(DEFAULT).unwrap().name, DEFAULT);
    }

    #[test]
    fn theme_names_are_unique() {
        let mut names: Vec<_> = names().collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), THEMES.len());
    }

    #[test]
    fn hex_splits_into_channels() {
        assert_eq!(hex(0x12_34_56), Color::rgb(0x12, 0x34, 0x56));
    }
}
