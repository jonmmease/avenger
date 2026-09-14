use avenger_color::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_geometry::{marks::MarkGeometryUtils, rtree::EnvelopeUtils};
use avenger_scales::{error::AvengerScaleError, scales::ConfiguredScale};
use avenger_scenegraph::marks::{group::SceneGroup, rule::SceneRuleMark, text::SceneTextMark};
use avenger_text::{
    measurement::TextMeasurementConfig,
    types::{FontStyle, FontWeight, TextAlign, TextBaseline},
};
use rstar::AABB;

use super::opts::{AxisConfig, AxisOrientation};
use crate::error::AvengerGuidesError;

const TICK_LENGTH: f32 = 5.0;
const TEXT_MARGIN: f32 = 3.0;
const TITLE_MARGIN: f32 = 4.0;
const TITLE_FONT_SIZE: f32 = 12.0;
const TICK_FONT_SIZE: f32 = 12.0;
const PIXEL_OFFSET: f32 = 0.5;

pub fn make_band_axis_marks(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
) -> Result<SceneGroup, AvengerGuidesError> {
    make_band_axis_marks_with_text_engine(
        scale,
        title,
        origin,
        config,
        &avenger_text::default_text_engine(),
    )
}

pub fn make_band_axis_marks_with_text_engine(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
    text_engine: &avenger_text::TextEngine,
) -> Result<SceneGroup, AvengerGuidesError> {
    // Make sure ticks end up centered in the band
    // Unwrap is safe because this band value is always valid
    let scale = scale.clone().with_option("band", 0.5);

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
            .push(make_tick_labels(&scale, config)?.into());
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
    scale: &ConfiguredScale,
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
    tick_length: Option<f32>,
    color: Option<[f32; 4]>,
) -> Result<SceneRuleMark, AvengerScaleError> {
    let scaled_values = scale.scale_to_numeric(scale.domain())?;
    let tick_len = tick_length.unwrap_or(TICK_LENGTH);

    let (x0, x1, y0, y1) = match orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(-tick_len),
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
        stroke: ColorOrGradient::Color(color.unwrap_or([0.0, 0.0, 0.0, 1.0])).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    })
}

fn make_tick_grid_marks(
    scale: &ConfiguredScale,
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
    color: Option<[f32; 4]>,
    width: Option<f32>,
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
        interactive: false,
        len: scale.domain().len() as u32,
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
    scale: &ConfiguredScale,
    config: &AxisConfig,
) -> Result<SceneTextMark, AvengerScaleError> {
    let scaled_values = scale.scale_to_numeric(scale.domain())?;
    let label_angle = config.label_angle.unwrap_or(0.0);

    // Adjust y position slightly for font metrics
    // Text appears too low with Middle baseline, shift up by ~10% of font size
    let font_adjustment = config.label_font_size.unwrap_or(TICK_FONT_SIZE) * 0.10;
    let adjusted_values_left_right = scaled_values
        .as_vec(scale.domain().len(), None)
        .into_iter()
        .map(|v| v - font_adjustment)
        .collect::<Vec<_>>();

    let (x, y, align, baseline, angle) = match config.orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(-TICK_LENGTH - TEXT_MARGIN),
            ScalarOrArray::new_array(adjusted_values_left_right.clone()),
            TextAlign::Right,
            TextBaseline::Middle,
            label_angle,
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(config.dimensions[0] + TICK_LENGTH + TEXT_MARGIN),
            ScalarOrArray::new_array(adjusted_values_left_right),
            TextAlign::Left,
            TextBaseline::Middle,
            label_angle,
        ),
        AxisOrientation::Top => (
            scaled_values,
            ScalarOrArray::new_scalar(-TICK_LENGTH - TEXT_MARGIN),
            TextAlign::Center,
            TextBaseline::Bottom,
            label_angle,
        ),
        AxisOrientation::Bottom => (
            scaled_values,
            ScalarOrArray::new_scalar(config.dimensions[1] + TICK_LENGTH + TEXT_MARGIN),
            TextAlign::Center,
            TextBaseline::Top,
            label_angle,
        ),
    };

    Ok(SceneTextMark {
        clip: false,
        len: scale.domain().len() as u32,
        text: scale.format(scale.domain())?,
        x,
        y,
        align: align.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        color: ColorOrGradient::Color(config.label_color.unwrap_or([0.0, 0.0, 0.0, 1.0])).into(), // Default: black
        font_size: config.label_font_size.unwrap_or(TICK_FONT_SIZE).into(),
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
    text_engine: &avenger_text::TextEngine,
) -> Result<SceneTextMark, AvengerGuidesError> {
    let range = scale.numeric_interval_range()?;
    let mid = (range.0 + range.1) / 2.0;
    let title_font_size = config.title_font_size.unwrap_or(TITLE_FONT_SIZE);
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{ArrayRef, StringArray};
    use avenger_scales::scales::band::BandScale;

    use super::*;

    #[test]
    fn band_axis_tick_labels_honor_configured_angle() {
        let domain = Arc::new(StringArray::from(vec!["a", "b"])) as ArrayRef;
        let scale = BandScale::configured(domain, (0.0, 100.0));
        let labels = make_tick_labels(
            &scale,
            &AxisConfig {
                label_angle: Some(-90.0),
                ..Default::default()
            },
        )
        .expect("tick labels");

        assert_eq!(
            labels.angle.as_vec(labels.len as usize, None),
            vec![-90.0, -90.0]
        );
    }
}
