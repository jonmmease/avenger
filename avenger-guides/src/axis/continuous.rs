use arrow::{
    array::{Array, ArrayRef},
    datatypes::DataType,
};
use avenger_common::types::{FontStyle, TextSyntaxMode};
use avenger_format::{FormattedNumber, NumberTypesetting, PreparedFormatter};
use avenger_scales::scales::{time, ConfiguredScale};
use avenger_scenegraph::marks::{group::SceneGroup, text::text_style};
use avenger_typst_label::LabelEngine;

use super::{
    marks::{make_axis, AxisParts, TickLabels},
    opts::{AxisConfig, AxisOrientation, AxisTickInterval},
    pixel_range, tick_label_text, tick_labels, tick_numbers, TickText,
};
use crate::error::AvengerGuidesError;

/// The gap between the labels and the title.
const TITLE_MARGIN: f32 = 2.0;
const DEFAULT_MAX_TICK_COUNT: f64 = 10.0;
const HORIZONTAL_MIN_TICK_SPACING_PX: f32 = 25.0;
const VERTICAL_TICK_SPACING_FONT_HEIGHT_FACTOR: f32 = 1.6;

/// Axis marks for a scale with a numeric or temporal domain, such as a linear, log, or time
/// scale, with tick labels from `config.tick_format`.
pub fn make_continuous_axis_marks(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
    text_engine: &LabelEngine,
) -> Result<SceneGroup, AvengerGuidesError> {
    // For scales with a band option, make sure ticks end up centered in the band.
    // Other scales reject unknown options.
    let has_band = scale
        .scale_impl
        .option_definitions()
        .iter()
        .any(|option| option.name == "band");
    let scale = if has_band {
        scale.clone().with_option("band", 0.5)
    } else {
        scale.clone()
    };

    let tick_count = tick_count(config, text_engine);
    let (ticks, label_ticks) = axis_ticks(&scale, config, tick_count)?;
    let positions = |ticks: &ArrayRef| -> Result<Vec<f32>, AvengerGuidesError> {
        Ok(scale.scale_to_numeric(ticks)?.as_vec(ticks.len(), None))
    };
    let labels = if config.style.labels_visible {
        let (text, syntax) = tick_label_text(
            continuous_tick_text(&label_ticks, &scale, config)?,
            config.tick_label.as_deref(),
        )?;
        TickLabels {
            positions: positions(&label_ticks)?,
            text,
            syntax,
        }
    } else {
        TickLabels::default()
    };
    let parts = AxisParts {
        range: pixel_range(&scale)?,
        ticks: positions(&ticks)?,
        labels,
        title,
        title_margin: TITLE_MARGIN,
    };
    make_axis(parts, origin, config, text_engine)
}

/// The axis's tick count, or one fitted to the axis length.
fn tick_count(config: &AxisConfig, text_engine: &LabelEngine) -> Option<f64> {
    config
        .style
        .tick_count
        .or_else(|| adaptive_tick_count(config, text_engine))
}

/// The labels an axis with this configuration shows, in tick order, before they are placed.
pub(crate) fn axis_tick_labels(
    scale: &ConfiguredScale,
    config: &AxisConfig,
    text_engine: &LabelEngine,
) -> Result<(Vec<String>, TextSyntaxMode), AvengerGuidesError> {
    let (_, ticks) = axis_ticks(scale, config, tick_count(config, text_engine))?;
    tick_label_text(
        continuous_tick_text(&ticks, scale, config)?,
        config.tick_label.as_deref(),
    )
}

/// The ticks an axis draws, and the ones it labels.
fn axis_ticks(
    scale: &ConfiguredScale,
    config: &AxisConfig,
    tick_count: Option<f64>,
) -> Result<(ArrayRef, ArrayRef), AvengerGuidesError> {
    let ticks = match config.style.tick_start_step {
        Some(AxisTickInterval::Numeric { start, step }) => scale.start_step_ticks(start, step)?,
        Some(AxisTickInterval::Temporal {
            start_millis,
            months,
            days,
            nanos,
        }) => scale.temporal_start_step_ticks(start_millis, months, days, nanos)?,
        None => {
            let ticks = scale.ticks(tick_count)?;
            let labeled = scale.label_ticks(&ticks, tick_count);
            return Ok((ticks, labeled));
        }
    };
    Ok((ticks.clone(), ticks))
}

fn adaptive_tick_count(config: &AxisConfig, text_engine: &LabelEngine) -> Option<f64> {
    // Estimate reasonable tick count from available axis length.
    // For vertical axes (Left/Right), use height; for horizontal (Top/Bottom), use width.
    let axis_length_px = match config.orientation {
        AxisOrientation::Left | AxisOrientation::Right => config.dimensions[1],
        AxisOrientation::Top | AxisOrientation::Bottom => config.dimensions[0],
    };

    let min_tick_spacing = match config.orientation {
        AxisOrientation::Left | AxisOrientation::Right => {
            vertical_min_tick_spacing_px(config, text_engine)
        }
        AxisOrientation::Top | AxisOrientation::Bottom => HORIZONTAL_MIN_TICK_SPACING_PX,
    };

    let adaptive_count = f64::from((axis_length_px / min_tick_spacing).max(2.0));

    // Only override the default scale tick count if the axis is small enough to need fewer ticks.
    if adaptive_count < DEFAULT_MAX_TICK_COUNT {
        Some(adaptive_count)
    } else {
        None
    }
}

fn vertical_min_tick_spacing_px(config: &AxisConfig, text_engine: &LabelEngine) -> f32 {
    let style = &config.style;
    let font_height = text_engine
        .font_metrics(&text_style(
            &style.label_font_family,
            style.label_font_size,
            style.label_font_weight,
            FontStyle::Normal,
            style.label_color,
        ))
        .map_or(style.label_font_size, |metrics| {
            metrics.ascent + metrics.descent
        });

    (font_height * VERTICAL_TICK_SPACING_FONT_HEIGHT_FACTOR).max(1.0)
}

/// The ticks as the axis's formatter writes them, together, so that they share digits.
fn continuous_tick_text(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    config: &AxisConfig,
) -> Result<Vec<TickText>, AvengerGuidesError> {
    check_timezone(ticks, scale, &config.tick_format)?;
    let spacing = scale.tick_spacing();
    if let (PreparedFormatter::Number(format), true) =
        (&config.tick_format, ticks.data_type().is_numeric())
    {
        let labels = format.format_ticks(&tick_numbers(ticks)?, spacing);
        return Ok(labels.iter().map(number_tick_text).collect());
    }
    let labels = tick_labels(ticks, &config.tick_format, spacing)?;
    Ok(labels
        .as_vec(ticks.len(), None)
        .into_iter()
        .map(TickText::Plain)
        .collect())
}

/// Zoned labels must use the timezone that the scale places ticks in.
fn check_timezone(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    format: &PreparedFormatter,
) -> Result<(), AvengerGuidesError> {
    if let (DataType::Timestamp(_, Some(_)), PreparedFormatter::ZonedDateTime(zoned)) =
        (ticks.data_type(), format)
    {
        let scale_timezone = time::timezone(&scale.config)?;
        if zoned.timezone() != scale_timezone {
            return Err(AvengerGuidesError::TimezoneMismatch {
                scale: scale_timezone,
                formatter: zoned.timezone(),
            });
        }
    }
    Ok(())
}

/// A formatted number as a tick. Scientific notation is math, as `#numfmt` sets it: the mantissa
/// is a string, which math sets as text, so that a localized mantissa such as "1,2" keeps its
/// punctuation, and its sign is a minus.
fn number_tick_text(formatted: &FormattedNumber) -> TickText {
    match &formatted.typesetting {
        NumberTypesetting::Plain => TickText::Plain(formatted.text.clone()),
        NumberTypesetting::Exponent { mantissa, exponent } => {
            let (sign, digits) = match mantissa.strip_prefix(['-', '\u{2212}']) {
                Some(digits) => ("-", digits),
                None => ("", mantissa.as_str()),
            };
            let digits = digits.replace('\\', "\\\\").replace('"', "\\\"");
            TickText::Markup(format!("${sign}#\"{digits}\" times 10^({exponent})$"))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::opts::AxisStyle;
    use super::*;
    use arrow::array::{AsArray, Date32Array, Float64Array};
    use arrow::datatypes::Float64Type;
    use avenger_format::{DateTimeFormatProvider, NumberFormatProvider};
    use avenger_format_datetime_d3::D3DateTimeFormatProvider;
    use avenger_format_number_d3::D3NumberFormatProvider;
    use avenger_scales::scales::linear::LinearScale;
    use avenger_scales::scales::time::TimeScale;
    use avenger_scenegraph::marks::text::text_label;
    use avenger_typst_label::{bundled_font_options, EngineOptions, FontOptions, LabelOptions};

    fn config(pattern: &str, tick_label: Option<&str>, style: AxisStyle) -> AxisConfig {
        AxisConfig {
            orientation: AxisOrientation::Bottom,
            dimensions: [100.0, 100.0],
            grid: false,
            tick_format: D3NumberFormatProvider::new()
                .prepare(pattern)
                .unwrap()
                .into(),
            tick_label: tick_label.map(str::to_string),
            style,
        }
    }

    fn values(array: &ArrayRef) -> Vec<f64> {
        array.as_primitive::<Float64Type>().values().to_vec()
    }

    /// Tick label text as the axis shows it.
    fn label_text(
        ticks: &ArrayRef,
        scale: &ConfiguredScale,
        config: &AxisConfig,
    ) -> (Vec<String>, TextSyntaxMode) {
        let ticks = continuous_tick_text(ticks, scale, config).unwrap();
        tick_label_text(ticks, config.tick_label.as_deref()).unwrap()
    }

    fn engine() -> LabelEngine {
        LabelEngine::new(EngineOptions {
            fonts: FontOptions {
                load_system_fonts: false,
                ..bundled_font_options()
            },
        })
    }

    /// Start-step settings replace the scale's ticks and label every one of them.
    #[test]
    fn start_step_ticks_replace_the_scale_ticks() {
        let scale = LinearScale::configured((0.0, 0.035), (0.0, 100.0));
        let style = AxisStyle {
            tick_start_step: Some(AxisTickInterval::Numeric {
                start: 0.0,
                step: 0.01,
            }),
            ..Default::default()
        };
        let numbers = config(",", None, style);
        let (ticks, labeled) = axis_ticks(&scale, &numbers, None).unwrap();
        assert_eq!(values(&ticks), vec![0.0, 0.01, 0.02, 0.03]);
        assert_eq!(values(&labeled), values(&ticks));
        let (text, _) = label_text(&ticks, &scale, &numbers);
        assert_eq!(text, ["0", "0.01", "0.02", "0.03"]);

        let start = Arc::new(Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(Date32Array::from(vec![19800])) as ArrayRef;
        let scale = TimeScale::configured((start, end), (0.0, 100.0));
        let style = AxisStyle {
            tick_start_step: Some(AxisTickInterval::Temporal {
                start_millis: 1_704_067_200_000,
                months: 1,
                days: 0,
                nanos: 0,
            }),
            ..Default::default()
        };
        let format = D3DateTimeFormatProvider::new()
            .prepare_date("%b %d")
            .unwrap();
        let dates = AxisConfig {
            tick_format: format.into(),
            ..config(",", None, style)
        };
        let (ticks, _) = axis_ticks(&scale, &dates, None).unwrap();
        let (text, _) = label_text(&ticks, &scale, &dates);
        assert_eq!(text, ["Jan 01", "Feb 01", "Mar 01"]);
    }

    /// Exponent ticks are Typst math, whose mantissa is text, so that a localized mantissa such
    /// as "1,2" keeps its punctuation.
    #[test]
    fn exponent_ticks_are_math_with_text_mantissas() {
        let scale = LinearScale::configured((0.0, 2000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1200.0])) as ArrayRef;
        let (text, syntax) = label_text(&ticks, &scale, &config(".1e", None, Default::default()));
        assert_eq!(syntax, TextSyntaxMode::TypstMarkup);
        assert_eq!(text, [r#"$#"1.2" times 10^(3)$"#]);

        let label = |mantissa: &str, exponent| match number_tick_text(&FormattedNumber {
            text: String::new(),
            typesetting: NumberTypesetting::Exponent {
                mantissa: mantissa.into(),
                exponent,
            },
        }) {
            TickText::Markup(markup) => markup,
            tick => panic!("{tick:?}"),
        };
        assert_eq!(label("1,2", 3), r#"$#"1,2" times 10^(3)$"#);
        assert_eq!(label("\u{2212}1,2", -3), r#"$-#"1,2" times 10^(-3)$"#);

        // A comma in the mantissa is text, not math punctuation.
        let engine = engine();
        let measure = |text: &str| {
            engine
                .bounds(&text_label(
                    text,
                    TextSyntaxMode::TypstMarkup,
                    LabelOptions::default(),
                ))
                .unwrap()
                .width
        };
        assert!(measure(&label("1,2", 3)) < measure("$1,2 times 10^(3)$"));
    }

    #[test]
    fn tick_labels_bind_each_tick() {
        let scale = LinearScale::configured((0.0, 2000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1000.0, 2000.0])) as ArrayRef;
        let units = config(",.0f", Some("#tick m/s, `#tick`"), Default::default());
        let (text, syntax) = label_text(&ticks, &scale, &units);
        assert_eq!(syntax, TextSyntaxMode::TypstMarkup);
        assert_eq!(
            text,
            [r#"#("1,000") m/s, `#tick`"#, r#"#("2,000") m/s, `#tick`"#]
        );

        // Exponent ticks are math in the label.
        let ticks = Arc::new(Float64Array::from(vec![1200.0])) as ArrayRef;
        let exponent = config(".1e", Some("*#tick*"), Default::default());
        let (text, _) = label_text(&ticks, &scale, &exponent);
        assert_eq!(text, [r#"*#([$#"1.2" times 10^(3)$])*"#]);
        for text in text {
            engine()
                .bounds(&text_label(
                    &text,
                    TextSyntaxMode::TypstMarkup,
                    LabelOptions::default(),
                ))
                .unwrap();
        }

        // Dates are bound as strings, so markup characters in them show as written.
        let start = Arc::new(Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(Date32Array::from(vec![19754])) as ArrayRef;
        let scale = TimeScale::configured((start.clone(), end), (0.0, 100.0));
        let format = D3DateTimeFormatProvider::new()
            .prepare_date("#%d*")
            .unwrap();
        let dates = AxisConfig {
            tick_format: format.into(),
            ..config(",", Some("[#tick]"), Default::default())
        };
        let (text, _) = label_text(&start, &scale, &dates);
        assert_eq!(text, [r##"[#("#01*")]"##]);
    }
}
