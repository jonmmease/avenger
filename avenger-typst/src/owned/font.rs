use std::ops::Range;
use std::sync::{Arc, LazyLock};

use crate::error::MathTypesetError;
use crate::fonts::{EmbeddedFontFace, ATKINSON_FACES};
use crate::paths::MathPathData;
use crate::pdf::{MathFontResource, MathFontResourceId};
use crate::style::{FontStyle, FontWeight, PlainTextStyle};
use unicode_bidi::BidiInfo;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone)]
pub(crate) struct OwnedTextFace {
    data: OwnedTextFontData,
    face_index: u32,
}

#[derive(Clone)]
enum OwnedTextFontData {
    Static(&'static [u8]),
    Shared(Arc<[u8]>),
}

impl OwnedTextFontData {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Static(data) => data,
            Self::Shared(data) => data.as_ref(),
        }
    }

    fn resource_data(&self) -> Arc<[u8]> {
        match self {
            Self::Static(data) => Arc::<[u8]>::from(*data),
            Self::Shared(data) => data.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OwnedShapedMetrics {
    pub(crate) width: f32,
    pub(crate) ascent: f32,
    pub(crate) descent: f32,
    pub(crate) height: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OwnedShapedGlyph {
    pub(crate) glyph_id: ttf_parser::GlyphId,
    pub(crate) unicode: String,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) x_advance: f32,
    pub(crate) y_advance: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OwnedShapedText {
    pub(crate) metrics: OwnedShapedMetrics,
    pub(crate) glyphs: Vec<OwnedShapedGlyph>,
    pub(crate) has_missing_glyph: bool,
}

#[derive(Clone)]
pub(crate) struct OwnedShapedTextRun {
    pub(crate) face: OwnedTextFace,
    #[allow(dead_code)]
    pub(crate) text: String,
    #[allow(dead_code)]
    pub(crate) byte_range: Range<usize>,
    pub(crate) x: f32,
    pub(crate) shaped: OwnedShapedText,
}

#[derive(Clone)]
pub(crate) struct OwnedSegmentedText {
    pub(crate) metrics: OwnedShapedMetrics,
    pub(crate) runs: Vec<OwnedShapedTextRun>,
    pub(crate) has_missing_glyph: bool,
}

impl OwnedSegmentedText {
    pub(crate) fn single(
        face: OwnedTextFace,
        text: &str,
        font_size: f32,
        features: &[rustybuzz::Feature],
    ) -> Self {
        let shaped = face.shaped_text_with_features(text, font_size, features);
        Self {
            metrics: shaped.metrics,
            runs: vec![OwnedShapedTextRun {
                face,
                text: text.to_string(),
                byte_range: 0..text.len(),
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
pub(crate) struct OwnedFontMetrics {
    pub(crate) ascent: f32,
    pub(crate) descent: f32,
    pub(crate) height: f32,
}

impl OwnedTextFace {
    pub(crate) fn for_plain_style(
        style: &PlainTextStyle,
    ) -> Result<Option<Self>, MathTypesetError> {
        if resolves_to_embedded_atkinson(&style.font_family) {
            return embedded_atkinson_face(style);
        }

        Ok(fontdb_face_for_style_and_text(style, ""))
    }

    pub(crate) fn for_plain_style_and_text(
        style: &PlainTextStyle,
        text: &str,
    ) -> Result<Option<Self>, MathTypesetError> {
        if let Some(primary) = Self::for_plain_style(style)? {
            if !primary.shaped_text(text, style.font_size).has_missing_glyph {
                return Ok(Some(primary));
            }
            return Ok(Some(primary));
        }

        Ok(fontdb_face_for_style_and_text(style, text))
    }

    pub(crate) fn plain_style_uses_embedded_atkinson(style: &PlainTextStyle) -> bool {
        resolves_to_embedded_atkinson(&style.font_family)
    }

    pub(crate) fn same_font(&self, other: &Self) -> bool {
        self.face_index == other.face_index && self.data.as_slice() == other.data.as_slice()
    }

    pub(crate) fn default_text_edge_metrics(&self, font_size: f32) -> OwnedFontMetrics {
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

        OwnedFontMetrics {
            ascent: cap_height,
            descent: 0.0,
            height: cap_height,
        }
    }

    pub(crate) fn shaped_text(&self, text: &str, font_size: f32) -> OwnedShapedText {
        self.shaped_text_with_features(text, font_size, &[])
    }

    pub(crate) fn shaped_text_with_features(
        &self,
        text: &str,
        font_size: f32,
        features: &[rustybuzz::Feature],
    ) -> OwnedShapedText {
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

        for (info, position) in glyphs.glyph_infos().iter().zip(glyphs.glyph_positions()) {
            has_missing_glyph |= info.glyph_id == 0;
            let x = cursor_x + position.x_offset;
            let y = cursor_y + position.y_offset;
            cursor_x += position.x_advance;
            cursor_y += position.y_advance;
            advance_width += position.x_advance;
            shaped_glyphs.push(OwnedShapedGlyph {
                glyph_id: ttf_parser::GlyphId(info.glyph_id as u16),
                unicode: glyph_unicode_for_cluster(text, info.cluster),
                x: x as f32 * scale,
                y: -(y as f32) * scale,
                x_advance: position.x_advance as f32 * scale,
                y_advance: -(position.y_advance as f32) * scale,
            });
        }

        OwnedShapedText {
            metrics: OwnedShapedMetrics {
                width: advance_width as f32 * scale,
                ascent: edge_metrics.ascent,
                descent: edge_metrics.descent,
                height: edge_metrics.height,
            },
            glyphs: shaped_glyphs,
            has_missing_glyph,
        }
    }

    pub(crate) fn script_style(
        &self,
        parent_style: &PlainTextStyle,
        script: OwnedTextScript,
    ) -> PlainTextStyle {
        let Some(face) = self.parsed_face() else {
            return PlainTextStyle {
                font_size: parent_style.font_size.max(1.0) * 0.7,
                ..parent_style.clone()
            };
        };
        let scale = font_scale(&face, parent_style.font_size.max(1.0));
        let metrics = match script {
            OwnedTextScript::Subscript => face.subscript_metrics(),
            OwnedTextScript::Superscript => face.superscript_metrics(),
        };
        let font_size = metrics
            .and_then(|metrics| (metrics.y_size > 0).then_some(metrics.y_size as f32 * scale))
            .unwrap_or_else(|| parent_style.font_size.max(1.0) * 0.7)
            .max(1.0);

        PlainTextStyle {
            font_size,
            ..parent_style.clone()
        }
    }

    pub(crate) fn script_baseline_shift(
        &self,
        parent_font_size: f32,
        script: OwnedTextScript,
    ) -> f32 {
        let Some(face) = self.parsed_face() else {
            return fallback_script_shift(parent_font_size, script);
        };
        let scale = font_scale(&face, parent_font_size.max(1.0));
        let metrics = match script {
            OwnedTextScript::Subscript => face.subscript_metrics(),
            OwnedTextScript::Superscript => face.superscript_metrics(),
        };
        metrics
            .map(|metrics| {
                let offset = metrics.y_offset as f32 * scale;
                match script {
                    OwnedTextScript::Subscript => offset.abs(),
                    OwnedTextScript::Superscript => -offset.abs(),
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
    ) -> MathPathData {
        let Some(face) = self.parsed_face() else {
            return MathPathData {
                commands: Vec::new(),
            };
        };
        super::glyph_path::outline_glyph_path(&face, glyph_id, font_size, x, y)
    }

    pub(crate) fn font_resource(&self, id: MathFontResourceId) -> MathFontResource {
        let face = self.parsed_face();
        MathFontResource {
            id,
            family: face
                .as_ref()
                .and_then(|face| font_name(face, ttf_parser::name_id::TYPOGRAPHIC_FAMILY))
                .or_else(|| {
                    face.as_ref()
                        .and_then(|face| font_name(face, ttf_parser::name_id::FAMILY))
                })
                .unwrap_or_else(|| "Unknown".to_string()),
            postscript_name: face
                .as_ref()
                .and_then(|face| font_name(face, ttf_parser::name_id::POST_SCRIPT_NAME)),
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
    style: &PlainTextStyle,
    text: &str,
    font_size: f32,
    features: &[rustybuzz::Feature],
) -> Result<Option<OwnedSegmentedText>, MathTypesetError> {
    shape_plain_text_with_fallback_mode(style, text, font_size, features, OwnedFallbackMode::Full)
}

pub(crate) fn shape_plain_text_with_non_emoji_fallback(
    style: &PlainTextStyle,
    text: &str,
    font_size: f32,
    features: &[rustybuzz::Feature],
) -> Result<Option<OwnedSegmentedText>, MathTypesetError> {
    shape_plain_text_with_fallback_mode(
        style,
        text,
        font_size,
        features,
        OwnedFallbackMode::PreserveColorEmojiTofu,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnedFallbackMode {
    Full,
    PreserveColorEmojiTofu,
}

fn shape_plain_text_with_fallback_mode(
    style: &PlainTextStyle,
    text: &str,
    font_size: f32,
    features: &[rustybuzz::Feature],
    mode: OwnedFallbackMode,
) -> Result<Option<OwnedSegmentedText>, MathTypesetError> {
    let Some(primary) = OwnedTextFace::for_plain_style(style)?
        .or_else(|| fontdb_face_for_style_and_text(style, text))
    else {
        return Ok(None);
    };

    if text.is_empty() {
        return Ok(Some(OwnedSegmentedText::single(
            primary, text, font_size, features,
        )));
    }

    let mut spans = Vec::<OwnedTextSpan>::new();
    for visual_range in bidi_visual_ranges(text) {
        for (relative_start, grapheme) in text[visual_range.clone()].grapheme_indices(true) {
            let start = visual_range.start + relative_start;
            let end = start + grapheme.len();
            let face = if !primary
                .shaped_text_with_features(grapheme, font_size, features)
                .has_missing_glyph
            {
                primary.clone()
            } else if mode == OwnedFallbackMode::PreserveColorEmojiTofu
                && grapheme_contains_color_emoji(grapheme)
            {
                primary.clone()
            } else {
                fontdb_face_for_style_and_text(style, grapheme).unwrap_or_else(|| primary.clone())
            };

            if let Some(span) = spans.last_mut() {
                if span.face.same_font(&face)
                    && span.byte_range.end == start
                    && span.visual_range.end == start
                {
                    span.byte_range.end = end;
                    span.visual_range.end = end;
                    span.text.push_str(grapheme);
                    continue;
                }
            }

            spans.push(OwnedTextSpan {
                face,
                text: grapheme.to_string(),
                byte_range: start..end,
                visual_range: start..end,
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
        runs.push(OwnedShapedTextRun {
            face: span.face,
            text: span.text,
            byte_range: span.byte_range,
            x,
            shaped,
        });
        x += width;
    }

    Ok(Some(
        OwnedSegmentedText {
            metrics: OwnedShapedMetrics {
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

struct OwnedTextSpan {
    face: OwnedTextFace,
    text: String,
    byte_range: Range<usize>,
    visual_range: Range<usize>,
}

fn bidi_visual_ranges(text: &str) -> Vec<Range<usize>> {
    let bidi = BidiInfo::new(text, None);
    if !bidi.has_rtl() {
        return vec![0..text.len()];
    }

    let mut ranges = Vec::new();
    for paragraph in &bidi.paragraphs {
        let (_, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
        ranges.extend(runs);
    }
    ranges
}

fn grapheme_contains_color_emoji(grapheme: &str) -> bool {
    grapheme.chars().any(is_color_emoji_char)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnedTextScript {
    Subscript,
    Superscript,
}

static FONTDB: LazyLock<fontdb::Database> = LazyLock::new(|| {
    let mut db = fontdb::Database::new();
    for face in ATKINSON_FACES {
        db.load_font_data(face.data.to_vec());
    }
    db.set_sans_serif_family("Atkinson Hyperlegible Next");
    db.load_system_fonts();
    db
});

fn embedded_atkinson_face(
    style: &PlainTextStyle,
) -> Result<Option<OwnedTextFace>, MathTypesetError> {
    let face = select_atkinson_face(&style.font_weight, style.font_style).ok_or(
        MathTypesetError::UnsupportedOutput("owned plain text requires an embedded Atkinson face"),
    )?;
    ttf_parser::Face::parse(face.data, 0).map_err(|_| MathTypesetError::Engine {
        start: 0,
        end: 0,
        message: format!("failed to parse embedded font {}", face.name),
    })?;

    Ok(Some(OwnedTextFace {
        data: OwnedTextFontData::Static(face.data),
        face_index: 0,
    }))
}

fn fontdb_face_for_style_and_text(style: &PlainTextStyle, text: &str) -> Option<OwnedTextFace> {
    let db = &*FONTDB;
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

fn load_fontdb_face(db: &fontdb::Database, id: fontdb::ID) -> Option<OwnedTextFace> {
    db.with_face_data(id, |data, face_index| {
        ttf_parser::Face::parse(data, face_index).ok()?;
        Some(OwnedTextFace {
            data: OwnedTextFontData::Shared(Arc::<[u8]>::from(data)),
            face_index,
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

fn fallback_shaped_text(
    text: &str,
    font_size: f32,
    edge_metrics: OwnedFontMetrics,
) -> OwnedShapedText {
    let fallback = fallback_width(text, font_size);
    OwnedShapedText {
        metrics: OwnedShapedMetrics {
            width: fallback,
            ascent: edge_metrics.ascent,
            descent: edge_metrics.descent,
            height: edge_metrics.height,
        },
        glyphs: Vec::new(),
        has_missing_glyph: true,
    }
}

fn fallback_metrics(font_size: f32) -> OwnedFontMetrics {
    let font_size = font_size.max(1.0);
    OwnedFontMetrics {
        ascent: font_size * 0.8,
        descent: font_size * 0.2,
        height: font_size,
    }
}

fn fallback_script_shift(parent_font_size: f32, script: OwnedTextScript) -> f32 {
    match script {
        OwnedTextScript::Subscript => parent_font_size.max(1.0) * 0.2,
        OwnedTextScript::Superscript => -parent_font_size.max(1.0) * 0.35,
    }
}

fn glyph_unicode_for_cluster(text: &str, cluster: u32) -> String {
    let cluster = cluster as usize;
    let Some((start, _)) = text.char_indices().find(|(start, _)| *start == cluster) else {
        return String::new();
    };
    let end = text[start..]
        .grapheme_indices(true)
        .nth(1)
        .map_or(text.len(), |(next, _)| start + next);
    text[start..end].to_string()
}

fn font_name(face: &ttf_parser::Face<'_>, name_id: u16) -> Option<String> {
    face.names().into_iter().find_map(|name| {
        (name.name_id == name_id)
            .then(|| name.to_string())
            .flatten()
    })
}

fn select_atkinson_face(
    weight: &FontWeight,
    style: FontStyle,
) -> Option<&'static EmbeddedFontFace> {
    let target_weight = font_weight_number(weight);
    ATKINSON_FACES
        .iter()
        .filter(|face| face.style == style)
        .min_by_key(|face| face.weight.abs_diff(target_weight))
}

fn font_weight_number(weight: &FontWeight) -> u16 {
    match weight {
        FontWeight::Normal => 400,
        FontWeight::Bold => 700,
        FontWeight::Number(value) => (*value).clamp(1, 1000),
    }
}

fn resolves_to_embedded_atkinson(font_family: &str) -> bool {
    font_family.split(',').any(|family| {
        let family = family.trim().trim_matches('"').trim_matches('\'');
        family.eq_ignore_ascii_case("sans-serif")
            || family.eq_ignore_ascii_case("Atkinson Hyperlegible Next")
    })
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

    #[test]
    fn selects_nearest_embedded_weight() {
        let style = PlainTextStyle {
            font_weight: FontWeight::Number(575),
            ..PlainTextStyle::default()
        };

        let face = OwnedTextFace::for_plain_style(&style)
            .unwrap()
            .expect("default sans-serif should resolve");

        assert!(face.shaped_text("Hello", 12.0).metrics.width > 0.0);
        assert!(face.default_text_edge_metrics(12.0).height > 0.0);
    }

    #[test]
    fn can_resolve_fontdb_fallback_for_non_atkinson_family() {
        let style = PlainTextStyle {
            font_family: "serif".to_string(),
            ..PlainTextStyle::default()
        };

        let Some(face) = OwnedTextFace::for_plain_style_and_text(&style, "Hello").unwrap() else {
            return;
        };

        assert!(face.shaped_text("Hello", 12.0).metrics.width > 0.0);
    }

    #[test]
    fn segmented_fallback_preserves_grapheme_runs_when_fonts_are_available() {
        let style = PlainTextStyle {
            font_family: "Atkinson Hyperlegible Next".to_string(),
            ..PlainTextStyle::default()
        };

        let Some(segmented) =
            shape_plain_text_with_fallback(&style, "Hello 温度", style.font_size, &[]).unwrap()
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
    fn segmented_fallback_orders_bidi_runs_visually_when_fonts_are_available() {
        let style = PlainTextStyle {
            font_family: "Atkinson Hyperlegible Next".to_string(),
            ..PlainTextStyle::default()
        };

        let Some(segmented) =
            shape_plain_text_with_non_emoji_fallback(&style, "אבג ABC", style.font_size, &[])
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

    #[test]
    fn keeps_atkinson_fast_path_for_default_sans_serif() {
        let style = PlainTextStyle::default();
        let face = OwnedTextFace::for_plain_style_and_text(&style, "Hello")
            .unwrap()
            .expect("default sans-serif should resolve");

        assert_eq!(
            face.font_resource(MathFontResourceId(0)).family,
            "Atkinson Hyperlegible Next"
        );
    }
}
