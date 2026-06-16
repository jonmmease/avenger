use std::collections::BTreeMap;

use avenger_chart_core::AvengerChartError;
use avenger_color::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_geometry::{marks::MarkGeometryUtils, rtree::EnvelopeUtils};
use avenger_guides::axis::opts::{AxisConfig, AxisOrientation};
use avenger_scales::scales::{
    ConfiguredScale,
    nested_band::{NestedBandAxisBand, nested_axis_bands, nested_band_layout},
};
use avenger_scenegraph::marks::{group::SceneGroup, rule::SceneRuleMark, text::SceneTextMark};
use avenger_text::types::{FontWeight, TextAlign, TextBaseline};

const TICK_LENGTH: f32 = 5.0;
const TEXT_MARGIN: f32 = 3.0;
const TITLE_MARGIN: f32 = 4.0;
const TITLE_FONT_SIZE: f32 = 12.0;
const TICK_FONT_SIZE: f32 = 12.0;
const PIXEL_OFFSET: f32 = 0.5;
const LEVEL_GAP: f32 = 8.0;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NestedAxisLevelGuideConfig {
    pub(crate) visible: bool,
}

pub(crate) fn make_nested_axis_marks(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
    level_configs: Option<&BTreeMap<usize, NestedAxisLevelGuideConfig>>,
) -> Result<SceneGroup, AvengerChartError> {
    let layout = nested_band_layout(&scale.config)?;
    let level_count = layout.leaf_level() + 1;
    let leaf_bands = nested_axis_bands(&scale.config, level_count - 1)?;

    let mut main_group = SceneGroup {
        origin: [0.0, 0.0],
        ..Default::default()
    };

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

    let is_vertical = matches!(
        config.orientation,
        AxisOrientation::Left | AxisOrientation::Right
    );
    let offset = match config.orientation {
        AxisOrientation::Left => 0.0,
        AxisOrientation::Right => config.dimensions[0],
        AxisOrientation::Top => 0.0,
        AxisOrientation::Bottom => config.dimensions[1],
    };

    if config.grid {
        main_group.marks.push(
            make_grid_marks(&leaf_bands, &config.orientation, &config.dimensions, config)?.into(),
        );
    }

    let mut axis_group = SceneGroup {
        origin: [0.0, 0.0],
        zindex: Some(1),
        ..Default::default()
    };

    axis_group
        .marks
        .push(make_rule(start, end, is_vertical, offset, config.domain_color, 1.0).into());
    axis_group
        .marks
        .push(make_tick_marks(&leaf_bands, &config.orientation, offset, config).into());

    if config.labels_visible.unwrap_or(true) {
        for level in (0..level_count).rev() {
            if level_configs
                .and_then(|configs| configs.get(&level))
                .is_some_and(|config| !config.visible)
            {
                continue;
            }
            let bands = nested_axis_bands(&scale.config, level)?;
            axis_group
                .marks
                .push(make_level_labels(&bands, level_count, config)?.into());
            if level < level_count - 1 {
                axis_group
                    .marks
                    .push(make_level_boundaries(&bands, level_count, config)?.into());
            }
        }
    }

    if config.title_visible.unwrap_or(true) && !title.is_empty() {
        let envelope = axis_group.bounding_box();
        axis_group
            .marks
            .push(make_title(title, scale, envelope.lower(), envelope.upper(), config)?.into());
    }

    main_group.marks.push(axis_group.into());

    let bbox = main_group.bounding_box();
    let padding = 2.0;
    main_group.clip = avenger_scenegraph::marks::group::Clip::Rect {
        x: bbox.lower()[0] - padding,
        y: bbox.lower()[1] - padding,
        width: bbox.width() + 2.0 * padding,
        height: bbox.height() + 2.0 * padding,
    };
    main_group.origin = origin;

    Ok(main_group)
}

fn make_rule(
    start: f32,
    end: f32,
    is_vertical: bool,
    offset: f32,
    color: Option<[f32; 4]>,
    width: f32,
) -> SceneRuleMark {
    let (x, x2, y, y2) = if is_vertical {
        (offset, offset, start, end)
    } else {
        (start, end, offset, offset)
    };
    SceneRuleMark {
        x: x.into(),
        x2: x2.into(),
        y: y.into(),
        y2: y2.into(),
        stroke: ColorOrGradient::Color(color.unwrap_or([0.0, 0.0, 0.0, 1.0])).into(),
        stroke_width: width.into(),
        ..Default::default()
    }
}

fn make_tick_marks(
    bands: &[NestedBandAxisBand],
    orientation: &AxisOrientation,
    offset: f32,
    config: &AxisConfig,
) -> SceneRuleMark {
    let tick_len = config.tick_length.unwrap_or(TICK_LENGTH);
    let centers = band_centers(bands);
    let n = centers.len() as u32;
    let stroke = ColorOrGradient::Color(config.tick_color.unwrap_or([0.0, 0.0, 0.0, 1.0]));

    let (x, x2, y, y2) = match orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(offset),
            ScalarOrArray::new_scalar(offset - tick_len),
            ScalarOrArray::new_array(centers.clone()),
            ScalarOrArray::new_array(centers),
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(offset),
            ScalarOrArray::new_scalar(offset + tick_len),
            ScalarOrArray::new_array(centers.clone()),
            ScalarOrArray::new_array(centers),
        ),
        AxisOrientation::Top => (
            ScalarOrArray::new_array(centers.clone()),
            ScalarOrArray::new_array(centers),
            ScalarOrArray::new_scalar(offset),
            ScalarOrArray::new_scalar(offset - tick_len),
        ),
        AxisOrientation::Bottom => (
            ScalarOrArray::new_array(centers.clone()),
            ScalarOrArray::new_array(centers),
            ScalarOrArray::new_scalar(offset),
            ScalarOrArray::new_scalar(offset + tick_len),
        ),
    };

    SceneRuleMark {
        len: n,
        clip: false,
        x,
        x2,
        y,
        y2,
        stroke: stroke.into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    }
}

fn make_grid_marks(
    bands: &[NestedBandAxisBand],
    orientation: &AxisOrientation,
    dimensions: &[f32; 2],
    config: &AxisConfig,
) -> Result<SceneGroup, AvengerChartError> {
    let centers = band_centers(bands);
    let n = centers.len() as u32;
    let stroke = ColorOrGradient::Color(config.grid_color.unwrap_or([0.878, 0.878, 0.878, 0.5]));
    let (x, x2, y, y2) = match orientation {
        AxisOrientation::Left | AxisOrientation::Right => (
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(dimensions[0]),
            ScalarOrArray::new_array(centers.clone()),
            ScalarOrArray::new_array(centers),
        ),
        AxisOrientation::Top | AxisOrientation::Bottom => (
            ScalarOrArray::new_array(centers.clone()),
            ScalarOrArray::new_array(centers),
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(dimensions[1]),
        ),
    };

    Ok(SceneGroup {
        origin: [0.0, 0.0],
        zindex: Some(-1),
        marks: vec![
            SceneRuleMark {
                len: n,
                clip: false,
                x,
                x2,
                y,
                y2,
                stroke: stroke.into(),
                stroke_width: config.grid_width.unwrap_or(0.5).into(),
                ..Default::default()
            }
            .into(),
        ],
        ..Default::default()
    })
}

fn make_level_labels(
    bands: &[NestedBandAxisBand],
    level_count: usize,
    config: &AxisConfig,
) -> Result<SceneTextMark, AvengerChartError> {
    let font_size = config.label_font_size.unwrap_or(TICK_FONT_SIZE);
    let font_adjustment = font_size * 0.10;
    let centers = band_centers(bands);
    let labels = bands
        .iter()
        .map(|band| band.label.clone())
        .collect::<Vec<_>>();
    let level = bands.first().map(|band| band.level).unwrap_or(0);
    let row = level_count.saturating_sub(1).saturating_sub(level) as f32;
    let distance =
        config.tick_length.unwrap_or(TICK_LENGTH) + TEXT_MARGIN + row * (font_size + LEVEL_GAP);
    let leaf_level = level_count.saturating_sub(1);
    let leaf_angle = if level == leaf_level {
        config.label_angle.unwrap_or(0.0)
    } else {
        0.0
    };

    let (x, y, align, baseline, angle) = match config.orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(-distance),
            ScalarOrArray::new_array(centers.iter().map(|v| v - font_adjustment).collect()),
            TextAlign::Right,
            TextBaseline::Middle,
            0.0,
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(config.dimensions[0] + distance),
            ScalarOrArray::new_array(centers.iter().map(|v| v - font_adjustment).collect()),
            TextAlign::Left,
            TextBaseline::Middle,
            0.0,
        ),
        AxisOrientation::Top => (
            ScalarOrArray::new_array(centers),
            ScalarOrArray::new_scalar(-distance),
            angled_label_align(leaf_angle, true),
            TextBaseline::Bottom,
            leaf_angle,
        ),
        AxisOrientation::Bottom => (
            ScalarOrArray::new_array(centers),
            ScalarOrArray::new_scalar(config.dimensions[1] + distance),
            angled_label_align(leaf_angle, false),
            TextBaseline::Top,
            leaf_angle,
        ),
    };

    Ok(SceneTextMark {
        len: labels.len() as u32,
        text: ScalarOrArray::new_array(labels),
        x,
        y,
        align: align.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        color: ColorOrGradient::Color(config.label_color.unwrap_or([0.0, 0.0, 0.0, 1.0])).into(),
        font_size: font_size.into(),
        font_weight: FontWeight::Number(config.label_font_weight.unwrap_or(400.0)).into(),
        font: config
            .label_font_family
            .clone()
            .unwrap_or_else(|| "sans-serif".to_string())
            .into(),
        ..Default::default()
    })
}

fn make_level_boundaries(
    bands: &[NestedBandAxisBand],
    level_count: usize,
    config: &AxisConfig,
) -> Result<SceneRuleMark, AvengerChartError> {
    let Some(first) = bands.first() else {
        return Ok(SceneRuleMark::default());
    };
    let font_size = config.label_font_size.unwrap_or(TICK_FONT_SIZE);
    let row = level_count.saturating_sub(1).saturating_sub(first.level) as f32;
    let tick_len = config.tick_length.unwrap_or(TICK_LENGTH);
    let inner = tick_len + row * (font_size + LEVEL_GAP);
    let outer = inner + font_size + LEVEL_GAP * 0.5;
    let mut positions = Vec::with_capacity(bands.len() + 1);
    positions.push(first.start);
    positions.extend(bands.iter().map(|band| band.end));

    let (x, x2, y, y2) = match config.orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(-inner),
            ScalarOrArray::new_scalar(-outer),
            ScalarOrArray::new_array(positions.clone()),
            ScalarOrArray::new_array(positions),
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(config.dimensions[0] + inner),
            ScalarOrArray::new_scalar(config.dimensions[0] + outer),
            ScalarOrArray::new_array(positions.clone()),
            ScalarOrArray::new_array(positions),
        ),
        AxisOrientation::Top => (
            ScalarOrArray::new_array(positions.clone()),
            ScalarOrArray::new_array(positions),
            ScalarOrArray::new_scalar(-inner),
            ScalarOrArray::new_scalar(-outer),
        ),
        AxisOrientation::Bottom => (
            ScalarOrArray::new_array(positions.clone()),
            ScalarOrArray::new_array(positions),
            ScalarOrArray::new_scalar(config.dimensions[1] + inner),
            ScalarOrArray::new_scalar(config.dimensions[1] + outer),
        ),
    };

    Ok(SceneRuleMark {
        len: (bands.len() + 1) as u32,
        clip: false,
        x,
        x2,
        y,
        y2,
        stroke: ColorOrGradient::Color(config.tick_color.unwrap_or([0.0, 0.0, 0.0, 1.0])).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    })
}

fn make_title(
    title: &str,
    scale: &ConfiguredScale,
    lower: [f32; 2],
    upper: [f32; 2],
    config: &AxisConfig,
) -> Result<SceneTextMark, AvengerChartError> {
    let range = scale.numeric_interval_range()?;
    let mid = (range.0 + range.1) / 2.0;
    let (x, y, align, baseline, angle) = match config.orientation {
        AxisOrientation::Left => (
            (lower[0] - TITLE_MARGIN).into(),
            mid.into(),
            TextAlign::Center,
            TextBaseline::LineBottom,
            -90.0,
        ),
        AxisOrientation::Right => (
            (upper[0] + TITLE_MARGIN).into(),
            mid.into(),
            TextAlign::Center,
            TextBaseline::LineBottom,
            90.0,
        ),
        AxisOrientation::Top => (
            mid.into(),
            (lower[1] - TITLE_MARGIN).into(),
            TextAlign::Center,
            TextBaseline::Bottom,
            0.0,
        ),
        AxisOrientation::Bottom => (
            mid.into(),
            (upper[1] + TITLE_MARGIN).into(),
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
        color: ColorOrGradient::Color(config.title_color.unwrap_or([0.0, 0.0, 0.0, 1.0])).into(),
        font_size: config.title_font_size.unwrap_or(TITLE_FONT_SIZE).into(),
        font_weight: FontWeight::Number(config.title_font_weight.unwrap_or(400.0)).into(),
        font: config
            .title_font_family
            .clone()
            .unwrap_or_else(|| "sans-serif".to_string())
            .into(),
        ..Default::default()
    })
}

fn band_centers(bands: &[NestedBandAxisBand]) -> Vec<f32> {
    bands.iter().map(|band| band.center).collect()
}

fn angled_label_align(angle: f32, top_axis: bool) -> TextAlign {
    if angle == 0.0 {
        TextAlign::Center
    } else if (angle < 0.0) ^ top_axis {
        TextAlign::Right
    } else {
        TextAlign::Left
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use avenger_guides::axis::opts::{AxisConfig, AxisOrientation};
    use avenger_scales::scales::nested_band::NestedBandScale;
    use avenger_scenegraph::marks::mark::SceneMark;
    use datafusion::arrow::{
        array::{ArrayRef, StringArray, StructArray},
        datatypes::Field,
    };

    use super::make_nested_axis_marks;

    fn utf8_struct(columns: &[(&str, Vec<&str>)]) -> ArrayRef {
        let columns = columns
            .iter()
            .map(|(name, values)| {
                (
                    Arc::new(Field::new(
                        *name,
                        datafusion::arrow::datatypes::DataType::Utf8,
                        true,
                    )),
                    Arc::new(StringArray::from(values.clone())) as ArrayRef,
                )
            })
            .collect::<Vec<_>>();
        Arc::new(StructArray::from(columns)) as ArrayRef
    }

    #[test]
    fn nested_axis_renders_leaf_and_parent_levels() {
        let domain = utf8_struct(&[
            ("cylinders", vec!["4", "4", "6"]),
            ("manufacturer", vec!["ford", "toyota", "ford"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 240.0))
            .with_option("nest_scopes", "free,shared")
            .with_option("padding_inner_levels", "0.3,0.1");
        let axis = make_nested_axis_marks(
            &scale,
            "Manufacturer grouped by cylinders",
            [12.0, 24.0],
            &AxisConfig {
                orientation: AxisOrientation::Bottom,
                dimensions: [240.0, 120.0],
                labels_visible: Some(true),
                title_visible: Some(true),
                ..Default::default()
            },
            None,
        )
        .expect("nested axis");

        assert_eq!(axis.origin, [12.0, 24.0]);
        assert_eq!(axis.marks.len(), 1);
        let SceneMark::Group(axis_elements) = &axis.marks[0] else {
            panic!("expected axis element group");
        };
        assert!(
            axis_elements.marks.len() >= 5,
            "expected axis line, ticks, two label levels, boundary, and title"
        );
    }
}
