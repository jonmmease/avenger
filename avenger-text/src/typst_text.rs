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

#[derive(Debug, Clone)]
pub struct TypstTextMeasurer {
    typst: avenger_typst::AvengerTypst,
    math: TextMathConfig,
}

impl TypstTextMeasurer {
    pub fn new(typst: avenger_typst::AvengerTypst, math: TextMathConfig) -> Self {
        Self { typst, math }
    }

    pub fn with_vendor_typst(math: TextMathConfig) -> Result<Self, avenger_typst::TypstInitError> {
        Ok(Self::new(
            avenger_typst::AvengerTypst::new(avenger_typst::TypstEngineConfig {
                backend: avenger_typst::TypstEngineBackend::VendorTypst,
                ..avenger_typst::TypstEngineConfig::default()
            })?,
            math,
        ))
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
            },
        ) {
            Ok(artifact) => bounds_from_metrics(artifact.metrics),
            Err(_) => fallback_text_bounds(config.text, config.font_size),
        }
    }

    fn measure_font_metrics(&self, config: &FontMetricsConfig) -> FontMetrics {
        FontMetrics::fallback(config.font_size)
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

    pub fn with_vendor_typst(math: TextMathConfig) -> Result<Self, avenger_typst::TypstInitError> {
        Ok(Self::new(
            avenger_typst::AvengerTypst::new(avenger_typst::TypstEngineConfig {
                backend: avenger_typst::TypstEngineBackend::VendorTypst,
                ..avenger_typst::TypstEngineConfig::default()
            })?,
            math,
        ))
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
        let fill = color_key(config.color);
        let artifact = typeset_line(
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
            },
        )
        .map_err(|err| AvengerTextError::TextMeasurementError(err.to_string()))?;
        let bounds = bounds_from_metrics(artifact.metrics);
        let raster = artifact.raster.ok_or_else(|| {
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
                    y: raster.origin_y - bounds.ascent,
                    physical_x: (raster.origin_x * scale).round(),
                    physical_y: ((raster.origin_y - bounds.ascent) * scale).round(),
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
        },
    )
    .map(|artifact| artifact.metrics.width)
    .unwrap_or_else(|_| fallback_text_bounds(text, font_size).width)
}

fn typeset_line(
    typst: &avenger_typst::AvengerTypst,
    math: &TextMathConfig,
    text: &str,
    font: &str,
    font_size: f32,
    font_weight: &FontWeight,
    font_style: &FontStyle,
    color: [f32; 4],
    outputs: avenger_typst::TextLineOutputRequest,
) -> Result<avenger_typst::TextLineArtifact, avenger_typst::MathTypesetError> {
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

    match typst.typeset_text_line(text, &options) {
        Ok(artifact) => Ok(artifact),
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
        FontMetrics::fallback(config.font_size)
    }
}

fn text_line_options(
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

    avenger_typst::TextLineOptions {
        text_style: avenger_typst::PlainTextStyle {
            font_family: font.to_string(),
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

fn bounds_from_metrics(metrics: avenger_typst::TypesetMetrics) -> TextBounds {
    TextBounds {
        width: metrics.width,
        height: metrics.height,
        ascent: metrics.ascent,
        descent: metrics.descent,
        line_height: metrics.height.max(metrics.ascent + metrics.descent),
    }
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
}
