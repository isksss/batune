//! The window frame drawn by the system (title bar and corners), matched to
//! the terminal's colors.

use winit::window::{Theme, Window, WindowAttributes};

use crate::config::Backdrop;
use crate::palette::Palette;
use crate::render::Color;

/// Light or dark frame for a terminal drawn on `background`: dark when white
/// text on it would contrast more than black text (relative luminance below
/// the point where both contrast ratios are equal).
fn theme_for(background: Color) -> Theme {
    let linear = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(background.r)
        + 0.7152 * linear(background.g)
        + 0.0722 * linear(background.b);
    if luminance < 0.179 {
        Theme::Dark
    } else {
        Theme::Light
    }
}

/// `attributes` with the frame set up for `palette`, so the window does not
/// flash the system colors before [`apply`] would run.
#[cfg_attr(not(windows), allow(unused_variables))]
pub fn attributes(
    attributes: WindowAttributes,
    palette: &Palette,
    backdrop: Backdrop,
) -> WindowAttributes {
    // A fixed theme also keeps winit on Windows from switching the frame back
    // to the system theme when the system setting changes.
    let attributes = attributes.with_theme(Some(theme_for(palette.background)));
    #[cfg(windows)]
    let attributes = windows::attributes(attributes, palette, backdrop);
    attributes
}

/// Matches the frame of an open window to `palette` and `backdrop`.
#[cfg_attr(not(windows), allow(unused_variables))]
pub fn apply(window: &Window, palette: &Palette, backdrop: Backdrop) {
    window.set_theme(Some(theme_for(palette.background)));
    #[cfg(windows)]
    windows::apply(window, palette, backdrop);
}

/// DWM window attributes. Each one exists only from some Windows build on
/// (dark mode from Windows 10 20H1, the colors and corners from Windows 11),
/// and DWM rejects unknown ones without other effect, so failures are
/// ignored and older systems keep their default frame.
///
/// The backdrop materials show only through the title bar. Letting them show
/// behind the grid as well (and an opacity setting with it) would need a
/// transparent surface, but wgpu's DX12 surface on an HWND offers only
/// `CompositeAlphaMode::Opaque` (alpha needs a DirectComposition visual,
/// which winit does not set up), and Vulkan drivers on Windows commonly do
/// the same, so it is not offered.
#[cfg(windows)]
mod windows {
    use crate::render::Color;
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
    use winit::platform::windows::{
        BackdropType, Color as DwmColor, CornerPreference, WindowAttributesExtWindows,
        WindowExtWindows,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::{Theme, Window, WindowAttributes};

    use super::theme_for;
    use crate::config::Backdrop;
    use crate::palette::Palette;

    /// What the title bar is painted with.
    struct Caption {
        backdrop: BackdropType,
        background: DwmColor,
        text: DwmColor,
    }

    impl Caption {
        fn new(palette: &Palette, backdrop: Backdrop) -> Self {
            let theme = |backdrop| Self {
                backdrop,
                background: dwm_color(palette.background),
                text: dwm_color(palette.foreground),
            };
            // An explicit caption color is painted over the material, so the
            // system colors are kept to let it show; dark mode still follows
            // the theme and tints the material.
            let material = |backdrop| Self {
                backdrop,
                background: DwmColor::SYSTEM_DEFAULT,
                text: DwmColor::SYSTEM_DEFAULT,
            };
            match backdrop {
                Backdrop::None => theme(BackdropType::Auto),
                Backdrop::Mica => material(BackdropType::MainWindow),
                Backdrop::Acrylic => material(BackdropType::TransientWindow),
            }
        }
    }

    fn dwm_color(c: Color) -> DwmColor {
        DwmColor::from_rgb(c.r, c.g, c.b)
    }

    pub fn attributes(
        attributes: WindowAttributes,
        palette: &Palette,
        backdrop: Backdrop,
    ) -> WindowAttributes {
        let caption = Caption::new(palette, backdrop);
        attributes
            .with_corner_preference(CornerPreference::Round)
            .with_system_backdrop(caption.backdrop)
            .with_title_background_color(Some(caption.background))
            .with_title_text_color(caption.text)
    }

    pub fn apply(window: &Window, palette: &Palette, backdrop: Backdrop) {
        let caption = Caption::new(palette, backdrop);
        set_dark_mode(window, theme_for(palette.background) == Theme::Dark);
        window.set_corner_preference(CornerPreference::Round);
        window.set_system_backdrop(caption.backdrop);
        window.set_title_background_color(Some(caption.background));
        window.set_title_text_color(caption.text);
    }

    /// winit's theme switches the frame through the undocumented
    /// `SetWindowCompositionAttribute`; the documented DWM attribute is set
    /// as well so that a system without the former still gets a dark frame.
    fn set_dark_mode(window: &Window, dark: bool) {
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return;
        };
        let value: i32 = dark.into();
        // SAFETY: the handle is the live window's, and `value` is the BOOL
        // the attribute takes, valid for the duration of the call.
        unsafe {
            DwmSetWindowAttribute(
                handle.hwnd.get() as _,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                (&raw const value).cast(),
                size_of::<i32>() as u32,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;

    #[test]
    fn dark_backgrounds_get_a_dark_frame() {
        for name in [
            "batune-dark",
            "solarized-dark",
            "gruvbox-dark",
            "tokyo-night",
        ] {
            let background = theme::find(name).unwrap().background;
            assert_eq!(theme_for(background), Theme::Dark, "{name}");
        }
        assert_eq!(theme_for(Color::BLACK), Theme::Dark);
    }

    #[test]
    fn light_backgrounds_get_a_light_frame() {
        for name in ["batune-light", "solarized-light"] {
            let background = theme::find(name).unwrap().background;
            assert_eq!(theme_for(background), Theme::Light, "{name}");
        }
        assert_eq!(theme_for(Color::WHITE), Theme::Light);
    }

    #[test]
    fn mid_gray_is_split_by_luminance_not_by_channel_average() {
        // #777777 averages below the middle but is closer to white in
        // contrast; pure blue averages the same as dark gray but is dark.
        assert_eq!(theme_for(Color::rgb(0x77, 0x77, 0x77)), Theme::Light);
        assert_eq!(theme_for(Color::rgb(0, 0, 0xff)), Theme::Dark);
    }
}
