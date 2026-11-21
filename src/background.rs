//! Reads the `[background_image]` file for the renderer.

use std::path::Path;

use crate::render::BackgroundImage;

use crate::config;

/// The image to draw behind the grid, or `None` when none is configured or
/// the file cannot be read; the latter is reported and the window goes on
/// without one, as a broken image is no reason to refuse a terminal.
pub fn load(config: &config::BackgroundImage) -> Option<BackgroundImage> {
    let path = config.path.as_deref()?;
    let (rgba, width, height) = decode(path)
        .inspect_err(|e| eprintln!("batune: background_image.path {}: {e}", path.display()))
        .ok()?;
    Some(BackgroundImage {
        width,
        height,
        rgba: rgba.into(),
        fit: config.fit.into(),
        opacity: config.opacity,
    })
}

fn decode(path: &Path) -> Result<(Vec<u8>, u32, u32), image::ImageError> {
    let image = image::ImageReader::open(path)?
        .with_guessed_format()?
        .decode()?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Ok((image.into_raw(), width, height))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::render::BackgroundFit;

    use super::*;
    use crate::config::Fit;

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("batune-bg-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// A 2x1 PNG: red, then half-transparent blue.
    fn png() -> Vec<u8> {
        let image =
            image::RgbaImage::from_raw(2, 1, vec![0xff, 0, 0, 0xff, 0, 0, 0xff, 0x80]).unwrap();
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn nothing_is_drawn_without_a_path() {
        assert!(load(&config::BackgroundImage::default()).is_none());
    }

    #[test]
    fn the_file_is_decoded_to_rgba_with_the_configured_look() {
        // No extension: the format is told from the contents.
        let path = temp_file("image", &png());
        let image = load(&config::BackgroundImage {
            path: Some(path.clone()),
            opacity: 0.5,
            fit: Fit::Contain,
        })
        .unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(&image.rgba[..], [0xff, 0, 0, 0xff, 0, 0, 0xff, 0x80]);
        assert_eq!((image.fit, image.opacity), (BackgroundFit::Contain, 0.5));
    }

    #[test]
    fn missing_or_broken_files_mean_no_image() {
        let broken = temp_file("broken.png", b"not a png");
        for path in [broken.clone(), PathBuf::from("/nonexistent/batune/bg.png")] {
            let config = config::BackgroundImage {
                path: Some(path),
                ..Default::default()
            };
            assert!(load(&config).is_none());
        }
        std::fs::remove_file(broken).unwrap();
    }
}
