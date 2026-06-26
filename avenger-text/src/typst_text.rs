use std::{collections::HashMap, marker::PhantomData};

use ordered_float::OrderedFloat;

use crate::{
    error::AvengerTextError,
    measurement::{
        FontMetrics, FontMetricsConfig, TextBounds, TextMeasurementConfig, TextMeasurer,
    },
    rasterization::{
        GlyphBBox, GlyphData, GlyphPosition, TextRasterizationBuffer, TextRasterizationConfig,
        TextRasterizer,
    },
    types::{FontStyle, FontWeight, FontWeightNameSpec},
};

use crate::math::{MathMarkupErrorPolicy, TextMarkupMode, TextMathConfig};

const TYPST_LINE_LEADING_FACTOR: f32 = crate::math::DEFAULT_MATH_LINE_LEADING_FACTOR;

#[derive(Debug, Clone)]
pub struct TypstTextMeasurer {
    typst: avenger_typst::AvengerTypst,
    math: TextMathConfig,
}

impl TypstTextMeasurer {
    pub fn new(typst: avenger_typst::AvengerTypst, math: TextMathConfig) -> Self {
        Self { typst, math }
    }

    pub fn with_config(math: TextMathConfig) -> Result<Self, avenger_typst::TypstInitError> {
        Ok(Self::new(
            avenger_typst::AvengerTypst::new(avenger_typst::TypstEngineConfig {
                backend: avenger_typst::TypstEngineBackend::OwnedTypst,
                ..avenger_typst::TypstEngineConfig::default()
            })?,
            math,
        ))
    }

    pub fn with_owned_typst(math: TextMathConfig) -> Result<Self, avenger_typst::TypstInitError> {
        Self::with_config(math)
    }
}

impl TextMeasurer for TypstTextMeasurer {
    fn measure_text_bounds(&self, config: &TextMeasurementConfig) -> TextBounds {
        match typeset_line(
            &self.typst,
            &self.math,
            config.text,
            config.font,
            config.font_size,
            config.font_weight,
            config.font_style,
            [0.0, 0.0, 0.0, 1.0],
            avenger_typst::TextLineOutputRequest {
                paths: false,
                raster: None,
                pdf_text_layer: false,
                positioned_runs: false,
            },
        ) {
            Ok(result) => bounds_from_metrics(
                result.artifact.metrics,
                config.font_size,
                result.has_math_spans,
            ),
            Err(_) => fallback_text_bounds(config.text, config.font_size),
        }
    }

    fn measure_font_metrics(&self, config: &FontMetricsConfig) -> FontMetrics {
        typst_font_metrics(config)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypstTextRasterCacheKey {
    pub text: String,
    pub font: String,
    pub font_size: OrderedFloat<f32>,
    pub font_weight: String,
    pub font_style: String,
    pub fill: [u8; 4],
    pub scale: OrderedFloat<f32>,
    pub math: String,
}

#[derive(Debug, Clone)]
pub struct TypstTextRasterizer<CacheValue> {
    typst: avenger_typst::AvengerTypst,
    math: TextMathConfig,
    _cache_value: PhantomData<CacheValue>,
}

impl<CacheValue> TypstTextRasterizer<CacheValue> {
    pub fn new(typst: avenger_typst::AvengerTypst, math: TextMathConfig) -> Self {
        Self {
            typst,
            math,
            _cache_value: PhantomData,
        }
    }

    pub fn with_config(math: TextMathConfig) -> Result<Self, avenger_typst::TypstInitError> {
        Ok(Self::new(
            avenger_typst::AvengerTypst::new(avenger_typst::TypstEngineConfig {
                backend: avenger_typst::TypstEngineBackend::OwnedTypst,
                ..avenger_typst::TypstEngineConfig::default()
            })?,
            math,
        ))
    }

    pub fn with_owned_typst(math: TextMathConfig) -> Result<Self, avenger_typst::TypstInitError> {
        Self::with_config(math)
    }
}

impl<CacheValue> TextRasterizer for TypstTextRasterizer<CacheValue>
where
    CacheValue: Clone + 'static,
{
    type CacheKey = TypstTextRasterCacheKey;
    type CacheValue = CacheValue;

    fn rasterize(
        &self,
        config: &TextRasterizationConfig,
        scale: f32,
        cached_glyphs: &HashMap<Self::CacheKey, Self::CacheValue>,
    ) -> Result<TextRasterizationBuffer<Self::CacheKey>, AvengerTextError> {
        let raster_text = truncate_raster_text(config, |candidate| {
            measure_text_width_with_typst(
                &self.typst,
                &self.math,
                candidate,
                config.font,
                config.font_size,
                config.font_weight,
                config.font_style,
            )
        });
        if raster_text.is_empty() {
            return Ok(TextRasterizationBuffer {
                text_bounds: bounds_from_metrics(
                    avenger_typst::TypesetMetrics {
                        width: 0.0,
                        height: 0.0,
                        baseline: 0.0,
                        ascent: 0.0,
                        descent: 0.0,
                    },
                    config.font_size,
                    false,
                ),
                glyphs: Vec::new(),
            });
        }

        let fill = color_key(config.color);
        let result = typeset_line(
            &self.typst,
            &self.math,
            &raster_text,
            config.font,
            config.font_size,
            config.font_weight,
            config.font_style,
            *config.color,
            avenger_typst::TextLineOutputRequest {
                paths: false,
                raster: Some(avenger_typst::RasterRequest { scale }),
                pdf_text_layer: false,
                positioned_runs: false,
            },
        )
        .map_err(|err| AvengerTextError::TextMeasurementError(err.to_string()))?;
        let tight_bounds = tight_bounds_from_metrics(result.artifact.metrics);
        let bounds = bounds_from_metrics(
            result.artifact.metrics,
            config.font_size,
            result.has_math_spans,
        );
        let raster = result.artifact.raster.ok_or_else(|| {
            AvengerTextError::InternalError(
                "Typst text raster output was requested but missing".to_string(),
            )
        })?;
        let cache_key = TypstTextRasterCacheKey {
            text: raster_text,
            font: config.font.clone(),
            font_size: OrderedFloat(config.font_size),
            font_weight: format!("{:?}", config.font_weight),
            font_style: format!("{:?}", config.font_style),
            fill,
            scale: OrderedFloat(scale),
            math: format!("{:?}", self.math),
        };
        let image = if cached_glyphs.contains_key(&cache_key) {
            None
        } else {
            Some(
                image::RgbaImage::from_vec(
                    raster.image.width,
                    raster.image.height,
                    raster.image.data,
                )
                .ok_or_else(|| {
                    AvengerTextError::ImageAllocationError(
                        "Typst text raster image dimensions did not match data".to_string(),
                    )
                })?,
            )
        };

        Ok(TextRasterizationBuffer {
            text_bounds: bounds.clone(),
            glyphs: vec![(
                GlyphData {
                    cache_key,
                    image,
                    path: None,
                    bbox: GlyphBBox {
                        top: 0,
                        left: 0,
                        width: raster.image.width,
                        height: raster.image.height,
                    },
                },
                GlyphPosition {
                    x: raster.origin_x,
                    y: raster.origin_y - tight_bounds.ascent,
                    physical_x: (raster.origin_x * scale).round(),
                    physical_y: ((raster.origin_y - tight_bounds.ascent) * scale).round(),
                },
            )],
        })
    }
}

fn measure_text_width_with_typst(
    typst: &avenger_typst::AvengerTypst,
    math: &TextMathConfig,
    text: &str,
    font: &str,
    font_size: f32,
    font_weight: &FontWeight,
    font_style: &FontStyle,
) -> f32 {
    typeset_line(
        typst,
        math,
        text,
        font,
        font_size,
        font_weight,
        font_style,
        [0.0, 0.0, 0.0, 1.0],
        avenger_typst::TextLineOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: false,
            positioned_runs: false,
        },
    )
    .map(|result| result.artifact.metrics.width)
    .unwrap_or_else(|_| fallback_text_bounds(text, font_size).width)
}

pub(crate) struct TypesetLineResult {
    pub(crate) artifact: avenger_typst::TextLineArtifact,
    pub(crate) has_math_spans: bool,
}

pub(crate) fn typeset_line(
    typst: &avenger_typst::AvengerTypst,
    math: &TextMathConfig,
    text: &str,
    font: &str,
    font_size: f32,
    font_weight: &FontWeight,
    font_style: &FontStyle,
    color: [f32; 4],
    outputs: avenger_typst::TextLineOutputRequest,
) -> Result<TypesetLineResult, avenger_typst::MathTypesetError> {
    let options = text_line_options(
        math,
        text,
        font,
        font_size,
        font_weight,
        font_style,
        color,
        outputs.clone(),
    );
    let has_math_spans = contains_active_math_span(math, text);

    match typst.typeset_text_line(text, &options) {
        Ok(artifact) => Ok(TypesetLineResult {
            artifact,
            has_math_spans,
        }),
        Err(err) if should_retry_as_plain(math) => {
            let mut plain_math = math.clone();
            plain_math.mode = TextMarkupMode::Plain;
            let plain_options = text_line_options(
                &plain_math,
                text,
                font,
                font_size,
                font_weight,
                font_style,
                color,
                outputs,
            );
            typst
                .typeset_text_line(text, &plain_options)
                .map(|artifact| TypesetLineResult {
                    artifact,
                    has_math_spans: false,
                })
                .map_err(|_| err)
        }
        Err(err) => Err(err),
    }
}

fn should_retry_as_plain(math: &TextMathConfig) -> bool {
    !matches!(math.mode, TextMarkupMode::Plain)
        && matches!(
            math.error_policy,
            MathMarkupErrorPolicy::TreatInvalidMathAsLiteral
                | MathMarkupErrorPolicy::ErrorOnPathExtraction
        )
}

fn truncate_raster_text(
    config: &TextRasterizationConfig,
    measure_width: impl FnMut(&str) -> f32,
) -> String {
    if config.limit.is_finite() {
        crate::measurement::truncate_text_to_limit_with(config.text, config.limit, measure_width)
    } else {
        config.text.to_string()
    }
}

impl<CacheValue> TextMeasurer for TypstTextRasterizer<CacheValue>
where
    CacheValue: Clone + Send + Sync + 'static,
{
    fn measure_text_bounds(&self, config: &TextMeasurementConfig) -> TextBounds {
        TypstTextMeasurer::new(self.typst.clone(), self.math.clone()).measure_text_bounds(config)
    }

    fn measure_font_metrics(&self, config: &FontMetricsConfig) -> FontMetrics {
        typst_font_metrics(config)
    }
}

pub(crate) fn text_line_options(
    math: &TextMathConfig,
    text: &str,
    font: &str,
    font_size: f32,
    font_weight: &FontWeight,
    font_style: &FontStyle,
    color: [f32; 4],
    outputs: avenger_typst::TextLineOutputRequest,
) -> avenger_typst::TextLineOptions {
    let (delimiters, math_style, syntax, limits) = match math_mode_parts(math) {
        Some(parts) => parts,
        None => (
            plain_text_delimiters(),
            avenger_typst::MathStyle::default(),
            avenger_typst::MathSyntaxMode::default(),
            avenger_typst::MathLimits::default(),
        ),
    };

    let mut math_style = math_style;
    math_style.font_size = font_size;
    math_style.fill = avenger_typst::Color::rgba(color[0], color[1], color[2], color[3]);
    let font_family = if font.trim().is_empty() {
        avenger_typst::PlainTextStyle::default().font_family
    } else {
        font.to_string()
    };

    avenger_typst::TextLineOptions {
        text_style: avenger_typst::PlainTextStyle {
            font_family,
            font_size,
            fill: avenger_typst::Color::rgba(color[0], color[1], color[2], color[3]),
            font_weight: typst_font_weight(font_weight),
            font_style: typst_font_style(*font_style),
        },
        math_style,
        outputs,
        delimiters,
        syntax,
        limits: limits_for_text(text, limits),
    }
}

fn math_mode_parts(
    math: &TextMathConfig,
) -> Option<(
    avenger_typst::MathDelimiterOptions,
    avenger_typst::MathStyle,
    avenger_typst::MathSyntaxMode,
    avenger_typst::MathLimits,
)> {
    match &math.mode {
        TextMarkupMode::Plain => None,
        TextMarkupMode::TypstMathDelimited(delimiters) => Some((
            delimiters.clone(),
            math.math_style.clone(),
            math.syntax,
            math.limits,
        )),
    }
}

fn plain_text_delimiters() -> avenger_typst::MathDelimiterOptions {
    avenger_typst::MathDelimiterOptions {
        delimiter: '\0',
        escape: None,
        unmatched: avenger_typst::UnmatchedDelimiterPolicy::TreatAsLiteral,
        allow_display_style: false,
    }
}

fn limits_for_text(text: &str, mut limits: avenger_typst::MathLimits) -> avenger_typst::MathLimits {
    limits.max_source_bytes = limits.max_source_bytes.max(text.len());
    limits
}

pub(crate) fn bounds_from_metrics(
    metrics: avenger_typst::TypesetMetrics,
    font_size: f32,
    has_math_spans: bool,
) -> TextBounds {
    let tight = tight_bounds_from_metrics(metrics);
    if has_math_spans {
        padded_math_line_bounds(tight, font_size)
    } else {
        plain_line_bounds(tight, font_size)
    }
}

pub(crate) fn tight_bounds_from_metrics(metrics: avenger_typst::TypesetMetrics) -> TextBounds {
    TextBounds {
        width: metrics.width,
        height: metrics.height,
        ascent: metrics.ascent,
        descent: metrics.descent,
        line_height: metrics.height.max(metrics.ascent + metrics.descent),
    }
}

fn padded_math_line_bounds(tight: TextBounds, font_size: f32) -> TextBounds {
    let leading = font_size.max(0.0) * TYPST_LINE_LEADING_FACTOR;
    let top = leading * 0.5;
    let bottom = leading - top;
    let height = tight.height + leading;

    TextBounds {
        width: tight.width,
        height,
        ascent: tight.ascent + top,
        descent: tight.descent + bottom,
        line_height: height.max(tight.line_height),
    }
}

fn plain_line_bounds(tight: TextBounds, font_size: f32) -> TextBounds {
    let height = tight.height.max(font_size.max(1.0));
    let extra = (height - tight.height).max(0.0);
    let top = extra * 0.5;
    let bottom = extra - top;

    TextBounds {
        width: tight.width,
        height,
        ascent: tight.ascent + top,
        descent: tight.descent + bottom,
        line_height: height,
    }
}

fn typst_font_metrics(config: &FontMetricsConfig) -> FontMetrics {
    embedded_atkinson_font_metrics(config)
        .unwrap_or_else(|| FontMetrics::fallback(config.font_size))
}

fn embedded_atkinson_font_metrics(config: &FontMetricsConfig) -> Option<FontMetrics> {
    let data = embedded_atkinson_face_data(config.font_weight, *config.font_style)?;
    let face = ttf_parser::Face::parse(data, 0).ok()?;
    Some(metrics_from_ttf_face(&face, config.font_size))
}

fn embedded_atkinson_face_data(
    font_weight: &FontWeight,
    font_style: FontStyle,
) -> Option<&'static [u8]> {
    let target_weight = font_weight_number(font_weight);
    crate::fonts::embedded_fonts()
        .iter()
        .filter_map(|font| {
            let (weight, style) = atkinson_face_info(font.name)?;
            (style == font_style).then_some((font.data, weight.abs_diff(target_weight)))
        })
        .min_by_key(|(_, distance)| *distance)
        .map(|(data, _)| data)
}

fn atkinson_face_info(name: &str) -> Option<(u16, FontStyle)> {
    let style = if name.ends_with("Italic") {
        FontStyle::Italic
    } else {
        FontStyle::Normal
    };
    let weight = if name.contains("ExtraBold") {
        800
    } else if name.contains("ExtraLight") {
        250
    } else if name.contains("SemiBold") {
        600
    } else if name.contains("Light") {
        300
    } else if name.contains("Medium") {
        500
    } else if name.contains("Bold") {
        700
    } else if name.contains("Regular") || name.ends_with("-Italic") {
        400
    } else {
        return None;
    };
    Some((weight, style))
}

fn font_weight_number(weight: &FontWeight) -> u16 {
    match weight {
        FontWeight::Name(FontWeightNameSpec::Normal) => 400,
        FontWeight::Name(FontWeightNameSpec::Bold) => 700,
        FontWeight::Number(value) => value.round().clamp(1.0, 1000.0) as u16,
    }
}

fn metrics_from_ttf_face(face: &ttf_parser::Face<'_>, font_size: f32) -> FontMetrics {
    let scale = font_size / face.units_per_em() as f32;
    let ascent = face.ascender().max(0) as f32 * scale;
    let descent = (-face.descender()).max(0) as f32 * scale;
    let height = ascent + descent;
    let line_gap = face.line_gap().max(0) as f32 * scale;
    let line_height = (height + line_gap).max(height).max(font_size);

    FontMetrics {
        ascent,
        descent,
        height,
        line_gap,
        line_height,
    }
}

fn contains_active_math_span(math: &TextMathConfig, text: &str) -> bool {
    match &math.mode {
        TextMarkupMode::Plain => false,
        TextMarkupMode::TypstMathDelimited(delimiters) => contains_delimited_span(text, delimiters),
    }
}

fn contains_delimited_span(text: &str, delimiters: &avenger_typst::MathDelimiterOptions) -> bool {
    let mut pos = 0usize;
    while let Some((idx, ch)) = next_char(text, pos) {
        let next_pos = idx + ch.len_utf8();
        if Some(ch) == delimiters.escape {
            if let Some((_, next)) = next_char(text, next_pos) {
                if next == delimiters.delimiter || Some(next) == delimiters.escape {
                    pos = next_pos + next.len_utf8();
                    continue;
                }
            }
        }

        if ch == delimiters.delimiter && has_closing_delimiter(text, next_pos, delimiters) {
            return true;
        }

        pos = next_pos;
    }

    false
}

fn has_closing_delimiter(
    text: &str,
    start: usize,
    delimiters: &avenger_typst::MathDelimiterOptions,
) -> bool {
    let mut pos = start;
    while let Some((idx, ch)) = next_char(text, pos) {
        let next_pos = idx + ch.len_utf8();
        if Some(ch) == delimiters.escape {
            if let Some((_, next)) = next_char(text, next_pos) {
                if next == delimiters.delimiter || Some(next) == delimiters.escape {
                    pos = next_pos + next.len_utf8();
                    continue;
                }
            }
        }

        if ch == delimiters.delimiter {
            return true;
        }

        pos = next_pos;
    }

    false
}

fn next_char(text: &str, start: usize) -> Option<(usize, char)> {
    text[start..]
        .char_indices()
        .next()
        .map(|(offset, ch)| (start + offset, ch))
}

fn fallback_text_bounds(text: &str, font_size: f32) -> TextBounds {
    let metrics = FontMetrics::fallback(font_size);
    TextBounds {
        width: text.chars().count() as f32 * font_size.max(1.0) * 0.6,
        height: metrics.height,
        ascent: metrics.ascent,
        descent: metrics.descent,
        line_height: metrics.line_height,
    }
}

fn typst_font_weight(weight: &FontWeight) -> avenger_typst::FontWeight {
    match weight {
        FontWeight::Name(FontWeightNameSpec::Normal) => avenger_typst::FontWeight::Normal,
        FontWeight::Name(FontWeightNameSpec::Bold) => avenger_typst::FontWeight::Bold,
        FontWeight::Number(value) => {
            avenger_typst::FontWeight::Number(value.round().clamp(1.0, u16::MAX as f32) as u16)
        }
    }
}

fn typst_font_style(style: FontStyle) -> avenger_typst::FontStyle {
    match style {
        FontStyle::Normal => avenger_typst::FontStyle::Normal,
        FontStyle::Italic => avenger_typst::FontStyle::Italic,
    }
}

fn color_key(color: &[f32; 4]) -> [u8; 4] {
    [
        channel(color[0]),
        channel(color[1]),
        channel(color[2]),
        channel(color[3]),
    ]
}

fn channel(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    static WEIGHT: FontWeight = FontWeight::Name(FontWeightNameSpec::Normal);
    static STYLE: FontStyle = FontStyle::Normal;

    #[test]
    fn plain_mode_uses_literal_dollars() {
        let options = text_line_options(
            &TextMathConfig::default(),
            "Cost $5",
            "sans-serif",
            12.0,
            &WEIGHT,
            &STYLE,
            [0.0, 0.0, 0.0, 1.0],
            avenger_typst::TextLineOutputRequest::default(),
        );

        assert_eq!(options.delimiters.delimiter, '\0');
    }

    #[test]
    fn active_math_spans_ignore_escaped_dollars() {
        let math = TextMathConfig {
            mode: TextMarkupMode::TypstMathDelimited(Default::default()),
            ..Default::default()
        };

        assert!(!contains_active_math_span(&math, r"Cost is \$5"));
        assert!(contains_active_math_span(&math, r"Cost is \$5 and $x$"));
    }

    #[test]
    fn typst_bounds_report_plain_line_box_and_math_leading() {
        let metrics = avenger_typst::TypesetMetrics {
            width: 20.0,
            height: 10.0,
            baseline: 7.0,
            ascent: 7.0,
            descent: 3.0,
        };

        let plain = bounds_from_metrics(metrics, 16.0, false);
        let math = bounds_from_metrics(metrics, 10.0, true);

        assert_eq!(plain.width, 20.0);
        assert_eq!(plain.height, 16.0);
        assert_eq!(plain.line_height, 16.0);
        assert_eq!(plain.ascent, 10.0);
        assert_eq!(plain.descent, 6.0);
        assert!((math.height - 16.5).abs() <= 1e-4);
        assert!((math.ascent - 10.25).abs() <= 1e-4);
        assert!((math.descent - 6.25).abs() <= 1e-4);
        assert_eq!(math.line_height, 16.5);
    }

    #[test]
    fn typst_font_metrics_use_embedded_face_metrics() {
        let metrics = typst_font_metrics(&FontMetricsConfig {
            font: "sans-serif",
            font_size: 16.0,
            font_weight: &WEIGHT,
            font_style: &STYLE,
        });

        assert!(metrics.ascent > 0.0);
        assert!(metrics.descent > 0.0);
        assert!(metrics.height > 16.0);
        assert!(metrics.line_height >= metrics.height);
    }

    #[test]
    fn typst_rasterizer_reports_one_line_entry_with_mock_engine() {
        let rasterizer = TypstTextRasterizer::<()>::new(
            avenger_typst::AvengerTypst::new(avenger_typst::TypstEngineConfig::default()).unwrap(),
            TextMathConfig::default(),
        );
        let text = "Price $7".to_string();
        let font = "sans-serif".to_string();
        let color = [0.0, 0.0, 0.0, 1.0];
        let buffer = rasterizer
            .rasterize(
                &TextRasterizationConfig {
                    text: &text,
                    color: &color,
                    font: &font,
                    font_size: 12.0,
                    font_weight: &WEIGHT,
                    font_style: &STYLE,
                    limit: f32::INFINITY,
                },
                1.0,
                &HashMap::new(),
            )
            .unwrap();

        assert_eq!(buffer.glyphs.len(), 1);
        assert_eq!(buffer.glyphs[0].0.bbox.width, 1);
        assert_eq!(buffer.glyphs[0].0.bbox.height, 1);
    }

    #[test]
    fn typst_rasterizer_accepts_empty_text() {
        let rasterizer = TypstTextRasterizer::<()>::new(
            avenger_typst::AvengerTypst::new(avenger_typst::TypstEngineConfig::default()).unwrap(),
            TextMathConfig::default(),
        );
        let text = String::new();
        let font = "sans-serif".to_string();
        let color = [0.0, 0.0, 0.0, 1.0];
        let buffer = rasterizer
            .rasterize(
                &TextRasterizationConfig {
                    text: &text,
                    color: &color,
                    font: &font,
                    font_size: 12.0,
                    font_weight: &WEIGHT,
                    font_style: &STYLE,
                    limit: f32::INFINITY,
                },
                1.0,
                &HashMap::new(),
            )
            .unwrap();

        assert!(buffer.glyphs.is_empty());
        assert_eq!(buffer.text_bounds.width, 0.0);
        assert_eq!(buffer.text_bounds.height, 12.0);
    }
}
