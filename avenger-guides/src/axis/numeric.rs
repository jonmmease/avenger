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
use avenger_format::{
    FormattedNumber, NaiveDateTimeInput, NumberTypesetting, PreparedCivilDateTimeFormatter,
    PreparedInstantFormatter, PreparedNumberFormatter,
};
use avenger_geometry::{marks::MarkGeometryUtils, rtree::EnvelopeUtils};
use avenger_scales::formatter::{DateTimeFormatAdapter, NumberFormatAdapter, NumberLabelContext};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::SceneGroup, rule::SceneRuleMark, text::SceneTextMark};
use avenger_text::{
    measurement::{FontMetricsConfig, TextMeasurementConfig},
    types::TextSyntaxMode,
    types::{FontStyle, FontWeight, TextAlign, TextBaseline},
    TextEngine,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
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

pub fn make_numeric_axis_marks_with_text_engine(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
    text_engine: &TextEngine,
) -> Result<SceneGroup, AvengerGuidesError> {
    let mut resolved_config = config.clone();
    resolved_config.tick_count = config
        .tick_count
        .or_else(|| adaptive_tick_count(config, text_engine));
    let config = &resolved_config;
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
        let tick_count = config.tick_count;
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
        axis_elements_group.marks.push(
            make_title(
                title,
                &scale,
                &axis_elements_group.bounding_box_with_text_engine(text_engine),
                config,
                text_engine,
            )?
            .into(),
        );
    }

    // Add the axis elements group to the main group
    main_group.marks.push(axis_elements_group.into());

    // Measure the overall bounds to create a clip rect
    let bbox = main_group.bounding_box_with_text_engine(text_engine);

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
    let font_metrics = text_engine
        .font_metrics(&FontMetricsConfig {
            font: font_family,
            font_size,
            font_weight,
            font_style: FontStyle::Normal,
        })
        .unwrap_or_else(|_| avenger_text::measurement::FontMetrics::fallback(font_size));

    (font_metrics.height * VERTICAL_TICK_SPACING_FONT_HEIGHT_FACTOR).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Float64Array;
    use avenger_format::{DateTimeFormatBinding, NumberFormatBinding};
    use avenger_format::{
        DateTimeFormatError, DateTimeFormatProvider, NumberFormatError, NumberFormatProvider,
        ZonedDateTimeInput,
    };
    use avenger_format_datetime_d3::D3DateTimeFormatConfig;
    use avenger_format_number_d3::D3NumberFormatConfig;

    fn d3_scale(scale: &ConfiguredScale) -> ConfiguredScale {
        let mut scale = scale.clone();
        scale.config.context.formatting = avenger_scales::formatter::ScaleFormatting::d3(
            D3NumberFormatConfig::new(),
            D3DateTimeFormatConfig::new().with_timezone(scale.option_string("timezone", "UTC")),
        );
        scale
    }
    fn d3_engine() -> TextEngine {
        avenger_scales::formatter::ScaleFormatting::d3(Default::default(), Default::default())
            .configure_text_engine(avenger_text::default_text_engine())
    }
    fn make_numeric_axis_marks(
        scale: &ConfiguredScale,
        title: &str,
        origin: [f32; 2],
        config: &AxisConfig,
    ) -> Result<SceneGroup, AvengerGuidesError> {
        make_numeric_axis_marks_with_text_engine(
            &d3_scale(scale),
            title,
            origin,
            config,
            &d3_engine(),
        )
    }
    fn make_tick_label_text(
        ticks: &ArrayRef,
        scale: &ConfiguredScale,
        config: &AxisConfig,
    ) -> Result<TickLabelText, AvengerGuidesError> {
        super::make_tick_label_text(ticks, &d3_scale(scale), config)
    }
    fn make_tick_labels(
        ticks: &ArrayRef,
        scale: &ConfiguredScale,
        config: &AxisConfig,
    ) -> Result<SceneTextMark, AvengerGuidesError> {
        super::make_tick_labels(ticks, &d3_scale(scale), config)
    }
    use avenger_geometry::rtree::SceneGraphRTree;
    use avenger_scales::scales::linear::LinearScale;
    use avenger_scales::scales::log::LogScale;
    use avenger_scales::scales::time::TimeScale;
    use avenger_scenegraph::{marks::symbol::SceneSymbolMark, scene_graph::SceneGraph};

    fn values(array: &ArrayRef) -> Vec<f32> {
        array
            .as_primitive::<Float32Type>()
            .values()
            .iter()
            .copied()
            .collect()
    }

    #[test]
    fn temporal_axes_and_scales_use_the_selected_provider() {
        #[derive(Debug)]
        struct Provider;
        #[derive(Debug)]
        struct Prepared(String);
        impl DateTimeFormatProvider for Provider {
            type Config = String;
            fn prepare_naive(
                &self,
                config: &String,
                pattern: &str,
            ) -> Result<Arc<dyn PreparedCivilDateTimeFormatter>, DateTimeFormatError> {
                Ok(Arc::new(Prepared(format!("{config} {pattern}"))))
            }
            fn prepare_zoned(
                &self,
                config: &String,
                pattern: &str,
            ) -> Result<Arc<dyn PreparedInstantFormatter>, DateTimeFormatError> {
                Ok(Arc::new(Prepared(format!("{config} {pattern}"))))
            }
        }
        impl PreparedCivilDateTimeFormatter for Prepared {
            fn format(&self, _: NaiveDateTimeInput) -> Result<String, DateTimeFormatError> {
                Ok(format!("civil {}", self.0))
            }
        }
        impl PreparedInstantFormatter for Prepared {
            fn format(&self, _: ZonedDateTimeInput) -> Result<String, DateTimeFormatError> {
                Ok(format!("instant {}", self.0))
            }
        }
        let adapter = DateTimeFormatAdapter::new(
            DateTimeFormatBinding::new(Provider, "custom".to_owned()),
            std::array::from_fn(|_| "automatic".to_owned()),
            chrono_tz::UTC,
        );
        let civil = Arc::new(arrow::array::Date32Array::from(vec![Some(19723), None])) as ArrayRef;
        let instant =
            Arc::new(TimestampMillisecondArray::from(vec![Some(0), None]).with_timezone("UTC"))
                as ArrayRef;
        let mut scale = TimeScale::configured((civil.slice(0, 1), civil.slice(0, 1)), (0.0, 100.0));
        scale.config.context.formatting.datetime = Some(adapter.clone());
        for (values, kind) in [(&civil, "civil"), (&instant, "instant")] {
            let mut axis = AxisConfig::default();
            for (pattern, fragment, expected) in [
                (None, None, "automatic"),
                (Some("explicit"), None, "explicit"),
                (None, Some(r#"#datefmt(value, "explicit")"#), "explicit"),
            ] {
                axis.format_datetime = pattern.map(str::to_owned);
                axis.tick_label = fragment.map(str::to_owned);
                let labels = super::make_tick_label_text(values, &scale, &axis).unwrap();
                assert_eq!(
                    labels.text.as_vec(2, None),
                    [format!("{kind} custom {expected}"), String::new()]
                );
            }
        }
        scale.config.context.formatters.instant = Some(adapter.prepare_zoned(None).unwrap());
        assert_eq!(
            scale.scale_to_string(&instant).unwrap().as_vec(2, None),
            ["instant custom automatic", ""]
        );
        scale.config.context.formatting.datetime = None;
        assert!(
            super::make_tick_label_text(&instant, &scale, &AxisConfig::default())
                .err()
                .unwrap()
                .to_string()
                .contains("datetime formatting is not configured")
        );
    }

    #[test]
    fn axes_supply_tick_facts_to_custom_scale_adapters() {
        #[derive(Debug)]
        struct Provider;
        impl NumberFormatProvider for Provider {
            type Config = ();
            fn prepare(
                &self,
                _: &(),
                pattern: &str,
            ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
                assert_eq!(pattern, "custom");
                Ok(Arc::new(Provider))
            }
        }
        impl PreparedNumberFormatter for Provider {
            fn format(&self, value: f64) -> FormattedNumber {
                FormattedNumber::plain(format!("custom:{value}"))
            }
        }
        let binding = NumberFormatBinding::new(Provider, ());
        let adapter = NumberFormatAdapter::new(binding.clone(), move |pattern, context| {
            assert!(matches!(
                context,
                NumberLabelContext::Ticks {
                    step: 0.5,
                    reference_value: 2.0
                }
            ));
            binding.prepare(pattern.unwrap())
        });
        let engine = avenger_text::default_text_engine();
        let mut scale = LinearScale::configured((0.0, 2.0), (0.0, 200.0));
        scale.config.context.formatting.number = Some(adapter);
        let mut config = AxisConfig {
            format_number: Some("custom".into()),
            tick_start_step: Some(AxisTickSpacing::Numeric {
                start: 0.0,
                step: 0.5,
            }),
            ..Default::default()
        };
        for template in [None, Some(r#"#numfmt(value, "custom")"#.into())] {
            config.tick_label = template;
            let group =
                make_numeric_axis_marks_with_text_engine(&scale, "", [0.0, 0.0], &config, &engine)
                    .unwrap();
            assert!(collect_text_marks(&group).iter().any(|mark| mark
                .text
                .as_vec(mark.len as usize, None)
                == [
                    "custom:0",
                    "custom:0.5",
                    "custom:1",
                    "custom:1.5",
                    "custom:2"
                ]));
        }
    }

    #[test]
    fn tick_labels_keep_their_clearance_when_tick_length_changes() {
        let scale = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![0.0, 5.0, 10.0])) as ArrayRef;
        for orientation in [
            AxisOrientation::Left,
            AxisOrientation::Right,
            AxisOrientation::Top,
            AxisOrientation::Bottom,
        ] {
            for tick_length in [None, Some(25.0)] {
                let config = AxisConfig {
                    orientation,
                    tick_length,
                    ..Default::default()
                };
                let marks = make_tick_marks(
                    &ticks,
                    &scale,
                    &orientation,
                    &config.dimensions,
                    tick_length,
                    None,
                )
                .unwrap();
                let labels = make_tick_labels(&ticks, &scale, &config).unwrap();
                let (clearance, expected) = match orientation {
                    AxisOrientation::Left => (
                        marks.x2.as_vec(3, None)[0] - labels.x.as_vec(3, None)[0],
                        TEXT_MARGIN,
                    ),
                    AxisOrientation::Right => (
                        labels.x.as_vec(3, None)[0] - marks.x2.as_vec(3, None)[0],
                        TEXT_MARGIN,
                    ),
                    AxisOrientation::Top => (
                        marks.y2.as_vec(3, None)[0] - labels.y.as_vec(3, None)[0],
                        0.0,
                    ),
                    AxisOrientation::Bottom => (
                        labels.y.as_vec(3, None)[0] - marks.y2.as_vec(3, None)[0],
                        TEXT_MARGIN,
                    ),
                };
                assert_eq!(clearance, expected, "{orientation:?}, {tick_length:?}");
            }
        }
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
    fn grid_behind_a_symbol_cannot_replace_it_as_the_top_hit() {
        let scale = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![5.0])) as ArrayRef;
        let grid = make_tick_grid_marks(
            &ticks,
            &scale,
            &AxisOrientation::Left,
            &[100.0, 100.0],
            None,
            None,
        )
        .expect("numeric grid");
        assert!(!grid.interactive);
        assert!(grid.marks.iter().all(|mark| !mark.interactive()));

        let point = SceneSymbolMark {
            name: "point".into(),
            x: 50.0.into(),
            y: 50.0.into(),
            size: 64.0.into(),
            ..Default::default()
        };
        let scene = SceneGraph {
            // Keep the guide later in document order to reproduce the prior
            // conflict between its scene path and its negative visual z-index.
            marks: vec![point.into(), grid.into()],
            width: 100.0,
            height: 100.0,
            origin: [0.0, 0.0],
        };
        let rtree = SceneGraphRTree::from_scene_graph(&scene);

        assert_eq!(
            rtree
                .pick_top_mark_at_point(&[50.0, 50.0])
                .expect("point at grid intersection")
                .name,
            "point"
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
    fn automatic_labels_preserve_log_magnitudes_and_explicit_tick_spacing() {
        let log = LogScale::configured((0.001, 1000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![0.001, 0.01, 1.0, 1000.0])) as ArrayRef;
        let labels = make_tick_label_text(&ticks, &log, &AxisConfig::default()).unwrap();
        assert_eq!(labels.text.as_vec(4, None), ["0.001", "0.01", "1", "1,000"]);
        let linear = LinearScale::configured((0.0, 1.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![0.01, 0.02, 0.03])) as ArrayRef;
        let labels = make_tick_label_text(
            &ticks,
            &linear,
            &AxisConfig {
                tick_start_step: Some(AxisTickSpacing::Numeric {
                    start: 0.0,
                    step: 0.01,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(labels.text.as_vec(3, None), ["0.01", "0.02", "0.03"]);
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
                number_format: Some(
                    D3NumberFormatConfig {
                        locale: Some("en-US".into()),
                        ..D3NumberFormatConfig::new()
                    }
                    .into(),
                ),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::Plain);
        assert_eq!(labels.text.as_vec(1, None), vec!["1,234.5"]);
    }

    #[test]
    fn bare_tick_format_uses_custom_locale_specs() {
        let scale = LinearScale::configured((0.0, 2000.0), (0.0, 100.0));
        let ticks = Arc::new(Float64Array::from(vec![1234.5])) as ArrayRef;
        let number_format = D3NumberFormatConfig {
            locale: Some("tick-test".into()),
            locales: [(
                "tick-test".into(),
                avenger_format_number_d3::NumberLocaleSpec {
                    decimal: "~".into(),
                    thousands: "_".into(),
                    grouping: vec![3],
                    ..Default::default()
                },
            )]
            .into(),
            ..D3NumberFormatConfig::new()
        };

        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                format_number: Some(",.1f".to_string()),
                number_format: Some(number_format.into()),
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
                tick_label: Some("d=#datefmt(value, \"%b %-d, %Y\")".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(labels.text.as_vec(1, None), vec!["d=Jan 5, 2024"]);
    }

    #[test]
    fn default_date32_ticks_use_vega_multi_format() {
        let start = Arc::new(arrow::array::Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(arrow::array::Date32Array::from(vec![19730])) as ArrayRef;
        let scale = TimeScale::configured((start, end), (0.0, 100.0));
        let ticks = Arc::new(arrow::array::Date32Array::from(vec![19727])) as ArrayRef;
        let labels = make_tick_label_text(&ticks, &scale, &AxisConfig::default()).expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::Plain);
        assert_eq!(labels.text.as_vec(1, None), vec!["Fri 05"]);
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
                format_datetime: Some("%b %-d, %Y".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::Plain);
        assert_eq!(labels.text.as_vec(1, None), vec!["Jan 5, 2024"]);
    }

    #[test]
    fn civil_axis_patterns_are_validated_without_non_null_values() {
        let start = Arc::new(arrow::array::Date32Array::from(vec![19723])) as ArrayRef;
        let scale = d3_scale(&TimeScale::configured((start.clone(), start), (0.0, 100.0)));
        for len in [0, 2] {
            let civil = Arc::new(arrow::array::Date32Array::new_null(len)) as ArrayRef;
            let instant =
                Arc::new(TimestampMillisecondArray::new_null(len).with_timezone("UTC")) as ArrayRef;
            for config in [
                AxisConfig {
                    format_datetime: Some("%Z".into()),
                    ..Default::default()
                },
                AxisConfig {
                    tick_label: Some("#datefmt(value, \"%Z\")".into()),
                    ..Default::default()
                },
            ] {
                let error = super::make_tick_label_text(&civil, &scale, &config)
                    .err()
                    .unwrap();
                assert!(error.to_string().contains("%Z"));
                assert!(super::make_tick_label_text(&instant, &scale, &config).is_ok());
            }
        }
    }

    #[test]
    fn datefmt_tick_fragment_uses_configured_locale_specs() {
        let start = Arc::new(arrow::array::Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(arrow::array::Date32Array::from(vec![19730])) as ArrayRef;
        let scale = TimeScale::configured((start, end), (0.0, 100.0));
        let ticks = Arc::new(arrow::array::Date32Array::from(vec![19727])) as ArrayRef;
        let mut datetime_format = D3DateTimeFormatConfig::new();
        datetime_format.locales.insert(
            "tick-date".into(),
            avenger_format_datetime_d3::DateTimeLocaleSpec {
                date: "%Y~%m~%d".into(),
                ..Default::default()
            },
        );
        let labels = make_tick_label_text(
            &ticks,
            &scale,
            &AxisConfig {
                tick_label: Some("#datefmt(value, \"%x\")".to_string()),
                datetime_format: Some(
                    D3DateTimeFormatConfig {
                        locale: Some("tick-date".into()),
                        ..datetime_format
                    }
                    .into(),
                ),
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
                tick_label: Some("#datefmt(value, \"%Y-%m-%d %H:%M\")".to_string()),
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
                tick_label: Some("#datefmt(value, \"%Y-%m-%d %H:%M\")".to_string()),
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
                tick_label: Some("#datefmt(value, \"%Y-%m-%d %H:%M\")".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

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
                format_number: Some("#numfmt(value, \".2~s\")".to_string()),
                number_format: Some(
                    D3NumberFormatConfig {
                        locale: Some("en-US".into()),
                        ..D3NumberFormatConfig::new()
                    }
                    .into(),
                ),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(labels.text.as_vec(1, None), vec!["1.2M"]);
    }

    #[test]
    fn numfmt_tick_fragment_derives_shared_precision() {
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
                format_number: Some("#numfmt(value, \"s\")".to_string()),
                ..Default::default()
            },
        )
        .expect("labels");

        assert_eq!(labels.syntax_mode, TextSyntaxMode::TypstMarkup);
        assert_eq!(labels.text.as_vec(1, None), vec!["0.9M", "1.0M", "1.1M"]);
    }

    #[test]
    fn tick_fragments_reject_per_call_settings() {
        for source in [
            "prefix #numfmt(value, \".0f\", precision: 1)",
            "prefix #datefmt(value, \"%Y\", locale: \"fr-FR\")",
            "prefix #datefmt(value, \"%Y\", timezone: \"UTC\")",
            "prefix #datefmt(value, \"%Y\", tz: \"UTC\")",
        ] {
            let error = if source.contains("#numfmt") {
                parse_numfmt_tick_call(source, 7).err().unwrap()
            } else {
                parse_datefmt_tick_call(source, 7).err().unwrap()
            };
            let message = error.to_string();
            assert!(message.contains("byte 7"), "{message}");
            assert!(
                message.contains("Use axis or scale formatting settings"),
                "{message}"
            );
        }
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
        interactive: false,
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
        interactive: false,
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
    let tick_length = config.tick_length.unwrap_or(DEFAULT_TICK_LENGTH);
    let font_adjustment = tick_font_size * 0.10;
    let adjusted_values_left_right = scaled_values
        .as_vec(ticks.len(), None)
        .into_iter()
        .map(|v| v - font_adjustment)
        .collect::<Vec<_>>();

    let (x, y, align, baseline, angle) = match config.orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(-tick_length - TEXT_MARGIN),
            ScalarOrArray::new_array(adjusted_values_left_right.clone()),
            TextAlign::Right,
            TextBaseline::Middle,
            label_angle,
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(config.dimensions[0] + tick_length + TEXT_MARGIN),
            ScalarOrArray::new_array(adjusted_values_left_right),
            TextAlign::Left,
            TextBaseline::Middle,
            label_angle,
        ),
        AxisOrientation::Top => (
            scaled_values,
            ScalarOrArray::new_scalar(-tick_length),
            TextAlign::Center,
            TextBaseline::Bottom,
            label_angle,
        ),
        AxisOrientation::Bottom => (
            scaled_values,
            ScalarOrArray::new_scalar(config.dimensions[1] + tick_length + TEXT_MARGIN),
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
    let environment = NumberFormatEnvironment {
        adapter: config
            .number_format
            .as_ref()
            .map(NumberFormatAdapter::from_config)
            .or_else(|| scale.config.context.formatting.number.clone()),
    };
    if let Some(template) = config.tick_label.as_ref() {
        if ticks.data_type().is_numeric() {
            let values = cast(ticks, &DataType::Float64)
                .map_err(|err| AvengerGuidesError::InvalidScale(err.into()))?;
            let values = values.as_primitive::<Float64Type>();
            let nums: Vec<Option<f64>> = values.iter().collect();
            return format_numfmt_tick_fragment(template, &nums, &environment);
        }
        if let Some(values) = temporal_tick_values(ticks)? {
            let format_env = DateTimeFormatEnvironment::from_axis_config(config, scale, ticks)?;
            return format_datefmt_tick_fragment(template, &values, &format_env);
        }
        return Err(AvengerGuidesError::InvalidAxisLabelFormat(
            "tick_label fragments are only supported for numeric or temporal ticks".to_string(),
        ));
    }

    if let Some(spec) = config.format_datetime.as_ref() {
        if let Some(values) = temporal_tick_values(ticks)? {
            let format_env = DateTimeFormatEnvironment::from_axis_config(config, scale, ticks)?;
            return format_datetime_ticks(spec, &values, &format_env);
        }
        return Err(AvengerGuidesError::InvalidAxisLabelFormat(
            "datetime_format is only supported for temporal ticks".to_string(),
        ));
    }

    let Some(pattern) = config.format_number.as_ref() else {
        return Ok(TickLabelText {
            text: format_default_tick_values(ticks, scale, config, &environment)?,
            syntax_mode: TextSyntaxMode::Plain,
        });
    };

    if !ticks.data_type().is_numeric() {
        if pattern.contains("#datefmt") {
            if let Some(values) = temporal_tick_values(ticks)? {
                let format_env = DateTimeFormatEnvironment::from_axis_config(config, scale, ticks)?;
                return format_datefmt_tick_fragment(pattern, &values, &format_env);
            }
        }
        return Ok(TickLabelText {
            text: format_default_tick_values(ticks, scale, config, &environment)?,
            syntax_mode: TextSyntaxMode::Plain,
        });
    }

    let values = cast(ticks, &DataType::Float64)
        .map_err(|err| AvengerGuidesError::InvalidScale(err.into()))?;
    let values = values.as_primitive::<Float64Type>();
    let nums: Vec<Option<f64>> = values.iter().collect();
    if pattern.contains("#numfmt") {
        format_numfmt_tick_fragment(pattern, &nums, &environment)
    } else {
        format_bare_number_ticks(pattern, &nums, scale, config, &environment)
    }
}

fn format_default_tick_values(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    config: &AxisConfig,
    number_environment: &NumberFormatEnvironment,
) -> Result<ScalarOrArray<String>, AvengerGuidesError> {
    if let Some(values) = temporal_tick_values(ticks)? {
        let environment = DateTimeFormatEnvironment::from_axis_config(config, scale, ticks)?;
        let prepared = environment.prepare(None)?;
        let labels = values
            .into_iter()
            .map(|value| {
                value.map_or_else(
                    || Ok(String::new()),
                    |value| format_datetime_tick_value(value, &prepared),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ScalarOrArray::new_array(labels))
    } else if ticks.data_type().is_numeric() {
        let values = cast(ticks, &DataType::Float64)
            .map_err(|error| AvengerGuidesError::InvalidScale(error.into()))?;
        let numbers = values
            .as_primitive::<Float64Type>()
            .iter()
            .collect::<Vec<_>>();
        let prepared =
            prepare_axis_number_format(&numbers, scale, config, None, number_environment)?;
        Ok(ScalarOrArray::new_array(
            values
                .as_primitive::<Float64Type>()
                .iter()
                .map(|value| value.map_or_else(String::new, |value| prepared.format(value).text))
                .collect(),
        ))
    } else {
        Ok(scale.format(ticks)?)
    }
}

struct NumberFormatEnvironment {
    adapter: Option<NumberFormatAdapter>,
}

impl NumberFormatEnvironment {
    fn prepare(
        &self,
        pattern: Option<&str>,
        context: NumberLabelContext,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, AvengerGuidesError> {
        self.adapter
            .as_ref()
            .ok_or_else(|| invalid_axis_label_format("number formatting is not configured"))?
            .prepare(pattern, context)
            .map_err(|error| invalid_axis_label_format(error.to_string()))
    }
}

struct DateTimeFormatEnvironment {
    adapter: DateTimeFormatAdapter,
    is_instant: bool,
}

enum DatefmtTickFormatter {
    Civil(Arc<dyn PreparedCivilDateTimeFormatter>),
    Instant(Arc<dyn PreparedInstantFormatter>),
}

impl DateTimeFormatEnvironment {
    fn from_axis_config(
        config: &AxisConfig,
        scale: &ConfiguredScale,
        ticks: &ArrayRef,
    ) -> Result<Self, AvengerGuidesError> {
        let adapter = config
            .datetime_format
            .as_ref()
            .map(|config| DateTimeFormatAdapter::from_config(config, Default::default()))
            .or_else(|| scale.config.context.formatting.datetime.clone())
            .ok_or_else(|| invalid_axis_label_format("datetime formatting is not configured"))?;
        Ok(Self {
            adapter,
            is_instant: matches!(ticks.data_type(), DataType::Timestamp(_, Some(_))),
        })
    }

    fn prepare(&self, pattern: Option<&str>) -> Result<DatefmtTickFormatter, AvengerGuidesError> {
        if self.is_instant {
            self.adapter
                .prepare_zoned(pattern)
                .map(DatefmtTickFormatter::Instant)
        } else {
            self.adapter
                .prepare_naive(pattern)
                .map(DatefmtTickFormatter::Civil)
        }
        .map_err(|error| invalid_axis_label_format(error.to_string()))
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
                .map(|i| {
                    (!values.is_null(i))
                        .then(|| values.value_as_date(i))
                        .flatten()
                        .map(DatefmtTickValue::Date)
                })
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

/// Use one unit and precision for the finite tick set, including descending ticks.
fn prepare_tick_number_format(
    values: &[f64],
    spec: Option<&str>,
    environment: &NumberFormatEnvironment,
) -> Result<Arc<dyn PreparedNumberFormatter>, AvengerGuidesError> {
    let mut values: Vec<_> = values.iter().copied().filter(|v| v.is_finite()).collect();
    values.sort_by(f64::total_cmp);
    values.dedup();
    let start = values.first().copied().unwrap_or(0.0);
    let stop = values.last().copied().unwrap_or(start);
    environment.prepare(
        spec,
        NumberLabelContext::Ticks {
            step: avenger_scales::array::tick_step(
                start,
                stop,
                values.len().saturating_sub(1) as f64,
            ),
            reference_value: start.abs().max(stop.abs()),
        },
    )
}

fn prepare_axis_number_format(
    values: &[Option<f64>],
    scale: &ConfiguredScale,
    config: &AxisConfig,
    spec: Option<&str>,
    environment: &NumberFormatEnvironment,
) -> Result<Arc<dyn PreparedNumberFormatter>, AvengerGuidesError> {
    let context = if let Some(AxisTickSpacing::Numeric { step, .. }) = config.tick_start_step {
        let reference_value = values
            .iter()
            .flatten()
            .copied()
            .filter(|v| v.is_finite())
            .map(f64::abs)
            .fold(0.0, f64::max);
        // Preserve the f32 step's decimal spelling at precision boundaries.
        let step = step.to_string().parse::<f64>().expect("numeric tick step");
        NumberLabelContext::Ticks {
            step,
            reference_value,
        }
    } else if scale.scale_impl.scale_type() == "log" {
        NumberLabelContext::Continuous
    } else if matches!(
        scale.scale_impl.scale_type(),
        "band" | "nested_band" | "point" | "ordinal" | "quantile" | "threshold"
    ) {
        NumberLabelContext::Categorical
    } else {
        let domain = cast(&scale.config.domain, &DataType::Float64)
            .map_err(|error| AvengerGuidesError::InvalidScale(error.into()))?;
        let domain = domain.as_primitive::<Float64Type>();
        let (start, stop) = if domain.len() >= 2 {
            (domain.value(0), domain.value(domain.len() - 1))
        } else {
            (0.0, 0.0)
        };
        NumberLabelContext::Ticks {
            step: avenger_scales::array::tick_step(
                start,
                stop,
                config.tick_count.unwrap_or(DEFAULT_MAX_TICK_COUNT) as f64,
            ),
            reference_value: start.abs().max(stop.abs()),
        }
    };
    environment.prepare(spec, context)
}

fn format_bare_number_ticks(
    spec: &str,
    values: &[Option<f64>],
    scale: &ConfiguredScale,
    config: &AxisConfig,
    context: &NumberFormatEnvironment,
) -> Result<TickLabelText, AvengerGuidesError> {
    let prepared = prepare_axis_number_format(values, scale, config, Some(spec), context)?;
    let labels = values
        .iter()
        .map(|value| match value {
            Some(value) => prepared.format(*value),
            None => FormattedNumber::plain(""),
        })
        .collect::<Vec<_>>();
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
    context: &NumberFormatEnvironment,
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
            Some(value) => format_numfmt_tick_fragment_value(&parsed_template, *value),
            None => String::new(),
        })
        .collect::<Vec<_>>();
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
    Call(Arc<dyn PreparedNumberFormatter>),
}

struct NumfmtTickCall {
    spec: String,
}

fn parse_numfmt_tick_template(
    template: &str,
    values: &[f64],
    context: &NumberFormatEnvironment,
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
        let prepared = prepare_tick_number_format(values, Some(&call.spec), context)
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

fn format_numfmt_tick_fragment_value(template: &NumfmtTickTemplate, value: f64) -> String {
    let mut output = String::new();
    for piece in &template.pieces {
        match piece {
            NumfmtTickPiece::Literal(text) => output.push_str(text),
            NumfmtTickPiece::Call(prepared) => {
                let formatted = prepared.format(value);
                output.push_str(&formatted_number_to_typst(&formatted));
            }
        }
    }
    output
}

fn format_datefmt_tick_fragment(
    template: &str,
    values: &[Option<DatefmtTickValue>],
    environment: &DateTimeFormatEnvironment,
) -> Result<TickLabelText, AvengerGuidesError> {
    let parsed_template = parse_datefmt_tick_template(template, environment)?;
    let text = values
        .iter()
        .map(|value| match value {
            Some(value) => format_datefmt_tick_fragment_value(&parsed_template, *value),
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
    environment: &DateTimeFormatEnvironment,
) -> Result<TickLabelText, AvengerGuidesError> {
    let prepared = environment.prepare(Some(spec))?;
    let text = values
        .iter()
        .map(|value| {
            value.map_or_else(
                || Ok(String::new()),
                |value| format_datetime_tick_value(value, &prepared),
            )
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
    Call(DatefmtTickFormatter),
}

struct DatefmtTickCall {
    spec: String,
}

fn parse_datefmt_tick_template(
    template: &str,
    environment: &DateTimeFormatEnvironment,
) -> Result<DatefmtTickTemplate, AvengerGuidesError> {
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
        let prepared = environment.prepare(Some(&call.spec))?;
        pieces.push(DatefmtTickPiece::Call(prepared));
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
) -> Result<String, AvengerGuidesError> {
    let mut output = String::new();
    for piece in &template.pieces {
        match piece {
            DatefmtTickPiece::Literal(text) => output.push_str(text),
            DatefmtTickPiece::Call(prepared) => output.push_str(&escape_typst_markup_text(
                &format_datetime_tick_value(value, prepared)?,
            )),
        }
    }
    Ok(output)
}

fn format_datetime_tick_value(
    value: DatefmtTickValue,
    prepared: &DatefmtTickFormatter,
) -> Result<String, AvengerGuidesError> {
    match (value, prepared) {
        (DatefmtTickValue::Date(value), DatefmtTickFormatter::Civil(format)) => {
            format.format(NaiveDateTimeInput::Date(value))
        }
        (DatefmtTickValue::DateTime(value), DatefmtTickFormatter::Civil(format)) => {
            format.format(NaiveDateTimeInput::DateTime(value))
        }
        (DatefmtTickValue::UtcDateTime(value), DatefmtTickFormatter::Instant(format)) => {
            format.format(value)
        }
        _ => {
            return Err(invalid_axis_label_format(
                "datetime formatter does not match the tick type",
            ))
        }
    }
    .map_err(|error| invalid_axis_label_format(error.to_string()))
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
    let call = parse_numfmt_tick_args(args)
        .map_err(|error| invalid_axis_label_format(format!("numfmt at byte {start}: {error}")))?;
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
    let call = parse_datefmt_tick_args(args)
        .map_err(|error| invalid_axis_label_format(format!("datefmt at byte {start}: {error}")))?;
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

fn parse_numfmt_tick_args(raw: &str) -> Result<NumfmtTickCall, AvengerGuidesError> {
    let rest = raw
        .trim()
        .strip_prefix("value")
        .ok_or_else(|| {
            invalid_axis_label_format("axis numfmt requires value as its first argument")
        })?
        .trim();
    if rest.is_empty() {
        return Ok(NumfmtTickCall {
            spec: String::new(),
        });
    }
    let rest = rest
        .strip_prefix(',')
        .ok_or_else(|| invalid_axis_label_format("axis numfmt expects a pattern after value"))?;
    let (spec, rest) = parse_axis_string_arg(rest.trim(), "format")?;
    reject_format_settings(rest)?;
    Ok(NumfmtTickCall { spec })
}

fn parse_datefmt_tick_args(raw: &str) -> Result<DatefmtTickCall, AvengerGuidesError> {
    let rest = raw
        .trim()
        .strip_prefix("value")
        .ok_or_else(|| {
            invalid_axis_label_format("axis datefmt requires value as its first argument")
        })?
        .trim();
    let rest = rest.strip_prefix(',').ok_or_else(|| {
        invalid_axis_label_format("axis datefmt requires a format string argument")
    })?;
    let (spec, rest) = parse_axis_string_arg(rest.trim(), "format")?;
    reject_format_settings(rest)?;
    Ok(DatefmtTickCall { spec })
}

fn reject_format_settings(rest: &str) -> Result<(), AvengerGuidesError> {
    if !rest.trim().is_empty() {
        return Err(invalid_axis_label_format("axis format calls accept value and pattern only. Use axis or scale formatting settings"));
    }
    Ok(())
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
        NumberTypesetting::Exponent { mantissa, exponent } => {
            format!("${} times 10^({})$", mantissa, exponent)
        }
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
    text_engine: &avenger_text::TextEngine,
) -> Result<SceneTextMark, AvengerGuidesError> {
    let range = scale.numeric_interval_range()?;
    let mid = (range.0 + range.1) / 2.0;
    let title_font_size = config.title_font_size.unwrap_or(DEFAULT_TITLE_FONT_SIZE);
    let title_font_weight = FontWeight::Number(config.title_font_weight.unwrap_or(400.0));
    let title_font_family = config
        .title_font_family
        .clone()
        .unwrap_or_else(|| "sans-serif".to_string());
    text_engine.measure_bounds(&TextMeasurementConfig {
        text: title,
        font: &title_font_family,
        font_size: title_font_size,
        font_weight: title_font_weight,
        font_style: FontStyle::Normal,
        syntax_mode: config.title_syntax_mode,
        params: &config.title_text_params,
        number_format: config
            .number_format
            .as_ref()
            .map(|config| config.binding())
            .as_ref(),
        datetime_format: config
            .datetime_format
            .as_ref()
            .map(|config| config.binding())
            .as_ref(),
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
        number_format: config.number_format.clone(),
        datetime_format: config.datetime_format.clone(),
        ..Default::default()
    })
}
