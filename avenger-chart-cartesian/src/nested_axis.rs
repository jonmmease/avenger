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
const TEXT_WIDTH_FACTOR: f32 = 0.56;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NestedAxisLevelGuideConfig {
    pub(crate) visible: bool,
    pub(crate) title: Option<String>,
    pub(crate) title_visible: bool,
    pub(crate) label_angle: Option<f32>,
}

#[derive(Clone, Copy, Debug)]
struct NestedAxisLevelLabelLayout {
    label_distance: f32,
    boundary_inner: f32,
    boundary_outer: f32,
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
    let label_layouts =
        nested_axis_level_label_layouts(&level_bands, is_vertical, config, level_configs);
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
                axis_group
                    .marks
                    .push(make_level_boundaries(bands, config, label_layouts[level])?.into());
            }
        }
    }

    let title = nested_axis_title(title, &layout, level_configs);
    if config.title_visible.unwrap_or(true) && !title.is_empty() {
        let envelope = axis_group.bounding_box();
        axis_group
            .marks
            .push(make_title(&title, scale, envelope.lower(), envelope.upper(), config)?.into());
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

fn level_visible(
    level_configs: Option<&BTreeMap<usize, NestedAxisLevelGuideConfig>>,
    level: usize,
) -> bool {
    !level_configs
        .and_then(|configs| configs.get(&level))
        .is_some_and(|config| !config.visible)
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
    leaf_level: usize,
    config: &AxisConfig,
    level_config: Option<&NestedAxisLevelGuideConfig>,
    layout: NestedAxisLevelLabelLayout,
) -> Result<SceneTextMark, AvengerChartError> {
    let font_size = config.label_font_size.unwrap_or(TICK_FONT_SIZE);
    let font_adjustment = font_size * 0.10;
    let centers = band_centers(bands);
    let labels = bands
        .iter()
        .map(|band| band.label.clone())
        .collect::<Vec<_>>();
    let level = bands.first().map(|band| band.level).unwrap_or(0);
    let leaf_angle = level_label_angle(level, leaf_level, config, level_config);

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
            ScalarOrArray::new_array(centers),
            ScalarOrArray::new_scalar(-layout.label_distance),
            angled_label_align(leaf_angle, true),
            TextBaseline::Bottom,
            leaf_angle,
        ),
        AxisOrientation::Bottom => (
            ScalarOrArray::new_array(centers),
            ScalarOrArray::new_scalar(config.dimensions[1] + layout.label_distance),
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
    config: &AxisConfig,
    layout: NestedAxisLevelLabelLayout,
) -> Result<SceneRuleMark, AvengerChartError> {
    let Some(first) = bands.first() else {
        return Ok(SceneRuleMark::default());
    };
    let mut positions = Vec::with_capacity(bands.len() + 1);
    positions.push(first.start);
    positions.extend(bands.iter().map(|band| band.end));

    let (x, x2, y, y2) = match config.orientation {
        AxisOrientation::Left => (
            ScalarOrArray::new_scalar(-layout.boundary_inner),
            ScalarOrArray::new_scalar(-layout.boundary_outer),
            ScalarOrArray::new_array(positions.clone()),
            ScalarOrArray::new_array(positions),
        ),
        AxisOrientation::Right => (
            ScalarOrArray::new_scalar(config.dimensions[0] + layout.boundary_inner),
            ScalarOrArray::new_scalar(config.dimensions[0] + layout.boundary_outer),
            ScalarOrArray::new_array(positions.clone()),
            ScalarOrArray::new_array(positions),
        ),
        AxisOrientation::Top => (
            ScalarOrArray::new_array(positions.clone()),
            ScalarOrArray::new_array(positions),
            ScalarOrArray::new_scalar(-layout.boundary_inner),
            ScalarOrArray::new_scalar(-layout.boundary_outer),
        ),
        AxisOrientation::Bottom => (
            ScalarOrArray::new_array(positions.clone()),
            ScalarOrArray::new_array(positions),
            ScalarOrArray::new_scalar(config.dimensions[1] + layout.boundary_inner),
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

fn nested_axis_level_label_layouts(
    level_bands: &[Vec<NestedBandAxisBand>],
    is_vertical: bool,
    config: &AxisConfig,
    level_configs: Option<&BTreeMap<usize, NestedAxisLevelGuideConfig>>,
) -> Vec<NestedAxisLevelLabelLayout> {
    let font_size = config.label_font_size.unwrap_or(TICK_FONT_SIZE);
    let tick_len = config.tick_length.unwrap_or(TICK_LENGTH);
    let leaf_level = level_bands.len().saturating_sub(1);
    let mut layouts = vec![
        NestedAxisLevelLabelLayout {
            label_distance: tick_len + TEXT_MARGIN,
            boundary_inner: tick_len,
            boundary_outer: tick_len + font_size + LEVEL_GAP * 0.5,
        };
        level_bands.len()
    ];
    let mut label_distance = tick_len + TEXT_MARGIN;

    for level in (0..level_bands.len()).rev() {
        let level_config = level_configs.and_then(|configs| configs.get(&level));
        let angle = level_label_angle(level, leaf_level, config, level_config);
        let extent = level_label_cross_extent(&level_bands[level], font_size, angle, is_vertical);
        layouts[level] = NestedAxisLevelLabelLayout {
            label_distance,
            boundary_inner: (label_distance - TEXT_MARGIN).max(tick_len),
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
    level_config: Option<&NestedAxisLevelGuideConfig>,
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
    font_size: f32,
    angle: f32,
    is_vertical: bool,
) -> f32 {
    let label_width = bands
        .iter()
        .map(|band| band.label.chars().count() as f32 * font_size * TEXT_WIDTH_FACTOR)
        .fold(font_size, f32::max);
    if is_vertical {
        return label_width;
    }

    let radians = angle.to_radians().abs();
    if radians == 0.0 {
        font_size
    } else {
        (label_width * radians.sin().abs() + font_size * radians.cos().abs()).max(font_size)
    }
}

fn nested_axis_title(
    explicit_title: &str,
    layout: &avenger_scales::scales::nested_band::NestedBandLayout,
    level_configs: Option<&BTreeMap<usize, NestedAxisLevelGuideConfig>>,
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

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use avenger_guides::axis::opts::{AxisConfig, AxisOrientation};
    use avenger_scales::scales::nested_band::{NestedBandScale, nested_axis_bands};
    use avenger_scenegraph::marks::mark::SceneMark;
    use datafusion::arrow::{
        array::{ArrayRef, StringArray, StructArray},
        datatypes::Field,
    };

    use super::{NestedAxisLevelGuideConfig, make_nested_axis_marks, nested_axis_title};

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
            NestedAxisLevelGuideConfig {
                visible: false,
                title: None,
                title_visible: true,
                label_angle: None,
            },
        )]);
        let axis = make_nested_axis_marks(
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
            NestedAxisLevelGuideConfig {
                visible: false,
                title: None,
                title_visible: true,
                label_angle: None,
            },
        )]);

        make_nested_axis_marks(
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
                NestedAxisLevelGuideConfig {
                    visible: true,
                    title: Some("# Cylinders".to_string()),
                    title_visible: true,
                    label_angle: None,
                },
            ),
            (
                1,
                NestedAxisLevelGuideConfig {
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
            NestedAxisLevelGuideConfig {
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
            NestedAxisLevelGuideConfig {
                visible: false,
                title: None,
                title_visible: true,
                label_angle: None,
            },
        )]);
        let axis = make_nested_axis_marks(
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
}
