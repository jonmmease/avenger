use std::{collections::HashMap, marker::PhantomData};

use ordered_float::OrderedFloat;

use crate::{
    error::AvengerTextError,
    measurement::{FontMetrics, FontMetricsConfig, TextBounds, TextMeasurementConfig},
    rasterization::{
        TextRasterBBox, TextRasterCacheKey, TextRasterCacheValue, TextRasterEntry,
        TextRasterPosition, TextRasterizationBuffer, TextRasterizationConfig,
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
            config.number_format,
            config.datetime_format,
        )?;
        Ok(bounds_from_metrics(
            &result.label.metrics,
            config.font_size,
            result.has_math_spans,
        ))
    }

    pub(crate) fn measure_font_metrics(
        &self,
        config: &FontMetricsConfig,
    ) -> Result<FontMetrics, AvengerTextError> {
        let metrics = self.typst.font_metrics(&avenger_typst_label::TextStyle {
            font_family: config.font.to_string(),
            font_size: config.font_size,
            font_weight: typst_font_weight(config.font_weight),
            font_style: typst_font_style(config.font_style),
            ..Default::default()
        })?;
        let height = metrics.ascent + metrics.descent;
        Ok(FontMetrics {
            ascent: metrics.ascent,
            descent: metrics.descent,
            height,
            line_gap: metrics.line_gap,
            line_height: height + metrics.line_gap,
        })
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
        CacheValue: TextRasterCacheValue,
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
                config.number_format,
                config.datetime_format,
            )
        })?;
        if raster_text.is_empty() {
            return Ok(TextRasterizationBuffer {
                text_bounds: bounds_from_metrics(
                    TextMetricParts::default(),
                    config.font_size,
                    false,
                ),
                entries: Vec::new(),
            });
        }

        let fill = color_key(&config.color);
        let cache_key = TextRasterCacheKey {
            text: raster_text.clone(),
            limit: OrderedFloat(if config.limit.is_finite() && config.limit > 0.0 {
                config.limit
            } else {
                0.0
            }),
            font: config.font.to_string(),
            font_size: OrderedFloat(config.font_size),
            font_weight: format!("{:?}", config.font_weight),
            font_style: format!("{:?}", config.font_style),
            fill,
            scale: OrderedFloat(scale),
            markup: format!("{:?}", math),
            params: crate::math::label_params_fingerprint(config.params),
            number_format: config
                .number_format
                .or(self.typst.number_format())
                .map(crate::ProviderIdentity::new),
            datetime_format: config
                .datetime_format
                .or(self.typst.datetime_format())
                .map(crate::ProviderIdentity::new),
        };
        if let Some(cached) = cached_entries
            .get(&cache_key)
            .and_then(TextRasterCacheValue::cached_text_rasterization)
        {
            return Ok(cached);
        }

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
            config.number_format,
            config.datetime_format,
        )?;
        let tight_bounds = tight_bounds_from_metrics(&result.label.metrics);
        let mut bounds = bounds_from_metrics(
            &result.label.metrics,
            config.font_size,
            result.has_math_spans,
        );
        let clip_width =
            crate::measurement::apply_text_limit(&mut bounds, config.syntax_mode, config.limit);
        let mut raster = avenger_typst_label::rasterize(
            &result.label,
            &avenger_typst_label::RasterOptions { scale },
        )?;
        if let Some(cutoff) = clip_width {
            let width = (((cutoff - raster.origin_x) * scale).floor().max(0.0) as u32)
                .min(raster.image.width);
            if width == 0 {
                return Ok(TextRasterizationBuffer {
                    text_bounds: bounds,
                    entries: Vec::new(),
                });
            }
            if width < raster.image.width {
                raster.image.data = raster
                    .image
                    .data
                    .chunks_exact(raster.image.width as usize * 4)
                    .flat_map(|row| row[..width as usize * 4].iter().copied())
                    .collect();
                raster.image.width = width;
            }
        }
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

#[allow(
    clippy::too_many_arguments,
    reason = "Keep the explicit inputs of the existing layout and rendering pipeline."
)]
fn measure_text_width_with_typst(
    typst: &avenger_typst_label::LabelEngine,
    math: &TextMarkupConfig,
    text: &str,
    font: &str,
    font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
    params: &avenger_typst_label::LabelParams,
    number_format: Option<&std::sync::Arc<dyn crate::NumberFormatProvider>>,
    datetime_format: Option<&std::sync::Arc<dyn crate::DateTimeFormatProvider>>,
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
        number_format,
        datetime_format,
    )
    .map(|result| result.label.metrics.width)?)
}

pub(crate) struct TypesetLineResult {
    pub(crate) label: avenger_typst_label::CompiledLabel,
    pub(crate) has_math_spans: bool,
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep the explicit inputs of the existing layout and rendering pipeline."
)]
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
    number_format: Option<&std::sync::Arc<dyn crate::NumberFormatProvider>>,
    datetime_format: Option<&std::sync::Arc<dyn crate::DateTimeFormatProvider>>,
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
    );
    let label = if math.syntax_mode == crate::types::TextSyntaxMode::Plain {
        typst.compile_text(text, &options)?
    } else {
        typst.compile_with_formatting(
            text,
            &options,
            avenger_typst_label::LabelFormatting {
                number: number_format,
                datetime: datetime_format,
            },
        )?
    };
    for warning in &label.warnings {
        tracing::warn!(?warning, "label typesetting warning");
    }
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
    crate::measurement::prepare_text_to_limit_with(
        config.text,
        config.syntax_mode,
        config.limit,
        measure_width,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep the explicit inputs of the existing layout and rendering pipeline."
)]
pub(crate) fn label_options(
    math: &TextMarkupConfig,
    _text: &str,
    font: &str,
    font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
    color: [f32; 4],
    params: &avenger_typst_label::LabelParams,
) -> avenger_typst_label::LabelOptions {
    let font_family = if font.trim().is_empty() {
        avenger_typst_label::TextStyle::default().font_family
    } else {
        font.to_string()
    };

    avenger_typst_label::LabelOptions {
        text: avenger_typst_label::TextStyle {
            font_family,
            font_size,
            fill: avenger_color::AbsoluteColor::from_rgba(color),
            font_weight: typst_font_weight(font_weight),
            font_style: typst_font_style(font_style),
            lang: math.lang,
            region: math.region,
            dir: math.dir,
        },
        math: math.math_style.clone(),
        params: params.clone(),
        limits: math.limits,
        ..Default::default()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TextMetricParts {
    width: f32,
    height: f32,
    ascent: f32,
    descent: f32,
}

impl From<&avenger_typst_label::LabelMetrics> for TextMetricParts {
    /// A label aligns by its first line's baseline, so its ascent is the first line's and its
    /// descent the rest of its height.
    fn from(metrics: &avenger_typst_label::LabelMetrics) -> Self {
        let ascent = metrics.lines.first().map_or(0.0, |line| line.baseline);
        Self {
            width: metrics.width,
            height: metrics.height,
            ascent,
            descent: metrics.height - ascent,
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

fn typst_font_weight(weight: FontWeight) -> avenger_typst_label::FontWeight {
    match weight {
        FontWeight::Name(FontWeightNameSpec::Normal) => avenger_typst_label::FontWeight::REGULAR,
        FontWeight::Name(FontWeightNameSpec::Bold) => avenger_typst_label::FontWeight::BOLD,
        FontWeight::Number(value) => avenger_typst_label::FontWeight::from_number(
            value.round().clamp(1.0, u16::MAX as f32) as u16,
        ),
    }
}

fn typst_font_style(style: FontStyle) -> avenger_typst_label::FontStyle {
    match style {
        FontStyle::Normal => avenger_typst_label::FontStyle::Normal,
        FontStyle::Italic => avenger_typst_label::FontStyle::Italic,
    }
}

/// Whether a text item's text runs right to left. Its glyphs are in visual order, so their
/// clusters descend; a run of one cluster takes the direction of its script.
pub(crate) fn is_rtl(item: &avenger_typst_label::TextItem) -> bool {
    match (item.glyphs.first(), item.glyphs.last()) {
        (Some(first), Some(last)) if first.range.start != last.range.start => {
            first.range.start > last.range.start
        }
        _ => {
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(&item.text);
            buffer.guess_segment_properties();
            buffer.direction() == rustybuzz::Direction::RightToLeft
        }
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
            line_pitch: 16.5,
            lines: vec![avenger_typst_label::LineMetrics {
                left: 0.0,
                right: 20.0,
                top: 0.0,
                baseline: 7.0,
                bottom: 10.0,
            }],
        };

        let plain = bounds_from_metrics(&metrics, 16.0, false);
        let math = bounds_from_metrics(&metrics, 10.0, true);

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
        let typst = avenger_typst_label::LabelEngine::new(Default::default());
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
                number_format: None,
                datetime_format: None,
            })
            .unwrap();

        assert!(bounds.width > 0.0);
    }

    #[test]
    fn typst_numfmt_uses_number_locale_specs() {
        let typst = avenger_typst_label::LabelEngine::new(Default::default());
        let mut params = crate::LabelParams::default();
        params.insert("value".to_string(), crate::LabelParamValue::Float(1234.5));
        let config: std::sync::Arc<dyn crate::NumberFormatProvider> = std::sync::Arc::new(
            avenger_format_number_d3::D3NumberFormatProvider::new()
                .with_locale("tick-test")
                .with_custom_locale(
                    "tick-test",
                    avenger_format_number_d3::NumberLocaleSpec {
                        decimal: "~".into(),
                        thousands: "_".into(),
                        grouping: vec![3],
                        ..Default::default()
                    },
                ),
        );

        let result = typeset_line(
            &typst,
            &TextMarkupConfig::default().with_syntax_mode(TextSyntaxMode::TypstMarkup),
            "#numfmt(value, \",.1f\")",
            "sans-serif",
            12.0,
            WEIGHT,
            STYLE,
            [0.0, 0.0, 0.0, 1.0],
            &params,
            Some(&config),
            None,
        );

        assert_eq!(result.unwrap().label.semantic_text, "1_234~5");
    }

    #[test]
    fn typst_datetimefmt_uses_datetime_locale_specs() {
        let typst = avenger_typst_label::LabelEngine::new(Default::default());
        let mut params = crate::LabelParams::default();
        params.insert(
            "value".to_string(),
            crate::LabelParamValue::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 5).unwrap()),
        );
        let config: std::sync::Arc<dyn crate::DateTimeFormatProvider> = std::sync::Arc::new(
            avenger_format_datetime_d3::D3DateTimeFormatProvider::new()
                .with_locale("label-date-test")
                .with_custom_locale(
                    "label-date-test",
                    avenger_format_datetime_d3::DateTimeLocaleSpec {
                        date: "%Y~%m~%d".into(),
                        ..Default::default()
                    },
                ),
        );
        let result = typeset_line(
            &typst,
            &TextMarkupConfig::default().with_syntax_mode(TextSyntaxMode::TypstMarkup),
            "#datetimefmt(value, \"%x\")",
            "sans-serif",
            12.0,
            WEIGHT,
            STYLE,
            [0.0, 0.0, 0.0, 1.0],
            &params,
            None,
            Some(&config),
        )
        .expect("datetimefmt label");

        assert_eq!(result.label.semantic_text, "2024~01~05");
    }

    #[test]
    fn raster_text_limit_zero_and_infinity_skip_measurement() {
        let text = "Price $7".to_string();
        let font = "sans-serif".to_string();
        let color = [0.0, 0.0, 0.0, 1.0];

        for limit in [0.0, f32::INFINITY] {
            let config = TextRasterizationConfig {
                text: &text,
                color,
                font: &font,
                font_size: 12.0,
                font_weight: WEIGHT,
                font_style: STYLE,
                limit,
                syntax_mode: TextSyntaxMode::Plain,
                params: crate::empty_label_params(),
                number_format: None,
                datetime_format: None,
            };

            let raster_text = truncate_raster_text(&config, |_candidate| {
                panic!("no-limit text should not be measured for truncation")
            })
            .unwrap();

            assert_eq!(raster_text, text);
        }
    }

    #[test]
    fn typst_rasterizer_reports_one_line_entry_with_typst_engine() {
        let rasterizer = TextLineRasterizer::<()>::new(
            avenger_typst_label::LabelEngine::new(Default::default()),
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
                    number_format: None,
                    datetime_format: None,
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
            avenger_typst_label::LabelEngine::new(Default::default()),
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
                    number_format: None,
                    datetime_format: None,
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
