use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_scales::{error::AvengerScaleError, scales::ConfiguredScale};
use avenger_scenegraph::marks::{group::SceneGroup, rule::SceneRuleMark, text::SceneTextMark};

use avenger_text::types::{FontWeight, FontWeightNameSpec, TextAlign, TextBaseline};
use rstar::AABB;

use crate::error::AvengerGuidesError;

use super::opts::{AxisConfig, AxisOrientation};

const TICK_LENGTH: f32 = 5.0;
const TEXT_MARGIN: f32 = 3.0;
const TITLE_MARGIN: f32 = 4.0;
const TITLE_FONT_SIZE: f32 = 10.0;
const TICK_FONT_SIZE: f32 = 8.0;
const PIXEL_OFFSET: f32 = 0.5;

pub fn make_band_axis_marks(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
) -> Result<SceneGroup, AvengerGuidesError> {
    // Make sure ticks end up centered in the band
    // Unwrap is safe because this band value is always valid
    let scale = scale.clone().with_option("band", 0.5);

    let mut group = SceneGroup {
        origin,
        ..Default::default()
    };

    // Get range bounds considering orientation
    let range = scale.numeric_interval_range()?;
    let (start, end) = match config.orientation {
        AxisOrientation::Left | AxisOrientation::Right => {
            let upper = f32::min(range.1, range.0);
            let lower = f32::max(range.0, range.1);
            (lower, upper)
        }
        AxisOrientation::Top | AxisOrientation::Bottom => {
            let left = f32::min(range.0, range.1);
            let right = f32::max(range.0, range.1);
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
        AxisOrientation::Top => 0.0,
        AxisOrientation::Left => 0.0,
    };

    // Add axis line
    group
        .marks
        .push(make_axis_line(start, end, is_vertical, offset).into());

    // Add tick grid
    if config.grid {
        group
            .marks
            .push(make_tick_grid_marks(&scale, &config.orientation, &config.dimensions)?.into());
    }

    // Add tick marks
    group
        .marks
        .push(make_tick_marks(&scale, &config.orientation, &config.dimensions)?.into());

    // Add tick labels
    group
        .marks
        .push(make_tick_labels(&scale, &config.orientation, &config.dimensions)?.into());

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
    scale: &ConfiguredScale,
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
) -> Result<SceneRuleMark, AvengerScaleError> {
    let scaled_values = scale.scale_to_numeric(scale.domain())?;

    let (x0, x1, y0, y1) = match orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(-TICK_LENGTH),
            scaled_values.clone(),
            scaled_values,
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(dimensions[0]),
            ScalarOrArray::new_scalar(dimensions[0] + TICK_LENGTH),
            scaled_values.clone(),
            scaled_values,
        ),
        AxisOrientation::Top => (
            scaled_values.clone(),
            scaled_values,
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(-TICK_LENGTH),
        ),
        AxisOrientation::Bottom => (
            scaled_values.clone(),
            scaled_values,
            ScalarOrArray::new_scalar(dimensions[1]),
            ScalarOrArray::new_scalar(dimensions[1] + TICK_LENGTH),
        ),
    };

    Ok(SceneRuleMark {
        len: scale.domain().len() as u32,
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
    scale: &ConfiguredScale,
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
) -> Result<SceneRuleMark, AvengerScaleError> {
    let scaled_values = scale.scale_to_numeric(scale.domain())?;

    let (x0, x1, y0, y1) = match orientation {
        AxisOrientation::Left | AxisOrientation::Right => (
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(dimensions[0] + PIXEL_OFFSET),
            scaled_values.clone(),
            scaled_values,
        ),
        AxisOrientation::Top | AxisOrientation::Bottom => (
            scaled_values.clone(),
            scaled_values,
            ScalarOrArray::new_scalar(PIXEL_OFFSET),
            ScalarOrArray::new_scalar(dimensions[1] + PIXEL_OFFSET),
        ),
    };

    Ok(SceneRuleMark {
        len: scale.domain().len() as u32,
        clip: false,
        x: x0,
        x2: x1,
        y: y0,
        y2: y1,
        stroke: ColorOrGradient::Color([8.0, 8.0, 8.0, 0.2]).into(),
        stroke_width: 0.1.into(),
        ..Default::default()
    })
}

fn make_tick_labels(
    scale: &ConfiguredScale,
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
) -> Result<SceneTextMark, AvengerScaleError> {
    let scaled_values = scale.scale_to_numeric(scale.domain())?;

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
            ScalarOrArray::new_scalar(-TICK_LENGTH - TEXT_MARGIN + PIXEL_OFFSET),
            TextAlign::Center,
            TextBaseline::Bottom,
            0.0,
        ),
        AxisOrientation::Bottom => (
            scaled_values,
            ScalarOrArray::new_scalar(dimensions[1] + PIXEL_OFFSET + TICK_LENGTH + TEXT_MARGIN),
            TextAlign::Center,
            TextBaseline::Top,
            0.0,
        ),
    };

    Ok(SceneTextMark {
        len: scale.domain().len() as u32,
        text: scale.format(scale.domain())?,
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

fn make_title(
    title: &str,
    scale: &ConfiguredScale,
    envelope: &AABB<[f32; 2]>,
    orientation: &AxisOrientation,
) -> Result<SceneTextMark, AvengerScaleError> {
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
