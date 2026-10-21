//! Nested categorical axes.
//!
//! A nested band axis draws the leaf level as a band axis does, with its ticks and labels next
//! to the plot, and stacks a label row for each outer level outward from it. Separators between
//! an outer level's bands run from the axis line through that level's row. Each level can be
//! hidden, retitled, or given its own label angle or tick format.

use std::sync::Arc;

use arrow::{array::Array, compute::concat};
use avenger_common::types::{FontStyle, TextSyntaxMode};
use avenger_format::TickSpacing;
use avenger_scales::{
    error::AvengerScaleError,
    scales::{
        nested_band::{NestedBandAxisBand, NestedBandLayout},
        ConfiguredScale,
    },
};
use avenger_scenegraph::marks::text::{text_label, text_style};
use avenger_typst_label::{LabelEngine, LabelError, LabelOptions, LabelValue, TextStyle};

use super::{
    marks::{LabelRow, Labels, Separators, TEXT_MARGIN},
    opts::{AxisConfig, AxisStyle},
    tick_label_text,
};
use crate::error::AvengerGuidesError;

/// The space between adjacent levels' label rows.
const LEVEL_GAP: f32 = 8.0;

/// How one level of a nested band axis differs from the axis's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct NestedBandAxisLevelConfig {
    /// Whether the level's labels show. A hidden leaf level also hides the ticks.
    pub visible: bool,
    /// The level's part of the default axis title, in place of its field name.
    pub title: Option<String>,
    /// Whether the level takes part in the default axis title.
    pub title_visible: bool,
    /// Degrees clockwise. Without it, the leaf takes the axis's label angle and outer levels are
    /// unrotated.
    pub label_angle: Option<f32>,
    /// The pattern for the level's numeric and temporal keys, in place of the axis's
    /// `tick_format`.
    pub tick_format: Option<String>,
}

/// A level's settings when the axis doesn't override them.
static DEFAULT_LEVEL_CONFIG: NestedBandAxisLevelConfig = NestedBandAxisLevelConfig {
    visible: true,
    title: None,
    title_visible: true,
    label_angle: None,
    tick_format: None,
};

impl Default for NestedBandAxisLevelConfig {
    fn default() -> Self {
        DEFAULT_LEVEL_CONFIG.clone()
    }
}

#[derive(Clone, Copy, Debug)]
struct NestedAxisLevelLabelLayout {
    label_distance: f32,
    boundary_outer: f32,
}

/// The settings for `level`: its override, or else the defaults.
fn level_config(levels: &[NestedBandAxisLevelConfig], level: usize) -> &NestedBandAxisLevelConfig {
    levels.get(level).unwrap_or(&DEFAULT_LEVEL_CONFIG)
}

/// Each level's labels as `tick_label` markup, with `#tick` bound to each label, or else as the
/// labels themselves, and how all of them read.
fn bind_level_labels(
    level_ticks: Vec<Vec<LabelValue>>,
    tick_label: Option<&str>,
) -> Result<(Vec<Vec<String>>, TextSyntaxMode), AvengerGuidesError> {
    let lengths: Vec<usize> = level_ticks.iter().map(Vec::len).collect();
    let (labels, syntax) =
        tick_label_text(level_ticks.into_iter().flatten().collect(), tick_label)?;
    let mut labels = labels.into_iter();
    let level_labels = lengths
        .into_iter()
        .map(|length| labels.by_ref().take(length).collect())
        .collect();
    Ok((level_labels, syntax))
}

/// The bands' labels: keys that the text engine formats with the level's pattern, each on its
/// own, as band axes format their categories, and explicit labels as text.
fn band_labels(
    bands: &[NestedBandAxisBand],
    pattern: &str,
    text_engine: &LabelEngine,
) -> Result<Vec<LabelValue>, AvengerGuidesError> {
    fn key(band: &NestedBandAxisBand) -> Option<&dyn Array> {
        Some(band.path.last()?.value.as_ref()?.as_ref())
    }
    let keys: Vec<_> = bands.iter().filter_map(key).collect();
    let mut formatted = if keys.is_empty() {
        Vec::new()
    } else {
        let keys = concat(&keys).map_err(AvengerScaleError::from)?;
        text_engine.format_array(&keys, pattern, TickSpacing::Varying)?
    }
    .into_iter();
    Ok(bands
        .iter()
        .map(|band| match key(band) {
            Some(_) => formatted.next().expect("a label for each key"),
            None => LabelValue::Str(band.label.clone()),
        })
        .collect())
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
            .map(|pair| ((pair[0].end + pair[1].start) / 2.0) as f32),
    );
    positions.push(outer_end);
    positions
}

fn band_centers(bands: &[NestedBandAxisBand]) -> Vec<f32> {
    bands
        .iter()
        .map(|band| ((band.start + band.end) / 2.0) as f32)
        .collect()
}

/// How far each level's label row lies from the axis line, and how far its separators reach.
fn nested_axis_level_label_layouts(
    level_labels: &[Vec<String>],
    syntax: TextSyntaxMode,
    is_vertical: bool,
    axis_style: &AxisStyle,
    levels: &[NestedBandAxisLevelConfig],
    text_engine: &LabelEngine,
) -> Result<Vec<NestedAxisLevelLabelLayout>, LabelError> {
    let font_size = axis_style.label_font_size;
    let style = text_style(
        &axis_style.label_font_family,
        font_size,
        axis_style.label_font_weight,
        FontStyle::default(),
        [0.0, 0.0, 0.0, 1.0],
    );
    let leaf_level = level_labels.len().saturating_sub(1);
    let inner_tick_space = if level_config(levels, leaf_level).visible {
        axis_style.tick_length
    } else {
        0.0
    };
    let initial_label_distance = inner_tick_space + TEXT_MARGIN;
    let mut layouts = vec![
        NestedAxisLevelLabelLayout {
            label_distance: initial_label_distance,
            boundary_outer: initial_label_distance + font_size + LEVEL_GAP * 0.5,
        };
        level_labels.len()
    ];
    let mut label_distance = initial_label_distance;

    for level in (0..level_labels.len()).rev() {
        let level_config = level_config(levels, level);
        if !level_config.visible {
            continue;
        }
        let angle = level_label_angle(level, leaf_level, axis_style, level_config);
        let extent = level_label_cross_extent(
            &level_labels[level],
            &style,
            syntax,
            angle,
            is_vertical,
            text_engine,
        )?;
        layouts[level] = NestedAxisLevelLabelLayout {
            label_distance,
            boundary_outer: label_distance + extent + LEVEL_GAP * 0.5,
        };
        label_distance += extent + LEVEL_GAP;
    }

    Ok(layouts)
}

fn level_label_angle(
    level: usize,
    leaf_level: usize,
    style: &AxisStyle,
    level_config: &NestedBandAxisLevelConfig,
) -> f32 {
    let axis_angle = if level == leaf_level {
        style.label_angle
    } else {
        0.0
    };
    level_config.label_angle.unwrap_or(axis_angle)
}

fn level_label_cross_extent(
    labels: &[String],
    style: &TextStyle,
    syntax: TextSyntaxMode,
    angle: f32,
    is_vertical: bool,
    text_engine: &LabelEngine,
) -> Result<f32, LabelError> {
    let (mut max_width, mut max_height) = (0.0_f32, 0.0_f32);
    for label in labels {
        let bounds = text_engine.bounds(&text_label(
            label,
            syntax,
            LabelOptions {
                text: style.clone(),
                ..Default::default()
            },
        ))?;
        max_width = max_width.max(bounds.width);
        max_height = max_height.max(bounds.height);
    }
    if is_vertical {
        return Ok(max_width);
    }

    let radians = angle.to_radians().abs();
    Ok(if radians == 0.0 {
        max_height
    } else {
        max_width * radians.sin().abs() + max_height * radians.cos().abs()
    })
}

/// The axis title: `explicit_title`, or else the visible levels' titles joined outermost first
/// with " / ". A level's title is its override, or else its field name, and a level with neither
/// is skipped.
fn nested_axis_title(
    explicit_title: &str,
    layout: &NestedBandLayout,
    levels: &[NestedBandAxisLevelConfig],
) -> String {
    if !explicit_title.is_empty() {
        return explicit_title.to_string();
    }

    (0..=layout.leaf_level())
        .filter_map(|level| {
            let level_config = level_config(levels, level);
            if !level_config.visible || !level_config.title_visible {
                return None;
            }
            let nonempty = |title: &&str| !title.is_empty();
            level_config.title.as_deref().filter(nonempty).or_else(|| {
                layout
                    .field_names()
                    .get(level)
                    .map(String::as_str)
                    .filter(nonempty)
            })
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

/// A nested band axis's labels and their rows, prepared once.
#[derive(Debug)]
pub(crate) struct NestedLevels {
    leaf_level: usize,
    leaf_visible: bool,
    /// The visible levels, leaf first, when the labels show.
    rows: Vec<NestedRow>,
    /// The axis title: the explicit one, or the levels' titles.
    pub(crate) title: String,
}

#[derive(Debug)]
struct NestedRow {
    level: usize,
    labels: Arc<Labels>,
    /// The row's distance from the domain line, and how far its separators reach from it.
    layout: NestedAxisLevelLabelLayout,
    angle: f32,
}

/// One level's row at a length, with its separators for an outer level.
pub(crate) struct NestedLevelRow {
    pub(crate) row: LabelRow,
    pub(crate) separators: Option<Separators>,
}

impl NestedLevels {
    /// Formats, binds and measures every level's labels. `levels` overrides the axis's settings
    /// for each level, outermost first, and levels past its end keep the defaults.
    pub(crate) fn new(
        scale: &ConfiguredScale,
        title: &str,
        config: &AxisConfig,
        levels: &[NestedBandAxisLevelConfig],
        engine: &LabelEngine,
    ) -> Result<Self, AvengerGuidesError> {
        let layout = NestedBandLayout::from_config(&scale.config)?;
        let leaf_level = layout.leaf_level();
        let level_bands = (0..=leaf_level)
            .map(|level| layout.axis_bands(level))
            .collect::<Result<Vec<_>, _>>()?;
        let level_ticks = level_bands
            .iter()
            .enumerate()
            .map(|(level, bands)| {
                let pattern = level_config(levels, level)
                    .tick_format
                    .as_deref()
                    .unwrap_or(&config.tick_format);
                band_labels(bands, pattern, engine)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (level_labels, syntax) = bind_level_labels(level_ticks, config.tick_label.as_deref())?;
        let style = &config.style;
        let layouts = nested_axis_level_label_layouts(
            &level_labels,
            syntax,
            config.side.is_vertical(),
            style,
            levels,
            engine,
        )?;
        let mut rows = Vec::new();
        if style.labels_visible {
            for level in (0..=leaf_level).rev() {
                let level_config = level_config(levels, level);
                if !level_config.visible {
                    continue;
                }
                rows.push(NestedRow {
                    level,
                    labels: Arc::new(Labels::measure(
                        level_labels[level].clone(),
                        syntax,
                        style,
                        engine,
                    )),
                    layout: layouts[level],
                    angle: level_label_angle(level, leaf_level, style, level_config),
                });
            }
        }
        Ok(Self {
            leaf_level,
            leaf_visible: level_config(levels, leaf_level).visible,
            rows,
            title: nested_axis_title(title, &layout, levels),
        })
    }

    /// Whether the axis draws its leaf ticks.
    pub(crate) fn ticks_visible(&self) -> bool {
        self.leaf_visible
    }

    /// The leaf ticks and each visible level's row on a scale ranged to `range`, in the labels
    /// part's frame, which begins `ticks_thickness` past the domain line.
    pub(crate) fn at(
        &self,
        scale: &ConfiguredScale,
        range: (f32, f32),
        ticks_thickness: f32,
    ) -> (Vec<f32>, Vec<NestedLevelRow>) {
        // The constructor laid out the same configuration, so this succeeds.
        let Ok(layout) = NestedBandLayout::from_config(&scale.config) else {
            return (Vec::new(), Vec::new());
        };
        let bands = |level| layout.axis_bands(level).unwrap_or_default();
        let ticks = band_centers(bands(self.leaf_level));
        let rows = self
            .rows
            .iter()
            .map(|row| {
                let level_bands = bands(row.level);
                let separators = (row.level < self.leaf_level)
                    .then(|| level_boundary_positions(level_bands, range.0, range.1))
                    .filter(|positions| !positions.is_empty())
                    .map(|positions| Separators {
                        positions,
                        inner: -ticks_thickness,
                        outer: row.layout.boundary_outer - ticks_thickness,
                    });
                NestedLevelRow {
                    row: LabelRow {
                        labels: row.labels.clone(),
                        positions: band_centers(level_bands),
                        distance: row.layout.label_distance - ticks_thickness,
                        angle: row.angle,
                    },
                    separators,
                }
            })
            .collect();
        (ticks, rows)
    }
}

#[cfg(test)]
mod tests {
    use avenger_common::types::{TextBaseline, TextSyntaxMode};
    use std::sync::Arc;

    use arrow::array::{create_array, ArrayRef, Date32Array, Int32Array, StringArray, StructArray};
    use avenger_color::ColorOrGradient;
    use avenger_scales::scales::{band::BandScale, nested_band::NestedBandScale};
    use avenger_scenegraph::marks::mark::SceneMark;

    use avenger_scenegraph::marks::group::SceneGroup;
    use avenger_typst_label::LabelEngine;

    use crate::axis::opts::{AxisConfig, AxisStyle};
    use crate::axis::Axis;
    use crate::error::AvengerGuidesError;
    use crate::GuideSide;

    use super::{
        level_boundary_positions, nested_axis_level_label_layouts, nested_axis_title,
        ConfiguredScale, NestedBandAxisBand, NestedBandAxisLevelConfig, NestedBandLayout,
        LEVEL_GAP, TEXT_MARGIN,
    };

    /// The scale's length along the axis, which the tests' plots are square on.
    fn plot_size(scale: &ConfiguredScale) -> [f32; 2] {
        let (start, end) = scale.numeric_interval_range().unwrap();
        let length = (end - start).abs() as f32;
        [length, length]
    }

    /// A nested band axis's parts, stacked in the frame of a square plot as long as the scale.
    fn nested_axis_marks(
        scale: &ConfiguredScale,
        title: &str,
        config: &AxisConfig,
        levels: &[NestedBandAxisLevelConfig],
        engine: &LabelEngine,
    ) -> Result<SceneGroup, AvengerGuidesError> {
        Axis::nested_band(scale, title, levels, config, engine)
            .map(|axis| axis.marks(plot_size(scale)))
    }

    /// Every mark of the axis's parts, in drawing order.
    fn part_marks(axis: &SceneGroup) -> Vec<&SceneMark> {
        axis.marks
            .iter()
            .flat_map(|part| match part {
                SceneMark::Group(part) => part.marks.iter().collect::<Vec<_>>(),
                _ => panic!("expected a part group"),
            })
            .collect()
    }

    fn utf8_struct(columns: &[(&str, Vec<&str>)]) -> ArrayRef {
        let columns: Vec<(&str, ArrayRef)> = columns
            .iter()
            .map(|(name, values)| {
                (
                    *name,
                    Arc::new(StringArray::from(values.clone())) as ArrayRef,
                )
            })
            .collect();
        Arc::new(StructArray::try_from(columns).unwrap())
    }

    fn i32_struct(columns: &[(&str, Vec<Option<i32>>)]) -> ArrayRef {
        let columns: Vec<(&str, ArrayRef)> = columns
            .iter()
            .map(|(name, values)| {
                (
                    *name,
                    Arc::new(Int32Array::from(values.clone())) as ArrayRef,
                )
            })
            .collect();
        Arc::new(StructArray::try_from(columns).unwrap())
    }

    #[allow(
        clippy::type_complexity,
        reason = "The tuple describes the input or output of this test fixture."
    )]
    fn labeled_i32_struct(columns: &[(&str, Vec<Option<i32>>, Vec<Option<&str>>)]) -> ArrayRef {
        let columns: Vec<(&str, ArrayRef)> = columns
            .iter()
            .map(|(name, keys, labels)| {
                let component: Vec<(&str, ArrayRef)> = vec![
                    ("key", Arc::new(Int32Array::from(keys.clone()))),
                    ("label", Arc::new(StringArray::from(labels.clone()))),
                ];
                (
                    *name,
                    Arc::new(StructArray::try_from(component).unwrap()) as ArrayRef,
                )
            })
            .collect();
        Arc::new(StructArray::try_from(columns).unwrap())
    }

    fn axis_text_marks(axis: &SceneGroup) -> Vec<&avenger_scenegraph::marks::text::SceneTextMark> {
        part_marks(axis)
            .into_iter()
            .filter_map(|mark| match mark {
                SceneMark::Text(text) => Some(text.as_ref()),
                _ => None,
            })
            .collect()
    }

    fn axis_text_strings(axis: &SceneGroup) -> Vec<String> {
        axis_text_marks(axis)
            .into_iter()
            .flat_map(|text| text.text_iter().cloned().collect::<Vec<_>>())
            .collect()
    }

    fn axis_text_lengths(axis: &SceneGroup) -> Vec<u32> {
        part_marks(axis)
            .into_iter()
            .filter_map(|mark| match mark {
                SceneMark::Text(text) => Some(text.len),
                _ => None,
            })
            .collect()
    }

    fn bands(scale: &ConfiguredScale, level: usize) -> Vec<NestedBandAxisBand> {
        NestedBandLayout::from_config(&scale.config)
            .expect("layout")
            .axis_bands(level)
            .expect("level bands")
            .to_vec()
    }

    /// The scale's labels for a level's bands.
    fn labels(scale: &ConfiguredScale, level: usize) -> Vec<String> {
        bands(scale, level)
            .into_iter()
            .map(|band| band.label)
            .collect()
    }

    fn hidden() -> NestedBandAxisLevelConfig {
        NestedBandAxisLevelConfig {
            visible: false,
            ..Default::default()
        }
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
            .with_option("nest_scopes", "shared")
            .with_option("padding_inner", vec![0.3, 0.1]);
        let axis = nested_axis_marks(
            &scale,
            "Manufacturer grouped by cylinders",
            &AxisConfig {
                side: GuideSide::Bottom,
                tick_format: ",".into(),
                tick_label: None,
                style: AxisStyle {
                    labels_visible: true,
                    title_visible: true,
                    ..Default::default()
                },
            },
            &[],
            &crate::test_engine(),
        )
        .expect("nested axis");

        assert_eq!(
            axis.marks.len(),
            3,
            "expected the ticks, labels and title parts"
        );
        assert!(
            part_marks(&axis).len() >= 5,
            "expected axis line, ticks, two label levels, boundary, and title"
        );
    }

    #[test]
    fn nested_axis_labels_bind_tick_label_at_every_level() {
        let domain = utf8_struct(&[
            ("cylinders", vec!["4", "4", "6"]),
            ("manufacturer", vec!["ford", "toyota", "ford"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 240.0));
        let axis = |tick_label: Option<&str>| {
            nested_axis_marks(
                &scale,
                "",
                &AxisConfig {
                    side: GuideSide::Left,
                    tick_format: ",".into(),
                    tick_label: tick_label.map(str::to_string),
                    style: AxisStyle::default(),
                },
                &[],
                &crate::test_engine(),
            )
            .expect("nested axis")
        };
        let plain = axis(None);
        let bound = axis(Some("*#tick* cyl"));
        // The leaf and parent label levels come before the title, which stays plain.
        let marks = axis_text_marks(&bound);
        let syntaxes: Vec<_> = marks.iter().map(|mark| mark.text_syntax).collect();
        assert_eq!(
            syntaxes,
            [
                TextSyntaxMode::TypstMarkup,
                TextSyntaxMode::TypstMarkup,
                TextSyntaxMode::Plain
            ]
        );
        let mut labels: Vec<String> = marks[..2]
            .iter()
            .flat_map(|mark| mark.text_iter().cloned().collect::<Vec<_>>())
            .collect();
        labels.sort();
        assert_eq!(
            labels,
            [
                r#"*#("4")* cyl"#,
                r#"*#("6")* cyl"#,
                r#"*#("ford")* cyl"#,
                r#"*#("ford")* cyl"#,
                r#"*#("toyota")* cyl"#,
            ]
        );
        // The parent level sits past the longer bound leaf labels.
        let parent_x = |axis| {
            let marks = axis_text_marks(axis);
            marks[1].x.as_vec(1, None)[0]
        };
        assert!(parent_x(&bound) < parent_x(&plain));
    }

    #[test]
    fn nested_axis_level_boundaries_center_internal_group_gaps() {
        let domain = utf8_struct(&[
            ("cylinders", vec!["4", "4", "6"]),
            ("manufacturer", vec!["ford", "toyota", "ford"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 370.0))
            .with_option("padding_inner", vec![0.3, 0.0])
            .with_option("padding_outer", vec![0.2, 0.0]);
        let parent_bands = bands(&scale, 0);

        assert_close(parent_bands[0].start as f32, 20.0);
        assert_close(parent_bands[0].end as f32, 220.0);
        assert_close(parent_bands[1].start as f32, 250.0);
        assert_close(parent_bands[1].end as f32, 350.0);

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
        let tick_color = [0.2, 0.3, 0.4, 1.0];
        let axis = nested_axis_marks(
            &scale,
            "",
            &AxisConfig {
                side: GuideSide::Bottom,
                tick_format: ",".into(),
                tick_label: None,
                style: AxisStyle {
                    tick_color,
                    title_visible: false,
                    ..Default::default()
                },
            },
            &[],
            &crate::test_engine(),
        )
        .expect("nested axis");
        // The labels part begins at the ticks' ends, below the axis line at the plot's bottom.
        let SceneMark::Group(labels) = &axis.marks[1] else {
            panic!("expected the labels part");
        };
        let boundaries = labels
            .marks
            .iter()
            .find_map(|mark| match mark {
                SceneMark::Rule(rule) => Some(rule),
                _ => None,
            })
            .expect("boundaries");
        let parent = axis_text_marks(&axis)[1];
        let axis_line = 240.0;
        assert!(
            boundaries.y.equals_scalar(axis_line - labels.origin[1]),
            "bottom-axis separators should start at the axis line"
        );
        let outer = boundaries.y2.as_vec(1, None)[0];
        assert!(
            outer > parent.y.as_vec(1, None)[0],
            "bottom-axis separators should extend outward through the category label band"
        );
        assert_eq!(
            boundaries.stroke.first(),
            Some(&ColorOrGradient::Color(tick_color)),
            "category separators should use the configured tick color"
        );
    }

    #[test]
    fn nested_axis_layout_skips_hidden_leaf_tick_space() {
        let domain = utf8_struct(&[
            ("quarter", vec!["Q1", "Q1", "Q2"]),
            ("team", vec!["East", "North", "East"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 240.0));
        let level_labels = vec![labels(&scale, 0), labels(&scale, 1)];
        let configs = [NestedBandAxisLevelConfig::default(), hidden()];

        let text_engine = crate::test_engine();
        let layouts = nested_axis_level_label_layouts(
            &level_labels,
            TextSyntaxMode::Plain,
            false,
            &AxisStyle::default(),
            &configs,
            &text_engine,
        )
        .unwrap();

        assert_close(layouts[0].label_distance, TEXT_MARGIN);
        assert_close(
            layouts[0].boundary_outer,
            TEXT_MARGIN + AxisStyle::default().label_font_size + LEVEL_GAP * 0.5,
        );
    }

    #[test]
    fn nested_axis_title_defaults_to_struct_field_names() {
        let domain = utf8_struct(&[
            ("cylinders", vec!["4", "4", "6"]),
            ("manufacturer", vec!["ford", "toyota", "ford"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 240.0));
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");

        assert_eq!(
            nested_axis_title("", &layout, &[]),
            "cylinders / manufacturer"
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
        let configs = [NestedBandAxisLevelConfig::default(), hidden()];
        let axis = nested_axis_marks(
            &scale,
            "",
            &AxisConfig {
                side: GuideSide::Bottom,
                tick_format: ",".into(),
                tick_label: None,
                style: AxisStyle {
                    labels_visible: true,
                    title_visible: true,
                    ..Default::default()
                },
            },
            &configs,
            &crate::test_engine(),
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
        let before = bands(&scale, 1);
        let configs = [NestedBandAxisLevelConfig::default(), hidden()];

        nested_axis_marks(
            &scale,
            "Member grouped by group",
            &AxisConfig {
                side: GuideSide::Bottom,
                tick_format: ",".into(),
                tick_label: None,
                style: AxisStyle {
                    labels_visible: true,
                    title_visible: true,
                    ..Default::default()
                },
            },
            &configs,
            &crate::test_engine(),
        )
        .expect("nested axis");

        let after = bands(&scale, 1);
        assert_eq!(after, before);
    }

    #[test]
    fn nested_axis_title_uses_level_overrides_and_outer_title_precedence() {
        let domain = utf8_struct(&[
            ("cylinders", vec!["4", "4", "6"]),
            ("manufacturer", vec!["ford", "toyota", "ford"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 240.0));
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");
        let configs = [
            NestedBandAxisLevelConfig {
                title: Some("# Cylinders".to_string()),
                ..Default::default()
            },
            NestedBandAxisLevelConfig {
                title: Some("Maker".to_string()),
                ..Default::default()
            },
        ];

        assert_eq!(
            nested_axis_title("", &layout, &configs),
            "# Cylinders / Maker"
        );
        assert_eq!(
            nested_axis_title("Custom Axis", &layout, &configs),
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
        let layout = NestedBandLayout::from_config(&scale.config).expect("layout");
        let configs = [
            NestedBandAxisLevelConfig::default(),
            NestedBandAxisLevelConfig {
                title: Some("Member".to_string()),
                title_visible: false,
                ..Default::default()
            },
        ];

        assert_eq!(nested_axis_title("", &layout, &configs), "group");
    }

    #[test]
    fn nested_axis_hidden_leaf_level_suppresses_leaf_ticks() {
        let domain = utf8_struct(&[
            ("group", vec!["A", "A", "B"]),
            ("member", vec!["one", "two", "one"]),
        ]);
        let scale = NestedBandScale::configured(domain, (0.0, 240.0));
        let configs = [NestedBandAxisLevelConfig::default(), hidden()];
        let axis = nested_axis_marks(
            &scale,
            "Member grouped by group",
            &AxisConfig {
                side: GuideSide::Bottom,
                tick_format: ",".into(),
                tick_label: None,
                style: AxisStyle {
                    labels_visible: true,
                    title_visible: true,
                    ..Default::default()
                },
            },
            &configs,
            &crate::test_engine(),
        )
        .expect("nested axis");

        assert_eq!(
            part_marks(&axis).len(),
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

        let axis = nested_axis_marks(
            &scale,
            "",
            &AxisConfig {
                side: GuideSide::Bottom,
                tick_format: ",".into(),
                tick_label: None,
                style: AxisStyle {
                    labels_visible: true,
                    title_visible: true,
                    ..Default::default()
                },
            },
            &[],
            &crate::test_engine(),
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

    /// The text engine formats each level's keys by their kind, with the level's pattern or else
    /// the axis's, and labels of every level read alike.
    #[test]
    fn nested_axis_formats_each_level_by_kind() {
        let columns: Vec<(&str, ArrayRef)> = vec![
            ("day", Arc::new(Date32Array::from(vec![19723, 19723]))),
            ("units", create_array!(Int32, [1000, 2500])),
        ];
        let domain: ArrayRef = Arc::new(StructArray::try_from(columns).unwrap());
        let scale = NestedBandScale::configured(domain, (0.0, 240.0));
        let dates = NestedBandAxisLevelConfig {
            tick_format: Some("%b %d".into()),
            ..Default::default()
        };
        let exponent = [r#"$#"1.0" times 10^(3)$"#, r#"$#"2.5" times 10^(3)$"#];
        for (tick_format, levels, expected, syntax) in [
            (
                "",
                vec![],
                ["2024", "1,000", "2,500"],
                TextSyntaxMode::Plain,
            ),
            (
                ".2f",
                vec![dates.clone()],
                ["Jan 01", "1000.00", "2500.00"],
                TextSyntaxMode::Plain,
            ),
            (
                ".1e",
                vec![dates],
                ["Jan 01", exponent[0], exponent[1]],
                TextSyntaxMode::TypstMarkup,
            ),
        ] {
            let axis = nested_axis_marks(
                &scale,
                "",
                &AxisConfig {
                    side: GuideSide::Bottom,
                    tick_format: tick_format.into(),
                    tick_label: None,
                    style: AxisStyle {
                        title_visible: false,
                        ..Default::default()
                    },
                },
                &levels,
                &crate::test_engine(),
            )
            .expect("nested axis");
            // Rows are leaf first.
            let labels = axis_text_strings(&axis);
            assert_eq!(
                labels[..3],
                [expected[1], expected[2], expected[0]],
                "{tick_format}"
            );
            for text in axis_text_marks(&axis) {
                assert_eq!(text.text_syntax, syntax, "{tick_format}");
            }
        }
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
            side: GuideSide::Bottom,
            tick_format: ",".into(),
            tick_label: None,
            style: AxisStyle {
                label_angle: -90.0,
                labels_visible: true,
                title_visible: true,
                ..Default::default()
            },
        };
        let key_labels = vec![labels(&key_scale, 0), labels(&key_scale, 1)];
        let labeled_labels = vec![labels(&labeled_scale, 0), labels(&labeled_scale, 1)];

        let text_engine = crate::test_engine();
        let key_layouts = nested_axis_level_label_layouts(
            &key_labels,
            TextSyntaxMode::Plain,
            false,
            &axis_config.style,
            &[],
            &text_engine,
        )
        .unwrap();
        let labeled_layouts = nested_axis_level_label_layouts(
            &labeled_labels,
            TextSyntaxMode::Plain,
            false,
            &axis_config.style,
            &[],
            &text_engine,
        )
        .unwrap();

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
        let axis = nested_axis_marks(
            &scale,
            "",
            &AxisConfig {
                side: GuideSide::Bottom,
                tick_format: ",".into(),
                tick_label: None,
                style: AxisStyle {
                    label_angle: -90.0,
                    labels_visible: true,
                    title_visible: true,
                    ..Default::default()
                },
            },
            &[],
            &crate::test_engine(),
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

    #[test]
    fn nested_axis_leaf_labels_sit_where_a_band_axis_puts_them() {
        let keys = vec!["alpha", "beta", "gamma"];
        let flat = Arc::new(StringArray::from(keys.clone())) as ArrayRef;
        let nested = utf8_struct(&[("key", keys)]);
        let engine = crate::test_engine();
        for (orientation, angle) in [(GuideSide::Left, 0.0), (GuideSide::Bottom, 60.0)] {
            let config = AxisConfig {
                side: orientation,
                tick_format: ",".into(),
                tick_label: None,
                style: AxisStyle {
                    label_angle: angle,
                    title_visible: false,
                    ..Default::default()
                },
            };
            let band_axis = Axis::band(
                &BandScale::configured(flat.clone(), (0.0, 240.0)),
                "",
                &config,
                &engine,
            )
            .expect("band axis")
            .marks([240.0, 240.0]);
            let nested_axis = nested_axis_marks(
                &NestedBandScale::configured(nested.clone(), (0.0, 240.0)),
                "",
                &config,
                &[],
                &engine,
            )
            .expect("nested axis");
            let expected = axis_text_marks(&band_axis)[0];
            let actual = axis_text_marks(&nested_axis)[0];
            assert_eq!(
                actual.x.as_vec(3, None),
                expected.x.as_vec(3, None),
                "{orientation:?}"
            );
            assert_eq!(
                actual.y.as_vec(3, None),
                expected.y.as_vec(3, None),
                "{orientation:?}"
            );
            assert_eq!(actual.align, expected.align, "{orientation:?}");
            assert_eq!(actual.baseline, expected.baseline, "{orientation:?}");
            assert_eq!(actual.angle, expected.angle, "{orientation:?}");
        }
    }
}
