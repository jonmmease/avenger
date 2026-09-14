//! Nested categorical axis rendering.
//!
//! This follows the same outward-stacked guide-band shape as facet/container
//! band guides: visible leaf ticks stay closest to the plot area, visible
//! parent labels occupy progressively outer slabs, and separator rules show
//! parent spans.

use std::collections::BTreeMap;

use avenger_color::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_geometry::{marks::MarkGeometryUtils, rtree::EnvelopeUtils};
use avenger_scales::scales::{
    nested_band::{nested_axis_bands, nested_band_layout, NestedBandAxisBand},
    ConfiguredScale,
};
use avenger_scenegraph::marks::{group::SceneGroup, rule::SceneRuleMark, text::SceneTextMark};
use avenger_text::{
    measurement::TextMeasurementConfig,
    types::{FontStyle, FontWeight, TextAlign, TextBaseline},
    TextEngine,
};

use super::opts::{AxisConfig, AxisOrientation};
use crate::error::AvengerGuidesError;

const TICK_LENGTH: f32 = 5.0;
const TEXT_MARGIN: f32 = 3.0;
const TITLE_MARGIN: f32 = 4.0;
const TITLE_FONT_SIZE: f32 = 12.0;
const TICK_FONT_SIZE: f32 = 12.0;
const PIXEL_OFFSET: f32 = 0.5;
const LEVEL_GAP: f32 = 8.0;

#[derive(Clone, Debug, PartialEq)]
pub struct NestedBandAxisLevelConfig {
    pub visible: bool,
    pub title: Option<String>,
    pub title_visible: bool,
    pub label_angle: Option<f32>,
}

#[derive(Clone, Copy, Debug)]
struct NestedAxisLevelLabelLayout {
    label_distance: f32,
    boundary_outer: f32,
}

pub fn make_nested_band_axis_marks(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
    level_configs: Option<&BTreeMap<usize, NestedBandAxisLevelConfig>>,
) -> Result<SceneGroup, AvengerGuidesError> {
    let text_engine = avenger_text::default_text_engine();
    make_nested_band_axis_marks_with_text_engine(
        scale,
        title,
        origin,
        config,
        level_configs,
        &text_engine,
    )
}

pub fn make_nested_band_axis_marks_with_text_engine(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
    level_configs: Option<&BTreeMap<usize, NestedBandAxisLevelConfig>>,
    text_engine: &TextEngine,
) -> Result<SceneGroup, AvengerGuidesError> {
    let layout = nested_band_layout(&scale.config)?;
    let level_count = layout.leaf_level() + 1;
    let level_bands = (0..level_count)
        .map(|level| nested_axis_bands(&scale.config, level))
        .collect::<Result<Vec<_>, _>>()?;
    let leaf_bands = &level_bands[level_count - 1];

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
    let label_layouts = nested_axis_level_label_layouts(
        &level_bands,
        is_vertical,
        config,
        level_configs,
        text_engine,
    );
    let offset = match config.orientation {
        AxisOrientation::Left => 0.0,
        AxisOrientation::Right => config.dimensions[0],
        AxisOrientation::Top => 0.0,
        AxisOrientation::Bottom => config.dimensions[1],
    };

    if config.grid {
        main_group.marks.push(
            make_grid_marks(leaf_bands, &config.orientation, &config.dimensions, config)?.into(),
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
    if level_visible(level_configs, level_count - 1) {
        axis_group
            .marks
            .push(make_tick_marks(leaf_bands, &config.orientation, offset, config).into());
    }

    if config.labels_visible.unwrap_or(true) {
        for level in (0..level_count).rev() {
            let level_config = level_configs.and_then(|configs| configs.get(&level));
            if level_config.is_some_and(|config| !config.visible) {
                continue;
            }
            let bands = &level_bands[level];
            axis_group.marks.push(
                make_level_labels(
                    bands,
                    level_count - 1,
                    config,
                    level_config,
                    label_layouts[level],
                )?
                .into(),
            );
            if level < level_count - 1 {
                axis_group.marks.push(
                    make_level_boundaries(bands, config, label_layouts[level], range.0, range.1)?
                        .into(),
                );
            }
        }
    }

    let title = nested_axis_title(title, &layout, level_configs);
    if config.title_visible.unwrap_or(true) && !title.is_empty() {
        let envelope = axis_group.bounding_box_with_text_engine(text_engine);
        axis_group.marks.push(
            make_title(
                &title,
                scale,
                envelope.lower(),
                envelope.upper(),
                config,
                text_engine,
            )?
            .into(),
        );
    }

    main_group.marks.push(axis_group.into());

    let bbox = main_group.bounding_box_with_text_engine(text_engine);
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

fn level_visible(
    level_configs: Option<&BTreeMap<usize, NestedBandAxisLevelConfig>>,
    level: usize,
) -> bool {
    level_configs
        .and_then(|configs| configs.get(&level))
        .is_none_or(|config| config.visible)
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
) -> Result<SceneGroup, AvengerGuidesError> {
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
        interactive: false,
        origin: [0.0, 0.0],
        zindex: Some(-1),
        marks: vec![SceneRuleMark {
            interactive: false,
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
        .into()],
        ..Default::default()
    })
}

fn make_level_labels(
    bands: &[NestedBandAxisBand],
    leaf_level: usize,
    config: &AxisConfig,
    level_config: Option<&NestedBandAxisLevelConfig>,
    layout: NestedAxisLevelLabelLayout,
) -> Result<SceneTextMark, AvengerGuidesError> {
    let font_size = config.label_font_size.unwrap_or(TICK_FONT_SIZE);
    let font_adjustment = font_size * 0.10;
    let centers = band_centers(bands);
    let labels = bands
        .iter()
        .map(|band| band.label.clone())
        .collect::<Vec<_>>();
    let level = bands.first().map(|band| band.level).unwrap_or(0);
    let leaf_angle = level_label_angle(level, leaf_level, config, level_config);
    let (horizontal_label_dx, horizontal_label_dy) =
        horizontal_axis_label_optical_offset(leaf_angle, font_adjustment);

    let (x, y, align, baseline, angle) = match config.orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(-layout.label_distance),
            ScalarOrArray::new_array(centers.iter().map(|v| v - font_adjustment).collect()),
            TextAlign::Right,
            TextBaseline::Middle,
            0.0,
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(config.dimensions[0] + layout.label_distance),
            ScalarOrArray::new_array(centers.iter().map(|v| v - font_adjustment).collect()),
            TextAlign::Left,
            TextBaseline::Middle,
            0.0,
        ),
        AxisOrientation::Top => (
            ScalarOrArray::new_array(
                centers
                    .iter()
                    .map(|v| v + horizontal_label_dx)
                    .collect::<Vec<_>>(),
            ),
            ScalarOrArray::new_scalar(-layout.label_distance + horizontal_label_dy),
            angled_label_align(leaf_angle, true),
            horizontal_axis_label_baseline(leaf_angle, true),
            leaf_angle,
        ),
        AxisOrientation::Bottom => (
            ScalarOrArray::new_array(
                centers
                    .iter()
                    .map(|v| v + horizontal_label_dx)
                    .collect::<Vec<_>>(),
            ),
            ScalarOrArray::new_scalar(
                config.dimensions[1] + layout.label_distance + horizontal_label_dy,
            ),
            angled_label_align(leaf_angle, false),
            horizontal_axis_label_baseline(leaf_angle, false),
            leaf_angle,
        ),
    };

    Ok(SceneTextMark {
        clip: false,
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
    config: &AxisConfig,
    layout: NestedAxisLevelLabelLayout,
    outer_start: f32,
    outer_end: f32,
) -> Result<SceneRuleMark, AvengerGuidesError> {
    let positions = level_boundary_positions(bands, outer_start, outer_end);
    if positions.is_empty() {
        return Ok(SceneRuleMark::default());
    };

    let (x, x2, y, y2) = match config.orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(-layout.boundary_outer),
            ScalarOrArray::new_array(positions.clone()),
            ScalarOrArray::new_array(positions),
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(config.dimensions[0]),
            ScalarOrArray::new_scalar(config.dimensions[0] + layout.boundary_outer),
            ScalarOrArray::new_array(positions.clone()),
            ScalarOrArray::new_array(positions),
        ),
        AxisOrientation::Top => (
            ScalarOrArray::new_array(positions.clone()),
            ScalarOrArray::new_array(positions),
            ScalarOrArray::new_scalar(0.0),
            ScalarOrArray::new_scalar(-layout.boundary_outer),
        ),
        AxisOrientation::Bottom => (
            ScalarOrArray::new_array(positions.clone()),
            ScalarOrArray::new_array(positions),
            ScalarOrArray::new_scalar(config.dimensions[1]),
            ScalarOrArray::new_scalar(config.dimensions[1] + layout.boundary_outer),
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

fn level_boundary_positions(
    bands: &[NestedBandAxisBand],
    outer_start: f32,
    outer_end: f32,
) -> Vec<f32> {
    if bands.is_empty() {
        return Vec::new();
    };

    let mut positions = Vec::with_capacity(bands.len() + 1);
    positions.push(outer_start);
    positions.extend(
        bands
            .windows(2)
            .map(|pair| (pair[0].end + pair[1].start) / 2.0),
    );
    positions.push(outer_end);
    positions
}

fn make_title(
    title: &str,
    scale: &ConfiguredScale,
    lower: [f32; 2],
    upper: [f32; 2],
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
        clip: false,
        len: 1,
        text: title.to_string().into(),
        x,
        y,
        align: align.into(),
        baseline: baseline.into(),
        angle: angle.into(),
        color: ColorOrGradient::Color(config.title_color.unwrap_or([0.0, 0.0, 0.0, 1.0])).into(),
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

fn band_centers(bands: &[NestedBandAxisBand]) -> Vec<f32> {
    bands.iter().map(|band| band.center).collect()
}

fn nested_axis_level_label_layouts(
    level_bands: &[Vec<NestedBandAxisBand>],
    is_vertical: bool,
    config: &AxisConfig,
    level_configs: Option<&BTreeMap<usize, NestedBandAxisLevelConfig>>,
    text_engine: &TextEngine,
) -> Vec<NestedAxisLevelLabelLayout> {
    let font_size = config.label_font_size.unwrap_or(TICK_FONT_SIZE);
    let font_weight = FontWeight::Number(config.label_font_weight.unwrap_or(400.0));
    let font_family = config.label_font_family.as_deref().unwrap_or("sans-serif");
    let tick_len = config.tick_length.unwrap_or(TICK_LENGTH);
    let leaf_level = level_bands.len().saturating_sub(1);
    let ticks_visible = level_visible(level_configs, leaf_level);
    let inner_tick_space = if ticks_visible { tick_len } else { 0.0 };
    let initial_label_distance = inner_tick_space + TEXT_MARGIN;
    let mut layouts = vec![
        NestedAxisLevelLabelLayout {
            label_distance: initial_label_distance,
            boundary_outer: initial_label_distance + font_size + LEVEL_GAP * 0.5,
        };
        level_bands.len()
    ];
    let mut label_distance = initial_label_distance;

    for level in (0..level_bands.len()).rev() {
        let level_config = level_configs.and_then(|configs| configs.get(&level));
        if level_config.is_some_and(|config| !config.visible) {
            continue;
        }
        let angle = level_label_angle(level, leaf_level, config, level_config);
        let extent = level_label_cross_extent(
            &level_bands[level],
            font_family,
            font_size,
            &font_weight,
            angle,
            is_vertical,
            text_engine,
        );
        layouts[level] = NestedAxisLevelLabelLayout {
            label_distance,
            boundary_outer: label_distance + extent + LEVEL_GAP * 0.5,
        };
        label_distance += extent + LEVEL_GAP;
    }

    layouts
}

fn level_label_angle(
    level: usize,
    leaf_level: usize,
    config: &AxisConfig,
    level_config: Option<&NestedBandAxisLevelConfig>,
) -> f32 {
    level_config
        .and_then(|config| config.label_angle)
        .unwrap_or_else(|| {
            if level == leaf_level {
                config.label_angle.unwrap_or(0.0)
            } else {
                0.0
            }
        })
}

fn level_label_cross_extent(
    bands: &[NestedBandAxisBand],
    font_family: &str,
    font_size: f32,
    font_weight: &FontWeight,
    angle: f32,
    is_vertical: bool,
    text_engine: &TextEngine,
) -> f32 {
    let (max_width, max_height) = bands
        .iter()
        .map(|band| {
            let bounds =
                text_engine.measure_bounds_with_plain_fallback_or_approx(&TextMeasurementConfig {
                    text: &band.label,
                    font: font_family,
                    font_size,
                    font_weight: *font_weight,
                    font_style: FontStyle::Normal,
                    syntax_mode: avenger_text::types::TextSyntaxMode::Plain,
                    params: avenger_text::empty_label_params(),
                    number_locale: None,
                    number_locale_specs: None,
                    datetime_locale: None,
                    datetime_timezone: None,
                    datetime_locale_specs: None,
                });
            (bounds.width, bounds.height)
        })
        .fold(
            (0.0_f32, 0.0_f32),
            |(max_width, max_height), (width, height)| {
                (max_width.max(width), max_height.max(height))
            },
        );
    if is_vertical {
        return max_width;
    }

    let radians = angle.to_radians().abs();
    if radians == 0.0 {
        max_height
    } else {
        max_width * radians.sin().abs() + max_height * radians.cos().abs()
    }
}

fn nested_axis_title(
    explicit_title: &str,
    layout: &avenger_scales::scales::nested_band::NestedBandLayout,
    level_configs: Option<&BTreeMap<usize, NestedBandAxisLevelConfig>>,
) -> String {
    if !explicit_title.is_empty() {
        return explicit_title.to_string();
    }

    let mut level_titles = Vec::new();
    for level in 0..=layout.leaf_level() {
        let level_config = level_configs.and_then(|configs| configs.get(&level));
        if !level_visible(level_configs, level)
            || level_config.is_some_and(|config| !config.title_visible)
        {
            continue;
        }

        let title = level_config
            .and_then(|config| config.title.as_ref())
            .filter(|title| !title.is_empty())
            .cloned()
            .or_else(|| {
                layout
                    .field_names()
                    .get(level)
                    .filter(|title| !title.is_empty())
                    .cloned()
            })
            .unwrap_or_else(|| format!("level {level}"));
        level_titles.push(title);
    }

    match level_titles.as_slice() {
        [] => String::new(),
        [title] => title.clone(),
        titles => {
            let leaf = titles.last().expect("non-empty titles");
            let parents = titles[..titles.len() - 1].join(" / ");
            format!("{leaf} grouped by {parents}")
        }
    }
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

fn horizontal_axis_label_baseline(angle: f32, top_axis: bool) -> TextBaseline {
    if angle_is_steep(angle) {
        TextBaseline::Middle
    } else if top_axis {
        TextBaseline::Bottom
    } else {
        TextBaseline::Top
    }
}

fn horizontal_axis_label_optical_offset(angle: f32, font_adjustment: f32) -> (f32, f32) {
    if !angle_is_steep(angle) {
        return (0.0, 0.0);
    }

    let radians = angle.to_radians();
    (
        font_adjustment * radians.sin(),
        -font_adjustment * radians.cos(),
    )
}

fn angle_is_steep(angle: f32) -> bool {
    let normalized = angle.rem_euclid(180.0);
    let angle_from_horizontal = normalized.min(180.0 - normalized);
    angle_from_horizontal > 45.0
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use arrow::{
        array::{ArrayRef, Int32Array, StringArray, StructArray},
        datatypes::{DataType, Field},
    };
    use avenger_color::ColorOrGradient;
    use avenger_scales::scales::nested_band::{nested_axis_bands, NestedBandScale};
    use avenger_scenegraph::marks::mark::SceneMark;
    use avenger_text::types::{TextAlign, TextBaseline};

    use crate::axis::opts::{AxisConfig, AxisOrientation};

    use super::{
        angled_label_align, horizontal_axis_label_baseline, horizontal_axis_label_optical_offset,
        level_boundary_positions, make_level_boundaries, make_nested_band_axis_marks,
        nested_axis_level_label_layouts, nested_axis_title, NestedAxisLevelLabelLayout,
        NestedBandAxisLevelConfig, LEVEL_GAP, TEXT_MARGIN, TICK_FONT_SIZE,
    };

    fn utf8_struct(columns: &[(&str, Vec<&str>)]) -> ArrayRef {
        let columns = columns
            .iter()
            .map(|(name, values)| {
                (
                    Arc::new(Field::new(*name, DataType::Utf8, true)),
                    Arc::new(StringArray::from(values.clone())) as ArrayRef,
                )
            })
            .collect::<Vec<_>>();
        Arc::new(StructArray::from(columns)) as ArrayRef
    }

    fn i32_struct(columns: &[(&str, Vec<Option<i32>>)]) -> ArrayRef {
        let columns = columns
            .iter()
            .map(|(name, values)| {
                (
                    Arc::new(Field::new(*name, DataType::Int32, true)),
                    Arc::new(Int32Array::from(values.clone())) as ArrayRef,
                )
            })
            .collect::<Vec<_>>();
        Arc::new(StructArray::from(columns)) as ArrayRef
    }

    #[allow(
        clippy::type_complexity,
        reason = "The tuple describes the input or output of this test fixture."
    )]
    fn labeled_i32_struct(columns: &[(&str, Vec<Option<i32>>, Vec<Option<&str>>)]) -> ArrayRef {
        let columns = columns
            .iter()
            .map(|(name, keys, labels)| {
                let key_field = Arc::new(Field::new("key", DataType::Int32, true));
                let label_field = Arc::new(Field::new("label", DataType::Utf8, true));
                let component = Arc::new(StructArray::from(vec![
                    (
                        key_field,
                        Arc::new(Int32Array::from(keys.clone())) as ArrayRef,
                    ),
                    (
                        label_field,
                        Arc::new(StringArray::from(labels.clone())) as ArrayRef,
                    ),
                ])) as ArrayRef;
                (
                    Arc::new(Field::new(*name, component.data_type().clone(), true)),
                    component,
                )
            })
            .collect::<Vec<_>>();
        Arc::new(StructArray::from(columns)) as ArrayRef
    }

    fn axis_text_marks(
        axis: &avenger_scenegraph::marks::group::SceneGroup,
    ) -> Vec<&avenger_scenegraph::marks::text::SceneTextMark> {
        let SceneMark::Group(axis_elements) = &axis.marks[0] else {
            panic!("expected axis element group");
        };
        axis_elements
            .marks
            .iter()
            .filter_map(|mark| match mark {
                SceneMark::Text(text) => Some(text.as_ref()),
                _ => None,
            })
            .collect()
    }

    fn axis_text_strings(axis: &avenger_scenegraph::marks::group::SceneGroup) -> Vec<String> {
        axis_text_marks(axis)
            .into_iter()
            .flat_map(|text| text.text_iter().cloned().collect::<Vec<_>>())
            .collect()
    }

    fn axis_text_lengths(axis: &avenger_scenegraph::marks::group::SceneGroup) -> Vec<u32> {
        let SceneMark::Group(axis_elements) = &axis.marks[0] else {
            panic!("expected axis element group");
        };
        axis_elements
            .marks
            .iter()
            .filter_map(|mark| match mark {
                SceneMark::Text(text) => Some(text.len),
                _ => None,
            })
            .collect()
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "expected {actual} to be close to {expected}"
        );
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
        let axis = make_nested_band_axis_marks(
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

    #[test]
    fn nested_axis_level_boundaries_center_internal_group_gaps() {
        let domain = utf8_struct(&[
            ("cylinders", vec!["4", "4", "6"]),
            ("manufacturer", vec!["ford", "toyota", "ford"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 370.0))
            .with_option("padding_inner_levels", "0.3,0")
            .with_option("padding_outer_levels", "0.2,0");
        let parent_bands = nested_axis_bands(&scale.config, 0).expect("parent bands");

        assert_close(parent_bands[0].start, 20.0);
        assert_close(parent_bands[0].end, 220.0);
        assert_close(parent_bands[1].start, 250.0);
        assert_close(parent_bands[1].end, 350.0);

        let positions = level_boundary_positions(&parent_bands, 0.0, 370.0);
        assert_close(positions[0], 0.0);
        assert_close(positions[1], 235.0);
        assert_close(positions[2], 370.0);
    }

    #[test]
    fn nested_axis_boundaries_start_at_axis_and_use_tick_color() {
        let domain = utf8_struct(&[
            ("quarter", vec!["Q1", "Q1", "Q2"]),
            ("team", vec!["East", "North", "East"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 240.0));
        let parent_bands = nested_axis_bands(&scale.config, 0).expect("parent bands");
        let tick_color = [0.2, 0.3, 0.4, 1.0];

        let boundaries = make_level_boundaries(
            &parent_bands,
            &AxisConfig {
                orientation: AxisOrientation::Bottom,
                dimensions: [240.0, 120.0],
                tick_color: Some(tick_color),
                ..Default::default()
            },
            NestedAxisLevelLabelLayout {
                label_distance: 0.0,
                boundary_outer: 17.0,
            },
            0.0,
            240.0,
        )
        .expect("boundaries");

        assert!(
            boundaries.y.equals_scalar(120.0),
            "bottom-axis separators should start at the axis line"
        );
        assert!(
            boundaries.y2.equals_scalar(137.0),
            "bottom-axis separators should extend outward through the category label band"
        );
        assert_eq!(
            boundaries.stroke.first(),
            Some(&ColorOrGradient::Color(tick_color)),
            "category separators should use the configured tick color"
        );
    }

    #[test]
    fn vertical_nested_axis_labels_rotate_around_edge_midpoint() {
        assert_eq!(angled_label_align(-90.0, false), TextAlign::Right);
        assert_eq!(angled_label_align(90.0, false), TextAlign::Left);
        assert_eq!(
            horizontal_axis_label_baseline(-90.0, false),
            TextBaseline::Middle
        );
        assert_eq!(
            horizontal_axis_label_baseline(90.0, false),
            TextBaseline::Middle
        );
        assert_eq!(
            horizontal_axis_label_baseline(-60.0, false),
            TextBaseline::Middle
        );
        assert_eq!(
            horizontal_axis_label_baseline(60.0, false),
            TextBaseline::Middle
        );
        assert_eq!(angled_label_align(-45.0, false), TextAlign::Right);
        assert_eq!(angled_label_align(45.0, false), TextAlign::Left);
        assert_eq!(
            horizontal_axis_label_baseline(-45.0, false),
            TextBaseline::Top
        );
        assert_eq!(
            horizontal_axis_label_baseline(45.0, false),
            TextBaseline::Top
        );
    }

    #[test]
    fn steep_horizontal_axis_labels_use_rotated_y_tick_optical_offset() {
        assert_eq!(horizontal_axis_label_optical_offset(-45.0, 1.2), (0.0, 0.0));

        let (dx, dy) = horizontal_axis_label_optical_offset(-90.0, 1.2);
        assert_close(dx, -1.2);
        assert_close(dy, 0.0);

        let (dx, dy) = horizontal_axis_label_optical_offset(90.0, 1.2);
        assert_close(dx, 1.2);
        assert_close(dy, 0.0);
    }

    #[test]
    fn nested_axis_layout_skips_hidden_leaf_tick_space() {
        let domain = utf8_struct(&[
            ("quarter", vec!["Q1", "Q1", "Q2"]),
            ("team", vec!["East", "North", "East"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 240.0));
        let level_bands = vec![
            nested_axis_bands(&scale.config, 0).expect("parent bands"),
            nested_axis_bands(&scale.config, 1).expect("leaf bands"),
        ];
        let configs = BTreeMap::from([(
            1,
            NestedBandAxisLevelConfig {
                visible: false,
                title: None,
                title_visible: true,
                label_angle: None,
            },
        )]);

        let text_engine = avenger_text::default_text_engine();
        let layouts = nested_axis_level_label_layouts(
            &level_bands,
            false,
            &AxisConfig {
                orientation: AxisOrientation::Bottom,
                dimensions: [240.0, 120.0],
                labels_visible: Some(true),
                title_visible: Some(true),
                ..Default::default()
            },
            Some(&configs),
            &text_engine,
        );

        assert_close(layouts[0].label_distance, TEXT_MARGIN);
        assert_close(
            layouts[0].boundary_outer,
            TEXT_MARGIN + TICK_FONT_SIZE + LEVEL_GAP * 0.5,
        );
    }

    #[test]
    fn nested_axis_title_defaults_to_struct_field_names() {
        let domain = utf8_struct(&[
            ("cylinders", vec!["4", "4", "6"]),
            ("manufacturer", vec!["ford", "toyota", "ford"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 240.0));
        let layout =
            avenger_scales::scales::nested_band::nested_band_layout(&scale.config).expect("layout");

        assert_eq!(
            nested_axis_title("", &layout, None),
            "manufacturer grouped by cylinders"
        );
    }

    #[test]
    fn nested_axis_renders_label_groups_for_visible_levels() {
        let domain = utf8_struct(&[
            ("region", vec!["east", "east", "west", "west"]),
            ("category", vec!["cars", "trucks", "cars", "trucks"]),
            ("make", vec!["ford", "volvo", "toyota", "gm"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 320.0));
        let configs = BTreeMap::from([(
            1,
            NestedBandAxisLevelConfig {
                visible: false,
                title: None,
                title_visible: true,
                label_angle: None,
            },
        )]);
        let axis = make_nested_band_axis_marks(
            &scale,
            "",
            [0.0, 0.0],
            &AxisConfig {
                orientation: AxisOrientation::Bottom,
                dimensions: [320.0, 120.0],
                labels_visible: Some(true),
                title_visible: Some(true),
                ..Default::default()
            },
            Some(&configs),
        )
        .expect("nested axis");

        let label_lengths = axis_text_lengths(&axis)
            .into_iter()
            .filter(|len| *len > 1)
            .collect::<Vec<_>>();
        assert_eq!(
            label_lengths,
            vec![4, 2],
            "expected labels for visible leaf and parent levels only"
        );
    }

    #[test]
    fn nested_axis_visibility_does_not_change_scale_geometry() {
        let domain = utf8_struct(&[
            ("group", vec!["A", "A", "B"]),
            ("member", vec!["one", "two", "one"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 240.0));
        let before = nested_axis_bands(&scale.config, 1).expect("leaf bands before");
        let configs = BTreeMap::from([(
            1,
            NestedBandAxisLevelConfig {
                visible: false,
                title: None,
                title_visible: true,
                label_angle: None,
            },
        )]);

        make_nested_band_axis_marks(
            &scale,
            "Member grouped by group",
            [0.0, 0.0],
            &AxisConfig {
                orientation: AxisOrientation::Bottom,
                dimensions: [240.0, 120.0],
                labels_visible: Some(true),
                title_visible: Some(true),
                ..Default::default()
            },
            Some(&configs),
        )
        .expect("nested axis");

        let after = nested_axis_bands(&scale.config, 1).expect("leaf bands after");
        assert_eq!(after, before);
    }

    #[test]
    fn nested_axis_title_uses_level_overrides_and_outer_title_precedence() {
        let domain = utf8_struct(&[
            ("cylinders", vec!["4", "4", "6"]),
            ("manufacturer", vec!["ford", "toyota", "ford"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 240.0));
        let layout =
            avenger_scales::scales::nested_band::nested_band_layout(&scale.config).expect("layout");
        let configs = BTreeMap::from([
            (
                0,
                NestedBandAxisLevelConfig {
                    visible: true,
                    title: Some("# Cylinders".to_string()),
                    title_visible: true,
                    label_angle: None,
                },
            ),
            (
                1,
                NestedBandAxisLevelConfig {
                    visible: true,
                    title: Some("Maker".to_string()),
                    title_visible: true,
                    label_angle: None,
                },
            ),
        ]);

        assert_eq!(
            nested_axis_title("", &layout, Some(&configs)),
            "Maker grouped by # Cylinders"
        );
        assert_eq!(
            nested_axis_title("Custom Axis", &layout, Some(&configs)),
            "Custom Axis"
        );
    }

    #[test]
    fn nested_axis_title_omits_hidden_or_title_hidden_levels() {
        let domain = utf8_struct(&[
            ("group", vec!["A", "A", "B"]),
            ("member", vec!["one", "two", "one"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 240.0));
        let layout =
            avenger_scales::scales::nested_band::nested_band_layout(&scale.config).expect("layout");
        let configs = BTreeMap::from([(
            1,
            NestedBandAxisLevelConfig {
                visible: true,
                title: Some("Member".to_string()),
                title_visible: false,
                label_angle: None,
            },
        )]);

        assert_eq!(nested_axis_title("", &layout, Some(&configs)), "group");
    }

    #[test]
    fn nested_axis_hidden_leaf_level_suppresses_leaf_ticks() {
        let domain = utf8_struct(&[
            ("group", vec!["A", "A", "B"]),
            ("member", vec!["one", "two", "one"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 240.0));
        let configs = BTreeMap::from([(
            1,
            NestedBandAxisLevelConfig {
                visible: false,
                title: None,
                title_visible: true,
                label_angle: None,
            },
        )]);
        let axis = make_nested_band_axis_marks(
            &scale,
            "Member grouped by group",
            [0.0, 0.0],
            &AxisConfig {
                orientation: AxisOrientation::Bottom,
                dimensions: [240.0, 120.0],
                labels_visible: Some(true),
                title_visible: Some(true),
                ..Default::default()
            },
            Some(&configs),
        )
        .expect("nested axis");

        let SceneMark::Group(axis_elements) = &axis.marks[0] else {
            panic!("expected axis element group");
        };
        assert_eq!(
            axis_elements.marks.len(),
            4,
            "expected axis line, parent labels, parent boundaries, and title"
        );
    }

    #[test]
    fn nested_band_axis_uses_display_labels_from_scale_domain() {
        let domain = labeled_i32_struct(&[
            (
                "year",
                vec![Some(2024), Some(2024), Some(2025)],
                vec![Some("FY 2024"), Some("FY 2024"), Some("FY 2025")],
            ),
            (
                "month",
                vec![Some(1), Some(2), Some(1)],
                vec![Some("January"), Some("February"), Some("January")],
            ),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 300.0));

        let axis = make_nested_band_axis_marks(
            &scale,
            "",
            [0.0, 0.0],
            &AxisConfig {
                orientation: AxisOrientation::Bottom,
                dimensions: [300.0, 120.0],
                labels_visible: Some(true),
                title_visible: Some(true),
                ..Default::default()
            },
            None,
        )
        .expect("nested axis");

        let labels = axis_text_strings(&axis);
        assert!(labels.iter().any(|label| label == "FY 2024"));
        assert!(labels.iter().any(|label| label == "FY 2025"));
        assert!(labels.iter().any(|label| label == "January"));
        assert!(labels.iter().any(|label| label == "February"));
        assert!(
            !labels.iter().any(|label| label == "2024" || label == "1"),
            "axis should render display labels, not raw key strings"
        );
    }

    #[test]
    fn nested_band_axis_measurement_uses_display_label_width() {
        let key_domain = i32_struct(&[
            ("portfolio", vec![Some(1), Some(1)]),
            ("division", vec![Some(1), Some(2)]),
        ]);
        let labeled_domain = labeled_i32_struct(&[
            (
                "portfolio",
                vec![Some(1), Some(1)],
                vec![Some("1"), Some("1")],
            ),
            (
                "division",
                vec![Some(1), Some(2)],
                vec![
                    Some("International growth markets"),
                    Some("North America enterprise"),
                ],
            ),
        ]);
        let key_scale = NestedBandScale::configured(key_domain, (0.0, 320.0));
        let labeled_scale = NestedBandScale::configured(labeled_domain, (0.0, 320.0));
        let axis_config = AxisConfig {
            orientation: AxisOrientation::Bottom,
            dimensions: [320.0, 120.0],
            label_angle: Some(-90.0),
            labels_visible: Some(true),
            title_visible: Some(true),
            ..Default::default()
        };
        let key_bands = vec![
            nested_axis_bands(&key_scale.config, 0).expect("key parent bands"),
            nested_axis_bands(&key_scale.config, 1).expect("key leaf bands"),
        ];
        let labeled_bands = vec![
            nested_axis_bands(&labeled_scale.config, 0).expect("labeled parent bands"),
            nested_axis_bands(&labeled_scale.config, 1).expect("labeled leaf bands"),
        ];

        let text_engine = avenger_text::default_text_engine();
        let key_layouts =
            nested_axis_level_label_layouts(&key_bands, false, &axis_config, None, &text_engine);
        let labeled_layouts = nested_axis_level_label_layouts(
            &labeled_bands,
            false,
            &axis_config,
            None,
            &text_engine,
        );

        assert_close(
            key_layouts[1].label_distance,
            labeled_layouts[1].label_distance,
        );
        assert!(
            labeled_layouts[0].label_distance > key_layouts[0].label_distance + 100.0,
            "parent labels should be pushed outward by measured display-label width"
        );
    }

    #[test]
    fn nested_band_axis_rotated_long_display_labels_do_not_overlap_parent_bands() {
        let domain = labeled_i32_struct(&[
            (
                "portfolio",
                vec![Some(1), Some(1)],
                vec![
                    Some("International growth markets"),
                    Some("International growth markets"),
                ],
            ),
            (
                "division",
                vec![Some(1), Some(2)],
                vec![Some("Analytics platform"), Some("Customer operations")],
            ),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 320.0));
        let axis = make_nested_band_axis_marks(
            &scale,
            "",
            [0.0, 0.0],
            &AxisConfig {
                orientation: AxisOrientation::Bottom,
                dimensions: [320.0, 120.0],
                label_angle: Some(-90.0),
                labels_visible: Some(true),
                title_visible: Some(true),
                ..Default::default()
            },
            None,
        )
        .expect("nested axis");
        let text_marks = axis_text_marks(&axis);
        let leaf_labels = text_marks
            .iter()
            .find(|text| text.text_iter().any(|label| label == "Analytics platform"))
            .expect("leaf labels");
        let parent_labels = text_marks
            .iter()
            .find(|text| {
                text.text_iter()
                    .any(|label| label == "International growth markets")
            })
            .expect("parent labels");

        assert_eq!(leaf_labels.angle.first(), Some(&-90.0));
        assert_eq!(leaf_labels.baseline.first(), Some(&TextBaseline::Middle));
        let leaf_y = *leaf_labels.y.first().expect("leaf y");
        let parent_y = *parent_labels.y.first().expect("parent y");
        assert!(
            parent_y - leaf_y > 100.0,
            "parent band should be placed outside the measured rotated leaf labels"
        );
    }
}
