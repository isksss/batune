//! Font lookup, rasterization and the CPU side of the glyph atlas.

use std::collections::HashMap;
use std::sync::Arc;

use cosmic_text::{
    Attrs, Buffer, CacheKeyFlags, Family, FontSystem, Metrics, Shaping, Style, Weight,
};
use swash::scale::image::Content;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Angle, Format, Transform};

use crate::render::atlas::{Rect, ShelfPacker};
use crate::render::metrics::{CellMetrics, FontMetrics};

/// Font selection. Sizes are in logical pixels; the renderer multiplies
/// them by the window's scale factor.
#[derive(Clone, Debug, PartialEq)]
pub struct FontConfig {
    /// Family name of the primary font; `None` uses the built-in
    /// [`DEFAULT_FAMILY`]. Characters it lacks (emoji, ...) fall back to
    /// whatever installed font covers them.
    pub family: Option<String>,
    pub size: f32,
    /// Cell height as a multiple of the font's line height.
    pub line_height: f32,
    /// Cell width as a multiple of the font's advance.
    pub cell_width: f32,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: None,
            size: 14.0,
            line_height: 1.0,
            cell_width: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GlyphKey {
    pub ch: char,
    /// Marks shaped together with `ch`; see `CellView::zerowidth`.
    pub zerowidth: Option<Arc<[char]>>,
    pub bold: bool,
    pub italic: bool,
}

/// A rasterized glyph positioned relative to the pen at the baseline.
#[derive(Debug)]
pub(crate) struct RasterGlyph {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    /// Color glyphs (emoji) keep their own colors; masks take the cell's fg.
    pub color: bool,
    /// Premultiplied RGBA. Masks are stored as white so one atlas format
    /// serves both kinds.
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy)]
struct ShapedGlyph {
    font_id: cosmic_text::fontdb::ID,
    glyph_id: u16,
    weight: Weight,
    fake_italic: bool,
    /// Pen position relative to the first glyph's, in pixels, y up.
    x: f32,
    y: f32,
    advance: f32,
}

/// Color emoji families, most common first. cosmic-text ignores VS16
/// (emoji presentation) when it picks a font, so a character the primary
/// font also covers, such as U+2764, would stay a text glyph without
/// asking for one of these by name.
const EMOJI_FAMILIES: &[&str] = &[
    "Noto Color Emoji",
    "Apple Color Emoji",
    "Segoe UI Emoji",
    "Twemoji",
];

const EMOJI_PRESENTATION: char = '\u{FE0F}';
const TEXT_PRESENTATION: char = '\u{FE0E}';

/// How a cell asks to be drawn (UTS #51): a variation selector decides,
/// otherwise the base character's `Emoji_Presentation` property does.
/// kitty, WezTerm and Ghostty choose the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Presentation {
    Text,
    Emoji,
}

impl Presentation {
    fn of(key: &GlyphKey) -> Self {
        let selector = key
            .zerowidth
            .iter()
            .flat_map(|z| z.iter())
            .find_map(|&c| match c {
                EMOJI_PRESENTATION => Some(Self::Emoji),
                TEXT_PRESENTATION => Some(Self::Text),
                _ => None,
            });
        selector.unwrap_or(if is_emoji_presentation(key.ch) {
            Self::Emoji
        } else {
            Self::Text
        })
    }
}

/// unicode-properties is a no_std table crate from unicode-rs (the same
/// people as unicode-width). Why not the alternatives: unicode-width keeps
/// its emoji table private; a hand-copied range list would go stale with
/// each Unicode release; `emojis` carries names and sequences we never use.
fn is_emoji_presentation(ch: char) -> bool {
    use unicode_properties::{EmojiStatus, UnicodeEmoji};
    matches!(
        ch.emoji_status(),
        EmojiStatus::EmojiPresentation
            | EmojiStatus::EmojiPresentationAndModifierBase
            | EmojiStatus::EmojiPresentationAndEmojiComponent
            | EmojiStatus::EmojiPresentationAndModifierAndEmojiComponent
    )
}

/// Color glyph tables. A face with any of them is what fontconfig calls
/// `color=true`, which kitty and Ghostty exclude when looking for a text
/// presentation fallback.
const COLOR_TABLES: [&[u8; 4]; 4] = [b"COLR", b"CBDT", b"sbix", b"SVG "];

fn is_color_face(font: &swash::FontRef<'_>) -> bool {
    COLOR_TABLES
        .iter()
        .any(|t| font.table(swash::Tag::from_be_bytes(**t)).is_some())
}

/// Family of the fonts built into the binary, used when
/// [`FontConfig::family`] is `None` so the terminal looks the same without
/// anything installed.
pub const DEFAULT_FAMILY: &str = "HackGen Console NF";

// HackGen Console NF v2.10.0 (SIL OFL 1.1, see fonts/LICENSE). Embedded as
// plain TTF, not compressed: the release tar.gz/zip/MSIX are compressed
// anyway, so compressing here would save ~16 MB on disk but almost nothing
// to download, while it would cost a decompressor dependency, startup time
// and ~26 MB of heap; a `&'static` slice lets fontdb read the binary's pages
// only as glyphs are needed.
static BUILTIN_FONTS: [&[u8]; 2] = [
    include_bytes!("../../fonts/HackGenConsoleNF-Regular.ttf"),
    include_bytes!("../../fonts/HackGenConsoleNF-Bold.ttf"),
];

/// The regular face of [`DEFAULT_FAMILY`], for UI drawn outside the grid
/// (the desktop's tab bar) so it can show the same characters.
pub fn builtin_font() -> &'static [u8] {
    BUILTIN_FONTS[0]
}

pub(crate) struct Fonts {
    system: FontSystem,
    scale: ScaleContext,
    buffer: Buffer,
    family: String,
    /// First installed entry of [`EMOJI_FAMILIES`].
    emoji: Option<String>,
    /// Per face: whether it carries color glyphs.
    color_faces: HashMap<cosmic_text::fontdb::ID, bool>,
    /// Per character: a family without color glyphs that covers it, for
    /// text presentation when fallback lands on an emoji font.
    text_fallbacks: HashMap<char, Option<String>>,
    size: f32,
    /// `FontConfig::cell_width` and `line_height`.
    spacing: (f32, f32),
    metrics: CellMetrics,
}

impl Fonts {
    /// Scans the system fonts, which takes tens of milliseconds; create
    /// once and use [`Fonts::set_size`] for zoom and DPI changes.
    pub fn new(family: Option<String>, size: f32) -> Self {
        // Passed to the constructor rather than added through `db_mut`
        // afterwards, which would leave them out of cosmic-text's monospace
        // fallback lists built there.
        let mut system = FontSystem::new_with_fonts(
            BUILTIN_FONTS.map(|data| cosmic_text::fontdb::Source::Binary(Arc::new(data))),
        );
        let family = family.unwrap_or_else(|| DEFAULT_FAMILY.to_owned());
        let buffer = Buffer::new(&mut system, Metrics::new(size, size));
        let emoji = EMOJI_FAMILIES
            .iter()
            .find(|name| {
                system
                    .db()
                    .faces()
                    .any(|f| f.families.iter().any(|(n, _)| n == *name))
            })
            .map(|name| (*name).to_owned());
        let mut fonts = Self {
            system,
            scale: ScaleContext::new(),
            buffer,
            family,
            emoji,
            color_faces: HashMap::new(),
            text_fallbacks: HashMap::new(),
            size,
            spacing: (1.0, 1.0),
            metrics: CellMetrics::from_font(&FontMetrics {
                ascent: size,
                descent: 0.0,
                leading: 0.0,
                advance: size / 2.0,
                underline_offset: 0.0,
                strikeout_offset: size / 2.0,
                stroke_size: 1.0,
            }),
        };
        fonts.set_size(size);
        fonts
    }

    pub fn set_size(&mut self, size: f32) {
        self.size = size;
        self.buffer.set_metrics(Metrics::new(size, size));
        if let Some(m) = self.font_metrics() {
            let (width, height) = self.spacing;
            self.metrics = CellMetrics::from_font(&m).scaled(width, height);
        }
    }

    /// Takes effect at the next [`Fonts::set_size`].
    pub fn set_spacing(&mut self, cell_width: f32, line_height: f32) {
        self.spacing = (cell_width, line_height);
    }

    pub fn cell_metrics(&self) -> CellMetrics {
        self.metrics
    }

    /// Shapes a cell's text to find the fonts that cover it and where the
    /// marks sit. Going through the shaper (rather than querying fontdb for
    /// a codepoint) reuses cosmic-text's script-aware fallback lists.
    fn shape(&mut self, key: &GlyphKey) -> Vec<ShapedGlyph> {
        let primary = self.family.clone();
        match Presentation::of(key) {
            Presentation::Emoji => {
                let Some(emoji) = self.emoji.clone() else {
                    return self.shape_with(key, &primary);
                };
                let shaped = self.shape_with(key, &emoji);
                let intact = shaped.iter().all(|g| g.glyph_id != 0)
                    && shaped
                        .first()
                        .is_some_and(|g| self.is_color(g.font_id, g.weight));
                if intact {
                    return shaped;
                }
                self.shape_marks_apart(key, &emoji, &primary)
                    .unwrap_or(shaped)
            }
            Presentation::Text => {
                let shaped = self.shape_with(key, &primary);
                let landed_on_color = shaped
                    .first()
                    .is_some_and(|g| self.is_color(g.font_id, g.weight));
                if !landed_on_color {
                    return shaped;
                }
                // cosmic-text's fallback cannot be told to skip color fonts
                // for one run (its forbidden list is per FontSystem), so
                // find a text font ourselves and ask for it by name. With
                // none installed the emoji glyph beats drawing nothing.
                match self.text_fallback(key.ch) {
                    Some(family) => self.shape_with(key, &family),
                    None => shaped,
                }
            }
        }
    }

    /// An emoji font rarely has the combining marks a cell carries, and
    /// cosmic-text then moves the whole cluster to a text font (a gray ❤
    /// for U+2764 U+FE0F U+0301) or leaves the mark as .notdef. Instead,
    /// draw the base from the emoji font and the marks from text fonts,
    /// shaped on U+25CC (the dotted circle shapers use for a lone mark)
    /// that is then dropped and centered on the emoji. `None` when the
    /// emoji font does not have the base either.
    fn shape_marks_apart(
        &mut self,
        key: &GlyphKey,
        emoji: &str,
        primary: &str,
    ) -> Option<Vec<ShapedGlyph>> {
        let marks: Vec<char> = key
            .zerowidth
            .iter()
            .flat_map(|z| z.iter().copied())
            .filter(|&c| c != EMOJI_PRESENTATION && c != TEXT_PRESENTATION)
            .collect();
        let mut base = self.shape_with(
            &GlyphKey {
                zerowidth: None,
                ..key.clone()
            },
            emoji,
        );
        let first = base.first()?;
        if first.glyph_id == 0 || !self.is_color(first.font_id, first.weight) {
            return None;
        }
        let base_advance = first.advance;
        if marks.is_empty() {
            return Some(base);
        }
        let carrier = self.shape_with(
            &GlyphKey {
                ch: '\u{25CC}',
                zerowidth: Some(marks.into()),
                ..key.clone()
            },
            primary,
        );
        let Some((circle, marks)) = carrier.split_first() else {
            return Some(base);
        };
        let dx = (base_advance - circle.advance) / 2.0;
        base.extend(marks.iter().map(|g| ShapedGlyph { x: g.x + dx, ..*g }));
        Some(base)
    }

    fn shape_with(&mut self, key: &GlyphKey, family: &str) -> Vec<ShapedGlyph> {
        let attrs = Attrs::new()
            .family(Family::Name(family))
            .weight(if key.bold {
                Weight::BOLD
            } else {
                Weight::NORMAL
            })
            .style(if key.italic {
                Style::Italic
            } else {
                Style::Normal
            });
        let mut text = String::from(key.ch);
        text.extend(key.zerowidth.iter().flat_map(|z| z.iter()));
        self.buffer.set_text(&text, &attrs, Shaping::Advanced, None);
        self.buffer.shape_until_scroll(&mut self.system, false);
        let Some(run) = self.buffer.layout_runs().next() else {
            return Vec::new();
        };
        let origin = run.glyphs.first().map_or(0.0, |g| g.x);
        run.glyphs
            .iter()
            .map(|g| ShapedGlyph {
                font_id: g.font_id,
                glyph_id: g.glyph_id,
                weight: g.font_weight,
                fake_italic: g.cache_key_flags.contains(CacheKeyFlags::FAKE_ITALIC),
                x: g.x - origin + g.font_size * g.x_offset,
                y: g.font_size * g.y_offset - g.y,
                advance: g.w,
            })
            .collect()
    }

    fn is_color(&mut self, id: cosmic_text::fontdb::ID, weight: Weight) -> bool {
        if let Some(&color) = self.color_faces.get(&id) {
            return color;
        }
        let color = self
            .system
            .get_font(id, weight)
            .is_some_and(|f| is_color_face(&f.as_swash()));
        self.color_faces.insert(id, color);
        color
    }

    /// The first face without color glyphs that maps `ch`, monospaced
    /// faces first. Reads each face's cmap through fontdb's memory map
    /// rather than `FontSystem::get_font`, which would keep every font it
    /// opens in memory.
    fn text_fallback(&mut self, ch: char) -> Option<String> {
        if let Some(found) = self.text_fallbacks.get(&ch) {
            return found.clone();
        }
        let db = self.system.db();
        let mut faces: Vec<_> = db
            .faces()
            .filter(|f| {
                !f.families
                    .iter()
                    .any(|(n, _)| EMOJI_FAMILIES.contains(&n.as_str()))
            })
            .collect();
        faces.sort_by_key(|f| !f.monospaced);
        let found = faces.into_iter().find_map(|f| {
            let fits = db.with_face_data(f.id, |data, index| {
                swash::FontRef::from_index(data, index as usize)
                    .is_some_and(|font| font.charmap().map(ch) != 0 && !is_color_face(&font))
            });
            fits.unwrap_or(false)
                .then(|| f.families.first().map(|(n, _)| n.clone()))
                .flatten()
        });
        self.text_fallbacks.insert(ch, found.clone());
        found
    }

    fn font_metrics(&mut self) -> Option<FontMetrics> {
        let shaped = self
            .shape(&GlyphKey {
                ch: 'M',
                zerowidth: None,
                bold: false,
                italic: false,
            })
            .into_iter()
            .next()?;
        let font = self.system.get_font(shaped.font_id, shaped.weight)?;
        let face = font.as_swash();
        let m = face.metrics(&[]).scale(self.size);
        let advance = face
            .glyph_metrics(&[])
            .scale(self.size)
            .advance_width(shaped.glyph_id);
        Some(FontMetrics {
            ascent: m.ascent,
            descent: m.descent,
            leading: m.leading,
            advance,
            underline_offset: m.underline_offset,
            strikeout_offset: m.strikeout_offset,
            stroke_size: m.stroke_size,
        })
    }

    /// Empty for characters with no visible pixels (space) or no glyph in
    /// any installed font. A cell whose text shapes to several glyphs (a
    /// base and marks the font does not compose) becomes at most two
    /// images; see [`layers`].
    pub fn rasterize(&mut self, key: &GlyphKey) -> Vec<RasterGlyph> {
        let mut parts = Vec::new();
        for shaped in self.shape(key) {
            let Some(glyph) = self.rasterize_one(key.bold, &shaped) else {
                continue;
            };
            parts.push(RasterGlyph {
                left: glyph.left + shaped.x.round() as i32,
                top: glyph.top + shaped.y.round() as i32,
                ..glyph
            });
        }
        layers(parts)
    }

    fn rasterize_one(&mut self, bold: bool, shaped: &ShapedGlyph) -> Option<RasterGlyph> {
        let font = self.system.get_font(shaped.font_id, shaped.weight)?;
        // Synthesize bold only when fallback landed on a lighter face;
        // emboldening a real bold face would smear it.
        let fake_bold = bold
            && self
                .system
                .db()
                .face(shaped.font_id)
                .is_some_and(|f| f.weight.0 < 600);
        let face = font.as_swash();
        let mut builder = self.scale.builder(face).size(self.size).hint(true);
        let wght = swash::Tag::from_be_bytes(*b"wght");
        if let Some(axis) = face.variations().find_by_tag(wght) {
            let value = f32::from(shaped.weight.0).clamp(axis.min_value(), axis.max_value());
            builder =
                builder.normalized_coords(face.variations().normalized_coords([(wght, value)]));
        }
        let mut scaler = builder.build();
        let image = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ])
        .format(Format::Alpha)
        .transform(
            shaped
                .fake_italic
                .then(|| Transform::skew(Angle::from_degrees(14.0), Angle::ZERO)),
        )
        .embolden(if fake_bold { self.size / 28.0 } else { 0.0 })
        .render(&mut scaler, shaped.glyph_id)?;
        let p = image.placement;
        if p.width == 0 || p.height == 0 {
            return None;
        }
        let (color, rgba) = match image.content {
            Content::Mask => (false, image.data.iter().flat_map(|&a| [a; 4]).collect()),
            Content::Color => (true, premultiply(image.data)),
            Content::SubpixelMask => return None,
        };
        Some(self.fit_color_glyph(RasterGlyph {
            left: p.left,
            top: p.top,
            width: p.width,
            height: p.height,
            color,
            rgba,
        }))
    }

    /// Bitmap emoji fonts only ship a few fixed strikes (Noto Color Emoji
    /// has just 109px), so swash may hand back an image several times the
    /// cell height. Shrink it to one line, keeping the aspect ratio.
    fn fit_color_glyph(&self, glyph: RasterGlyph) -> RasterGlyph {
        let target_h = self.metrics.height;
        if !glyph.color || glyph.height <= target_h + target_h / 4 {
            return glyph;
        }
        let scale = target_h as f32 / glyph.height as f32;
        let w = ((glyph.width as f32 * scale).round() as u32).max(1);
        let h = target_h;
        let rgba = box_downscale(&glyph.rgba, glyph.width, glyph.height, w, h);
        RasterGlyph {
            // Center vertically in the cell instead of trusting the
            // strike's own bearings, which are for the unscaled size.
            left: (glyph.left as f32 * scale).round() as i32,
            top: self.metrics.baseline as i32,
            width: w,
            height: h,
            color: true,
            rgba,
        }
    }
}

/// Merges a cell's parts into one image per kind, in the order each kind
/// first appears, so a mark shaped after its base is drawn over it. Masks
/// stay white and separate from color parts, letting the renderer tint
/// them with the cell's fg as it does any mask. Why not tint the masks
/// here and keep one image: the fg would have to join the glyph key, so
/// every color (and the cursor's inverted fg passing over the cell) would
/// rasterize the cell again and take more atlas space.
fn layers(parts: Vec<RasterGlyph>) -> Vec<RasterGlyph> {
    let color_first = parts.first().is_some_and(|g| g.color);
    let (color, mask): (Vec<_>, Vec<_>) = parts.into_iter().partition(|g| g.color);
    let groups = if color_first {
        [color, mask]
    } else {
        [mask, color]
    };
    groups
        .into_iter()
        .filter_map(|mut group| match group.len() {
            0 => None,
            1 => group.pop(),
            _ => Some(composite(&group)),
        })
        .collect()
}

/// Draws `parts`, all of one kind, over each other into one image.
fn composite(parts: &[RasterGlyph]) -> RasterGlyph {
    let left = parts.iter().map(|g| g.left).min().unwrap_or(0);
    let top = parts.iter().map(|g| g.top).max().unwrap_or(0);
    let right = parts
        .iter()
        .map(|g| g.left + g.width as i32)
        .max()
        .unwrap_or(0);
    let bottom = parts
        .iter()
        .map(|g| g.top - g.height as i32)
        .min()
        .unwrap_or(0);
    let (width, height) = ((right - left) as u32, (top - bottom) as u32);
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for g in parts {
        let (ox, oy) = ((g.left - left) as u32, (top - g.top) as u32);
        for y in 0..g.height {
            for x in 0..g.width {
                let s = ((y * g.width + x) * 4) as usize;
                let d = (((oy + y) * width + ox + x) * 4) as usize;
                let a = u16::from(g.rgba[s + 3]);
                for c in 0..4 {
                    let under = u16::from(rgba[d + c]) * (255 - a) / 255;
                    rgba[d + c] = (u16::from(g.rgba[s + c]) + under).min(255) as u8;
                }
            }
        }
    }
    RasterGlyph {
        left,
        top,
        width,
        height,
        color: parts.first().is_some_and(|g| g.color),
        rgba,
    }
}

fn premultiply(mut rgba: Vec<u8>) -> Vec<u8> {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = u16::from(px[3]);
        for c in &mut px[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
    }
    rgba
}

/// Averages every source pixel that falls into each destination pixel.
/// Inputs are premultiplied, so averaging the channels is correct.
fn box_downscale(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dw * dh * 4) as usize];
    for dy in 0..dh {
        let (y0, y1) = (dy * sh / dh, ((dy + 1) * sh / dh).max(dy * sh / dh + 1));
        for dx in 0..dw {
            let (x0, x1) = (dx * sw / dw, ((dx + 1) * sw / dw).max(dx * sw / dw + 1));
            let mut sum = [0u32; 4];
            for y in y0..y1.min(sh) {
                for x in x0..x1.min(sw) {
                    let i = ((y * sw + x) * 4) as usize;
                    for c in 0..4 {
                        sum[c] += u32::from(src[i + c]);
                    }
                }
            }
            let n = (x1.min(sw) - x0) * (y1.min(sh) - y0);
            let o = ((dy * dw + dx) * 4) as usize;
            for c in 0..4 {
                out[o + c] = (sum[c] / n.max(1)) as u8;
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CachedGlyph {
    pub rect: Rect,
    pub left: i32,
    pub top: i32,
    pub color: bool,
}

/// A cell's atlas images in drawing order: one, or a color one and a mask
/// one when the cell mixes both (see [`layers`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CellGlyphs([Option<CachedGlyph>; 2]);

impl CellGlyphs {
    pub fn iter(&self) -> impl Iterator<Item = &CachedGlyph> {
        self.0.iter().flatten()
    }
}

pub(crate) struct Upload {
    pub rect: Rect,
    pub rgba: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AtlasFull;

/// Maps glyphs to atlas rectangles and queues the pixels the GPU texture
/// still needs.
/// Entries kept besides those the atlas bounds; see [`GlyphCache::get`].
const MAX_ENTRIES: usize = 1 << 16;

pub(crate) struct GlyphCache {
    packer: ShelfPacker,
    entries: HashMap<GlyphKey, CellGlyphs>,
    uploads: Vec<Upload>,
}

impl GlyphCache {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            packer: ShelfPacker::new(width, height),
            entries: HashMap::new(),
            uploads: Vec::new(),
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.packer.size()
    }

    pub fn get(&mut self, fonts: &mut Fonts, key: &GlyphKey) -> Result<CellGlyphs, AtlasFull> {
        if let Some(entry) = self.entries.get(key) {
            return Ok(*entry);
        }
        if key.ch == ' ' && key.zerowidth.is_none() {
            self.entries.insert(key.clone(), CellGlyphs::default());
            return Ok(CellGlyphs::default());
        }
        let images = fonts.rasterize(key);
        // Cells without an image take no atlas space, so a full atlas never
        // clears them; past the cap they are rasterized again each time
        // rather than letting distinct marks from the output pile up.
        if images.is_empty() && self.entries.len() >= MAX_ENTRIES {
            return Ok(CellGlyphs::default());
        }
        // A full atlas takes none of the layers, so the renderer can grow
        // it and ask again without a half-cached cell or unused space.
        let sizes: Vec<_> = images.iter().map(|g| (g.width, g.height)).collect();
        let rects = self.packer.allocate_all(&sizes).ok_or(AtlasFull)?;
        let mut entry = CellGlyphs::default();
        for ((glyph, rect), slot) in images.into_iter().zip(rects).zip(&mut entry.0) {
            *slot = Some(CachedGlyph {
                rect,
                left: glyph.left,
                top: glyph.top,
                color: glyph.color,
            });
            self.uploads.push(Upload {
                rect,
                rgba: glyph.rgba,
            });
        }
        self.entries.insert(key.clone(), entry);
        Ok(entry)
    }

    pub fn take_uploads(&mut self) -> Vec<Upload> {
        std::mem::take(&mut self.uploads)
    }

    pub fn grow(&mut self, height: u32) {
        self.packer.grow(height);
    }

    /// Forgets every glyph, e.g. after a font size change or when the atlas
    /// cannot grow any further.
    pub fn clear(&mut self) {
        self.packer.clear();
        self.entries.clear();
        self.uploads.clear();
    }
}

#[cfg(test)]
mod tests;
