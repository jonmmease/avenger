use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float32Array},
    compute::cast,
    datatypes::{DataType, Float32Type},
};
use avenger_color::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_geometry::{marks::MarkGeometryUtils, rtree::EnvelopeUtils};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::SceneGroup, rule::SceneRuleMark, text::SceneTextMark};
use avenger_text::{
    default_text_engine,
    measurement::{FontMetricsConfig, TextMeasurementConfig},
    types::{FontStyle, FontWeight, TextAlign, TextBaseline},
    TextEngine,
};
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

    let ticks = if let Some(tick_spacing) = config.tick_start_step {
        match tick_spacing {
            AxisTickSpacing::Numeric { start, step } => start_step_ticks(&scale, start, step)?,
            AxisTickSpacing::Temporal {
                start_millis,
                months,
                days,
                nanos,
            } => scale.temporal_start_step_ticks(start_millis, months, days, nanos)?,
        }
    } else {
        // Compute tick count: use explicit value, or adapt to available pixel space.
        let tick_count = config
            .tick_count
            .or_else(|| adaptive_tick_count(config, text_engine));
        scale.ticks(tick_count)?
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
            .push(make_tick_labels(&ticks, &scale, config)?.into());
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
    use avenger_scales::scales::linear::LinearScale;

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
    // If a numeric format string is provided, override the scale's number formatter
    let tick_text = if let Some(ref pattern) = config.format_number {
        use arrow::array::AsArray;
        use arrow::compute::kernels::cast;
        use arrow::datatypes::DataType;
        if ticks.data_type().is_numeric() {
            let values = cast(ticks, &DataType::Float32)
                .map_err(|e| AvengerGuidesError::InvalidScale(e.into()))?;
            let values = values.as_primitive::<arrow::datatypes::Float32Type>();
            let nums: Vec<Option<f32>> = values.iter().collect();
            let formatter = avenger_scales::format_num::NumberFormat::new();
            let labels: Vec<String> = nums
                .iter()
                .map(|v| v.map(|v| formatter.format(pattern, v)).unwrap_or_default())
                .collect();
            ScalarOrArray::new_array(labels)
        } else {
            scale.format(ticks)?
        }
    } else {
        scale.format(ticks)?
    };
    let scaled_values = scale.scale_to_numeric(ticks)?;

    // Adjust y position slightly for font metrics
    // Numbers don't use full descent, so shift up by ~10% of font size for better visual centering
    let tick_font_size = config.label_font_size.unwrap_or(DEFAULT_TICK_FONT_SIZE);
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
            0.0,
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(config.dimensions[0] + DEFAULT_TICK_LENGTH + TEXT_MARGIN),
            ScalarOrArray::new_array(adjusted_values_left_right),
            TextAlign::Left,
            TextBaseline::Middle,
            0.0,
        ),
        AxisOrientation::Top => (
            scaled_values,
            ScalarOrArray::new_scalar(-DEFAULT_TICK_LENGTH),
            TextAlign::Center,
            TextBaseline::Bottom,
            0.0,
        ),
        AxisOrientation::Bottom => (
            scaled_values,
            ScalarOrArray::new_scalar(config.dimensions[1] + DEFAULT_TICK_LENGTH + TEXT_MARGIN),
            TextAlign::Center,
            TextBaseline::Top,
            0.0,
        ),
    };

    Ok(SceneTextMark {
        clip: false,
        len: ticks.len() as u32,
        text: tick_text,
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
        ..Default::default()
    })
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
        params: avenger_text::empty_label_params(),
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
        ..Default::default()
    })
}
