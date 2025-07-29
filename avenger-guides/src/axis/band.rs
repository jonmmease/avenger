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

    // Get axis-specific dimensions for positioning axis elements
    let axis_dims = config.axis_dimensions();

    // Build main group with origin [0, 0] to get local bounding box
    let mut main_group = SceneGroup {
        origin: [0.0, 0.0],
        ..Default::default()
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
    let offset = match config.orientation {
        AxisOrientation::Right => axis_dims[0],
        AxisOrientation::Bottom => axis_dims[1],
        AxisOrientation::Top => 0.0,
        AxisOrientation::Left => 0.0,
    };

    // Add tick grid if enabled
    if config.grid {
        let grid_group = make_tick_grid_marks(&scale, &config.orientation, &config.dimensions)?;
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
        .push(make_axis_line(start, end, is_vertical, offset).into());

    // Add tick marks
    axis_elements_group
        .marks
        .push(make_tick_marks(&scale, &config.orientation, &axis_dims)?.into());

    // Add tick labels
    axis_elements_group
        .marks
        .push(make_tick_labels(&scale, &config.orientation, &axis_dims)?.into());

    // Add title
    axis_elements_group
        .marks
        .push(make_title(title, &scale, &axis_elements_group.bounding_box(), config)?.into());

    // Add the axis elements group to the main group
    main_group.marks.push(axis_elements_group.into());

    // Now set the actual origin
    main_group.origin = origin;

    Ok(main_group)
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
) -> Result<SceneGroup, AvengerScaleError> {
    let scaled_values = scale.scale_to_numeric(scale.domain())?;

    let (x0, x1, y0, y1) = match orientation {
        AxisOrientation::Left | AxisOrientation::Right => (
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(dimensions[0]),
            scaled_values.clone(),
            scaled_values,
        ),
        AxisOrientation::Top | AxisOrientation::Bottom => (
            scaled_values.clone(),
            scaled_values,
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(dimensions[1]),
        ),
    };

    let grid_mark = SceneRuleMark {
        len: scale.domain().len() as u32,
        clip: false,
        x: x0,
        x2: x1,
        y: y0,
        y2: y1,
        stroke: ColorOrGradient::Color([0.6, 0.6, 0.6, 0.2]).into(),
        stroke_width: 0.5.into(),
        ..Default::default()
    };

    // Grid lines need to be offset for bottom/right axes to align with plot area
    let grid_origin = match orientation {
        AxisOrientation::Bottom => [0.0, -dimensions[1]], // Move up by plot height
        AxisOrientation::Right => [-dimensions[0], 0.0],  // Move left by plot width
        _ => [0.0, 0.0],
    };

    Ok(SceneGroup {
        origin: grid_origin,
        zindex: Some(-1), // Grid lines behind data marks
        marks: vec![grid_mark.into()],
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
            ScalarOrArray::new_scalar(-TICK_LENGTH - TEXT_MARGIN),
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
    config: &AxisConfig,
) -> Result<SceneTextMark, AvengerScaleError> {
    let range = scale.numeric_interval_range()?;
    let mid = (range.0 + range.1) / 2.0;

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
