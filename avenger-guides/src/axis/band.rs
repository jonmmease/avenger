use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};
use avenger_geometry::{marks::MarkGeometryUtils, rtree::SceneGraphRTree};
use avenger_scales::band::BandScale;
use avenger_scenegraph::marks::{group::SceneGroup, rule::SceneRuleMark, text::SceneTextMark};

use avenger_text::types::{FontWeightNameSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec};
use rstar::AABB;

use super::opts::{AxisConfig, AxisOrientation};
use std::{fmt::Debug, hash::Hash};

const TICK_LENGTH: f32 = 5.0;
const TEXT_MARGIN: f32 = 3.0;
const TITLE_MARGIN: f32 = 4.0;
const TITLE_FONT_SIZE: f32 = 10.0;
const TICK_FONT_SIZE: f32 = 8.0;
const PIXEL_OFFSET: f32 = 0.5;

pub fn make_band_axis_marks<T>(
    scale: &BandScale<T>,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
) -> SceneGroup
where
    T: ToString + Debug + Clone + Hash + Eq + Sync + 'static,
{
    // Make sure ticks end up centered in the band
    // Unwrap is safe because this band value is always valid
    let scale = scale.clone().with_band(0.5).unwrap();

    let mut group = SceneGroup {
        origin,
        ..Default::default()
    };

    // Get range bounds considering orientation
    let range = scale.range();
    let (start, end) = match config.orientation {
        AxisOrientation::Left | AxisOrientation::Right { .. } => {
            let upper = f32::min(range.1, range.0);
            let lower = f32::max(range.0, range.1);
            (lower, upper)
        }
        AxisOrientation::Top | AxisOrientation::Bottom { .. } => {
            let left = f32::min(range.0, range.1);
            let right = f32::max(range.0, range.1);
            (left, right)
        }
    };

    // Add axis line
    let is_vertical = matches!(
        config.orientation,
        AxisOrientation::Left | AxisOrientation::Right { .. }
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
            .push(make_tick_grid_marks(&scale, &config.orientation, &config.dimensions).into());
    }

    // Add tick marks
    group
        .marks
        .push(make_tick_marks(&scale, &config.orientation, &config.dimensions).into());

    // Add tick labels
    group
        .marks
        .push(make_tick_labels(&scale, &config.orientation, &config.dimensions).into());

    // Add title
    group
        .marks
        .push(make_title(title, &scale, &group.bounding_box(), &config.orientation).into());

    group
}

fn make_axis_line(start: f32, end: f32, is_vertical: bool, offset: f32) -> SceneRuleMark {
    let (x0, x1, y0, y1) = if is_vertical {
        (offset, offset, start, end)
    } else {
        (start, end, offset, offset)
    };

    SceneRuleMark {
        x0: x0.into(),
        x1: x1.into(),
        y0: y0.into(),
        y1: y1.into(),
        stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    }
}

fn make_tick_marks<T>(
    scale: &BandScale<T>,
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
) -> SceneRuleMark
where
    T: ToString + Debug + Clone + Hash + Eq + Sync + 'static,
{
    let scaled_values = scale.scale(scale.domain());

    let (x0, x1, y0, y1) = match orientation {
        AxisOrientation::Left => (
            ScalarOrArray::Scalar(0.0),
            ScalarOrArray::Scalar(-TICK_LENGTH),
            scaled_values.clone(),
            scaled_values,
        ),
        AxisOrientation::Right => (
            ScalarOrArray::Scalar(dimensions[0]),
            ScalarOrArray::Scalar(dimensions[0] + TICK_LENGTH),
            scaled_values.clone(),
            scaled_values,
        ),
        AxisOrientation::Top => (
            scaled_values.clone(),
            scaled_values,
            ScalarOrArray::Scalar(0.0),
            ScalarOrArray::Scalar(-TICK_LENGTH),
        ),
        AxisOrientation::Bottom => (
            scaled_values.clone(),
            scaled_values,
            ScalarOrArray::Scalar(dimensions[1]),
            ScalarOrArray::Scalar(dimensions[1] + TICK_LENGTH),
        ),
    };

    SceneRuleMark {
        len: scale.domain().len() as u32,
        clip: false,
        x0,
        x1,
        y0,
        y1,
        stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    }
}

fn make_tick_grid_marks<T>(
    scale: &BandScale<T>,
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
) -> SceneRuleMark
where
    T: ToString + Debug + Clone + Hash + Eq + Sync + 'static,
{
    let scaled_values = scale.scale(scale.domain());

    let (x0, x1, y0, y1) = match orientation {
        AxisOrientation::Left | AxisOrientation::Right => (
            ScalarOrArray::Scalar(0.0),
            ScalarOrArray::Scalar(dimensions[0] + PIXEL_OFFSET),
            scaled_values.clone(),
            scaled_values,
        ),
        AxisOrientation::Top | AxisOrientation::Bottom => (
            scaled_values.clone(),
            scaled_values,
            ScalarOrArray::Scalar(PIXEL_OFFSET),
            ScalarOrArray::Scalar(dimensions[1] + PIXEL_OFFSET),
        ),
    };

    SceneRuleMark {
        len: scale.domain().len() as u32,
        clip: false,
        x0,
        x1,
        y0,
        y1,
        stroke: ColorOrGradient::Color([8.0, 8.0, 8.0, 0.2]).into(),
        stroke_width: 0.1.into(),
        ..Default::default()
    }
}

fn make_tick_labels<T>(
    scale: &BandScale<T>,
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
) -> SceneTextMark
where
    T: ToString + Debug + Clone + Hash + Eq + Sync + 'static,
{
    let scaled_values = scale.scale(scale.domain());

    let (x, y, align, baseline, angle) = match orientation {
        AxisOrientation::Left => (
            ScalarOrArray::Scalar(-TICK_LENGTH - TEXT_MARGIN),
            scaled_values,
            TextAlignSpec::Right,
            TextBaselineSpec::Middle,
            0.0,
        ),
        AxisOrientation::Right => (
            ScalarOrArray::Scalar(dimensions[0] + TICK_LENGTH + TEXT_MARGIN),
            scaled_values,
            TextAlignSpec::Left,
            TextBaselineSpec::Middle,
            0.0,
        ),
        AxisOrientation::Top => (
            scaled_values,
            ScalarOrArray::Scalar(-TICK_LENGTH - TEXT_MARGIN + PIXEL_OFFSET),
            TextAlignSpec::Center,
            TextBaselineSpec::Bottom,
            0.0,
        ),
        AxisOrientation::Bottom => (
            scaled_values,
            ScalarOrArray::Scalar(dimensions[1] + PIXEL_OFFSET + TICK_LENGTH + TEXT_MARGIN),
            TextAlignSpec::Center,
            TextBaselineSpec::Top,
            0.0,
        ),
    };

    SceneTextMark {
        len: scale.domain().len() as u32,
        text: scale
            .domain()
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .into(),
        x,
        y,
        align: align.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        color: [0.0, 0.0, 0.0, 1.0].into(),
        font_size: TICK_FONT_SIZE.into(),
        ..Default::default()
    }
}

fn make_title<T>(
    title: &str,
    scale: &BandScale<T>,
    envelope: &AABB<[f32; 2]>,
    orientation: &AxisOrientation,
) -> SceneTextMark
where
    T: ToString + Debug + Clone + Hash + Eq + Sync + 'static,
{
    let range = scale.range();
    let mid = (range.0 + range.1) / 2.0;

    let (x, y, align, baseline, angle) = match orientation {
        AxisOrientation::Left => (
            (envelope.lower()[0] - TITLE_MARGIN).into(),
            mid.into(),
            TextAlignSpec::Center,
            TextBaselineSpec::LineBottom,
            -90.0,
        ),
        AxisOrientation::Right => (
            (envelope.upper()[0] + TITLE_MARGIN).into(),
            mid.into(),
            TextAlignSpec::Center,
            TextBaselineSpec::LineBottom,
            90.0,
        ),
        AxisOrientation::Top => (
            mid.into(),
            (envelope.lower()[1] - TITLE_MARGIN).into(),
            TextAlignSpec::Center,
            TextBaselineSpec::Bottom,
            0.0,
        ),
        AxisOrientation::Bottom => (
            mid.into(),
            (envelope.upper()[1] + TITLE_MARGIN).into(),
            TextAlignSpec::Center,
            TextBaselineSpec::Top,
            0.0,
        ),
    };

    SceneTextMark {
        len: 1,
        text: title.to_string().into(),
        x,
        y,
        align: align.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        color: [0.0, 0.0, 0.0, 1.0].into(),
        font_size: TITLE_FONT_SIZE.into(),
        font_weight: FontWeightSpec::Name(FontWeightNameSpec::Bold).into(),
        ..Default::default()
    }
}
