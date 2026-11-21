//! The window and taskbar icons, from the Store package's logos so that the
//! app looks the same however it was installed.

use winit::window::{Icon, WindowAttributes};

/// Drawn in the title bar and the Alt+Tab list on Windows.
#[cfg_attr(not(windows), allow(dead_code))]
const SMALL: &[u8] = include_bytes!("../packaging/msix/Assets/Square44x44Logo.png");
/// The taskbar on Windows, and the only icon elsewhere (where docks and
/// switchers scale it down themselves).
const LARGE: &[u8] = include_bytes!("../packaging/msix/Assets/Square150x150Logo.png");

/// `attributes` with the app's icons. A logo that does not decode leaves the
/// system's default icon rather than stopping the app.
pub fn attributes(attributes: WindowAttributes) -> WindowAttributes {
    #[cfg(windows)]
    let attributes = {
        use winit::platform::windows::WindowAttributesExtWindows;
        attributes
            .with_window_icon(load(SMALL))
            .with_taskbar_icon(load(LARGE))
    };
    #[cfg(not(windows))]
    let attributes = attributes.with_window_icon(load(LARGE));
    attributes
}

fn load(png: &[u8]) -> Option<Icon> {
    let (rgba, width, height) = decode(png)
        .inspect_err(|e| eprintln!("batune: icon: {e}"))
        .ok()?;
    Icon::from_rgba(rgba, width, height)
        .inspect_err(|e| eprintln!("batune: icon: {e}"))
        .ok()
}

/// RGBA pixels of a PNG, whatever its color type (the logos are paletted).
fn decode(png: &[u8]) -> Result<(Vec<u8>, u32, u32), image::ImageError> {
    let rgba = image::load_from_memory_with_format(png, image::ImageFormat::Png)?.into_rgba8();
    let (width, height) = rgba.dimensions();
    Ok((rgba.into_raw(), width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_logos_decode_to_square_rgba() {
        for (png, size) in [(SMALL, 44), (LARGE, 150)] {
            let (rgba, width, height) = decode(png).unwrap();
            assert_eq!((width, height), (size, size));
            assert_eq!(rgba.len(), (size * size * 4) as usize);
            assert!(load(png).is_some());
        }
    }

    #[test]
    fn corners_stay_transparent() {
        // The logo is a rounded square; its palette's transparency must
        // survive the conversion, or the icon gets a solid box around it.
        let (rgba, _, _) = decode(LARGE).unwrap();
        assert_eq!(rgba[3], 0, "top-left alpha");
        assert_eq!(rgba[150 * 75 * 4 + 75 * 4 + 3], 0xff, "center alpha");
    }
}
