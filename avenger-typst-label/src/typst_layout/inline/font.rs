use std::ops::Range;
use std::sync::Arc;

use crate::label::{EngineOptions, FontResource, FontResourceId, LabelError};
use crate::typst_library::{FontStyle, FontWeight, TextStyle};
use crate::typst_svg::{PathData, PathImageFormat, PathImageItem, Transform};
use unicode_bidi::BidiInfo;
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

use crate::typst_library::text::content::DecorationLength;

#[derive(Clone)]
pub(crate) struct TextFace {
    data: TextFontData,
    face_index: u32,
    family_name: Option<String>,
    postscript_name: Option<String>,
}

#[derive(Clone)]
enum TextFontData {
    Shared(Arc<[u8]>),
}

impl TextFontData {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Shared(data) => data.as_ref(),
        }
    }

    fn resource_data(&self) -> Arc<[u8]> {
        match self {
            Self::Shared(data) => data.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ShapedTextMetrics {
    pub(crate) width: f32,
    pub(crate) ascent: f32,
    pub(crate) descent: f32,
    pub(crate) height: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShapedGlyph {
    pub(crate) glyph_id: ttf_parser::GlyphId,
    pub(crate) unicode: String,
    pub(crate) byte_range: Range<usize>,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) x_advance: f32,
    pub(crate) y_advance: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShapedText {
    pub(crate) metrics: ShapedTextMetrics,
    pub(crate) glyphs: Vec<ShapedGlyph>,
    pub(crate) has_missing_glyph: bool,
}

#[derive(Clone)]
pub(crate) struct ShapedTextRun {
    pub(crate) face: TextFace,
    #[allow(dead_code)]
    pub(crate) text: String,
    #[allow(dead_code)]
    pub(crate) byte_range: Range<usize>,
    pub(crate) is_rtl: bool,
    pub(crate) x: f32,
    pub(crate) shaped: ShapedText,
}

#[derive(Clone)]
pub(crate) struct SegmentedText {
    pub(crate) metrics: ShapedTextMetrics,
    pub(crate) runs: Vec<ShapedTextRun>,
    pub(crate) has_missing_glyph: bool,
}

impl SegmentedText {
    pub(crate) fn single(
        face: TextFace,
        text: &str,
        font_size: f32,
        features: &[rustybuzz::Feature],
    ) -> Self {
        let shaped = face.shaped_text_with_features(text, font_size, features);
        Self {
            metrics: shaped.metrics,
            runs: vec![ShapedTextRun {
                face,
                text: text.to_string(),
                byte_range: 0..text.len(),
                is_rtl: false,
                x: 0.0,
                shaped,
            }],
            has_missing_glyph: false,
        }
        .with_derived_missing_glyph()
    }

    fn with_derived_missing_glyph(mut self) -> Self {
        self.has_missing_glyph = self.runs.iter().any(|run| run.shaped.has_missing_glyph);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextFontMetrics {
    pub(crate) ascent: f32,
    pub(crate) descent: f32,
    pub(crate) height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextDecorationMetrics {
    pub(crate) underline: TextDecorationLineMetrics,
    pub(crate) strikethrough: TextDecorationLineMetrics,
    pub(crate) overline: TextDecorationLineMetrics,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextDecorationLineMetrics {
    /// Position relative to the baseline. Positive values are above the
    /// baseline, matching Typst and font-table conventions.
    pub(crate) position: f32,
    pub(crate) thickness: f32,
}

impl TextDecorationMetrics {
    pub(crate) fn fallback(font_size: f32) -> Self {
        let font_size = font_size.max(1.0);
        let thickness = font_size * 0.06;
        Self {
            underline: TextDecorationLineMetrics {
                position: -font_size * 0.2,
                thickness,
            },
            strikethrough: TextDecorationLineMetrics {
                position: font_size * 0.25,
                thickness,
            },
            overline: TextDecorationLineMetrics {
                position: font_size * 0.9,
                thickness,
            },
        }
    }
}

impl TextFace {
    pub(crate) fn for_plain_style(
        style: &TextStyle,
        fontdb: &fontdb::Database,
    ) -> Result<Option<Self>, LabelError> {
        Ok(fontdb_face_for_style_and_text(fontdb, style, ""))
    }

    pub(crate) fn for_plain_style_and_text(
        style: &TextStyle,
        text: &str,
        fontdb: &fontdb::Database,
    ) -> Result<Option<Self>, LabelError> {
        if let Some(primary) = Self::for_plain_style(style, fontdb)? {
            if !primary.shaped_text(text, style.font_size).has_missing_glyph {
                return Ok(Some(primary));
            }
            return Ok(fontdb_face_for_style_and_text(fontdb, style, text).or(Some(primary)));
        }

        Ok(fontdb_face_for_style_and_text(fontdb, style, text))
    }

    pub(crate) fn font_metrics(&self, font_size: f32) -> Option<crate::label::FontMetrics> {
        let face = self.parsed_face()?;
        let scale = font_scale(&face, font_size);
        Some(crate::label::FontMetrics {
            ascent: face
                .typographic_ascender()
                .unwrap_or_else(|| face.ascender())
                .max(0) as f32
                * scale,
            descent: -(face
                .typographic_descender()
                .unwrap_or_else(|| face.descender())
                .min(0) as f32)
                * scale,
            line_gap: face
                .typographic_line_gap()
                .unwrap_or_else(|| face.line_gap())
                .max(0) as f32
                * scale,
        })
    }

    pub(crate) fn same_font(&self, other: &Self) -> bool {
        self.face_index == other.face_index && self.data.as_slice() == other.data.as_slice()
    }

    pub(crate) fn default_text_edge_metrics(&self, font_size: f32) -> TextFontMetrics {
        let Some(face) = self.parsed_face() else {
            return fallback_metrics(font_size);
        };
        let scale = font_scale(&face, font_size);
        let cap_height = face
            .capital_height()
            .filter(|height| *height > 0)
            .unwrap_or_else(|| {
                face.typographic_ascender()
                    .unwrap_or_else(|| face.ascender())
            })
            .max(0) as f32
            * scale;

        TextFontMetrics {
            ascent: cap_height,
            descent: 0.0,
            height: cap_height,
        }
    }

    pub(crate) fn decoration_metrics(&self, font_size: f32) -> TextDecorationMetrics {
        let font_size = font_size.max(1.0);
        let Some(face) = self.parsed_face() else {
            return TextDecorationMetrics::fallback(font_size);
        };
        let scale = font_scale(&face, font_size);
        let to_px = |units: i16| units as f32 * scale;
        let thickness_to_px = |units: i16| (units as f32 * scale).abs().max(f32::EPSILON);
        let strikeout = face.strikeout_metrics();
        let underline = face.underline_metrics();

        let strikethrough = TextDecorationLineMetrics {
            position: strikeout
                .map(|metrics| to_px(metrics.position))
                .unwrap_or(font_size * 0.25),
            thickness: strikeout
                .or(underline)
                .map(|metrics| thickness_to_px(metrics.thickness))
                .unwrap_or(font_size * 0.06),
        };

        let underline = TextDecorationLineMetrics {
            position: underline
                .map(|metrics| to_px(metrics.position))
                .unwrap_or(-font_size * 0.2),
            thickness: underline
                .or(strikeout)
                .map(|metrics| thickness_to_px(metrics.thickness))
                .unwrap_or(font_size * 0.06),
        };

        let cap_height = face
            .capital_height()
            .filter(|height| *height > 0)
            .unwrap_or_else(|| {
                face.typographic_ascender()
                    .unwrap_or_else(|| face.ascender())
            })
            .max(0) as f32
            * scale;

        TextDecorationMetrics {
            underline,
            strikethrough,
            overline: TextDecorationLineMetrics {
                position: cap_height + font_size * 0.1,
                thickness: underline.thickness,
            },
        }
    }

    pub(crate) fn shaped_text(&self, text: &str, font_size: f32) -> ShapedText {
        self.shaped_text_with_features(text, font_size, &[])
    }

    pub(crate) fn shaped_text_with_features(
        &self,
        text: &str,
        font_size: f32,
        features: &[rustybuzz::Feature],
    ) -> ShapedText {
        let edge_metrics = self.default_text_edge_metrics(font_size);
        let Some(face) = self.parsed_face() else {
            return fallback_shaped_text(text, font_size, edge_metrics);
        };
        let Some(rusty) = rustybuzz::Face::from_slice(self.data.as_slice(), self.face_index) else {
            return fallback_shaped_text(text, font_size, edge_metrics);
        };
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        let glyphs = rustybuzz::shape(&rusty, features, buffer);
        let scale = font_scale(&face, font_size);
        let mut cursor_x = 0i32;
        let mut cursor_y = 0i32;
        let mut advance_width = 0i32;
        let mut shaped_glyphs = Vec::new();
        let mut has_missing_glyph = false;
        let cluster_starts = glyph_cluster_starts(text, glyphs.glyph_infos());

        for (info, position) in glyphs.glyph_infos().iter().zip(glyphs.glyph_positions()) {
            has_missing_glyph |= info.glyph_id == 0;
            let x = cursor_x + position.x_offset;
            let y = cursor_y + position.y_offset;
            cursor_x += position.x_advance;
            cursor_y += position.y_advance;
            advance_width += position.x_advance;
            let byte_range = glyph_cluster_range(text, info.cluster, &cluster_starts);
            shaped_glyphs.push(ShapedGlyph {
                glyph_id: ttf_parser::GlyphId(info.glyph_id as u16),
                unicode: text.get(byte_range.clone()).unwrap_or_default().to_string(),
                byte_range,
                x: x as f32 * scale,
                y: -(y as f32) * scale,
                x_advance: position.x_advance as f32 * scale,
                y_advance: -(position.y_advance as f32) * scale,
            });
        }

        ShapedText {
            metrics: ShapedTextMetrics {
                width: advance_width as f32 * scale,
                ascent: edge_metrics.ascent,
                descent: edge_metrics.descent,
                height: edge_metrics.height,
            },
            glyphs: shaped_glyphs,
            has_missing_glyph,
        }
    }

    pub(crate) fn script_style_with_size(
        &self,
        parent_style: &TextStyle,
        script: TextScript,
        explicit_size: Option<DecorationLength>,
    ) -> TextStyle {
        if let Some(size) = explicit_size {
            return TextStyle {
                font_size: size.resolve(parent_style.font_size.max(1.0)).max(1.0),
                ..parent_style.clone()
            };
        }

        let Some(face) = self.parsed_face() else {
            return TextStyle {
                font_size: parent_style.font_size.max(1.0) * 0.7,
                ..parent_style.clone()
            };
        };
        let scale = font_scale(&face, parent_style.font_size.max(1.0));
        let metrics = match script {
            TextScript::Subscript => face.subscript_metrics(),
            TextScript::Superscript => face.superscript_metrics(),
        };
        let font_size = metrics
            .and_then(|metrics| (metrics.y_size > 0).then_some(metrics.y_size as f32 * scale))
            .unwrap_or_else(|| parent_style.font_size.max(1.0) * 0.7)
            .max(1.0);

        TextStyle {
            font_size,
            ..parent_style.clone()
        }
    }

    pub(crate) fn script_baseline_shift_with_baseline(
        &self,
        parent_font_size: f32,
        script: TextScript,
        explicit_baseline: Option<DecorationLength>,
    ) -> f32 {
        if let Some(baseline) = explicit_baseline {
            return baseline.resolve(parent_font_size.max(1.0));
        }

        let Some(face) = self.parsed_face() else {
            return fallback_script_shift(parent_font_size, script);
        };
        let scale = font_scale(&face, parent_font_size.max(1.0));
        let metrics = match script {
            TextScript::Subscript => face.subscript_metrics(),
            TextScript::Superscript => face.superscript_metrics(),
        };
        metrics
            .map(|metrics| {
                let offset = metrics.y_offset as f32 * scale;
                match script {
                    TextScript::Subscript => offset.abs(),
                    TextScript::Superscript => -offset.abs(),
                }
            })
            .unwrap_or_else(|| fallback_script_shift(parent_font_size, script))
    }

    pub(crate) fn outline_glyph_path(
        &self,
        glyph_id: ttf_parser::GlyphId,
        font_size: f32,
        x: f32,
        y: f32,
    ) -> PathData {
        let Some(face) = self.parsed_face() else {
            return PathData {
                commands: Vec::new(),
            };
        };
        crate::typst_layout::glyph_path::outline_glyph_path(&face, glyph_id, font_size, x, y)
    }

    pub(crate) fn raster_glyph_image(
        &self,
        glyph_id: ttf_parser::GlyphId,
        font_size: f32,
        x: f32,
        y: f32,
    ) -> Option<PathImageItem> {
        let face = self.parsed_face()?;
        let raster_image = face
            .glyph_raster_image(glyph_id, u16::MAX)
            .filter(|image| image.format == ttf_parser::RasterImageFormat::PNG)?;
        let scale = font_size.max(1.0) / raster_image.pixels_per_em as f32;
        let width = raster_image.width as f32 * scale;
        let height = raster_image.height as f32 * scale;
        let x_offset = raster_image.x as f32 * scale;
        let mut y_offset = raster_image.y as f32 * scale;

        if self
            .family_name
            .clone()
            .or_else(|| font_family_name(&face))
            .as_deref()
            .is_some_and(|family| family.eq_ignore_ascii_case("Apple Color Emoji"))
        {
            y_offset -= 0.128 * font_size.max(1.0);
        }

        Some(PathImageItem {
            data: raster_image.data.to_vec(),
            format: PathImageFormat::Png,
            width,
            height,
            transform: Transform {
                tx: x - x_offset,
                ty: y - (height + y_offset),
                ..Transform::IDENTITY
            },
        })
    }

    pub(crate) fn font_resource(&self, id: FontResourceId) -> FontResource {
        let face = self.parsed_face();
        FontResource {
            id,
            family: self
                .family_name
                .clone()
                .or_else(|| {
                    face.as_ref()
                        .and_then(|face| font_name(face, ttf_parser::name_id::TYPOGRAPHIC_FAMILY))
                })
                .or_else(|| {
                    face.as_ref()
                        .and_then(|face| font_name(face, ttf_parser::name_id::FAMILY))
                })
                .unwrap_or_else(|| "Unknown".to_string()),
            postscript_name: self.postscript_name.clone().or_else(|| {
                face.as_ref()
                    .and_then(|face| font_name(face, ttf_parser::name_id::POST_SCRIPT_NAME))
            }),
            face_index: self.face_index,
            units_per_em: face
                .as_ref()
                .map(|face| face.units_per_em() as f32)
                .unwrap_or(1000.0),
            data: self.data.resource_data(),
        }
    }

    fn parsed_face(&self) -> Option<ttf_parser::Face<'_>> {
        ttf_parser::Face::parse(self.data.as_slice(), self.face_index).ok()
    }
}

pub(crate) fn shape_plain_text_with_fallback(
    fontdb: &fontdb::Database,
    style: &TextStyle,
    text: &str,
    font_size: f32,
    features: &[rustybuzz::Feature],
) -> Result<Option<SegmentedText>, LabelError> {
    let Some(primary) = TextFace::for_plain_style(style, fontdb)?
        .or_else(|| fontdb_face_for_style_and_text(fontdb, style, text))
    else {
        return Ok(None);
    };

    if text.is_empty() {
        return Ok(Some(SegmentedText::single(
            primary, text, font_size, features,
        )));
    }

    let mut spans = Vec::<TextMarkupSpan>::new();
    for visual_run in bidi_visual_runs(text) {
        for (relative_start, grapheme) in text[visual_run.byte_range.clone()].grapheme_indices(true)
        {
            let start = visual_run.byte_range.start + relative_start;
            let end = start + grapheme.len();
            let grapheme_script = script_for_grapheme(grapheme);
            let script = if is_neutral_script(grapheme_script) {
                spans
                    .last()
                    .map(|span| span.script)
                    .unwrap_or(grapheme_script)
            } else {
                grapheme_script
            };
            let face = if !primary
                .shaped_text_with_features(grapheme, font_size, features)
                .has_missing_glyph
            {
                primary.clone()
            } else {
                fontdb_face_for_style_and_text(fontdb, style, grapheme)
                    .unwrap_or_else(|| primary.clone())
            };

            if let Some(span) = spans.last_mut()
                && span.face.same_font(&face)
                && span.script == script
                && span.is_rtl == visual_run.is_rtl
                && span.byte_range.end == start
                && span.visual_range.end == start
            {
                span.byte_range.end = end;
                span.visual_range.end = end;
                span.text.push_str(grapheme);
                continue;
            }

            spans.push(TextMarkupSpan {
                face,
                text: grapheme.to_string(),
                byte_range: start..end,
                visual_range: start..end,
                script,
                is_rtl: visual_run.is_rtl,
            });
        }
    }

    let mut x = 0.0f32;
    let mut ascent = 0.0f32;
    let mut descent = 0.0f32;
    let mut runs = Vec::new();
    for span in spans {
        let shaped = span
            .face
            .shaped_text_with_features(&span.text, font_size, features);
        ascent = ascent.max(shaped.metrics.ascent);
        descent = descent.max(shaped.metrics.descent);
        let width = shaped.metrics.width;
        runs.push(ShapedTextRun {
            face: span.face,
            text: span.text,
            byte_range: span.byte_range,
            is_rtl: span.is_rtl,
            x,
            shaped,
        });
        x += width;
    }

    Ok(Some(
        SegmentedText {
            metrics: ShapedTextMetrics {
                width: x,
                ascent,
                descent,
                height: ascent + descent,
            },
            runs,
            has_missing_glyph: false,
        }
        .with_derived_missing_glyph(),
    ))
}

struct TextMarkupSpan {
    face: TextFace,
    text: String,
    byte_range: Range<usize>,
    visual_range: Range<usize>,
    script: Script,
    is_rtl: bool,
}

struct BidiVisualRun {
    byte_range: Range<usize>,
    is_rtl: bool,
}

fn bidi_visual_runs(text: &str) -> Vec<BidiVisualRun> {
    let bidi = BidiInfo::new(text, None);
    if !bidi.has_rtl() {
        return vec![BidiVisualRun {
            byte_range: 0..text.len(),
            is_rtl: false,
        }];
    }

    let mut ranges = Vec::new();
    for paragraph in &bidi.paragraphs {
        let (levels, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
        ranges.extend(runs.into_iter().map(|byte_range| BidiVisualRun {
            is_rtl: levels[byte_range.start].is_rtl(),
            byte_range,
        }));
    }
    ranges
}

fn script_for_grapheme(grapheme: &str) -> Script {
    grapheme
        .chars()
        .map(|ch| ch.script())
        .find(|script| !is_neutral_script(*script))
        .unwrap_or(Script::Common)
}

fn is_neutral_script(script: Script) -> bool {
    matches!(script, Script::Common | Script::Inherited | Script::Unknown)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextScript {
    Subscript,
    Superscript,
}

pub(crate) fn build_text_fontdb(config: &EngineOptions) -> fontdb::Database {
    let mut db = fontdb::Database::new();
    crate::label::fonts::load_registered_fonts_into_fontdb(&mut db, config);
    #[cfg(test)]
    if config.fonts.registered_fonts.is_empty() {
        crate::label::fonts::load_test_fonts_into_fontdb(&mut db);
    }
    if let Some(family) = &config.fonts.default_sans_serif_family {
        db.set_sans_serif_family(family);
    }
    if let Some(family) = &config.fonts.default_monospace_family {
        db.set_monospace_family(family);
    }
    if config.fonts.load_system_fonts {
        db.load_system_fonts();
    }
    for dir in &config.fonts.extra_font_dirs {
        db.load_fonts_dir(dir);
    }
    db
}

fn fontdb_face_for_style_and_text(
    db: &fontdb::Database,
    style: &TextStyle,
    text: &str,
) -> Option<TextFace> {
    if text.chars().any(is_color_emoji_char)
        && let Some(face) = emoji_fontdb_face_for_text(db, style, text)
    {
        return Some(face);
    }

    let query = fontdb::Query {
        families: &fontdb_families(&style.font_family),
        weight: fontdb::Weight(font_weight_number(&style.font_weight)),
        stretch: fontdb::Stretch::Normal,
        style: fontdb_style(style.font_style),
    };

    if let Some(face) = db
        .query(&query)
        .and_then(|id| load_fontdb_face(db, id))
        .filter(|face| {
            text.is_empty() || !face.shaped_text(text, style.font_size).has_missing_glyph
        })
    {
        return Some(face);
    }

    if text.is_empty() {
        return None;
    }

    db.faces()
        .filter(|info| info.style == fontdb_style(style.font_style))
        .filter_map(|info| load_fontdb_face(db, info.id))
        .find(|face| !face.shaped_text(text, style.font_size).has_missing_glyph)
}

fn emoji_fontdb_face_for_text(
    db: &fontdb::Database,
    style: &TextStyle,
    text: &str,
) -> Option<TextFace> {
    const EMOJI_FAMILIES: &[&str] = &[
        "Apple Color Emoji",
        "Noto Color Emoji",
        "Twitter Color Emoji",
        "Segoe UI Emoji",
    ];

    EMOJI_FAMILIES.iter().find_map(|family| {
        let families = [fontdb::Family::Name(family)];
        let query = fontdb::Query {
            families: &families,
            weight: fontdb::Weight(font_weight_number(&style.font_weight)),
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        db.query(&query)
            .and_then(|id| load_fontdb_face(db, id))
            .filter(|face| !face.shaped_text(text, style.font_size).has_missing_glyph)
    })
}

fn is_color_emoji_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{1F000}'..='\u{1FAFF}'
            | '\u{1FC00}'..='\u{1FFFD}'
            | '\u{2600}'..='\u{27BF}'
            | '\u{FE0F}'
            | '\u{200D}'
    )
}

fn load_fontdb_face(db: &fontdb::Database, id: fontdb::ID) -> Option<TextFace> {
    let family_name = db
        .face(id)
        .and_then(|info| info.families.first().map(|(family, _)| family.clone()));
    let postscript_name = db.face(id).map(|info| info.post_script_name.clone());
    db.with_face_data(id, |data, face_index| {
        ttf_parser::Face::parse(data, face_index).ok()?;
        Some(TextFace {
            data: TextFontData::Shared(Arc::<[u8]>::from(data)),
            face_index,
            family_name,
            postscript_name,
        })
    })?
}

fn fontdb_families(font_family: &str) -> Vec<fontdb::Family<'_>> {
    let mut families = Vec::new();
    for family in font_family.split(',') {
        let family = family.trim().trim_matches('"').trim_matches('\'');
        if family.is_empty() {
            continue;
        }
        let generic = match family.to_ascii_lowercase().as_str() {
            "sans-serif" | "sans serif" => Some(fontdb::Family::SansSerif),
            "serif" => Some(fontdb::Family::Serif),
            "monospace" => Some(fontdb::Family::Monospace),
            "cursive" => Some(fontdb::Family::Cursive),
            "fantasy" => Some(fontdb::Family::Fantasy),
            _ => None,
        };
        families.push(generic.unwrap_or(fontdb::Family::Name(family)));
    }
    if families.is_empty() {
        families.push(fontdb::Family::SansSerif);
    }
    families
}

fn fontdb_style(style: FontStyle) -> fontdb::Style {
    match style {
        FontStyle::Normal => fontdb::Style::Normal,
        FontStyle::Italic => fontdb::Style::Italic,
        FontStyle::Oblique => fontdb::Style::Oblique,
    }
}

fn fallback_shaped_text(text: &str, font_size: f32, edge_metrics: TextFontMetrics) -> ShapedText {
    let fallback = fallback_width(text, font_size);
    ShapedText {
        metrics: ShapedTextMetrics {
            width: fallback,
            ascent: edge_metrics.ascent,
            descent: edge_metrics.descent,
            height: edge_metrics.height,
        },
        glyphs: Vec::new(),
        has_missing_glyph: true,
    }
}

fn fallback_metrics(font_size: f32) -> TextFontMetrics {
    let font_size = font_size.max(1.0);
    TextFontMetrics {
        ascent: font_size * 0.8,
        descent: font_size * 0.2,
        height: font_size,
    }
}

fn fallback_script_shift(parent_font_size: f32, script: TextScript) -> f32 {
    match script {
        TextScript::Subscript => parent_font_size.max(1.0) * 0.2,
        TextScript::Superscript => -parent_font_size.max(1.0) * 0.35,
    }
}

fn glyph_cluster_starts(text: &str, glyph_infos: &[rustybuzz::GlyphInfo]) -> Vec<usize> {
    let mut starts = glyph_infos
        .iter()
        .map(|info| info.cluster as usize)
        .filter(|&start| start <= text.len() && text.is_char_boundary(start))
        .collect::<Vec<_>>();
    starts.sort_unstable();
    starts.dedup();
    starts
}

fn glyph_cluster_range(text: &str, cluster: u32, cluster_starts: &[usize]) -> Range<usize> {
    let cluster = cluster as usize;
    let Some((start, _)) = text.char_indices().find(|(start, _)| *start == cluster) else {
        return 0..0;
    };
    let end = cluster_starts
        .iter()
        .copied()
        .find(|candidate| *candidate > start)
        .unwrap_or(text.len());
    start..end
}

#[cfg(test)]
fn glyph_unicode_for_cluster(text: &str, cluster: u32) -> String {
    let cluster_starts = text
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    text.get(glyph_cluster_range(text, cluster, &cluster_starts))
        .unwrap_or_default()
        .to_string()
}

fn font_name(face: &ttf_parser::Face<'_>, name_id: u16) -> Option<String> {
    face.names().into_iter().find_map(|name| {
        (name.name_id == name_id)
            .then(|| name.to_string())
            .flatten()
    })
}

fn font_family_name(face: &ttf_parser::Face<'_>) -> Option<String> {
    font_name(face, ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
        .or_else(|| font_name(face, ttf_parser::name_id::FAMILY))
}

fn font_weight_number(weight: &FontWeight) -> u16 {
    match weight {
        FontWeight::Normal => 400,
        FontWeight::Bold => 700,
        FontWeight::Number(value) => (*value).clamp(1, 1000),
    }
}

fn font_scale(face: &ttf_parser::Face<'_>, font_size: f32) -> f32 {
    font_size.max(1.0) / face.units_per_em() as f32
}

fn fallback_width(text: &str, font_size: f32) -> f32 {
    text.chars().count() as f32 * font_size.max(1.0) * 0.6
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_fontdb() -> fontdb::Database {
        build_text_fontdb(&EngineOptions::default())
    }

    #[test]
    fn selects_nearest_embedded_weight() {
        let fontdb = test_fontdb();
        let style = TextStyle {
            font_weight: FontWeight::Number(575),
            ..TextStyle::default()
        };

        let face = TextFace::for_plain_style(&style, &fontdb)
            .unwrap()
            .expect("default sans-serif should resolve");

        assert!(face.shaped_text("Hello", 12.0).metrics.width > 0.0);
        assert!(face.default_text_edge_metrics(12.0).height > 0.0);
    }

    #[test]
    fn normal_lato_prefers_medium_when_regular_is_not_bundled() {
        let fontdb = test_fontdb();
        let style = TextStyle {
            font_weight: FontWeight::Normal,
            ..TextStyle::default()
        };

        let face = TextFace::for_plain_style(&style, &fontdb)
            .unwrap()
            .expect("default sans-serif should resolve");
        let resource = face.font_resource(FontResourceId(0));

        assert_eq!(resource.postscript_name.as_deref(), Some("Lato-Medium"));
    }

    #[test]
    fn can_resolve_fontdb_fallback_for_non_embedded_family() {
        let fontdb = test_fontdb();
        let style = TextStyle {
            font_family: "serif".to_string(),
            ..TextStyle::default()
        };

        let Some(face) = TextFace::for_plain_style_and_text(&style, "Hello", &fontdb).unwrap()
        else {
            return;
        };

        assert!(face.shaped_text("Hello", 12.0).metrics.width > 0.0);
    }

    #[test]
    fn segmented_fallback_preserves_grapheme_runs_when_fonts_are_available() {
        let fontdb = test_fontdb();
        let style = TextStyle {
            font_family: "Lato".to_string(),
            ..TextStyle::default()
        };

        let Some(segmented) =
            shape_plain_text_with_fallback(&fontdb, &style, "Hello 温度", style.font_size, &[])
                .unwrap()
        else {
            return;
        };

        if segmented.has_missing_glyph {
            return;
        }

        assert!(segmented.metrics.width > 0.0);
        assert!(segmented.runs.len() >= 2);
        assert_eq!(
            segmented
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Hello ", "温度"]
        );
        assert_eq!(segmented.runs[0].byte_range, 0..6);
        assert_eq!(segmented.runs[1].byte_range, 6.."Hello 温度".len());
        assert!(segmented.runs[1].x > segmented.runs[0].x);
    }

    #[test]
    fn script_for_grapheme_prefers_first_non_neutral_script() {
        assert_eq!(script_for_grapheme("a"), Script::Latin);
        assert_eq!(script_for_grapheme("न"), Script::Devanagari);
        assert_eq!(script_for_grapheme("e\u{301}"), Script::Latin);
        assert_eq!(script_for_grapheme(" "), Script::Common);
    }

    #[test]
    fn segmented_fallback_breaks_at_script_boundaries_when_fonts_are_available() {
        let fontdb = test_fontdb();
        let style = TextStyle {
            font_family: "Lato".to_string(),
            ..TextStyle::default()
        };

        let Some(segmented) =
            shape_plain_text_with_fallback(&fontdb, &style, "abc नमस्ते", style.font_size, &[])
                .unwrap()
        else {
            return;
        };

        if segmented.has_missing_glyph {
            return;
        }

        assert!(segmented.metrics.width > 0.0);
        assert!(segmented.runs.len() >= 2);
        assert_eq!(segmented.runs[0].text, "abc ");
        assert!(segmented.runs.iter().any(|run| run.text.contains("न")));
        assert!(segmented.runs[1].x > segmented.runs[0].x);
    }

    #[test]
    fn segmented_fallback_orders_bidi_runs_visually_when_fonts_are_available() {
        let fontdb = test_fontdb();
        let style = TextStyle {
            font_family: "Lato".to_string(),
            ..TextStyle::default()
        };

        let Some(segmented) =
            shape_plain_text_with_fallback(&fontdb, &style, "אבג ABC", style.font_size, &[])
                .unwrap()
        else {
            return;
        };

        if segmented.has_missing_glyph {
            return;
        }

        assert!(segmented.runs.len() >= 2);
        assert_eq!(segmented.runs[0].text, "ABC");
        assert!(segmented.runs[0].x == 0.0);
        assert!(segmented.runs.iter().any(|run| run.text.contains("אבג")));
    }

    #[test]
    fn glyph_unicode_for_cluster_returns_whole_grapheme() {
        assert_eq!(glyph_unicode_for_cluster("Tone 👍🏽", 5), "👍🏽");
        assert_eq!(glyph_unicode_for_cluster("Flag 🇯🇵", 5), "🇯🇵");
        assert_eq!(glyph_unicode_for_cluster("Cafe\u{301}", 3), "e\u{301}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn segmented_fallback_uses_apple_color_emoji_png_glyphs_on_macos() {
        let fontdb = test_fontdb();
        let style = TextStyle {
            font_family: "Lato".to_string(),
            ..TextStyle::default()
        };

        let segmented =
            shape_plain_text_with_fallback(&fontdb, &style, "Revenue 🚀", style.font_size, &[])
                .unwrap()
                .expect("text should shape");
        let emoji_run = segmented
            .runs
            .iter()
            .find(|run| run.text.contains('🚀'))
            .expect("emoji should be shaped in a fallback run");
        let resource = emoji_run.face.font_resource(FontResourceId(0));
        assert_eq!(resource.family, "Apple Color Emoji");
        let emoji_glyph = emoji_run
            .shaped
            .glyphs
            .iter()
            .find(|glyph| glyph.unicode.contains('🚀'))
            .expect("emoji run should retain emoji glyph semantic text");
        let image = emoji_run
            .face
            .raster_glyph_image(
                emoji_glyph.glyph_id,
                style.font_size,
                emoji_run.x + emoji_glyph.x,
                emoji_run.shaped.metrics.ascent + emoji_glyph.y,
            )
            .expect("Apple Color Emoji glyph should expose a PNG bitmap");

        assert_eq!(image.format, crate::typst_svg::PathImageFormat::Png);
        assert!(image.data.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(image.width > 0.0);
        assert!(image.height > 0.0);
    }

    #[test]
    fn keeps_lato_fast_path_for_default_sans_serif() {
        let fontdb = test_fontdb();
        let style = TextStyle::default();
        let face = TextFace::for_plain_style_and_text(&style, "Hello", &fontdb)
            .unwrap()
            .expect("default sans-serif should resolve");

        assert_eq!(face.font_resource(FontResourceId(0)).family, "Lato");
    }
}
