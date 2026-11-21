//! Renders the demo frame into an offscreen texture and checks pixels.
//! Skips (passes) when the machine has no usable adapter; CI installs
//! Mesa's lavapipe so it really runs there.

#[path = "../examples/demo/frame.rs"]
mod frame;

use std::sync::OnceLock;

use batune::render::{
    BackgroundFit, BackgroundImage, CellFlags, CellMetrics, Color, CursorShape, FontConfig, Frame,
    ImageData, ImageQuad, Renderer,
};
use frame::{BG, FG, PALETTE, demo_frame, put};

const W: u32 = 800;
const H: u32 = 300;
const PAD: u32 = 8;
const CURSOR: Color = Color::rgb(0xf5, 0xe0, 0xdc);

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

/// One device for the whole test binary. Creating an instance and device per
/// test is not used: the tests run in parallel, and the Vulkan loader's
/// `vkSetDebugUtilsObjectNameEXT` (which wgpu calls on every resource in debug
/// builds) walks the loader's global instance/device lists, so another test
/// creating or destroying its own instance at the same time made it
/// SIGSEGV on lavapipe (#62). Serializing the tests instead would only hide it
/// behind `--test-threads=1` that every caller has to remember.
fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
    GPU.get_or_init(|| {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).ok()?;
        Some(Gpu { device, queue })
    })
    .as_ref()
}

struct Image {
    pixels: Vec<u8>,
    metrics: CellMetrics,
}

impl Image {
    fn at(&self, x: u32, y: u32) -> Color {
        let i = ((y * W + x) * 4) as usize;
        Color::rgb(self.pixels[i], self.pixels[i + 1], self.pixels[i + 2])
    }

    /// Top-left pixel of a cell.
    fn origin(&self, col: u16, row: u16) -> (u32, u32) {
        (
            PAD + u32::from(col) * self.metrics.width,
            PAD + u32::from(row) * self.metrics.height,
        )
    }

    fn cell_pixels(&self, col: u16, row: u16, span: u32) -> Vec<Color> {
        let (x0, y0) = self.origin(col, row);
        let mut out = Vec::new();
        for y in y0..y0 + self.metrics.height {
            for x in x0..x0 + self.metrics.width * span {
                out.push(self.at(x, y));
            }
        }
        out
    }
}

fn render(gpu: &Gpu, build: impl FnOnce(u16, u16) -> Frame) -> Image {
    render_with(gpu, wgpu::TextureFormat::Rgba8Unorm, |_| {}, build)
}

/// [`render`] into a `format` target, with `setup` applied to the renderer
/// first.
fn render_with(
    gpu: &Gpu,
    format: wgpu::TextureFormat,
    setup: impl FnOnce(&mut Renderer),
    build: impl FnOnce(u16, u16) -> Frame,
) -> Image {
    let mut renderer = Renderer::new(&gpu.device, format, FontConfig::default(), 1.0);
    renderer.set_padding(PAD as f32);
    setup(&mut renderer);
    let (cols, rows) = renderer.grid_size(W, H);
    let frame = build(cols, rows);

    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    renderer.render(&gpu.device, &gpu.queue, &view, &frame, (W, H));

    let row_bytes = W * 4;
    let padded = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(padded * H),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: None,
            },
        },
        target.size(),
    );
    gpu.queue.submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    let data = buffer.slice(..).get_mapped_range().expect("mapped");
    let pixels = data
        .chunks(padded as usize)
        .flat_map(|row| &row[..row_bytes as usize])
        .copied()
        .collect();
    Image {
        pixels,
        metrics: renderer.cell_metrics(),
    }
}

macro_rules! require_gpu {
    () => {
        match gpu() {
            Some(gpu) => gpu,
            None => {
                eprintln!("no GPU adapter; skipping");
                return;
            }
        }
    };
}

#[test]
fn padding_and_empty_cells_show_the_frame_background() {
    let gpu = require_gpu!();
    let img = render(gpu, |c, r| demo_frame(c, r, CursorShape::Block));
    assert_eq!(img.at(0, 0), BG);
    assert_eq!(img.at(W - 1, H - 1), BG);
}

#[test]
fn cell_backgrounds_fill_the_whole_cell() {
    let gpu = require_gpu!();
    let img = render(gpu, |c, r| demo_frame(c, r, CursorShape::Block));
    // Row 5 is "Backgrounds: bg0  bg1 ...", each label padded with spaces;
    // col 13 is the space before "bg0".
    assert!(img.cell_pixels(13, 5, 1).iter().all(|&p| p == PALETTE[0]));
    assert!(img.cell_pixels(19, 5, 1).iter().all(|&p| p == PALETTE[1]));
}

#[test]
fn ascii_glyphs_use_the_foreground_color() {
    let gpu = require_gpu!();
    let img = render(gpu, |c, r| demo_frame(c, r, CursorShape::Block));
    // "ASCII:" at row 1; 'A' must have solid foreground pixels in its stems.
    let px = img.cell_pixels(0, 1, 1);
    assert!(px.contains(&FG), "no fully covered pixel in 'A'");
    assert!(px.contains(&BG));
}

#[test]
fn japanese_and_emoji_fall_back_to_other_fonts() {
    let gpu = require_gpu!();
    let img = render(gpu, |c, r| demo_frame(c, r, CursorShape::Block));
    // Row 2 "日本語: ..." starts with a wide glyph spanning cols 0-1.
    let kanji = img.cell_pixels(0, 2, 2);
    assert!(
        kanji.iter().filter(|&&p| p != BG).count() > 20,
        "日 rendered no pixels"
    );
    // Row 3 "Emoji: 🍣": the emoji sits at col 7 and is colored, so some
    // pixel is neither a shade of gray nor the background.
    let emoji = img.cell_pixels(7, 3, 2);
    let colorful = emoji
        .iter()
        .filter(|p| p.r.abs_diff(p.g) > 40 || p.g.abs_diff(p.b) > 40)
        .count();
    // Other machines may lack an emoji font; CI installs this one.
    if std::path::Path::new("/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf").exists() {
        assert!(colorful > 20, "🍣 is not in color");
    }
}

#[test]
fn a_text_mark_on_an_emoji_takes_the_foreground_color() {
    let gpu = require_gpu!();
    if !std::path::Path::new("/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf").exists() {
        eprintln!("no Noto Color Emoji; skipping");
        return;
    }
    let green = Color::rgb(0, 255, 0);
    let img = render(gpu, |c, r| {
        let mut f = Frame::new(c, r, FG, BG);
        put(&mut f, 0, 0, "\u{1F44D}", green, BG, CellFlags::empty());
        if let Some(cell) = f.cell_mut(0, 0) {
            cell.zerowidth = Some(['\u{301}'][..].into());
        }
        f
    });
    // The thumb is yellow, so only the acute can be green.
    let marked = img
        .cell_pixels(0, 0, 2)
        .iter()
        .filter(|p| p.g > 150 && p.r < 80 && p.b < 80)
        .count();
    assert!(marked > 0, "the acute is not in the foreground color");
}

#[test]
fn inverse_underline_and_strike_decorate_the_cell() {
    let gpu = require_gpu!();
    let img = render(gpu, |c, r| demo_frame(c, r, CursorShape::Block));
    let m = img.metrics;
    let reference = demo_frame(80, 24, CursorShape::Block);
    let row6: String = (0..80)
        .map(|c| reference.cell(c, 6).map_or(' ', |cell| cell.ch))
        .collect();
    let find = |label: &str| row6.find(label).unwrap() as u16;
    let inverse = find("inverse");
    let (x, y) = img.origin(inverse, 6);
    assert_eq!(img.at(x, y), FG, "inverse background");

    let underline = find("underline");
    let (x, y) = img.origin(underline, 6);
    assert_eq!(img.at(x, y + m.underline_y), FG, "underline stroke");
    assert_eq!(img.at(x, y + m.underline_y + m.stroke + 1), BG);

    let strike = find("strike");
    let (x, y) = img.origin(strike + 1, 6);
    assert_eq!(img.at(x, y + m.strikeout_y), FG, "strikeout stroke");
}

#[test]
fn underline_styles_differ_from_a_straight_line() {
    let gpu = require_gpu!();
    // The labels' letters are blanked so that every foreground pixel in
    // their cells is underline: with a font whose underline sits close to
    // the baseline, glyph ink and the wave would share rows.
    let img = render(gpu, |c, r| {
        let mut frame = demo_frame(c, r, CursorShape::Block);
        for col in 0..c {
            if let Some(cell) = frame.cell_mut(col, 8) {
                cell.ch = ' ';
            }
        }
        frame
    });
    let m = img.metrics;
    let reference = demo_frame(80, 24, CursorShape::Block);
    let row: String = (0..80)
        .map(|c| reference.cell(c, 8).map_or(' ', |cell| cell.ch))
        .collect();
    // Foreground pixels per column of the label's first two cells.
    let ink = |label: &str| -> Vec<usize> {
        let (x0, y0) = img.origin(row.find(label).unwrap() as u16, 8);
        (x0..x0 + 2 * m.width)
            .map(|x| (y0..y0 + m.height).filter(|&y| img.at(x, y) == FG).count())
            .collect()
    };
    let double = ink("double");
    assert!(double.iter().all(|&n| n >= 2), "double: {double:?}");
    let dotted = ink("dotted");
    assert!(
        dotted.contains(&0) && dotted.iter().any(|&n| n > 0),
        "dotted: {dotted:?}"
    );
    let dashed = ink("dashed");
    assert!(
        dashed.contains(&0) && dashed.iter().any(|&n| n > 0),
        "dashed: {dashed:?}"
    );
    let curly = ink("curly");
    assert!(curly.iter().all(|&n| n > 0), "curly: {curly:?}");
}

fn cursor_cell(shape: CursorShape) -> (Image, u16, u16) {
    let gpu = gpu().expect("checked by caller");
    let img = render(gpu, |c, r| demo_frame(c, r, shape));
    let cursor = demo_frame(80, 24, shape).cursor.unwrap();
    (img, cursor.col, cursor.row)
}

#[test]
fn cursor_shapes() {
    let _gpu = require_gpu!();

    let (img, col, row) = cursor_cell(CursorShape::Block);
    assert!(img.cell_pixels(col, row, 1).iter().all(|&p| p == CURSOR));

    let (img, col, row) = cursor_cell(CursorShape::Bar);
    let (x, y) = img.origin(col, row);
    let m = img.metrics;
    assert_eq!(img.at(x, y + m.height / 2), CURSOR);
    assert_eq!(img.at(x + m.width - 1, y + m.height / 2), BG);

    let (img, col, row) = cursor_cell(CursorShape::Underline);
    let (x, y) = img.origin(col, row);
    assert_eq!(img.at(x + m.width / 2, y + m.height - 1), CURSOR);
    assert_eq!(img.at(x + m.width / 2, y), BG);

    let (img, col, row) = cursor_cell(CursorShape::HollowBlock);
    let (x, y) = img.origin(col, row);
    assert_eq!(img.at(x, y), CURSOR);
    assert_eq!(img.at(x + m.width - 1, y + m.height - 1), CURSOR);
    assert_eq!(img.at(x + m.width / 2, y + m.height / 2), BG);
}

#[test]
fn images_cover_cell_backgrounds_and_stretch_over_their_cells() {
    const RED: Color = Color::rgb(0xff, 0, 0);
    const BLUE: Color = Color::rgb(0, 0, 0xff);
    const GREEN: Color = Color::rgb(0, 0xff, 0);
    let gpu = require_gpu!();
    let img = render(gpu, |c, r| {
        let mut frame = Frame::new(c, r, FG, BG);
        frame.cell_mut(2, 1).unwrap().bg = GREEN;
        frame.images.push(ImageQuad {
            // 2x1 pixels: red, blue.
            image: ImageData {
                id: 1,
                width: 2,
                height: 1,
                rgba: vec![0xff, 0, 0, 0xff, 0, 0, 0xff, 0xff].into(),
            },
            col: 2,
            row: 1,
            width: 4.0,
            height: 2.0,
            uv: [0.0, 0.0, 1.0, 1.0],
            z: -1,
        });
        frame
    });
    let center = |col, row| {
        let (x, y) = img.origin(col, row);
        img.at(x + img.metrics.width / 2, y + img.metrics.height / 2)
    };
    assert_eq!(center(2, 1), RED);
    assert_eq!(center(5, 2), BLUE);
    assert_eq!(center(6, 1), BG);
    assert_eq!(center(2, 3), BG);
}

#[test]
fn the_background_image_shows_through_default_cells_at_its_opacity() {
    const GREEN: Color = Color::rgb(0, 0xff, 0);
    let gpu = require_gpu!();
    let white = BackgroundImage {
        width: 1,
        height: 1,
        rgba: vec![0xff; 4].into(),
        fit: BackgroundFit::Stretch,
        opacity: 0.5,
    };
    // An sRGB target too: there the blend unit would mix in linear space
    // and make the image look much stronger than half.
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ] {
        let img = render_with(
            gpu,
            format,
            |r| r.set_background_image(&gpu.device, &gpu.queue, Some(&white)),
            |c, r| {
                let mut frame = Frame::new(c, r, FG, BG);
                frame.cell_mut(2, 1).unwrap().bg = GREEN;
                frame
            },
        );
        // Halfway from BG to white, give or take rounding.
        let half = BG.mix(Color::WHITE, 0.5);
        let near = |p: Color| {
            [(p.r, half.r), (p.g, half.g), (p.b, half.b)]
                .iter()
                .all(|&(a, b)| a.abs_diff(b) <= 2)
        };
        assert!(near(img.at(0, 0)), "{format:?} padding: {:?}", img.at(0, 0));
        let (x, y) = img.origin(5, 1);
        assert!(
            near(img.at(x, y)),
            "{format:?} default cell: {:?}",
            img.at(x, y)
        );
        assert!(
            img.cell_pixels(2, 1, 1).iter().all(|&p| p == GREEN),
            "{format:?}"
        );
    }
}

#[test]
fn a_removed_background_image_leaves_the_frame_background() {
    let gpu = require_gpu!();
    let white = BackgroundImage {
        width: 1,
        height: 1,
        rgba: vec![0xff; 4].into(),
        fit: BackgroundFit::Cover,
        opacity: 1.0,
    };
    let img = render_with(
        gpu,
        wgpu::TextureFormat::Rgba8Unorm,
        |r| {
            r.set_background_image(&gpu.device, &gpu.queue, Some(&white));
            r.set_background_image(&gpu.device, &gpu.queue, None);
        },
        |c, r| Frame::new(c, r, FG, BG),
    );
    assert_eq!(img.at(0, 0), BG);
}
