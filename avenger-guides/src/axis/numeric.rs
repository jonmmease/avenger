use std::sync::Arc;

use arrow::{
    array::{
        Array, ArrayRef, AsArray, Date64Array, Float32Array, TimestampMicrosecondArray,
        TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray,
    },
    compute::cast,
    datatypes::{DataType, Date32Type, Float32Type, Float64Type, TimeUnit},
};
use avenger_color::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_format_datetime::{
    format_naive_datetime, format_zoned_datetime, parse_datetime_timezone, DateTimeFormatContext,
    DateTimeFormatOverrides, DateTimeLocaleRegistry, DateTimeStyleLength, NaiveDateTimeInput,
    ResolvedDateTimeLocale,
};
use avenger_format_number::{
    prepare_number_tick_format, Align, CurrencyDisplay, DigitSpec, ExponentMarker, FormatType,
    FormattedNumber, NumberFormatContext, NumberFormatOverrides, NumberLocaleRegistry,
    NumberTypesetting, PreparedNumberTickFormat, ResolvedNumberLocale, SignPolicy, Symbol,
};
use avenger_geometry::{marks::MarkGeometryUtils, rtree::EnvelopeUtils};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::SceneGroup, rule::SceneRuleMark, text::SceneTextMark};
use avenger_text::{
    default_text_engine,
    measurement::{FontMetricsConfig, TextMeasurementConfig},
    types::TextSyntaxMode,
    types::{FontStyle, FontWeight, TextAlign, TextBaseline},
    TextEngine,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use rstar::AABB;

use super::opts::{AxisConfig, AxisOrientation, AxisTickSpacing};
use crate::error::AvengerGuidesError;

const TEXT_MARGIN: f32 = 3.0;
const TITLE_MARGIN: f32 = 2.0;
const PIXEL_OFFSET: f32 = 0.5;

const DEFAULT_TICK_LENGTH: f32 = 5.0;
const DEFAULT_TITLE_FONT_SIZE: f32 = 12.0;
const DEFAULT_TICK_FONT_SIZE: f32 = 12.0;
const DEFAULT_MAX_TICK_COUNT: f32 = 10.0;
const HORIZONTAL_MIN_TICK_SPACING_PX: f32 = 25.0;
const VERTICAL_TICK_SPACING_FONT_HEIGHT_FACTOR: f32 = 1.6;
const MAX_START_STEP_TICKS: usize = 10_000;

pub fn make_numeric_axis_marks(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
) -> Result<SceneGroup, AvengerGuidesError> {
    let text_engine = avenger_text::default_text_engine();
    make_numeric_axis_marks_with_text_engine(scale, title, origin, config, &text_engine)
}

fn make_numeric_axis_marks_with_text_engine(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
    text_engine: &TextEngine,
) -> Result<SceneGroup, AvengerGuidesError> {
    // For scales with a band option, make sure ticks end up centered in the band
    let scale = if scale.option("band").is_some() {
        scale.clone().with_option("band", 0.5)
    } else {
        scale.clone()
    };

    // Build main group with origin [0, 0] so we can work in local coordinate.
    // Then set final original at the end
    let mut main_group = SceneGroup {
        origin: [0.0, 0.0],
        ..Default::default()
    };

    let (ticks, label_ticks) = if let Some(tick_spacing) = config.tick_start_step {
        match tick_spacing {
            AxisTickSpacing::Numeric { start, step } => {
                let ticks = start_step_ticks(&scale, start, step)?;
                (ticks.clone(), ticks)
            }
            AxisTickSpacing::Temporal {
                start_millis,
                months,
                days,
                nanos,
            } => {
                let ticks = scale.temporal_start_step_ticks(start_millis, months, days, nanos)?;
                (ticks.clone(), ticks)
            }
        }
    } else {
        // Compute tick count: use explicit value, or adapt to available pixel space.
        let tick_count = config
            .tick_count
            .or_else(|| adaptive_tick_count(config, text_engine));
        let ticks = scale.ticks(tick_count)?;
        let label_ticks =
            log_label_ticks(&ticks, &scale, tick_count.unwrap_or(DEFAULT_MAX_TICK_COUNT));
        (ticks, label_ticks)
    };

    // Get range bounds considering orientation
    let range = scale.numeric_interval_range()?;

    let (start, end) = match config.orientation {
        AxisOrientation::Left | AxisOrientation::Right => {
            let upper = f32::min(range.1, range.0) - PIXEL_OFFSET;
            let lower = f32::max(range.0, range.1) + PIXEL_OFFSET;
            (lower, upper)
        }
        AxisOrientation::Top | AxisOrientation::Bottom => {
            let left = f32::min(range.0, range.1) - PIXEL_OFFSET;
            let right = f32::max(range.0, range.1) + PIXEL_OFFSET;
            (left, right)
        }
    };

    // Add axis line
    let is_vertical = matches!(
        config.orientation,
        AxisOrientation::Left | AxisOrientation::Right
    );

    // Calculate axis position based on orientation and plot dimensions
    let offset = match config.orientation {
        AxisOrientation::Left => 0.0,
        AxisOrientation::Right => config.dimensions[0],
        AxisOrientation::Top => 0.0,
        AxisOrientation::Bottom => config.dimensions[1],
    };

    // Add tick grid if enabled
    if config.grid {
        let grid_group = make_tick_grid_marks(
            &ticks,
            &scale,
            &config.orientation,
            &config.dimensions,
            config.grid_color,
            config.grid_width,
        )?;
        main_group.marks.push(grid_group.into());
    }

    // Create a group for axis elements that should render above data marks
    let mut axis_elements_group = SceneGroup {
        origin: [0.0, 0.0],
        zindex: Some(1), // Axis elements above data marks
        ..Default::default()
    };

    // Add axis line
    axis_elements_group
        .marks
        .push(make_axis_line(start, end, is_vertical, offset, config.domain_color).into());

    // Add tick marks
    axis_elements_group.marks.push(
        make_tick_marks(
            &ticks,
            &scale,
            &config.orientation,
            &config.dimensions,
            config.tick_length,
            config.tick_color,
        )?
        .into(),
    );

    // Add tick labels (if visible)
    if config.labels_visible.unwrap_or(true) {
        axis_elements_group
            .marks
            .push(make_tick_labels(&label_ticks, &scale, config)?.into());
    }

    // Add title if visible and non-empty
    if config.title_visible.unwrap_or(true) && !title.is_empty() {
        axis_elements_group
            .marks
            .push(make_title(title, &scale, &axis_elements_group.bounding_box(), config)?.into());
    }

    // Add the axis elements group to the main group
    main_group.marks.push(axis_elements_group.into());

    // Measure the overall bounds to create a clip rect
    let bbox = main_group.bounding_box();

    // Add clip rect to define bounds
    // Use the actual bounding box coordinates, not assuming 0,0
    let padding = 2.0;
    main_group.clip = avenger_scenegraph::marks::group::Clip::Rect {
        x: bbox.lower()[0] - padding,
        y: bbox.lower()[1] - padding,
        width: bbox.width() + 2.0 * padding,
        height: bbox.height() + 2.0 * padding,
    };

    // Now set the actual origin
    main_group.origin = origin;

    Ok(main_group)
}

/// Log scales generate minor ticks for visual context. Label only the subset
/// that fits the requested density, following the D3 log tick-format policy.
fn log_label_ticks(ticks: &ArrayRef, scale: &ConfiguredScale, count: f32) -> ArrayRef {
    if scale.scale_impl.scale_type() != "log" {
        return ticks.clone();
    }
    let Some(values) = ticks.as_any().downcast_ref::<Float32Array>() else {
        return ticks.clone();
    };
    let base = scale.option_f32("base", 10.0);
    if !base.is_finite() || base <= 0.0 || base == 1.0 || values.is_empty() {
        return ticks.clone();
    }
    let threshold = (base * count / values.len() as f32).max(1.0);
    Arc::new(Float32Array::from_iter_values(
        values.values().iter().copied().filter(|value| {
            let power = base.powf(value.log(base).round());
            let mut coefficient = value / power;
            if coefficient * base < base - 0.5 {
                coefficient *= base;
            }
            coefficient <= threshold
        }),
    ))
}

fn start_step_ticks(
    scale: &ConfiguredScale,
    start: f32,
    step: f32,
) -> Result<ArrayRef, AvengerGuidesError> {
    if !start.is_finite() {
        return Err(AvengerGuidesError::InvalidAxisTicks(
            "tick start must be finite".to_string(),
        ));
    }
    if !step.is_finite() || step <= 0.0 {
        return Err(AvengerGuidesError::InvalidAxisTicks(
            "tick step must be finite and greater than zero".to_string(),
        ));
    }

    let domain = scale.normalized_domain()?;
    if domain.len() != 2 {
        return Err(AvengerGuidesError::InvalidAxisTicks(
            "start-step ticks require a two-value numeric scale domain".to_string(),
        ));
    }
    let domain = cast(domain.as_ref(), &DataType::Float32)
        .map_err(|err| AvengerGuidesError::InvalidAxisTicks(err.to_string()))?;
    let domain = domain.as_primitive::<Float32Type>();
    let domain_start = domain.value(0);
    let domain_end = domain.value(1);
    let lo = domain_start.min(domain_end);
    let hi = domain_start.max(domain_end);

    if !lo.is_finite() || !hi.is_finite() {
        return Err(AvengerGuidesError::InvalidAxisTicks(
            "scale domain must be finite for start-step ticks".to_string(),
        ));
    }

    let first_index = ((lo - start) / step).ceil();
    if !first_index.is_finite() {
        return Err(AvengerGuidesError::InvalidAxisTicks(
            "failed to compute first tick index".to_string(),
        ));
    }

    let epsilon = (step.abs() * 1e-4).max(f32::EPSILON);
    let mut value = start + first_index * step;
    let mut ticks = Vec::new();
    while value <= hi + epsilon {
        if value >= lo - epsilon {
            if ticks.len() >= MAX_START_STEP_TICKS {
                return Err(AvengerGuidesError::InvalidAxisTicks(format!(
                    "start-step ticks would generate more than {MAX_START_STEP_TICKS} ticks"
                )));
            }
            ticks.push(if value.abs() <= epsilon { 0.0 } else { value });
        }
        value += step;
    }

    Ok(Arc::new(Float32Array::from(ticks)) as ArrayRef)
}

fn adaptive_tick_count(config: &AxisConfig, text_engine: &TextEngine) -> Option<f32> {
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

    let adaptive_count = (axis_length_px / min_tick_spacing).max(2.0);

    // Only override the default scale tick count if the axis is small enough to need fewer ticks.
    if adaptive_count < DEFAULT_MAX_TICK_COUNT {
        Some(adaptive_count)
    } else {
        None
    }
}

fn vertical_min_tick_spacing_px(config: &AxisConfig, text_engine: &TextEngine) -> f32 {
    let font_size = config.label_font_size.unwrap_or(DEFAULT_TICK_FONT_SIZE);
    let font_weight = FontWeight::Number(config.label_font_weight.unwrap_or(400.0));
    let font_family = config.label_font_family.as_deref().unwrap_or("sans-serif");
    let font_metrics = text_engine.font_metrics(&FontMetricsConfig {
        font: font_family,
        font_size,
        font_weight: font_weight,
        font_style: FontStyle::Normal,
    });

    (font_metrics.height * VERTICAL_TICK_SPACING_FONT_HEIGHT_FACTOR).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Float64Array;
    use avenger_format_number::{LocaleId, NumberLocaleSpec};
    use avenger_scales::scales::linear::LinearScale;
    use avenger_scales::scales::log::LogScale;
    use avenger_scales::scales::time::TimeScale;

    fn values(array: &ArrayRef) -> Vec<f32> {
        array
            .as_primitive::<Float32Type>()
            .values()
            .iter()
            .copied()
            .collect()
    }

    #[test]
    fn start_step_ticks_clip_to_domain() {
        let scale = LinearScale::configured((1.2, 8.8), (0.0, 100.0));
        let ticks = start_step_ticks(&scale, 0.0, 2.0).expect("ticks");
        assert_eq!(values(&ticks), vec![2.0, 4.0, 6.0, 8.0]);
    }

    #[test]
    fn start_step_ticks_support_reversed_domain() {
        let scale = LinearScale::configured((8.8, 1.2), (0.0, 100.0));
        let ticks = start_step_ticks(&scale, 0.0, 2.0).expect("ticks");
        assert_eq!(values(&ticks), vec![2.0, 4.0, 6.0, 8.0]);
    }

    #[test]
    fn start_step_ticks_reject_invalid_step() {
        let scale = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
        let err = start_step_ticks(&scale, 0.0, 0.0).expect_err("invalid step");
        assert!(matches!(err, AvengerGuidesError::InvalidAxisTicks(_)));
    }

    #[test]
    fn log_tick_labels_suppress_dense_minor_values() {
        let scale = LogScale::configured((0.1, 100.0), (0.0, 100.0)).with_option("base", 10.0);
        let ticks = scale.ticks(Some(10.0)).expect("ticks");
        let labels = log_label_ticks(&ticks, &scale, 10.0);

        assert_eq!(
            values(&labels),
            vec![0.1, 0.2, 0.3, 1.0, 2.0, 3.0, 10.0, 20.0, 30.0, 100.0]
        );
    }

    #[test]
    fn numeric_axis_tick_labels_honor_configured_angle() {
        let scale = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![0.0, 5.0, 10.0])) as ArrayRef;
        let labels = make_tick_labels(
            &ticks,
            &scale,
            &AxisConfig {
                label_angle: Some(-45.0),
                ..Default::default()
            },
        )
        .expect("tick labels");

        assert_eq!(
            labels.angle.as_vec(labels.len as usize, None),
            vec![-45.0, -45.0, -45.0]
        );
    }

    #[test]
    fn numeric_axis_title_forwards_text_params() {
        let scale = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
        let mut title_text_params = avenger_text::LabelParams::default();
        title_text_params.insert(
            "series".to_string(),
            avenger_text::LabelParamValue::Str("Revenue".to_string()),
        );

        let axis = make_numeric_axis_marks(
            &scale,
            "#series",
            [0.0, 0.0],
            &AxisConfig {
                title_syntax_mode: avenger_text::types::TextSyntaxMode::TypstMarkup,
                title_text_params: title_text_params.clone(),
                ..Default::default()
            },
        )
        .expect("axis renders");

        let text_marks = collect_text_marks(&axis);
        let title_mark = text_marks
            .iter()
            .find(|mark| mark.text.as_vec(1, None)[0] == "#series")
            .expect("title text mark");
        assert_eq!(title_mark.text_params, title_text_params);
    }

    #[test]
    fn bare_exponent_tick_format_uses_typst_markup() {
        let scale = LinearScale::configured((0.0, 2000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1200.0])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_number: Some(".1e".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(labels.text.as_vec(1, None), vec!["$1.2 times 10^(3)$"]);
    }

    #[test]
    fn bare_si_tick_format_locks_prefix_across_ticks() {
        let scale = LinearScale::configured((0.0, 2_000_000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![
            900_000.0,
            1_000_000.0,
            1_100_000.0,
        ])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_number: Some("s".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::Plain);
        assert_eq!(labels.text.as_vec(1, None), vec!["0.9M", "1.0M", "1.1M"]);
    }

    #[test]
    fn bare_tick_format_uses_configured_builtin_locale() {
        let scale = LinearScale::configured((0.0, 2000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1234.5])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_number: Some(",.1f".to_string()),
                number_locale: Some("de-DE".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::Plain);
        assert_eq!(labels.text.as_vec(1, None), vec!["1.234,5"]);
    }

    #[test]
    fn bare_tick_format_uses_custom_locale_registry() {
        let mut registry = NumberLocaleRegistry::with_builtins();
        registry
            .register_custom_locale_json(
                "tick-test",
                r#"{ "base": "en-US", "decimal": "~", "group": "_" }"#,
            )
            .expect("custom locale");
        let scale = LinearScale::configured((0.0, 2000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1234.5])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_number: Some(",.1f".to_string()),
                number_locale: Some("tick-test".to_string()),
                number_locale_registry: Some(Arc::new(registry)),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::Plain);
        assert_eq!(labels.text.as_vec(1, None), vec!["1_234~5"]);
    }

    #[test]
    fn bare_tick_format_uses_custom_locale_specs() {
        let scale = LinearScale::configured((0.0, 2000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1234.5])) as ArrayRef;
        let mut number_locale_specs = avenger_text::NumberLocaleSpecs::default();
        number_locale_specs.insert(
            "tick-test".to_string(),
            NumberLocaleSpec {
                base: Some(LocaleId::new("en-US")),
                decimal: Some("~".to_string()),
                group: Some("_".to_string()),
                ..Default::default()
            },
        );

        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_number: Some(",.1f".to_string()),
                number_locale: Some("tick-test".to_string()),
                number_locale_specs,
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::Plain);
        assert_eq!(labels.text.as_vec(1, None), vec!["1_234~5"]);
    }

    #[test]
    fn numfmt_tick_fragment_uses_value_context() {
        let scale = LinearScale::configured((0.0, 2000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1200.0])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_number: Some("v=#numfmt(value, \".1e\") m/s^2".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(
            labels.text.as_vec(1, None),
            vec!["v=$1.2 times 10^(3)$ m/s^2"]
        );
    }

    #[test]
    fn tick_label_fragment_takes_precedence_over_format_number() {
        let scale = LinearScale::configured((0.0, 2000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1200.0])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_number: Some(".0f".to_string()),
                tick_label: Some("v=#numfmt(value, \".1e\")".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(labels.text.as_vec(1, None), vec!["v=$1.2 times 10^(3)$"]);
    }

    #[test]
    fn datefmt_tick_fragment_formats_date32_ticks() {
        let start = Arc::new(arrow::array::Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(arrow::array::Date32Array::from(vec![19730])) as ArrayRef;
        let scale = TimeScale::configured((start, end), (0.0, 100.0));
        let ticks = Arc::new(arrow::array::Date32Array::from(vec![19727])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                tick_label: Some("d=#datefmt(value, \"MMM d, y\")".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(labels.text.as_vec(1, None), vec!["d=Jan 5, 2024"]);
    }

    #[test]
    fn default_date32_ticks_use_ldml_temporal_formatter() {
        let start = Arc::new(arrow::array::Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(arrow::array::Date32Array::from(vec![19730])) as ArrayRef;
        let scale = TimeScale::configured((start, end), (0.0, 100.0));
        let ticks = Arc::new(arrow::array::Date32Array::from(vec![19727])) as ArrayRef;
        let labels = make_tick_label_text(&ticks, &scale, &AxisConfig::default()).expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::Plain);
        assert_eq!(labels.text.as_vec(1, None), vec!["Jan 5"]);
    }

    #[test]
    fn datetime_format_formats_date32_ticks_directly() {
        let start = Arc::new(arrow::array::Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(arrow::array::Date32Array::from(vec![19730])) as ArrayRef;
        let scale = TimeScale::configured((start, end), (0.0, 100.0));
        let ticks = Arc::new(arrow::array::Date32Array::from(vec![19727])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_datetime: Some("MMM d, y".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::Plain);
        assert_eq!(labels.text.as_vec(1, None), vec!["Jan 5, 2024"]);
    }

    #[test]
    fn datetime_format_rejects_strftime_specs() {
        let start = Arc::new(arrow::array::Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(arrow::array::Date32Array::from(vec![19730])) as ArrayRef;
        let scale = TimeScale::configured((start, end), (0.0, 100.0));
        let ticks = Arc::new(arrow::array::Date32Array::from(vec![19727])) as ArrayRef;
        let err = match make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_datetime: Some("%Y".to_string()),
                ..Default::default()
            },
        ) {
            Ok(_) => panic!("strftime specs should fail"),
            Err(err) => err,
        };

        assert!(matches!(
            err,
            AvengerGuidesError::InvalidAxisLabelFormat(message)
                if message.contains("d3/strftime/chrono")
        ));
    }

    #[test]
    fn datefmt_tick_fragment_uses_configured_locale_specs() {
        let start = Arc::new(arrow::array::Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(arrow::array::Date32Array::from(vec![19730])) as ArrayRef;
        let scale = TimeScale::configured((start, end), (0.0, 100.0));
        let ticks = Arc::new(arrow::array::Date32Array::from(vec![19727])) as ArrayRef;
        let mut datetime_locale_specs = avenger_text::DateTimeLocaleSpecs::default();
        datetime_locale_specs.insert(
            "tick-date".to_string(),
            avenger_text::DateTimeLocaleSpec {
                date_patterns: Some(avenger_text::LengthsSpec {
                    long: Some("y'~'MM'~'dd".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );

        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                tick_label: Some("#datefmt(value, \"{date:long}\")".to_string()),
                datetime_locale: Some("tick-date".to_string()),
                datetime_locale_specs,
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(labels.text.as_vec(1, None), vec!["2024~01~05"]);
    }

    #[test]
    fn datefmt_tick_fragment_accepts_call_locale_override() {
        let start = Arc::new(arrow::array::Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(arrow::array::Date32Array::from(vec![19730])) as ArrayRef;
        let scale = TimeScale::configured((start, end), (0.0, 100.0));
        let ticks = Arc::new(arrow::array::Date32Array::from(vec![19727])) as ArrayRef;
        let mut datetime_locale_specs = avenger_text::DateTimeLocaleSpecs::default();
        datetime_locale_specs.insert(
            "tick-date".to_string(),
            avenger_text::DateTimeLocaleSpec {
                date_patterns: Some(avenger_text::LengthsSpec {
                    long: Some("y'~'MM'~'dd".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );

        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                tick_label: Some(
                    "#datefmt(value, \"{date:long}\", locale: \"tick-date\")".to_string(),
                ),
                datetime_locale_specs,
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(labels.text.as_vec(1, None), vec!["2024~01~05"]);
    }

    #[test]
    fn datefmt_tick_fragment_does_not_shift_naive_timestamps() {
        let start = Arc::new(TimestampMillisecondArray::from(vec![1_704_067_200_000])) as ArrayRef;
        let end = Arc::new(TimestampMillisecondArray::from(vec![1_704_070_800_000])) as ArrayRef;
        let scale = TimeScale::configured((start, end), (0.0, 100.0))
            .with_option("timezone", "America/New_York");
        let ticks = Arc::new(TimestampMillisecondArray::from(vec![1_704_067_200_000])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                tick_label: Some("#datefmt(value, \"y-MM-dd HH:mm\")".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.text.as_vec(1, None), vec!["2024-01-01 00:00"]);
    }

    #[test]
    fn datefmt_tick_fragment_does_not_shift_date64_ticks() {
        let start = Arc::new(Date64Array::from(vec![1_704_067_200_000])) as ArrayRef;
        let end = Arc::new(Date64Array::from(vec![1_704_070_800_000])) as ArrayRef;
        let scale = TimeScale::configured((start, end), (0.0, 100.0))
            .with_option("timezone", "America/New_York");
        let ticks = Arc::new(Date64Array::from(vec![1_704_067_200_000])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                tick_label: Some("#datefmt(value, \"y-MM-dd HH:mm\")".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.text.as_vec(1, None), vec!["2024-01-01 00:00"]);
    }

    #[test]
    fn datefmt_tick_fragment_shifts_zoned_timestamps() {
        let start = Arc::new(
            TimestampMillisecondArray::from(vec![1_704_067_200_000]).with_timezone_opt(Some("UTC")),
        ) as ArrayRef;
        let end = Arc::new(
            TimestampMillisecondArray::from(vec![1_704_070_800_000]).with_timezone_opt(Some("UTC")),
        ) as ArrayRef;
        let scale = TimeScale::configured((start, end), (0.0, 100.0))
            .with_option("timezone", "America/New_York");
        let ticks = Arc::new(
            TimestampMillisecondArray::from(vec![1_704_067_200_000]).with_timezone_opt(Some("UTC")),
        ) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                tick_label: Some("#datefmt(value, \"y-MM-dd HH:mm\")".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.text.as_vec(1, None), vec!["2023-12-31 19:00"]);
    }

    #[test]
    fn datefmt_tick_fragment_accepts_tz_alias_override() {
        let start = Arc::new(
            TimestampMillisecondArray::from(vec![1_704_067_200_000]).with_timezone_opt(Some("UTC")),
        ) as ArrayRef;
        let end = Arc::new(
            TimestampMillisecondArray::from(vec![1_704_070_800_000]).with_timezone_opt(Some("UTC")),
        ) as ArrayRef;
        let scale = TimeScale::configured((start, end), (0.0, 100.0));
        let ticks = Arc::new(
            TimestampMillisecondArray::from(vec![1_704_067_200_000]).with_timezone_opt(Some("UTC")),
        ) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                tick_label: Some(
                    "#datefmt(value, \"y-MM-dd HH:mm\", tz: \"America/New_York\")".to_string(),
                ),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(labels.text.as_vec(1, None), vec!["2023-12-31 19:00"]);
    }

    #[test]
    fn numfmt_tick_fragment_uses_configured_builtin_locale() {
        let scale = LinearScale::configured((0.0, 2_000_000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1_200_000.0])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_number: Some("#numfmt(value, \".1S\")".to_string()),
                number_locale: Some("de-DE".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(labels.text.as_vec(1, None), vec!["1,2\u{a0}Mio."]);
    }

    #[test]
    fn numfmt_tick_fragment_can_bind_set_precision() {
        let scale = LinearScale::configured((0.0, 2_000_000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![
            900_000.0,
            1_000_000.0,
            1_100_000.0,
        ])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_number: Some("#numfmt(value, \".3s\", precision: precision)".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(labels.text.as_vec(1, None), vec!["0.9M", "1.0M", "1.1M"]);
    }

    #[test]
    fn numfmt_tick_fragment_accepts_named_overrides() {
        let scale = LinearScale::configured((0.0, 2000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1234.5])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_number: Some(
                    "#numfmt(value, \"C[USD]\", currency: \"EUR\", currency_display: \"code\", fraction_digits: 0, group: false)"
                        .to_string(),
                ),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(labels.text.as_vec(1, None), vec!["EUR1234"]);
    }

    #[test]
    fn numfmt_tick_fragment_type_override_can_force_math_typesetting() {
        let scale = LinearScale::configured((0.0, 2000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1200.0])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_number: Some(
                    "#numfmt(value, \".3f\", type: \"e\", precision: 1)".to_string(),
                ),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(labels.text.as_vec(1, None), vec!["$1.2 times 10^(3)$"]);
    }

    #[test]
    fn numfmt_tick_fragment_accepts_width_fill_align_overrides() {
        let scale = LinearScale::configured((0.0, 100.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![42.0])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_number: Some(
                    "#numfmt(value, \".0f\", width: 5, fill: \".\", align: \"<\")".to_string(),
                ),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(labels.text.as_vec(1, None), vec!["42..."]);
    }

    #[test]
    fn numfmt_tick_fragment_rejects_duplicate_digit_options() {
        let scale = LinearScale::configured((0.0, 100.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![42.0])) as ArrayRef;
        let result = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_number: Some(
                    "#numfmt(value, \".0f\", precision: 1, fraction_digits: 2)".to_string(),
                ),
                ..Default::default()
            },
        );
        let Err(err) = result else {
            panic!("duplicate digit options should error");
        };

        assert!(matches!(
            err,
            AvengerGuidesError::InvalidAxisLabelFormat(message)
                if message == "axis numfmt accepts only one digit-control option"
        ));
    }

    fn collect_text_marks(group: &SceneGroup) -> Vec<&SceneTextMark> {
        let mut text_marks = Vec::new();
        collect_text_marks_into(&group.marks, &mut text_marks);
        text_marks
    }

    fn collect_text_marks_into<'a>(
        marks: &'a [avenger_scenegraph::marks::mark::SceneMark],
        text_marks: &mut Vec<&'a SceneTextMark>,
    ) {
        for mark in marks {
            match mark {
                avenger_scenegraph::marks::mark::SceneMark::Text(text) => text_marks.push(text),
                avenger_scenegraph::marks::mark::SceneMark::Group(group) => {
                    collect_text_marks_into(&group.marks, text_marks);
                }
                _ => {}
            }
        }
    }
}

fn make_axis_line(
    start: f32,
    end: f32,
    is_vertical: bool,
    offset: f32,
    color: Option<[f32; 4]>,
) -> SceneRuleMark {
    let (x0, x1, y0, y1) = if is_vertical {
        (offset, offset, start, end)
    } else {
        (start, end, offset, offset)
    };

    SceneRuleMark {
        x: x0.into(),
        x2: x1.into(),
        y: y0.into(),
        y2: y1.into(),
        stroke: ColorOrGradient::Color(color.unwrap_or([0.0, 0.0, 0.0, 1.0])).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    }
}

fn make_tick_marks(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
    tick_length: Option<f32>,
    color: Option<[f32; 4]>,
) -> Result<SceneRuleMark, AvengerGuidesError> {
    let scaled_values = scale.scale_to_numeric(ticks)?;
    let tick_len = tick_length.unwrap_or(DEFAULT_TICK_LENGTH);

    let (x0, x1, y0, y1) = match orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(-tick_len),
            scaled_values.clone(),
            scaled_values.clone(),
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(dimensions[0]),
            ScalarOrArray::new_scalar(dimensions[0] + tick_len),
            scaled_values.clone(),
            scaled_values.clone(),
        ),
        AxisOrientation::Top => (
            scaled_values.clone(),
            scaled_values.clone(),
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(-tick_len),
        ),
        AxisOrientation::Bottom => (
            scaled_values.clone(),
            scaled_values.clone(),
            ScalarOrArray::new_scalar(dimensions[1]),
            ScalarOrArray::new_scalar(dimensions[1] + tick_len),
        ),
    };

    Ok(SceneRuleMark {
        len: ticks.len() as u32,
        clip: false,
        x: x0,
        x2: x1,
        y: y0,
        y2: y1,
        stroke: ColorOrGradient::Color(color.unwrap_or([0.0, 0.0, 0.0, 1.0])).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    })
}

fn make_tick_grid_marks(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
    color: Option<[f32; 4]>,
    width: Option<f32>,
) -> Result<SceneGroup, AvengerGuidesError> {
    let scaled_values = scale.scale_to_numeric(ticks)?;

    let (x0, x1, y0, y1) = match orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(dimensions[0]),
            scaled_values.clone(),
            scaled_values.clone(),
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(dimensions[0]),
            scaled_values.clone(),
            scaled_values.clone(),
        ),
        AxisOrientation::Top => (
            scaled_values.clone(),
            scaled_values.clone(),
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(dimensions[1]),
        ),
        AxisOrientation::Bottom => (
            scaled_values.clone(),
            scaled_values.clone(),
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(dimensions[1]),
        ),
    };

    let grid_mark = SceneRuleMark {
        len: ticks.len() as u32,
        clip: false,
        x: x0,
        x2: x1,
        y: y0,
        y2: y1,
        stroke: ColorOrGradient::Color(color.unwrap_or([0.878, 0.878, 0.878, 0.5])).into(), // Default: #E0E0E0 with opacity 0.5
        stroke_width: width.unwrap_or(0.5).into(),
        ..Default::default()
    };

    // Grid lines are positioned relative to the axis group origin
    let grid_origin = [0.0, 0.0];

    Ok(SceneGroup {
        origin: grid_origin,
        zindex: Some(-1), // Grid lines behind data marks
        marks: vec![grid_mark.into()],
        ..Default::default()
    })
}

fn make_tick_labels(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    config: &AxisConfig,
) -> Result<SceneTextMark, AvengerGuidesError> {
    let tick_labels = make_tick_label_text(ticks, scale, config)?;
    let scaled_values = scale.scale_to_numeric(ticks)?;

    // Adjust y position slightly for font metrics
    // Numbers don't use full descent, so shift up by ~10% of font size for better visual centering
    let tick_font_size = config.label_font_size.unwrap_or(DEFAULT_TICK_FONT_SIZE);
    let label_angle = config.label_angle.unwrap_or(0.0);
    let font_adjustment = tick_font_size * 0.10;
    let adjusted_values_left_right = scaled_values
        .as_vec(ticks.len(), None)
        .into_iter()
        .map(|v| v - font_adjustment)
        .collect::<Vec<_>>();

    let (x, y, align, baseline, angle) = match config.orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(-DEFAULT_TICK_LENGTH - TEXT_MARGIN),
            ScalarOrArray::new_array(adjusted_values_left_right.clone()),
            TextAlign::Right,
            TextBaseline::Middle,
            label_angle,
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(config.dimensions[0] + DEFAULT_TICK_LENGTH + TEXT_MARGIN),
            ScalarOrArray::new_array(adjusted_values_left_right),
            TextAlign::Left,
            TextBaseline::Middle,
            label_angle,
        ),
        AxisOrientation::Top => (
            scaled_values,
            ScalarOrArray::new_scalar(-DEFAULT_TICK_LENGTH),
            TextAlign::Center,
            TextBaseline::Bottom,
            label_angle,
        ),
        AxisOrientation::Bottom => (
            scaled_values,
            ScalarOrArray::new_scalar(config.dimensions[1] + DEFAULT_TICK_LENGTH + TEXT_MARGIN),
            TextAlign::Center,
            TextBaseline::Top,
            label_angle,
        ),
    };

    Ok(SceneTextMark {
        clip: false,
        len: ticks.len() as u32,
        text: tick_labels.text,
        x,
        y,
        align: align.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        color: ColorOrGradient::Color(config.label_color.unwrap_or([0.0, 0.0, 0.0, 1.0])).into(), // Default: black
        font_size: tick_font_size.into(),
        font_weight: FontWeight::Number(config.label_font_weight.unwrap_or(400.0)).into(), // Default: normal weight for tick labels
        font: config
            .label_font_family
            .clone()
            .unwrap_or_else(|| "sans-serif".to_string())
            .into(),
        text_syntax: tick_labels.syntax_mode,
        ..Default::default()
    })
}

pub(crate) struct TickLabelText {
    pub(crate) text: ScalarOrArray<String>,
    pub(crate) syntax_mode: TextSyntaxMode,
}

pub(crate) fn make_tick_label_text(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    config: &AxisConfig,
) -> Result<TickLabelText, AvengerGuidesError> {
    if let Some(template) = config.tick_label.as_ref() {
        if ticks.data_type().is_numeric() {
            let values = cast(ticks, &DataType::Float64)
                .map_err(|err| AvengerGuidesError::InvalidScale(err.into()))?;
            let values = values.as_primitive::<Float64Type>();
            let nums: Vec<Option<f64>> = values.iter().collect();
            let format_env = NumberFormatEnvironment::from_axis_config(config)?;
            return format_numfmt_tick_fragment(template, &nums, format_env.context());
        }
        if let Some(values) = temporal_tick_values(ticks)? {
            let format_env = DateTimeFormatEnvironment::from_axis_config(config, scale)?;
            return format_datefmt_tick_fragment(template, &values, format_env.context());
        }
        return Err(AvengerGuidesError::InvalidAxisLabelFormat(
            "tick_label fragments are only supported for numeric or temporal ticks".to_string(),
        ));
    }

    if let Some(spec) = config.format_datetime.as_ref() {
        if let Some(values) = temporal_tick_values(ticks)? {
            let format_env = DateTimeFormatEnvironment::from_axis_config(config, scale)?;
            return format_datetime_ticks(spec, &values, format_env.context());
        }
        return Err(AvengerGuidesError::InvalidAxisLabelFormat(
            "datetime_format is only supported for temporal ticks".to_string(),
        ));
    }

    let Some(pattern) = config.format_number.as_ref() else {
        return Ok(TickLabelText {
            text: format_default_tick_values(ticks, scale)?,
            syntax_mode: TextSyntaxMode::Plain,
        });
    };

    if !ticks.data_type().is_numeric() {
        if pattern.contains("#datefmt") {
            if let Some(values) = temporal_tick_values(ticks)? {
                let format_env = DateTimeFormatEnvironment::from_axis_config(config, scale)?;
                return format_datefmt_tick_fragment(pattern, &values, format_env.context());
            }
        }
        return Ok(TickLabelText {
            text: format_default_tick_values(ticks, scale)?,
            syntax_mode: TextSyntaxMode::Plain,
        });
    }

    let values = cast(ticks, &DataType::Float64)
        .map_err(|err| AvengerGuidesError::InvalidScale(err.into()))?;
    let values = values.as_primitive::<Float64Type>();
    let nums: Vec<Option<f64>> = values.iter().collect();
    let format_env = NumberFormatEnvironment::from_axis_config(config)?;
    let context = format_env.context();

    if pattern.contains("#numfmt") {
        format_numfmt_tick_fragment(pattern, &nums, context)
    } else {
        format_bare_number_ticks(pattern, &nums, context)
    }
}

fn format_default_tick_values(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
) -> Result<ScalarOrArray<String>, AvengerGuidesError> {
    if is_temporal_type(ticks.data_type()) {
        Ok(scale.scale_to_string(ticks)?)
    } else {
        Ok(scale.format(ticks)?)
    }
}

fn is_temporal_type(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _)
    )
}

struct NumberFormatEnvironment {
    registry: Arc<NumberLocaleRegistry>,
    locale: ResolvedNumberLocale,
}

impl NumberFormatEnvironment {
    fn from_axis_config(config: &AxisConfig) -> Result<Self, AvengerGuidesError> {
        let registry = if let Some(registry) = &config.number_locale_registry {
            registry.clone()
        } else {
            avenger_text::number_locale_registry_from_specs(Some(&config.number_locale_specs))
                .map_err(AvengerGuidesError::InvalidAxisLabelFormat)?
                .unwrap_or_else(|| Arc::new(NumberLocaleRegistry::with_builtins()))
        };
        let locale_id = config.number_locale.as_deref().unwrap_or("en-US");
        let locale = registry
            .resolve(locale_id)
            .map_err(|err| AvengerGuidesError::InvalidAxisLabelFormat(err.to_string()))?;
        Ok(Self { registry, locale })
    }

    fn context(&self) -> NumberFormatContext<'_> {
        NumberFormatContext::new(&self.locale).with_registry(self.registry.as_ref())
    }
}

struct DateTimeFormatEnvironment {
    registry: Arc<DateTimeLocaleRegistry>,
    locale: ResolvedDateTimeLocale,
    timezone: Tz,
}

impl DateTimeFormatEnvironment {
    fn from_axis_config(
        config: &AxisConfig,
        scale: &ConfiguredScale,
    ) -> Result<Self, AvengerGuidesError> {
        let registry = if let Some(registry) = &config.datetime_locale_registry {
            registry.clone()
        } else {
            avenger_text::datetime_locale_registry_from_specs(Some(&config.datetime_locale_specs))
                .map_err(AvengerGuidesError::InvalidAxisLabelFormat)?
                .unwrap_or_else(|| Arc::new(DateTimeLocaleRegistry::with_builtins()))
        };
        let scale_locale = scale.option_string("locale", "en-US");
        let locale_id = if scale_locale == "en-US" {
            config
                .datetime_locale
                .clone()
                .unwrap_or_else(|| scale_locale.clone())
        } else {
            scale_locale
        };
        let locale = registry
            .resolve(&locale_id)
            .map_err(|err| AvengerGuidesError::InvalidAxisLabelFormat(err.to_string()))?;
        let scale_timezone = scale.option_string("timezone", "UTC");
        let timezone_id = if scale_timezone == "UTC" {
            config
                .datetime_timezone
                .clone()
                .unwrap_or_else(|| scale_timezone.clone())
        } else {
            scale_timezone
        };
        let timezone = parse_datetime_timezone(&timezone_id)
            .map_err(|err| AvengerGuidesError::InvalidAxisLabelFormat(err.to_string()))?;
        Ok(Self {
            registry,
            locale,
            timezone,
        })
    }

    fn context(&self) -> DateTimeFormatContext<'_> {
        DateTimeFormatContext::new(&self.locale, self.timezone)
            .with_registry(self.registry.as_ref())
    }
}

#[derive(Debug, Clone, Copy)]
enum DatefmtTickValue {
    Date(NaiveDate),
    DateTime(NaiveDateTime),
    UtcDateTime(DateTime<Utc>),
}

fn temporal_tick_values(
    ticks: &ArrayRef,
) -> Result<Option<Vec<Option<DatefmtTickValue>>>, AvengerGuidesError> {
    let values = match ticks.data_type() {
        DataType::Date32 => {
            let values = ticks.as_primitive::<Date32Type>();
            (0..values.len())
                .map(|i| values.value_as_date(i).map(DatefmtTickValue::Date))
                .collect()
        }
        DataType::Date64 => {
            let values = ticks.as_any().downcast_ref::<Date64Array>().unwrap();
            (0..values.len())
                .map(|i| {
                    if values.is_null(i) {
                        None
                    } else {
                        datetime_from_timestamp_parts(values.value(i), TimeUnit::Millisecond)
                            .map(|value| DatefmtTickValue::DateTime(value.naive_utc()))
                    }
                })
                .collect()
        }
        DataType::Timestamp(unit, timezone) => {
            timestamp_tick_values(ticks, unit, timezone.is_some())
        }
        _ => return Ok(None),
    };
    Ok(Some(values))
}

fn timestamp_tick_values(
    ticks: &ArrayRef,
    unit: &TimeUnit,
    timezone_aware: bool,
) -> Vec<Option<DatefmtTickValue>> {
    match unit {
        TimeUnit::Second => {
            let values = ticks
                .as_any()
                .downcast_ref::<TimestampSecondArray>()
                .unwrap();
            (0..values.len())
                .map(|i| {
                    timestamp_tick_value(values.is_null(i), values.value(i), *unit, timezone_aware)
                })
                .collect()
        }
        TimeUnit::Millisecond => {
            let values = ticks
                .as_any()
                .downcast_ref::<TimestampMillisecondArray>()
                .unwrap();
            (0..values.len())
                .map(|i| {
                    timestamp_tick_value(values.is_null(i), values.value(i), *unit, timezone_aware)
                })
                .collect()
        }
        TimeUnit::Microsecond => {
            let values = ticks
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap();
            (0..values.len())
                .map(|i| {
                    timestamp_tick_value(values.is_null(i), values.value(i), *unit, timezone_aware)
                })
                .collect()
        }
        TimeUnit::Nanosecond => {
            let values = ticks
                .as_any()
                .downcast_ref::<TimestampNanosecondArray>()
                .unwrap();
            (0..values.len())
                .map(|i| {
                    timestamp_tick_value(values.is_null(i), values.value(i), *unit, timezone_aware)
                })
                .collect()
        }
    }
}

fn timestamp_tick_value(
    is_null: bool,
    value: i64,
    unit: TimeUnit,
    timezone_aware: bool,
) -> Option<DatefmtTickValue> {
    if is_null {
        return None;
    }
    let datetime = datetime_from_timestamp_parts(value, unit)?;
    if timezone_aware {
        Some(DatefmtTickValue::UtcDateTime(datetime))
    } else {
        Some(DatefmtTickValue::DateTime(datetime.naive_utc()))
    }
}

fn datetime_from_timestamp_parts(value: i64, unit: TimeUnit) -> Option<DateTime<Utc>> {
    let (seconds, nanos) = match unit {
        TimeUnit::Second => (value, 0),
        TimeUnit::Millisecond => (
            value.div_euclid(1_000),
            value.rem_euclid(1_000) as u32 * 1_000_000,
        ),
        TimeUnit::Microsecond => (
            value.div_euclid(1_000_000),
            value.rem_euclid(1_000_000) as u32 * 1_000,
        ),
        TimeUnit::Nanosecond => (
            value.div_euclid(1_000_000_000),
            value.rem_euclid(1_000_000_000) as u32,
        ),
    };
    DateTime::from_timestamp(seconds, nanos)
}

fn format_bare_number_ticks(
    spec: &str,
    values: &[Option<f64>],
    context: NumberFormatContext<'_>,
) -> Result<TickLabelText, AvengerGuidesError> {
    let finite_values: Vec<f64> = values
        .iter()
        .filter_map(|value| *value)
        .filter(|value| value.is_finite())
        .collect();
    let prepared = prepare_number_tick_format(
        &finite_values,
        Some(spec),
        NumberFormatOverrides::default(),
        context,
    )
    .map_err(|err| AvengerGuidesError::InvalidAxisLabelFormat(err.to_string()))?;
    let labels = values
        .iter()
        .map(|value| match value {
            Some(value) => prepared
                .format(*value, context)
                .map_err(|err| AvengerGuidesError::InvalidAxisLabelFormat(err.to_string())),
            None => Ok(FormattedNumber::plain("")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let use_typst = labels
        .iter()
        .any(|label| !matches!(label.typesetting, NumberTypesetting::Plain));
    let text = labels
        .iter()
        .map(|label| {
            if use_typst {
                formatted_number_to_typst(label)
            } else {
                label.text.clone()
            }
        })
        .collect::<Vec<_>>();
    Ok(TickLabelText {
        text: ScalarOrArray::new_array(text),
        syntax_mode: if use_typst {
            TextSyntaxMode::TypstMarkup
        } else {
            TextSyntaxMode::Plain
        },
    })
}

fn format_numfmt_tick_fragment(
    template: &str,
    values: &[Option<f64>],
    context: NumberFormatContext<'_>,
) -> Result<TickLabelText, AvengerGuidesError> {
    let finite_values: Vec<f64> = values
        .iter()
        .filter_map(|value| *value)
        .filter(|value| value.is_finite())
        .collect();
    let parsed_template = parse_numfmt_tick_template(template, &finite_values, context)?;
    let text = values
        .iter()
        .map(|value| match value {
            Some(value) => format_numfmt_tick_fragment_value(&parsed_template, *value, context),
            None => Ok(String::new()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TickLabelText {
        text: ScalarOrArray::new_array(text),
        syntax_mode: TextSyntaxMode::TypstMarkup,
    })
}

struct NumfmtTickTemplate {
    pieces: Vec<NumfmtTickPiece>,
}

enum NumfmtTickPiece {
    Literal(String),
    Call(PreparedNumberTickFormat),
}

struct NumfmtTickCall {
    spec: String,
    overrides: NumberFormatOverrides,
}

fn parse_numfmt_tick_template(
    template: &str,
    values: &[f64],
    context: NumberFormatContext<'_>,
) -> Result<NumfmtTickTemplate, AvengerGuidesError> {
    let mut pieces = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = template[cursor..].find("#numfmt") {
        let start = cursor + relative;
        if start > cursor {
            pieces.push(NumfmtTickPiece::Literal(escape_typst_markup_text(
                &template[cursor..start],
            )));
        }
        let (end, call) = parse_numfmt_tick_call(template, start)?;
        let prepared =
            prepare_number_tick_format(values, Some(&call.spec), call.overrides, context)
                .map_err(|err| AvengerGuidesError::InvalidAxisLabelFormat(err.to_string()))?;
        pieces.push(NumfmtTickPiece::Call(prepared));
        cursor = end;
    }
    if cursor < template.len() {
        pieces.push(NumfmtTickPiece::Literal(escape_typst_markup_text(
            &template[cursor..],
        )));
    }
    Ok(NumfmtTickTemplate { pieces })
}

fn format_numfmt_tick_fragment_value(
    template: &NumfmtTickTemplate,
    value: f64,
    context: NumberFormatContext<'_>,
) -> Result<String, AvengerGuidesError> {
    let mut output = String::new();
    for piece in &template.pieces {
        match piece {
            NumfmtTickPiece::Literal(text) => output.push_str(text),
            NumfmtTickPiece::Call(prepared) => {
                let formatted = prepared
                    .format(value, context)
                    .map_err(|err| AvengerGuidesError::InvalidAxisLabelFormat(err.to_string()))?;
                output.push_str(&formatted_number_to_typst(&formatted));
            }
        }
    }
    Ok(output)
}

fn format_datefmt_tick_fragment(
    template: &str,
    values: &[Option<DatefmtTickValue>],
    context: DateTimeFormatContext<'_>,
) -> Result<TickLabelText, AvengerGuidesError> {
    let parsed_template = parse_datefmt_tick_template(template)?;
    let text = values
        .iter()
        .map(|value| match value {
            Some(value) => format_datefmt_tick_fragment_value(&parsed_template, *value, context),
            None => Ok(String::new()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TickLabelText {
        text: ScalarOrArray::new_array(text),
        syntax_mode: TextSyntaxMode::TypstMarkup,
    })
}

fn format_datetime_ticks(
    spec: &str,
    values: &[Option<DatefmtTickValue>],
    context: DateTimeFormatContext<'_>,
) -> Result<TickLabelText, AvengerGuidesError> {
    let text = values
        .iter()
        .map(|value| match value {
            Some(value) => format_datetime_tick_value(
                *value,
                spec,
                DateTimeFormatOverrides::default(),
                context,
            ),
            None => Ok(String::new()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TickLabelText {
        text: ScalarOrArray::new_array(text),
        syntax_mode: TextSyntaxMode::Plain,
    })
}

struct DatefmtTickTemplate {
    pieces: Vec<DatefmtTickPiece>,
}

enum DatefmtTickPiece {
    Literal(String),
    Call(DatefmtTickCall),
}

struct DatefmtTickCall {
    spec: String,
    overrides: DateTimeFormatOverrides,
    locale: Option<String>,
}

fn parse_datefmt_tick_template(template: &str) -> Result<DatefmtTickTemplate, AvengerGuidesError> {
    let mut pieces = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = template[cursor..].find("#datefmt") {
        let start = cursor + relative;
        if start > cursor {
            pieces.push(DatefmtTickPiece::Literal(escape_typst_markup_text(
                &template[cursor..start],
            )));
        }
        let (end, call) = parse_datefmt_tick_call(template, start)?;
        pieces.push(DatefmtTickPiece::Call(call));
        cursor = end;
    }
    if cursor < template.len() {
        pieces.push(DatefmtTickPiece::Literal(escape_typst_markup_text(
            &template[cursor..],
        )));
    }
    Ok(DatefmtTickTemplate { pieces })
}

fn format_datefmt_tick_fragment_value(
    template: &DatefmtTickTemplate,
    value: DatefmtTickValue,
    context: DateTimeFormatContext<'_>,
) -> Result<String, AvengerGuidesError> {
    let mut output = String::new();
    for piece in &template.pieces {
        match piece {
            DatefmtTickPiece::Literal(text) => output.push_str(text),
            DatefmtTickPiece::Call(call) => {
                let registry = context.registry;
                let override_locale = if let Some(locale_id) = call.locale.as_deref() {
                    let registry = registry.ok_or_else(|| {
                        AvengerGuidesError::InvalidAxisLabelFormat(
                            "axis datefmt locale override requires a locale registry".to_string(),
                        )
                    })?;
                    Some(registry.resolve(locale_id).map_err(|err| {
                        AvengerGuidesError::InvalidAxisLabelFormat(err.to_string())
                    })?)
                } else {
                    None
                };
                let call_context = if let Some(locale) = override_locale.as_ref() {
                    let mut call_context = DateTimeFormatContext::new(locale, context.timezone);
                    if let Some(registry) = registry {
                        call_context = call_context.with_registry(registry);
                    }
                    call_context
                } else {
                    context
                };
                let formatted = format_datetime_tick_value(
                    value,
                    &call.spec,
                    call.overrides.clone(),
                    call_context,
                )?;
                output.push_str(&escape_typst_markup_text(&formatted));
            }
        }
    }
    Ok(output)
}

fn format_datetime_tick_value(
    value: DatefmtTickValue,
    spec: &str,
    overrides: DateTimeFormatOverrides,
    context: DateTimeFormatContext<'_>,
) -> Result<String, AvengerGuidesError> {
    let formatted = match value {
        DatefmtTickValue::Date(value) => format_naive_datetime(
            NaiveDateTimeInput::Date(value),
            Some(spec),
            overrides,
            context,
        ),
        DatefmtTickValue::DateTime(value) => format_naive_datetime(
            NaiveDateTimeInput::DateTime(value),
            Some(spec),
            overrides,
            context,
        ),
        DatefmtTickValue::UtcDateTime(value) => {
            format_zoned_datetime(value, Some(spec), overrides, context)
        }
    }
    .map_err(|err| AvengerGuidesError::InvalidAxisLabelFormat(err.to_string()))?;
    Ok(formatted.text)
}

fn parse_numfmt_tick_call(
    template: &str,
    start: usize,
) -> Result<(usize, NumfmtTickCall), AvengerGuidesError> {
    let after_name = start + "#numfmt".len();
    let rest = &template[after_name..];
    let open_offset = rest
        .find('(')
        .ok_or_else(|| invalid_axis_label_format("numfmt call must include arguments"))?;
    if !rest[..open_offset].trim().is_empty() {
        return Err(invalid_axis_label_format(
            "numfmt call must be written as #numfmt(...)",
        ));
    }
    let open = after_name + open_offset;
    let close = find_call_close(template, open, "numfmt")?;
    let args = &template[open + 1..close];
    let call = parse_numfmt_tick_args(args)?;
    Ok((close + 1, call))
}

fn parse_datefmt_tick_call(
    template: &str,
    start: usize,
) -> Result<(usize, DatefmtTickCall), AvengerGuidesError> {
    let after_name = start + "#datefmt".len();
    let rest = &template[after_name..];
    let open_offset = rest
        .find('(')
        .ok_or_else(|| invalid_axis_label_format("datefmt call must include arguments"))?;
    if !rest[..open_offset].trim().is_empty() {
        return Err(invalid_axis_label_format(
            "datefmt call must be written as #datefmt(...)",
        ));
    }
    let open = after_name + open_offset;
    let close = find_call_close(template, open, "datefmt")?;
    let args = &template[open + 1..close];
    let call = parse_datefmt_tick_args(args)?;
    Ok((close + 1, call))
}

fn find_call_close(template: &str, open: usize, name: &str) -> Result<usize, AvengerGuidesError> {
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in template[open + 1..].char_indices() {
        let absolute = open + 1 + idx;
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if ch == ')' && !in_string {
            return Ok(absolute);
        }
    }
    Err(invalid_axis_label_format(format!(
        "unterminated {name} call"
    )))
}

fn parse_numfmt_tick_args(args: &str) -> Result<NumfmtTickCall, AvengerGuidesError> {
    let Some(rest) = args.trim_start().strip_prefix("value") else {
        return Err(invalid_axis_label_format(
            "axis numfmt call first argument must be `value`",
        ));
    };
    let Some(rest) = rest.trim_start().strip_prefix(',') else {
        return Err(invalid_axis_label_format(
            "axis numfmt call requires a format string argument",
        ));
    };
    let (spec, rest) = parse_quoted_string(rest.trim_start())?;
    let overrides = parse_numfmt_tick_overrides(rest)?;
    Ok(NumfmtTickCall { spec, overrides })
}

fn parse_datefmt_tick_args(args: &str) -> Result<DatefmtTickCall, AvengerGuidesError> {
    let Some(rest) = args.trim_start().strip_prefix("value") else {
        return Err(invalid_axis_label_format(
            "axis datefmt call first argument must be `value`",
        ));
    };
    let Some(rest) = rest.trim_start().strip_prefix(',') else {
        return Err(invalid_axis_label_format(
            "axis datefmt call requires a format string argument",
        ));
    };
    let (spec, rest) = parse_axis_string_arg(rest.trim_start(), "format")?;
    let (overrides, locale) = parse_datefmt_tick_overrides(rest)?;
    Ok(DatefmtTickCall {
        spec,
        overrides,
        locale,
    })
}

fn parse_numfmt_tick_overrides(raw: &str) -> Result<NumberFormatOverrides, AvengerGuidesError> {
    let mut rest = raw.trim();
    let mut overrides = NumberFormatOverrides::default();
    while !rest.is_empty() {
        let Some(after_comma) = rest.strip_prefix(',') else {
            return Err(invalid_axis_label_format(
                "axis numfmt options must be comma-separated named arguments",
            ));
        };
        rest = after_comma.trim_start();
        let Some((name, after_name)) = parse_identifier(rest) else {
            return Err(invalid_axis_label_format(
                "axis numfmt option must start with an identifier",
            ));
        };
        let Some(after_colon) = after_name.trim_start().strip_prefix(':') else {
            return Err(invalid_axis_label_format(format!(
                "axis numfmt option `{name}` must use `:`"
            )));
        };
        let value = after_colon.trim_start();
        rest = match name {
            "style" | "type" => {
                let (format_type, after_value) = parse_axis_format_type_arg(value)?;
                overrides.format_type = Some(format_type);
                after_value
            }
            "precision" => {
                let (digit_spec, after_value) = parse_axis_precision_arg(value)?;
                set_axis_digit_spec(&mut overrides, digit_spec)?;
                after_value
            }
            "fraction_digits" => {
                let (fraction_digits, after_value) = parse_axis_u8_arg(value, "fraction_digits")?;
                set_axis_digit_spec(&mut overrides, DigitSpec::Fraction(fraction_digits))?;
                after_value
            }
            "significant_digits" => {
                let (significant_digits, after_value) =
                    parse_axis_u8_arg(value, "significant_digits")?;
                set_axis_digit_spec(&mut overrides, DigitSpec::Significant(significant_digits))?;
                after_value
            }
            "width" => {
                let (width, after_value) = parse_axis_optional_usize_arg(value, "width")?;
                overrides.width = Some(width);
                after_value
            }
            "fill" => {
                let (fill, after_value) = parse_axis_optional_char_arg(value, "fill")?;
                overrides.fill = Some(fill);
                after_value
            }
            "align" => {
                let (align, after_value) = parse_axis_optional_align_arg(value)?;
                overrides.align = Some(align);
                after_value
            }
            "group" => {
                let (group, after_value) = parse_axis_bool_arg(value, "group")?;
                overrides.group = Some(group);
                after_value
            }
            "trim" => {
                let (trim, after_value) = parse_axis_bool_arg(value, "trim")?;
                overrides.trim = Some(trim);
                after_value
            }
            "zero" => {
                let (zero, after_value) = parse_axis_bool_arg(value, "zero")?;
                overrides.zero = Some(zero);
                after_value
            }
            "currency" => {
                let (currency, after_value) = parse_axis_string_arg(value, "currency")?;
                overrides.currency = Some(currency);
                after_value
            }
            "currency_display" => {
                let (display, after_value) = parse_axis_currency_display_arg(value)?;
                overrides.currency_display = Some(display);
                after_value
            }
            "sign" => {
                let (sign, after_value) = parse_axis_sign_arg(value)?;
                overrides.sign = Some(sign);
                after_value
            }
            "symbol" => {
                let (symbol, after_value) = parse_axis_symbol_arg(value)?;
                overrides.symbol = Some(symbol);
                after_value
            }
            _ => {
                return Err(invalid_axis_label_format(format!(
                    "unsupported axis numfmt option `{name}`"
                )));
            }
        }
        .trim_start();
    }
    Ok(overrides)
}

fn parse_datefmt_tick_overrides(
    raw: &str,
) -> Result<(DateTimeFormatOverrides, Option<String>), AvengerGuidesError> {
    let mut rest = raw.trim();
    let mut overrides = DateTimeFormatOverrides::default();
    let mut locale = None;
    while !rest.is_empty() {
        let Some(after_comma) = rest.strip_prefix(',') else {
            return Err(invalid_axis_label_format(
                "axis datefmt options must be comma-separated named arguments",
            ));
        };
        rest = after_comma.trim_start();
        let Some((name, after_name)) = parse_identifier(rest) else {
            return Err(invalid_axis_label_format(
                "axis datefmt option must start with an identifier",
            ));
        };
        let Some(after_colon) = after_name.trim_start().strip_prefix(':') else {
            return Err(invalid_axis_label_format(format!(
                "axis datefmt option `{name}` must use `:`"
            )));
        };
        let value = after_colon.trim_start();
        rest = match name {
            "locale" => {
                let (value, after_value) = parse_axis_string_arg(value, "locale")?;
                locale = Some(value);
                after_value
            }
            "timezone" | "tz" => {
                let (timezone, after_value) = parse_axis_string_arg(value, "timezone")?;
                overrides.timezone = Some(timezone);
                after_value
            }
            "date_style" => {
                let (style, after_value) = parse_axis_datetime_style_arg(value, "date_style")?;
                overrides.date_style = Some(style);
                after_value
            }
            "time_style" => {
                let (style, after_value) = parse_axis_datetime_style_arg(value, "time_style")?;
                overrides.time_style = Some(style);
                after_value
            }
            "datetime_style" => {
                let (style, after_value) = parse_axis_datetime_style_arg(value, "datetime_style")?;
                overrides.datetime_style = Some(style);
                after_value
            }
            _ => {
                return Err(invalid_axis_label_format(format!(
                    "unsupported axis datefmt option `{name}`"
                )));
            }
        }
        .trim_start();
    }
    Ok((overrides, locale))
}

fn set_axis_digit_spec(
    overrides: &mut NumberFormatOverrides,
    digit_spec: DigitSpec,
) -> Result<(), AvengerGuidesError> {
    if overrides.digit_spec.is_some() {
        return Err(invalid_axis_label_format(
            "axis numfmt accepts only one digit-control option",
        ));
    }
    overrides.digit_spec = Some(digit_spec);
    Ok(())
}

fn parse_identifier(input: &str) -> Option<(&str, &str)> {
    let mut chars = input.char_indices();
    let (_, first) = chars.next()?;
    if first != '_' && !first.is_ascii_alphabetic() {
        return None;
    }
    let mut end = first.len_utf8();
    for (idx, ch) in chars {
        if ch == '_' || ch.is_ascii_alphanumeric() {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    Some((&input[..end], &input[end..]))
}

fn strip_identifier<'a>(input: &'a str, ident: &str) -> Option<&'a str> {
    let rest = input.strip_prefix(ident)?;
    let next = rest.chars().next();
    if next
        .map(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        .unwrap_or(false)
    {
        return None;
    }
    Some(rest)
}

fn parse_axis_precision_arg(raw: &str) -> Result<(DigitSpec, &str), AvengerGuidesError> {
    if let Some(rest) = strip_identifier(raw, "precision") {
        return Ok((DigitSpec::Auto, rest));
    }
    let (precision, rest) = parse_axis_u8_arg(raw, "precision")?;
    Ok((DigitSpec::Precision(precision), rest))
}

fn parse_axis_format_type_arg(raw: &str) -> Result<(FormatType, &str), AvengerGuidesError> {
    let (value, rest) = parse_axis_string_arg(raw, "type")?;
    let mut chars = value.chars();
    let Some(ch) = chars.next() else {
        return Err(invalid_axis_label_format("unsupported axis numfmt type"));
    };
    if chars.next().is_some() {
        return Err(invalid_axis_label_format("unsupported axis numfmt type"));
    }
    let Some(format_type) = FormatType::from_char(ch) else {
        return Err(invalid_axis_label_format("unsupported axis numfmt type"));
    };
    Ok((format_type, rest))
}

fn parse_axis_currency_display_arg(
    raw: &str,
) -> Result<(CurrencyDisplay, &str), AvengerGuidesError> {
    let (value, rest) = parse_axis_string_arg(raw, "currency_display")?;
    let display = match value.as_str() {
        "symbol" => CurrencyDisplay::Symbol,
        "code" => CurrencyDisplay::Code,
        "name" => CurrencyDisplay::Name,
        "narrow-symbol" | "narrow_symbol" => CurrencyDisplay::NarrowSymbol,
        _ => {
            return Err(invalid_axis_label_format(
                "unsupported axis numfmt currency_display",
            ));
        }
    };
    Ok((display, rest))
}

fn parse_axis_sign_arg(raw: &str) -> Result<(SignPolicy, &str), AvengerGuidesError> {
    let (value, rest) = parse_axis_string_arg(raw, "sign")?;
    let mut chars = value.chars();
    let Some(ch) = chars.next() else {
        return Err(invalid_axis_label_format("unsupported axis numfmt sign"));
    };
    if chars.next().is_some() {
        return Err(invalid_axis_label_format("unsupported axis numfmt sign"));
    }
    let Some(sign) = SignPolicy::from_char(ch) else {
        return Err(invalid_axis_label_format("unsupported axis numfmt sign"));
    };
    Ok((sign, rest))
}

fn parse_axis_symbol_arg(raw: &str) -> Result<(Option<Symbol>, &str), AvengerGuidesError> {
    let (value, rest) = parse_axis_string_arg(raw, "symbol")?;
    let symbol = match value.as_str() {
        "$" => Some(Symbol::CurrencyCompat),
        "#" => Some(Symbol::Alternate),
        "none" => None,
        _ => return Err(invalid_axis_label_format("unsupported axis numfmt symbol")),
    };
    Ok((symbol, rest))
}

fn parse_axis_datetime_style_arg<'a>(
    raw: &'a str,
    option: &str,
) -> Result<(DateTimeStyleLength, &'a str), AvengerGuidesError> {
    let (value, rest) = parse_axis_string_arg(raw, option)?;
    let Some(style) = DateTimeStyleLength::from_str(&value) else {
        return Err(invalid_axis_label_format(format!(
            "axis datefmt option `{option}` must be one of short, medium, long, or full"
        )));
    };
    Ok((style, rest))
}

fn parse_axis_optional_usize_arg<'a>(
    raw: &'a str,
    option: &str,
) -> Result<(Option<usize>, &'a str), AvengerGuidesError> {
    if let Some(rest) = strip_identifier(raw, "none") {
        return Ok((None, rest));
    }
    let (width, rest) = parse_axis_usize_arg(raw, option)?;
    Ok((Some(width), rest))
}

fn parse_axis_optional_char_arg<'a>(
    raw: &'a str,
    option: &str,
) -> Result<(Option<char>, &'a str), AvengerGuidesError> {
    if let Some(rest) = strip_identifier(raw, "none") {
        return Ok((None, rest));
    }
    let (value, rest) = parse_axis_string_arg(raw, option)?;
    let mut chars = value.chars();
    let Some(ch) = chars.next() else {
        return Err(invalid_axis_label_format(format!(
            "axis numfmt option `{option}` must be one character or none"
        )));
    };
    if chars.next().is_some() {
        return Err(invalid_axis_label_format(format!(
            "axis numfmt option `{option}` must be one character or none"
        )));
    }
    Ok((Some(ch), rest))
}

fn parse_axis_optional_align_arg(raw: &str) -> Result<(Option<Align>, &str), AvengerGuidesError> {
    if let Some(rest) = strip_identifier(raw, "none") {
        return Ok((None, rest));
    }
    let (value, rest) = parse_axis_string_arg(raw, "align")?;
    let mut chars = value.chars();
    let Some(ch) = chars.next() else {
        return Err(invalid_axis_label_format(
            "axis numfmt option `align` must be one of `<`, `>`, `^`, `=`, or none",
        ));
    };
    if chars.next().is_some() {
        return Err(invalid_axis_label_format(
            "axis numfmt option `align` must be one of `<`, `>`, `^`, `=`, or none",
        ));
    }
    let Some(align) = Align::from_char(ch) else {
        return Err(invalid_axis_label_format(
            "axis numfmt option `align` must be one of `<`, `>`, `^`, `=`, or none",
        ));
    };
    Ok((Some(align), rest))
}

fn parse_axis_string_arg<'a>(
    raw: &'a str,
    option: &str,
) -> Result<(String, &'a str), AvengerGuidesError> {
    parse_quoted_string(raw).map_err(|_| {
        invalid_axis_label_format(format!(
            "axis format option `{option}` must be a string literal"
        ))
    })
}

fn parse_axis_bool_arg<'a>(
    raw: &'a str,
    option: &str,
) -> Result<(bool, &'a str), AvengerGuidesError> {
    if let Some(rest) = strip_identifier(raw, "true") {
        return Ok((true, rest));
    }
    if let Some(rest) = strip_identifier(raw, "false") {
        return Ok((false, rest));
    }
    Err(invalid_axis_label_format(format!(
        "axis numfmt option `{option}` must be true or false"
    )))
}

fn parse_axis_u8_arg<'a>(raw: &'a str, option: &str) -> Result<(u8, &'a str), AvengerGuidesError> {
    let end = raw
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .map(|(idx, ch)| idx + ch.len_utf8())
        .last()
        .unwrap_or(0);
    if end == 0 {
        return Err(invalid_axis_label_format(format!(
            "axis numfmt option `{option}` must be an integer from 0 to 255"
        )));
    }
    let value = raw[..end].parse::<u8>().map_err(|_| {
        invalid_axis_label_format(format!(
            "axis numfmt option `{option}` must be an integer from 0 to 255"
        ))
    })?;
    Ok((value, &raw[end..]))
}

fn parse_axis_usize_arg<'a>(
    raw: &'a str,
    option: &str,
) -> Result<(usize, &'a str), AvengerGuidesError> {
    let end = raw
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .map(|(idx, ch)| idx + ch.len_utf8())
        .last()
        .unwrap_or(0);
    if end == 0 {
        return Err(invalid_axis_label_format(format!(
            "axis numfmt option `{option}` must be a nonnegative integer"
        )));
    }
    let value = raw[..end].parse::<usize>().map_err(|_| {
        invalid_axis_label_format(format!(
            "axis numfmt option `{option}` must be a nonnegative integer"
        ))
    })?;
    Ok((value, &raw[end..]))
}

fn parse_quoted_string(raw: &str) -> Result<(String, &str), AvengerGuidesError> {
    let Some(stripped) = raw.strip_prefix('"') else {
        return Err(invalid_axis_label_format(
            "axis format argument must be a string literal",
        ));
    };
    let mut output = String::new();
    let mut escaped = false;
    for (idx, ch) in stripped.char_indices() {
        if escaped {
            output.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            return Ok((output, &stripped[idx + 1..]));
        }
        output.push(ch);
    }
    Err(invalid_axis_label_format("unterminated axis format string"))
}

fn formatted_number_to_typst(formatted: &FormattedNumber) -> String {
    match &formatted.typesetting {
        NumberTypesetting::Plain => escape_typst_markup_text(&formatted.text),
        NumberTypesetting::Exponent {
            mantissa,
            exponent,
            marker: ExponentMarker::LowerE,
        } => format!("${} times 10^({})$", mantissa, exponent),
    }
}

fn escape_typst_markup_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' | '$' | '#' | '[' | ']' => {
                output.push('\\');
                output.push(ch);
            }
            _ => output.push(ch),
        }
    }
    output
}

fn invalid_axis_label_format(message: impl Into<String>) -> AvengerGuidesError {
    AvengerGuidesError::InvalidAxisLabelFormat(message.into())
}

fn make_title(
    title: &str,
    scale: &ConfiguredScale,
    envelope: &AABB<[f32; 2]>,
    config: &AxisConfig,
) -> Result<SceneTextMark, AvengerGuidesError> {
    let range = scale.numeric_interval_range()?;
    let mid = (range.0 + range.1) / 2.0;
    let title_font_size = config.title_font_size.unwrap_or(DEFAULT_TITLE_FONT_SIZE);
    let title_font_weight = FontWeight::Number(config.title_font_weight.unwrap_or(400.0));
    let title_font_family = config
        .title_font_family
        .clone()
        .unwrap_or_else(|| "sans-serif".to_string());
    default_text_engine().measure_bounds(&TextMeasurementConfig {
        text: title,
        font: &title_font_family,
        font_size: title_font_size,
        font_weight: title_font_weight,
        font_style: FontStyle::Normal,
        syntax_mode: config.title_syntax_mode,
        params: &config.title_text_params,
        number_locale: config.number_locale.as_deref(),
        number_locale_specs: Some(&config.number_locale_specs),
        datetime_locale: config.datetime_locale.as_deref(),
        datetime_timezone: config.datetime_timezone.as_deref(),
        datetime_locale_specs: Some(&config.datetime_locale_specs),
    })?;

    // Now the envelope is in the group's local coordinate system (origin = [0, 0])
    // For left/top axes, labels extend into negative coordinates
    // For right/bottom axes, labels extend beyond the axis dimensions

    let (x, y, align, baseline, angle) = match config.orientation {
        AxisOrientation::Left => {
            // Labels are right-aligned and extend left from the axis
            // envelope.lower()[0] gives us the leftmost edge of the labels
            (
                (envelope.lower()[0] - TITLE_MARGIN).into(),
                mid.into(),
                TextAlign::Center,
                TextBaseline::LineBottom,
                -90.0,
            )
        }
        AxisOrientation::Right => {
            // Labels are left-aligned and extend right from the axis
            // envelope.upper()[0] gives us the rightmost edge of the labels
            (
                (envelope.upper()[0] + TITLE_MARGIN).into(),
                mid.into(),
                TextAlign::Center,
                TextBaseline::LineBottom,
                90.0,
            )
        }
        AxisOrientation::Top => {
            // Labels are bottom-aligned and extend up from the axis
            // envelope.lower()[1] gives us the topmost edge of the labels
            (
                mid.into(),
                (envelope.lower()[1] - TITLE_MARGIN).into(),
                TextAlign::Center,
                TextBaseline::Bottom,
                0.0,
            )
        }
        AxisOrientation::Bottom => {
            // Labels are top-aligned and extend down from the axis
            // envelope.upper()[1] gives us the bottommost edge of the labels
            (
                mid.into(),
                (envelope.upper()[1] + TITLE_MARGIN).into(),
                TextAlign::Center,
                TextBaseline::Top,
                0.0,
            )
        }
    };

    Ok(SceneTextMark {
        clip: false,
        len: 1,
        text: title.to_string().into(),
        x,
        y,
        align: align.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        color: ColorOrGradient::Color(config.title_color.unwrap_or([0.0, 0.0, 0.0, 1.0])).into(), // Default: black
        font_size: title_font_size.into(),
        font_weight: title_font_weight.into(),
        font: title_font_family.into(),
        text_syntax: config.title_syntax_mode,
        text_params: config.title_text_params.clone(),
        number_locale: config.number_locale.clone(),
        number_locale_specs: config.number_locale_specs.clone(),
        datetime_locale: config.datetime_locale.clone(),
        datetime_timezone: config.datetime_timezone.clone(),
        datetime_locale_specs: config.datetime_locale_specs.clone(),
        ..Default::default()
    })
}
