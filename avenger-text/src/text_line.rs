use std::{collections::HashMap, marker::PhantomData};

use ordered_float::OrderedFloat;

use crate::{
    error::AvengerTextError,
    measurement::{FontMetrics, FontMetricsConfig, TextBounds, TextMeasurementConfig},
    rasterization::{
        TextRasterBBox, TextRasterCacheKey, TextRasterCacheValue, TextRasterEntry,
        TextRasterPosition, TextRasterizationBuffer, TextRasterizationConfig,
    },
    types::{FontStyle, FontWeight, FontWeightNameSpec, TextLayout, TextSyntaxMode},
};

use crate::math::TextMarkupConfig;

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
        let label = typeset_line(
            &self.typst,
            &math,
            config.text,
            config.font,
            config.font_size,
            config.font_weight,
            config.font_style,
            [0.0, 0.0, 0.0, 1.0],
            &config.layout,
            config.params,
            config.number_format,
            config.datetime_format,
        )?;
        Ok(bounds_from_metrics(&label.metrics, config.font_size))
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
        let fill = color_key(&config.color);
        let cache_key = TextRasterCacheKey {
            text: config.text.to_string(),
            layout: format!("{:?}", config.layout),
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

        let label = typeset_line(
            &self.typst,
            &math,
            config.text,
            config.font,
            config.font_size,
            config.font_weight,
            config.font_style,
            config.color,
            &config.layout,
            config.params,
            config.number_format,
            config.datetime_format,
        )?;
        let bounds = bounds_from_metrics(&label.metrics, config.font_size);
        if config.text.is_empty() {
            return Ok(TextRasterizationBuffer {
                text_bounds: bounds,
                entries: Vec::new(),
            });
        }
        let baseline = first_baseline(&label.metrics);
        let raster =
            avenger_typst_label::rasterize(&label, &avenger_typst_label::RasterOptions { scale })?;
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
                    y: raster.origin_y - baseline,
                    physical_x: (raster.origin_x * scale).round(),
                    physical_y: ((raster.origin_y - baseline) * scale).round(),
                },
            )],
        })
    }
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
    layout: &TextLayout,
    params: &avenger_typst_label::LabelParams,
    number_format: Option<&std::sync::Arc<dyn crate::NumberFormatProvider>>,
    datetime_format: Option<&std::sync::Arc<dyn crate::DateTimeFormatProvider>>,
) -> Result<avenger_typst_label::CompiledLabel, avenger_typst_label::LabelError> {
    let options = label_options(
        math,
        font,
        font_size,
        font_weight,
        font_style,
        color,
        layout,
        params,
    );
    let label = if math.syntax_mode != TextSyntaxMode::TypstMarkup {
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
    Ok(label)
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep the explicit inputs of the existing layout and rendering pipeline."
)]
pub(crate) fn label_options(
    math: &TextMarkupConfig,
    font: &str,
    font_size: f32,
    font_weight: FontWeight,
    font_style: FontStyle,
    color: [f32; 4],
    layout: &TextLayout,
    params: &avenger_typst_label::LabelParams,
) -> avenger_typst_label::LabelOptions {
    use avenger_typst_label::{LabelLineHeight, LabelWidth};
    let font_family = if font.trim().is_empty() {
        avenger_typst_label::TextStyle::default().font_family
    } else {
        font.to_string()
    };
    // A distance the label crate would reject means none, since that error has no plain-text
    // fallback.
    let valid = |value: f32| value.is_finite() && value >= 0.0;
    let width = match layout.width {
        LabelWidth::Max(width) | LabelWidth::Fixed(width) if !valid(width) => LabelWidth::Auto,
        width => width,
    };
    let line_height = match layout.line_height {
        LabelLineHeight::Fixed(value) | LabelLineHeight::Relative(value) if !valid(value) => {
            LabelLineHeight::Auto
        }
        line_height => line_height,
    };

    avenger_typst_label::LabelOptions {
        width,
        wrap: layout.wrap,
        align: layout.align,
        line_height,
        max_lines: layout.max_lines,
        ellipsis: layout.ellipsis,
        newline_breaks: math.syntax_mode == TextSyntaxMode::PlainLines,
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

/// The metrics a label's text box comes from.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TextMetricParts {
    width: f32,
    height: f32,
    /// The first line's baseline, from the label's top.
    ascent: f32,
    /// The first line's height and the last line's.
    first_line: f32,
    last_line: f32,
    /// The distance between plain lines' baselines.
    line_pitch: f32,
}

impl From<&avenger_typst_label::LabelMetrics> for TextMetricParts {
    fn from(metrics: &avenger_typst_label::LabelMetrics) -> Self {
        let height = |line: Option<&avenger_typst_label::LineMetrics>| {
            line.map_or(0.0, |line| line.bottom - line.top)
        };
        Self {
            width: metrics.width,
            height: metrics.height,
            ascent: first_baseline(metrics),
            first_line: height(metrics.lines.first()),
            last_line: height(metrics.lines.last()),
            line_pitch: metrics.line_pitch,
        }
    }
}

/// The first line's baseline, from the label's top. A label aligns by it.
pub(crate) fn first_baseline(metrics: &avenger_typst_label::LabelMetrics) -> f32 {
    metrics.lines.first().map_or(0.0, |line| line.baseline)
}

/// A label's text box: its lines, with the first line's top and the last line's bottom padded
/// so that each is at least the font size tall, half the shortfall on either side, and the gap
/// that plain lines leave between such boxes.
pub(crate) fn bounds_from_metrics(
    metrics: impl Into<TextMetricParts>,
    font_size: f32,
) -> TextBounds {
    let metrics = metrics.into();
    let size = font_size.max(1.0);
    let top = (size - metrics.first_line).max(0.0) / 2.0;
    let bottom = (size - metrics.last_line).max(0.0) / 2.0;
    TextBounds {
        width: metrics.width,
        height: metrics.height + top + bottom,
        ascent: metrics.ascent + top,
        descent: metrics.height - metrics.ascent + bottom,
        leading: metrics.line_pitch - size,
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
    fn label_options_take_the_layout() {
        use avenger_typst_label::{LabelAlign, LabelLineHeight, LabelWidth};
        let options = |syntax, layout: &TextLayout| {
            label_options(
                &TextMarkupConfig::default().with_syntax_mode(syntax),
                "sans-serif",
                12.0,
                WEIGHT,
                STYLE,
                [0.0, 0.0, 0.0, 1.0],
                layout,
                crate::empty_label_params(),
            )
        };
        let layout = TextLayout {
            width: LabelWidth::Fixed(80.0),
            wrap: false,
            max_lines: std::num::NonZeroUsize::new(2),
            ellipsis: true,
            line_height: LabelLineHeight::Relative(1.1),
            align: LabelAlign::Center,
        };
        let label = options(TextSyntaxMode::Plain, &layout);
        assert_eq!(
            (label.width, label.wrap, label.max_lines, label.ellipsis),
            (layout.width, false, layout.max_lines, true)
        );
        assert_eq!(
            (label.line_height, label.align),
            (layout.line_height, layout.align)
        );
        assert!(!label.newline_breaks);
        assert!(options(TextSyntaxMode::PlainLines, &layout).newline_breaks);
        assert!(!options(TextSyntaxMode::TypstMarkup, &layout).newline_breaks);

        // Distances the label crate would reject mean none.
        let invalid = TextLayout {
            width: LabelWidth::Max(f32::NAN),
            line_height: LabelLineHeight::Fixed(-1.0),
            ..layout
        };
        let label = options(TextSyntaxMode::Plain, &invalid);
        assert_eq!(
            (label.width, label.line_height),
            (LabelWidth::Auto, LabelLineHeight::Auto)
        );
    }

    #[test]
    fn text_boxes_pad_lines_to_the_font_size() {
        let line = |top: f32, baseline: f32, bottom: f32| avenger_typst_label::LineMetrics {
            left: 0.0,
            right: 20.0,
            top,
            baseline,
            bottom,
        };
        let one = avenger_typst_label::LabelMetrics {
            width: 20.0,
            height: 10.0,
            line_pitch: 16.5,
            lines: vec![line(0.0, 7.0, 10.0)],
        };

        // A line shorter than the font size gets the rest, half above and half below; the gap
        // between plain lines' boxes is their pitch less the font size.
        let padded = bounds_from_metrics(&one, 16.0);
        assert_eq!(
            (padded.width, padded.height, padded.ascent, padded.descent),
            (20.0, 16.0, 10.0, 6.0)
        );
        assert_eq!(padded.leading, 0.5);
        // A line as tall as the font size, as math can be, gets none.
        let tall = bounds_from_metrics(&one, 10.0);
        assert_eq!((tall.height, tall.ascent, tall.descent), (10.0, 7.0, 3.0));
        assert_eq!(tall.leading, 6.5);

        // Several lines pad the first line's top and the last line's bottom.
        let two = avenger_typst_label::LabelMetrics {
            width: 20.0,
            height: 26.5,
            line_pitch: 16.5,
            lines: vec![line(0.0, 7.0, 10.0), line(16.5, 23.5, 26.5)],
        };
        let padded = bounds_from_metrics(&two, 16.0);
        assert_eq!(
            (padded.height, padded.ascent, padded.descent),
            (32.5, 10.0, 22.5)
        );
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
                    layout: TextLayout::default(),
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
