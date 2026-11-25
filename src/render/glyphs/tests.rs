use super::*;

impl CellGlyphs {
    /// The cell's only image; fails on a cell with two layers.
    fn single(self) -> Option<CachedGlyph> {
        assert!(self.0[1].is_none(), "{self:?}");
        self.0[0]
    }
}

fn key(ch: char) -> GlyphKey {
    GlyphKey {
        ch,
        zerowidth: None,
        bold: false,
        italic: false,
    }
}

fn with_marks(ch: char, marks: &[char]) -> GlyphKey {
    GlyphKey {
        zerowidth: Some(marks.into()),
        ..key(ch)
    }
}

#[test]
fn premultiply_scales_color_by_alpha() {
    assert_eq!(premultiply(vec![255, 128, 0, 128]), vec![128, 64, 0, 128]);
}

fn part(left: i32, top: i32, px: [u8; 4]) -> RasterGlyph {
    RasterGlyph {
        left,
        top,
        width: 1,
        height: 1,
        color: false,
        rgba: px.to_vec(),
    }
}

#[test]
fn composite_places_parts_by_their_bearings() {
    let base = part(0, 2, [255; 4]);
    let mark = part(1, 4, [128; 4]);
    let out = composite(&[base, mark]);
    assert_eq!((out.left, out.top, out.width, out.height), (0, 4, 2, 3));
    let px = |x: u32, y: u32| &out.rgba[((y * 2 + x) * 4) as usize..][..4];
    assert_eq!(px(1, 0), [128; 4]);
    assert_eq!(px(0, 2), [255; 4]);
    assert_eq!(px(0, 0), [0; 4]);
}

#[test]
fn composite_blends_overlapping_parts_over_each_other() {
    let out = composite(&[part(0, 1, [255, 0, 0, 255]), part(0, 1, [0, 0, 128, 128])]);
    assert_eq!(out.rgba, vec![127, 0, 128, 255]);
    assert!(!out.color);
}

fn color_part(left: i32, top: i32, px: [u8; 4]) -> RasterGlyph {
    RasterGlyph {
        color: true,
        ..part(left, top, px)
    }
}

#[test]
fn a_cell_of_one_kind_is_one_layer() {
    let out = layers(vec![part(0, 1, [255; 4]), part(1, 1, [128; 4])]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].width, 2);
}

#[test]
fn mixed_cells_keep_masks_white_in_their_own_layer() {
    let out = layers(vec![
        color_part(0, 1, [200, 0, 0, 255]),
        part(1, 1, [255; 4]),
        color_part(1, 1, [0, 0, 200, 255]),
    ]);
    let kinds: Vec<_> = out.iter().map(|g| (g.color, g.width)).collect();
    assert_eq!(kinds, [(true, 2), (false, 1)]);
    assert_eq!(out[1].rgba, [255; 4]);
}

#[test]
fn the_first_part_decides_which_layer_is_drawn_first() {
    let out = layers(vec![part(0, 1, [255; 4]), color_part(0, 1, [9, 9, 9, 255])]);
    assert_eq!(
        out.iter().map(|g| g.color).collect::<Vec<_>>(),
        [false, true]
    );
}

#[test]
fn box_downscale_averages_blocks() {
    let src = [
        [255u8, 0, 0, 255],
        [0, 0, 255, 255],
        [255, 0, 0, 255],
        [0, 0, 255, 255],
    ]
    .concat();
    assert_eq!(box_downscale(&src, 2, 2, 1, 1), vec![127, 0, 127, 255]);
}

/// Family and PostScript name of the face `key` is drawn with.
fn face_of(fonts: &mut Fonts, key: &GlyphKey) -> (String, String) {
    let shaped = fonts.shape(key);
    let face = fonts
        .system
        .db()
        .face(shaped.first().expect("shaped glyph").font_id)
        .expect("face in the database");
    (face.families[0].0.clone(), face.post_script_name.clone())
}

#[test]
fn without_a_family_the_builtin_font_is_used() {
    let mut fonts = Fonts::new(None, 16.0);
    assert_eq!(
        face_of(&mut fonts, &key('A')),
        (
            DEFAULT_FAMILY.to_owned(),
            "HackGenConsoleNF-Regular".to_owned()
        )
    );
    let bold = GlyphKey {
        bold: true,
        ..key('A')
    };
    assert_eq!(face_of(&mut fonts, &bold).1, "HackGenConsoleNF-Bold");
}

#[test]
fn the_builtin_font_covers_japanese_itself() {
    let mut fonts = Fonts::new(None, 16.0);
    assert_eq!(face_of(&mut fonts, &key('あ')).0, DEFAULT_FAMILY);
    assert_eq!(face_of(&mut fonts, &key('漢')).0, DEFAULT_FAMILY);
}

// The tests below that name a family or draw emoji need those fonts
// installed; the CI image has DejaVu and Noto Color Emoji.

#[test]
fn cell_metrics_come_from_the_primary_font() {
    let fonts = Fonts::new(None, 16.0);
    let m = fonts.cell_metrics();
    assert!((6..=12).contains(&m.width), "{m:?}");
    assert!((15..=24).contains(&m.height), "{m:?}");
    assert!(m.baseline < m.height);
}

#[test]
fn an_explicit_family_replaces_the_builtin_font() {
    let mut fonts = Fonts::new(Some("DejaVu Sans Mono".into()), 16.0);
    assert_eq!(face_of(&mut fonts, &key('A')).0, "DejaVu Sans Mono");
}

#[test]
fn spacing_scales_the_cell_at_the_next_size() {
    let mut fonts = Fonts::new(None, 16.0);
    let plain = fonts.cell_metrics();
    fonts.set_spacing(2.0, 1.5);
    assert_eq!(fonts.cell_metrics(), plain);
    fonts.set_size(16.0);
    assert_eq!(fonts.cell_metrics(), plain.scaled(2.0, 1.5));
    assert_eq!(fonts.cell_metrics().width, plain.width * 2);
}

#[test]
fn glyphs_are_cached_and_uploaded_once() {
    let mut fonts = Fonts::new(None, 16.0);
    let mut cache = GlyphCache::new(256, 256);
    let a = cache
        .get(&mut fonts, &key('A'))
        .unwrap()
        .single()
        .expect("glyph for A");
    assert_eq!(cache.take_uploads().len(), 1);
    assert_eq!(cache.get(&mut fonts, &key('A')).unwrap().single(), Some(a));
    assert!(cache.take_uploads().is_empty());
    assert!(!a.color);
    assert!(a.top > 0 && a.rect.h > 0);
}

#[test]
fn combining_marks_are_drawn_with_their_base() {
    let mut fonts = Fonts::new(None, 16.0);
    let mut cache = GlyphCache::new(256, 256);
    let plain = cache
        .get(&mut fonts, &key('e'))
        .unwrap()
        .single()
        .expect("e");
    let acute = cache
        .get(&mut fonts, &with_marks('e', &['\u{301}']))
        .unwrap()
        .single()
        .expect("e with acute");
    assert_eq!(cache.take_uploads().len(), 2);
    assert!(acute.top > plain.top, "{acute:?} vs {plain:?}");
}

#[test]
fn a_mark_on_a_space_is_still_drawn() {
    let mut fonts = Fonts::new(None, 16.0);
    let mut cache = GlyphCache::new(256, 256);
    assert!(
        cache
            .get(&mut fonts, &with_marks(' ', &['\u{308}']))
            .unwrap()
            .single()
            .is_some()
    );
}

#[test]
fn emoji_presentation_selector_picks_a_color_font() {
    let mut fonts = Fonts::new(None, 16.0);
    if fonts.emoji.is_none() {
        eprintln!("skipping: no color emoji font installed");
        return;
    }
    let mut cache = GlyphCache::new(256, 256);
    let heart = cache
        .get(&mut fonts, &with_marks('\u{2764}', &[EMOJI_PRESENTATION]))
        .unwrap()
        .single()
        .expect("heart");
    assert!(heart.color);
}

#[test]
fn presentation_follows_the_selector_then_the_emoji_presentation_property() {
    assert_eq!(Presentation::of(&key('\u{1F44D}')), Presentation::Emoji);
    assert_eq!(Presentation::of(&key('\u{2764}')), Presentation::Text);
    assert_eq!(Presentation::of(&key('A')), Presentation::Text);
    assert_eq!(
        Presentation::of(&with_marks('\u{2764}', &[EMOJI_PRESENTATION])),
        Presentation::Emoji
    );
    assert_eq!(
        Presentation::of(&with_marks('\u{1F44D}', &[TEXT_PRESENTATION])),
        Presentation::Text
    );
}

#[test]
fn emoji_presentation_characters_use_a_color_font_without_a_selector() {
    let mut fonts = Fonts::new(None, 16.0);
    if fonts.emoji.is_none() {
        eprintln!("skipping: no color emoji font installed");
        return;
    }
    let mut cache = GlyphCache::new(256, 256);
    let thumbs = cache
        .get(&mut fonts, &key('\u{1F44D}'))
        .unwrap()
        .single()
        .expect("thumbs up");
    assert!(thumbs.color);
}

#[test]
fn text_presentation_is_colored_only_without_a_text_font_for_it() {
    let mut fonts = Fonts::new(None, 16.0);
    let mut cache = GlyphCache::new(512, 512);
    for ch in ['\u{1F44D}', '\u{2764}', '\u{2B50}', '\u{1F680}', '\u{2705}'] {
        let Some(glyph) = cache
            .get(&mut fonts, &with_marks(ch, &[TEXT_PRESENTATION]))
            .unwrap()
            .single()
        else {
            continue;
        };
        let has_text_font = fonts.text_fallback(ch).is_some();
        assert!(!(glyph.color && has_text_font), "{ch:?} drawn in color");
    }
}

#[test]
fn text_marks_on_an_emoji_are_a_separate_mask_layer() {
    let mut fonts = Fonts::new(None, 16.0);
    if fonts.emoji.is_none() {
        eprintln!("skipping: no color emoji font installed");
        return;
    }
    let mut cache = GlyphCache::new(256, 256);
    // U+1F44D has emoji presentation of its own; U+2764 asks with VS16.
    for key in [
        with_marks('\u{1F44D}', &['\u{301}']),
        with_marks('\u{2764}', &[EMOJI_PRESENTATION, '\u{301}']),
    ] {
        let cell = cache.get(&mut fonts, &key).unwrap();
        let kinds: Vec<_> = cell.iter().map(|g| g.color).collect();
        assert_eq!(kinds, [true, false], "{key:?}");
        assert_eq!(cache.take_uploads().len(), 2);
    }
}

#[test]
fn space_has_no_pixels() {
    let mut fonts = Fonts::new(None, 16.0);
    let mut cache = GlyphCache::new(256, 256);
    assert_eq!(cache.get(&mut fonts, &key(' ')), Ok(CellGlyphs::default()));
}

#[test]
fn full_atlas_reports_instead_of_overwriting() {
    let mut fonts = Fonts::new(None, 16.0);
    let mut cache = GlyphCache::new(4, 64);
    assert_eq!(cache.get(&mut fonts, &key('W')), Err(AtlasFull));
    assert!(cache.take_uploads().is_empty());
}

#[test]
fn cells_without_an_image_stop_being_cached_at_the_cap() {
    let mut fonts = Fonts::new(None, 16.0);
    let mut cache = GlyphCache::new(256, 256);
    let blank = key('\u{2003}');
    assert_eq!(
        cache.get(&mut fonts, &blank).unwrap(),
        CellGlyphs::default()
    );
    assert_eq!(cache.entries.len(), 1);
    cache.entries.clear();
    for i in 0..MAX_ENTRIES {
        let mark = char::from_u32(0x10_0000 + i as u32).unwrap();
        cache
            .entries
            .insert(with_marks('x', &[mark]), CellGlyphs::default());
    }
    assert_eq!(
        cache.get(&mut fonts, &blank).unwrap(),
        CellGlyphs::default()
    );
    assert_eq!(cache.entries.len(), MAX_ENTRIES);
    assert!(cache.get(&mut fonts, &key('A')).unwrap().single().is_some());
    assert_eq!(
        cache.entries.len(),
        MAX_ENTRIES + 1,
        "drawn glyphs are still cached"
    );
}
