use std::{collections::HashMap, marker::PhantomData};

use ordered_float::OrderedFloat;

use crate::{
    error::AvengerTextError,
    measurement::{FontMetrics, FontMetricsConfig, TextBounds, TextMeasurementConfig},
    rasterization::{
        TextRasterBBox, TextRasterCacheKey, TextRasterEntry, TextRasterPosition,
        TextRasterizationBuffer, TextRasterizationConfig,
    },
    types::{FontStyle, FontWeight, FontWeightNameSpec},
};

use crate::math::TextMarkupConfig;

const TYPST_LINE_LEADING_FACTOR: f32 = crate::math::DEFAULT_MARKUP_LINE_LEADING_FACTOR;

#[derive(Debug, Clone)]
pub(crate) struct TextLineMeasurer {
    typst: avenger_typst_label::LabelEngine,
    math: TextMarkupConfig,
}

impl TextLineMeasurer {
    pub(crate) fn new(typst: avenger_typst_label::LabelEngine, math: TextMarkupConfig) -> Self {
        Self { typst, math }
    }
    pub(crate) fn measure_text_bounds(
        &self,
        config: &TextMeasurementConfig,
    ) -> Result<TextBounds, AvengerTextError> {
        let math = self.math.with_syntax_mode(config.syntax_mode);
        let result = typeset_line(
            &self.typst,
            &math,
            config.text,
            config.font,
            config.font_size,
            config.font_weight,
            config.font_style,
            [0.0, 0.0, 0.0, 1.0],
            config.params,
            config.number_locale,
        )?;
        Ok(bounds_from_metrics(
            result.label.metrics,
            config.font_size,
            result.has_math_spans,
        ))
    }

    pub(crate) fn measure_font_metrics(&self, config: &FontMetricsConfig) -> FontMetrics {
        typst_font_metrics(config)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TextLineRasterizer<CacheValue> {
    typst: avenger_typst_label::LabelEngine,
    math: TextMarkupConfig,
    _cache_value: PhantomData<CacheValue>,
}

impl<CacheValue> TextLineRasterizer<CacheValue> {
    pub(crate) fn new(typst: avenger_typst_label::LabelEngine, math: TextMarkupConfig) -> Self {
        Self {
            typst,
            math,
            _cache_value: PhantomData,
        }
    }
    pub(crate) fn rasterize(
        &self,
        config: &TextRasterizationConfig,
        scale: f32,
        cached_entries: &HashMap<TextRasterCacheKey, CacheValue>,
    ) -> Result<TextRasterizationBuffer<TextRasterCacheKey>, AvengerTextError>
    where
        CacheValue: Clone,
    {
        let math = self.math.with_syntax_mode(config.syntax_mode);
        let raster_text = truncate_raster_text(config, |candidate| {
            measure_text_width_with_typst(
                &self.typst,
                &math,
                candidate,
                config.font,
                config.font_size,
                config.font_weight,
                config.font_style,
                config.params,
                config.number_locale,
            )
        })?;
        if raster_text.is_empty() {
            return Ok(TextRasterizationBuffer {
                text_bounds: bounds_from_metrics(
                    avenger_typst_label::LabelMetrics {
                        width: 0.0,
                        height: 0.0,
                        baseline: 0.0,
                        ascent: 0.0,
                        descent: 0.0,
                    },
                    config.font_size,
                    false,
                ),
                entries: Vec::new(),
            });
        }

        let fill = color_key(&config.color);
        let result = typeset_line(
            &self.typst,
            &math,
            &raster_text,
            config.font,
            config.font_size,
            config.font_weight,
            config.font_style,
            config.color,
            config.params,
            config.number_locale,
        )?;
        let tight_bounds = tight_bounds_from_metrics(result.label.metrics);
        let bounds = bounds_from_metrics(
            result.label.metrics,
            config.font_size,
            result.has_math_spans,
        );
        let raster = avenger_typst_label::rasterize(
            &result.label,
            &avenger_typst_label::RasterOptions { scale },
        )?;
        let cache_key = TextRasterCacheKey {
            text: raster_text,
            font: config.font.to_string(),
            font_size: OrderedFloat(config.font_size),
            font_weight: format!("{:?}", config.font_weight),
            font_style: format!("{:?}", config.font_style),
            fill,
            scale: OrderedFloat(scale),
            markup: format!("{:?}", math),
            params: crate::math::label_params_fingerprint(config.params),
            number_locale: config.number_locale.map(str::to_string),
        };
        let image = if cached_entries.contains_key(&cache_key) {
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
            entries: vec![(
                TextRasterEntry {
                    cache_key,
                    image,
                    bbox: TextRasterBBox {
                        top: 0,
                        left: 0,
                        width: raster.image.width,
                        height: raster.image.height,
                    },
                },
                TextRasterPosition {
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
    typst: &avenger_typst_label::LabelEngine,
    math: &TextMarkupConfig,
    text: &str,
    font: &str,
    font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
    params: &avenger_typst_label::LabelParams,
    number_locale: Option<&str>,
) -> Result<f32, AvengerTextError> {
    Ok(typeset_line(
        typst,
        math,
        text,
        font,
        font_size,
        font_weight,
        font_style,
        [0.0, 0.0, 0.0, 1.0],
        params,
        number_locale,
    )
    .map(|result| result.label.metrics.width)?)
}

pub(crate) struct TypesetLineResult {
    pub(crate) label: avenger_typst_label::CompiledLabel,
    pub(crate) has_math_spans: bool,
}

pub(crate) fn typeset_line(
    typst: &avenger_typst_label::LabelEngine,
    math: &TextMarkupConfig,
    text: &str,
    font: &str,
    font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
    color: [f32; 4],
    params: &avenger_typst_label::LabelParams,
    number_locale: Option<&str>,
) -> Result<TypesetLineResult, avenger_typst_label::LabelError> {
    let options = label_options(
        math,
        text,
        font,
        font_size,
        font_weight,
        font_style,
        color,
        params,
        number_locale,
    );
    let label = if math.syntax_mode == crate::types::TextSyntaxMode::Plain {
        typst.compile_text(text, &options)?
    } else {
        typst.compile(text, &options)?
    };
    let has_math_spans = label.flags.has_math;
    Ok(TypesetLineResult {
        label,
        has_math_spans,
    })
}

fn truncate_raster_text(
    config: &TextRasterizationConfig,
    measure_width: impl FnMut(&str) -> Result<f32, AvengerTextError>,
) -> Result<String, AvengerTextError> {
    if config.limit.is_finite() {
        crate::measurement::truncate_text_to_limit_with(config.text, config.limit, measure_width)
    } else {
        Ok(config.text.to_string())
    }
}

pub(crate) fn label_options(
    math: &TextMarkupConfig,
    text: &str,
    font: &str,
    font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
    color: [f32; 4],
    params: &avenger_typst_label::LabelParams,
    number_locale: Option<&str>,
) -> avenger_typst_label::LabelOptions {
    let mut math_style = math.math_style.clone();
    math_style.font_size = font_size;
    math_style.fill = avenger_typst_label::Color::rgba(color[0], color[1], color[2], color[3]);
    math_style.font_weight = typst_font_weight(font_weight);
    let font_family = if font.trim().is_empty() {
        avenger_typst_label::TextStyle::default().font_family
    } else {
        font.to_string()
    };

    avenger_typst_label::LabelOptions {
        text: avenger_typst_label::TextStyle {
            font_family,
            font_size,
            fill: avenger_typst_label::Color::rgba(color[0], color[1], color[2], color[3]),
            font_weight: typst_font_weight(font_weight),
            font_style: typst_font_style(font_style),
        },
        math: math_style,
        params: params.clone(),
        number_locale: number_locale.map(str::to_string),
        number_locale_registry: None,
        limits: limits_for_text(text, math.limits),
    }
}

fn limits_for_text(
    text: &str,
    mut limits: avenger_typst_label::LabelLimits,
) -> avenger_typst_label::LabelLimits {
    limits.max_source_bytes = limits.max_source_bytes.max(text.len());
    limits
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TextMetricParts {
    width: f32,
    height: f32,
    ascent: f32,
    descent: f32,
}

impl From<avenger_typst_label::LabelMetrics> for TextMetricParts {
    fn from(metrics: avenger_typst_label::LabelMetrics) -> Self {
        Self {
            width: metrics.width,
            height: metrics.height,
            ascent: metrics.ascent,
            descent: metrics.descent,
        }
    }
}

pub(crate) fn bounds_from_metrics(
    metrics: impl Into<TextMetricParts>,
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

pub(crate) fn tight_bounds_from_metrics(metrics: impl Into<TextMetricParts>) -> TextBounds {
    let metrics = metrics.into();
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
    FontMetrics::fallback(config.font_size)
}

fn typst_font_weight(weight: FontWeight) -> avenger_typst_label::FontWeight {
    match weight {
        FontWeight::Name(FontWeightNameSpec::Normal) => avenger_typst_label::FontWeight::Normal,
        FontWeight::Name(FontWeightNameSpec::Bold) => avenger_typst_label::FontWeight::Bold,
        FontWeight::Number(value) => {
            avenger_typst_label::FontWeight::Number(value.round().clamp(1.0, u16::MAX as f32) as u16)
        }
    }
}

fn typst_font_style(style: FontStyle) -> avenger_typst_label::FontStyle {
    match style {
        FontStyle::Normal => avenger_typst_label::FontStyle::Normal,
        FontStyle::Italic => avenger_typst_label::FontStyle::Italic,
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
    use crate::types::TextSyntaxMode;

    static WEIGHT: FontWeight = FontWeight::Name(FontWeightNameSpec::Normal);
    static STYLE: FontStyle = FontStyle::Normal;

    #[test]
    fn label_options_preserve_text_style_and_limits() {
        let options = label_options(
            &TextMarkupConfig::default(),
            "Cost $5",
            "sans-serif",
            12.0,
            WEIGHT,
            STYLE,
            [0.0, 0.0, 0.0, 1.0],
            crate::empty_label_params(),
            None,
        );

        assert_eq!(options.text.font_family, "sans-serif");
        assert_eq!(options.text.font_size, 12.0);
        assert!(options.limits.max_source_bytes >= "Cost $5".len());
    }

    #[test]
    fn typst_bounds_report_plain_line_box_and_math_leading() {
        let metrics = avenger_typst_label::LabelMetrics {
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
    fn plain_text_markup_measures_invalid_math_literal() {
        let typst = avenger_typst_label::LabelEngine::new(Default::default()).unwrap();
        let measurer = TextLineMeasurer::new(typst, TextMarkupConfig::default().plain_text());
        let bounds = measurer
            .measure_text_bounds(&TextMeasurementConfig {
                text: "before $x^$ after",
                font: "sans-serif",
                font_size: 14.0,
                font_weight: WEIGHT,
                font_style: STYLE,
                syntax_mode: TextSyntaxMode::Plain,
                params: crate::empty_label_params(),
                number_locale: None,
            })
            .unwrap();

        assert!(bounds.width > 0.0);
    }

    #[test]
    fn typst_font_metrics_use_generic_fallback_metrics() {
        let metrics = typst_font_metrics(&FontMetricsConfig {
            font: "sans-serif",
            font_size: 16.0,
            font_weight: WEIGHT,
            font_style: STYLE,
        });

        assert!(metrics.ascent > 0.0);
        assert!(metrics.descent > 0.0);
        assert_eq!(metrics.height, 16.0);
        assert!(metrics.line_height >= metrics.height);
    }

    #[test]
    fn label_options_include_number_locale() {
        let options = label_options(
            &TextMarkupConfig::default(),
            "#numfmt(value, \",.1f\")",
            "sans-serif",
            12.0,
            WEIGHT,
            STYLE,
            [0.0, 0.0, 0.0, 1.0],
            crate::empty_label_params(),
            Some("de-DE"),
        );

        assert_eq!(options.number_locale.as_deref(), Some("de-DE"));
    }

    #[test]
    fn typst_rasterizer_reports_one_line_entry_with_typst_engine() {
        let rasterizer = TextLineRasterizer::<()>::new(
            avenger_typst_label::LabelEngine::new(Default::default()).unwrap(),
            TextMarkupConfig::default(),
        );
        let text = "Price $7".to_string();
        let font = "sans-serif".to_string();
        let color = [0.0, 0.0, 0.0, 1.0];
        let buffer = rasterizer
            .rasterize(
                &TextRasterizationConfig {
                    text: &text,
                    color,
                    font: &font,
                    font_size: 12.0,
                    font_weight: WEIGHT,
                    font_style: STYLE,
                    limit: f32::INFINITY,
                    syntax_mode: TextSyntaxMode::Plain,
                    params: crate::empty_label_params(),
                    number_locale: None,
                },
                1.0,
                &HashMap::new(),
            )
            .unwrap();

        assert_eq!(buffer.entries.len(), 1);
        assert!(buffer.entries[0].0.bbox.width > 1);
        assert!(buffer.entries[0].0.bbox.height > 1);
        assert!(buffer.entries[0].0.image.is_some());
    }

    #[test]
    fn typst_rasterizer_accepts_empty_text() {
        let rasterizer = TextLineRasterizer::<()>::new(
            avenger_typst_label::LabelEngine::new(Default::default()).unwrap(),
            TextMarkupConfig::default(),
        );
        let text = String::new();
        let font = "sans-serif".to_string();
        let color = [0.0, 0.0, 0.0, 1.0];
        let buffer = rasterizer
            .rasterize(
                &TextRasterizationConfig {
                    text: &text,
                    color,
                    font: &font,
                    font_size: 12.0,
                    font_weight: WEIGHT,
                    font_style: STYLE,
                    limit: f32::INFINITY,
                    syntax_mode: TextSyntaxMode::Plain,
                    params: crate::empty_label_params(),
                    number_locale: None,
                },
                1.0,
                &HashMap::new(),
            )
            .unwrap();

        assert!(buffer.entries.is_empty());
        assert_eq!(buffer.text_bounds.width, 0.0);
        assert_eq!(buffer.text_bounds.height, 12.0);
    }
}
