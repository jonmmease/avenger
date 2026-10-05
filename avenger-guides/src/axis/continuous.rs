use arrow::{array::ArrayRef, datatypes::DataType};
use avenger_color::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_format::{PreparedFormatter, TickSpacing};
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_scales::scales::{time, ConfiguredScale};
use avenger_scenegraph::marks::{group::SceneGroup, rule::SceneRuleMark, text::SceneTextMark};
use avenger_text::types::{FontWeight, FontWeightNameSpec, TextAlign, TextBaseline};
use rstar::AABB;

use crate::error::AvengerGuidesError;

use super::{
    opts::{AxisConfig, AxisOrientation},
    tick_labels,
};

const TICK_LENGTH: f32 = 5.0;
const TEXT_MARGIN: f32 = 3.0;
const TITLE_MARGIN: f32 = 2.0;
const TITLE_FONT_SIZE: f32 = 10.0;
const TICK_FONT_SIZE: f32 = 8.0;
const PIXEL_OFFSET: f32 = 0.5;

/// Axis marks for a scale with a numeric or temporal domain, such as a linear, log, or time
/// scale, with tick labels from `config.format`.
pub fn make_continuous_axis_marks(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
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

    let mut group = SceneGroup {
        origin,
        ..Default::default()
    };

    // Get ticks
    let ticks = scale.ticks(None)?;

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
    let offset = match config.orientation {
        AxisOrientation::Right => config.dimensions[0],
        AxisOrientation::Bottom => config.dimensions[1],
        _ => 0.0,
    };

    // Add tick grid
    if config.grid {
        group.marks.push(
            make_tick_grid_marks(&ticks, &scale, &config.orientation, &config.dimensions)?.into(),
        );
    }

    // Add axis line
    group
        .marks
        .push(make_axis_line(start, end, is_vertical, offset).into());

    // Add tick marks
    group
        .marks
        .push(make_tick_marks(&ticks, &scale, &config.orientation, &config.dimensions)?.into());

    // Add tick labels
    group.marks.push(
        make_tick_labels(
            &ticks,
            &scale,
            &config.orientation,
            &config.dimensions,
            &config.format,
        )?
        .into(),
    );

    // Add title
    group
        .marks
        .push(make_title(title, &scale, &group.bounding_box(), &config.orientation)?.into());

    Ok(group)
}

fn make_axis_line(start: f32, end: f32, is_vertical: bool, offset: f32) -> SceneRuleMark {
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
        stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    }
}

fn make_tick_marks(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
) -> Result<SceneRuleMark, AvengerGuidesError> {
    let scaled_values = scale.scale_to_numeric(ticks)?;

    let (x0, x1, y0, y1) = match orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(-TICK_LENGTH),
            scaled_values.clone(),
            scaled_values.clone(),
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(dimensions[0]),
            ScalarOrArray::new_scalar(dimensions[0] + TICK_LENGTH),
            scaled_values.clone(),
            scaled_values.clone(),
        ),
        AxisOrientation::Top => (
            scaled_values.clone(),
            scaled_values.clone(),
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(-TICK_LENGTH),
        ),
        AxisOrientation::Bottom => (
            scaled_values.clone(),
            scaled_values.clone(),
            ScalarOrArray::new_scalar(dimensions[1]),
            ScalarOrArray::new_scalar(dimensions[1] + TICK_LENGTH),
        ),
    };

    Ok(SceneRuleMark {
        len: ticks.len() as u32,
        clip: false,
        x: x0,
        x2: x1,
        y: y0,
        y2: y1,
        stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    })
}

fn make_tick_grid_marks(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
) -> Result<SceneRuleMark, AvengerGuidesError> {
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

    Ok(SceneRuleMark {
        len: ticks.len() as u32,
        clip: false,
        x: x0,
        x2: x1,
        y: y0,
        y2: y1,
        stroke: ColorOrGradient::Color([0.6, 0.6, 0.6, 0.5]).into(),
        stroke_width: 0.2.into(),
        ..Default::default()
    })
}

fn make_tick_labels(
    ticks: &ArrayRef,
    scale: &ConfiguredScale,
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
    format: &PreparedFormatter,
) -> Result<SceneTextMark, AvengerGuidesError> {
    // Zoned labels must use the timezone that the scale places ticks in.
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
    let tick_text = tick_labels(ticks, format, tick_spacing(scale))?;
    let scaled_values = scale.scale_to_numeric(ticks)?;

    let (x, y, align, baseline, angle) = match orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(-TICK_LENGTH - TEXT_MARGIN),
            scaled_values,
            TextAlign::Right,
            TextBaseline::Middle,
            0.0,
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(dimensions[0] + TICK_LENGTH + TEXT_MARGIN),
            scaled_values,
            TextAlign::Left,
            TextBaseline::Middle,
            0.0,
        ),
        AxisOrientation::Top => (
            scaled_values,
            ScalarOrArray::new_scalar(-TICK_LENGTH),
            TextAlign::Center,
            TextBaseline::Bottom,
            0.0,
        ),
        AxisOrientation::Bottom => (
            scaled_values,
            ScalarOrArray::new_scalar(dimensions[1] + TICK_LENGTH + TEXT_MARGIN),
            TextAlign::Center,
            TextBaseline::Top,
            0.0,
        ),
    };

    Ok(SceneTextMark {
        len: ticks.len() as u32,
        text: tick_text,
        x,
        y,
        align: align.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        color: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
        font_size: TICK_FONT_SIZE.into(),
        ..Default::default()
    })
}

/// Log, threshold, and quantile ticks span magnitudes or arbitrary breaks. Other numeric
/// scales space their ticks evenly.
fn tick_spacing(scale: &ConfiguredScale) -> TickSpacing {
    match scale.scale_impl.scale_type() {
        "log" | "threshold" | "quantile" => TickSpacing::Varying,
        _ => TickSpacing::Uniform,
    }
}

fn make_title(
    title: &str,
    scale: &ConfiguredScale,
    envelope: &AABB<[f32; 2]>,
    orientation: &AxisOrientation,
) -> Result<SceneTextMark, AvengerGuidesError> {
    let range = scale.numeric_interval_range()?;
    let mid = (range.0 + range.1) / 2.0;

    let (x, y, align, baseline, angle) = match orientation {
        AxisOrientation::Left => (
            (envelope.lower()[0] - TITLE_MARGIN).into(),
            mid.into(),
            TextAlign::Center,
            TextBaseline::LineBottom,
            -90.0,
        ),
        AxisOrientation::Right => (
            (envelope.upper()[0] + TITLE_MARGIN).into(),
            mid.into(),
            TextAlign::Center,
            TextBaseline::LineBottom,
            90.0,
        ),
        AxisOrientation::Top => (
            mid.into(),
            (envelope.lower()[1] - TITLE_MARGIN).into(),
            TextAlign::Center,
            TextBaseline::Bottom,
            0.0,
        ),
        AxisOrientation::Bottom => (
            mid.into(),
            (envelope.upper()[1] + TITLE_MARGIN).into(),
            TextAlign::Center,
            TextBaseline::Top,
            0.0,
        ),
    };

    Ok(SceneTextMark {
        len: 1,
        text: title.to_string().into(),
        x,
        y,
        align: align.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        color: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
        font_size: TITLE_FONT_SIZE.into(),
        font_weight: FontWeight::Name(FontWeightNameSpec::Bold).into(),
        ..Default::default()
    })
}
