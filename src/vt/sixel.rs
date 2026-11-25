//! DEC Sixel graphics: the data of `DCS P1;P2;P3 q ... ST`, decoded into
//! RGBA as it streams in.

use crate::vt::image::Image;

/// Color registers per image, answered to XTSMGRAPHICS. Each image starts
/// with its own fresh registers, as xterm's private color registers do.
pub(crate) const COLOR_REGISTERS: usize = 1024;

/// Largest width and height in pixels. Pixels beyond are dropped instead of
/// growing the buffer, so a hostile stream cannot use more than
/// 4096 x 4096 x 4 bytes.
pub(crate) const MAX_SIDE: usize = 4096;

/// Numeric parameters kept per command; extra ones are ignored.
const MAX_PARAMS: usize = 5;

/// VT340's default palette in percent, as in xterm. The other registers
/// start black.
const VT340: [[u8; 3]; 16] = [
    [0, 0, 0],
    [20, 20, 80],
    [80, 13, 13],
    [20, 80, 20],
    [80, 20, 80],
    [20, 80, 80],
    [80, 80, 20],
    [53, 53, 53],
    [26, 26, 26],
    [33, 33, 60],
    [60, 26, 26],
    [33, 60, 33],
    [60, 33, 60],
    [33, 60, 60],
    [60, 60, 33],
    [80, 80, 80],
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    Repeat,
    Color,
    Raster,
}

pub(crate) struct Decoder {
    /// P2 = 1: pixels never drawn stay transparent instead of taking the
    /// background color.
    transparent: bool,
    palette: Vec<[u8; 3]>,
    color: [u8; 3],
    /// The command whose parameters are being read, if any.
    command: Option<Command>,
    params: Vec<u32>,
    x: usize,
    /// Top of the current band of six rows.
    y: usize,
    /// Size from the raster attributes (`"`).
    raster: (usize, usize),
    /// Right of the rightmost sixel and bottom of the lowest drawn pixel.
    extent: (usize, usize),
    /// Drawn pixels, `stride` wide, alpha 0 where nothing was drawn.
    pixels: Vec<[u8; 4]>,
    stride: usize,
    drawn: bool,
}

impl Decoder {
    pub(crate) fn new(transparent: bool) -> Self {
        let mut palette = vec![[0; 3]; COLOR_REGISTERS];
        for (reg, rgb) in palette.iter_mut().zip(VT340) {
            *reg = rgb.map(percent);
        }
        Self {
            transparent,
            color: palette[0],
            palette,
            command: None,
            params: Vec::new(),
            x: 0,
            y: 0,
            raster: (0, 0),
            extent: (0, 0),
            pixels: Vec::new(),
            stride: 0,
            drawn: false,
        }
    }

    pub(crate) fn put(&mut self, byte: u8) {
        if let Some(command) = self.command {
            match byte {
                b'0'..=b'9' => {
                    if self.params.is_empty() {
                        self.params.push(0);
                    }
                    let last = self.params.last_mut().unwrap();
                    *last = last
                        .saturating_mul(10)
                        .saturating_add(u32::from(byte - b'0'));
                    return;
                }
                b';' => {
                    if self.params.len() < MAX_PARAMS {
                        self.params.push(0);
                    }
                    return;
                }
                _ => {
                    self.command = None;
                    if command == Command::Repeat {
                        let count = self.param(0).max(1);
                        if let b'?'..=b'~' = byte {
                            return self.sixel(byte - b'?', count);
                        }
                    } else {
                        self.apply(command);
                    }
                }
            }
        }
        match byte {
            b'?'..=b'~' => self.sixel(byte - b'?', 1),
            b'!' => self.start(Command::Repeat),
            b'#' => self.start(Command::Color),
            b'"' => self.start(Command::Raster),
            b'$' => self.x = 0,
            b'-' => {
                self.x = 0;
                self.y = (self.y + 6).min(MAX_SIDE);
            }
            _ => {}
        }
    }

    /// The decoded image, as large as the raster attributes declared or the
    /// drawing reached, whichever is larger; pixels never drawn get
    /// `background` unless P2 asked for transparency. `None` when no pixel
    /// was drawn, even if a size was declared.
    pub(crate) fn finish(mut self, background: [u8; 3]) -> Option<Image> {
        if let Some(command) = self.command.take() {
            self.apply(command);
        }
        if self.extent.1 == 0 {
            return None;
        }
        let width = self.raster.0.max(self.extent.0);
        let height = self.raster.1.max(self.extent.1);
        let fill = if self.transparent {
            [0; 4]
        } else {
            [background[0], background[1], background[2], 0xff]
        };
        let mut rgba = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let pixel = match self.pixels.get(y * self.stride + x) {
                    Some(p) if x < self.stride && p[3] != 0 => *p,
                    _ => fill,
                };
                rgba.extend_from_slice(&pixel);
            }
        }
        Image::from_rgba(width as u32, height as u32, rgba)
    }

    fn start(&mut self, command: Command) {
        self.command = Some(command);
        self.params.clear();
    }

    fn param(&self, i: usize) -> usize {
        self.params.get(i).map_or(0, |&v| v as usize)
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::Repeat => {}
            Command::Color => {
                let register = self.param(0).min(COLOR_REGISTERS - 1);
                if self.params.len() >= 5 {
                    let [a, b, c] = [2, 3, 4].map(|i| self.param(i));
                    match self.param(1) {
                        1 => self.palette[register] = hls(a, b, c),
                        2 => self.palette[register] = [a, b, c].map(|v| percent(v.min(100) as u8)),
                        _ => {}
                    }
                }
                self.color = self.palette[register];
            }
            // xterm ignores raster attributes once drawing has begun: the
            // image size cannot change under pixels already placed.
            Command::Raster if !self.drawn => {
                // Pan;Pad (the pixel aspect ratio) are ignored as in foot
                // and wezterm: pixels are square on today's screens.
                // Not reserved here: a declared size alone must not cost
                // memory, so only `sixel` grows the buffer.
                self.raster = (self.param(2).min(MAX_SIDE), self.param(3).min(MAX_SIDE));
            }
            Command::Raster => {}
        }
    }

    /// Draws `count` copies of the six vertical pixels in `bits` (bit 0 at
    /// the top) and moves right past them.
    fn sixel(&mut self, bits: u8, count: usize) {
        self.drawn = true;
        let end = self.x.saturating_add(count).min(MAX_SIDE);
        let bottom = (self.y + 6).min(MAX_SIDE);
        if bits != 0 && self.x < end && self.y < bottom {
            self.reserve(end, bottom);
            for dy in 0..bottom - self.y {
                if bits & (1 << dy) == 0 {
                    continue;
                }
                let row = (self.y + dy) * self.stride;
                let [r, g, b] = self.color;
                self.pixels[row + self.x..row + end].fill([r, g, b, 0xff]);
                self.extent.1 = self.extent.1.max(self.y + dy + 1);
            }
        }
        self.x = end;
        self.extent.0 = self.extent.0.max(end);
    }

    /// Grows the buffer to at least `width` x `height`, doubling so that a
    /// stream without raster attributes does not copy on every band.
    fn reserve(&mut self, width: usize, height: usize) {
        let rows = self.pixels.len().checked_div(self.stride).unwrap_or(0);
        if width <= self.stride && height <= rows {
            return;
        }
        let stride = if width > self.stride {
            width.max(self.stride * 2).min(MAX_SIDE)
        } else {
            self.stride
        };
        let rows_new = if height > rows {
            height.max(rows * 2).min(MAX_SIDE)
        } else {
            rows
        };
        let mut pixels = vec![[0; 4]; stride * rows_new];
        for y in 0..rows {
            let old = &self.pixels[y * self.stride..(y + 1) * self.stride];
            pixels[y * stride..y * stride + self.stride].copy_from_slice(old);
        }
        self.pixels = pixels;
        self.stride = stride;
    }
}

/// 0..=100 to 0..=255.
fn percent(v: u8) -> u8 {
    ((u16::from(v.min(100)) * 255 + 50) / 100) as u8
}

/// DEC HLS (hue 0..=360 with blue at 0 and red at 120, lightness and
/// saturation 0..=100) to RGB.
fn hls(hue: usize, lightness: usize, saturation: usize) -> [u8; 3] {
    let h = ((hue % 360 + 240) % 360) as f32 / 60.0;
    let l = lightness.min(100) as f32 / 100.0;
    let s = saturation.min(100) as f32 / 100.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r, g, b].map(|v| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BG: [u8; 3] = [1, 2, 3];

    fn decode(transparent: bool, data: &str) -> Option<Image> {
        let mut d = Decoder::new(transparent);
        for &b in data.as_bytes() {
            d.put(b);
        }
        d.finish(BG)
    }

    fn pixel(image: &Image, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * image.width() + x) * 4) as usize;
        image.rgba()[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn a_sixel_paints_six_pixels_downward_from_bit_0() {
        // '@' = bit 0 only, 'A' = bit 1 only.
        let img = decode(true, "#1;2;100;0;0@A").unwrap();
        assert_eq!((img.width(), img.height()), (2, 2));
        assert_eq!(pixel(&img, 0, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(&img, 1, 0), [0; 4]);
        assert_eq!(pixel(&img, 1, 1), [255, 0, 0, 255]);
    }

    #[test]
    fn repeat_dollar_and_dash_move_the_drawing_position() {
        let img = decode(true, "#2;2;0;100;0!3~$#3;2;0;0;100@-!2N").unwrap();
        assert_eq!((img.width(), img.height()), (3, 10));
        // '$' returned to column 0, where blue overwrote green's top pixel.
        assert_eq!(pixel(&img, 0, 0), [0, 0, 255, 255]);
        assert_eq!(pixel(&img, 2, 5), [0, 255, 0, 255]);
        // 'N' = bits 0-3 of the second band, two columns wide.
        assert_eq!(pixel(&img, 1, 9), [0, 0, 255, 255]);
        assert_eq!(pixel(&img, 2, 9), [0; 4]);
    }

    #[test]
    fn hls_hue_starts_at_blue_and_red_is_at_120() {
        assert_eq!(hls(0, 50, 100), [0, 0, 255]);
        assert_eq!(hls(120, 50, 100), [255, 0, 0]);
        assert_eq!(hls(240, 50, 100), [0, 255, 0]);
        assert_eq!(hls(0, 100, 0), [255, 255, 255]);
    }

    #[test]
    fn color_registers_start_with_the_vt340_palette_and_register_0_selected() {
        let img = decode(true, "#4~").unwrap();
        assert_eq!(pixel(&img, 0, 0), [204, 51, 204, 255]);
        let img = decode(true, "~").unwrap();
        assert_eq!(pixel(&img, 0, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn raster_attributes_set_the_size_and_unset_pixels_take_the_background() {
        let img = decode(false, "\"1;1;4;3#1;2;100;100;100@").unwrap();
        assert_eq!((img.width(), img.height()), (4, 3));
        assert_eq!(pixel(&img, 0, 0), [255, 255, 255, 255]);
        assert_eq!(pixel(&img, 3, 2), [1, 2, 3, 255]);
    }

    #[test]
    fn raster_attributes_after_drawing_are_ignored() {
        let img = decode(true, "~\"1;1;9;9").unwrap();
        assert_eq!((img.width(), img.height()), (1, 6));
    }

    #[test]
    fn nothing_drawn_yields_no_image() {
        assert!(decode(true, "#1;2;1;2;3$-").is_none());
    }

    #[test]
    fn a_declared_size_alone_allocates_nothing_and_yields_no_image() {
        let mut d = Decoder::new(false);
        for &b in b"\"1;1;4096;4096?$-" {
            d.put(b);
        }
        assert!(d.pixels.is_empty());
        assert!(d.finish(BG).is_none());
    }

    #[test]
    fn huge_sizes_are_cut_at_the_maximum_side() {
        let img = decode(true, "\"1;1;99999;1!999999~").unwrap();
        assert_eq!((img.width(), img.height()), (MAX_SIDE as u32, 6));
        let mut d = Decoder::new(true);
        for _ in 0..1000 {
            d.put(b'-');
        }
        d.put(b'~');
        assert!(d.finish(BG).is_none());
    }

    #[test]
    fn the_buffer_grows_without_losing_drawn_pixels() {
        let data = format!("#1;2;100;0;0~{}!100~", "-".repeat(20));
        let img = decode(true, &data).unwrap();
        assert_eq!((img.width(), img.height()), (100, 126));
        assert_eq!(pixel(&img, 0, 0), [255, 0, 0, 255]);
        assert_eq!(pixel(&img, 99, 125), [255, 0, 0, 255]);
        assert_eq!(pixel(&img, 1, 0), [0; 4]);
    }
}
