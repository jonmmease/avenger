use avenger_color::{ColorOrGradient, Gradient, LinearGradient};
use avenger_common::types::TextSyntaxMode;
use avenger_geometry::{marks::TextGeometryUtils, rtree::EnvelopeUtils};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{
    group::SceneGroup, mark::SceneMark, rect::SceneRectMark, text::text_label,
};
use avenger_typst_label::{LabelEngine, LabelOptions};

use super::{make_background, LegendStyle};
use crate::{
    axis::{
        continuous::{axis_tick_labels, make_continuous_axis_marks},
        opts::{AxisConfig, AxisOrientation, AxisStyle},
    },
    error::AvengerGuidesError,
};

/// Ticks along horizontal colorbars.
const HORIZONTAL_TICK_COUNT: f64 = 10.0;
/// The least space left between an end label and the background's edge.
const LABEL_EDGE_MARGIN: f32 = 2.0;
/// The shortest gradient, however little room the labels leave.
const MIN_GRADIENT_LENGTH: f32 = 10.0;
/// The longest colorbar when its style doesn't set a length.
const MAX_DEFAULT_LENGTH: f32 = 200.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorbarOrientation {
    Top,
    Bottom,
    Left,
    Right,
}

impl ColorbarOrientation {
    fn is_horizontal(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }

    fn axis(self) -> AxisOrientation {
        match self {
            Self::Top => AxisOrientation::Top,
            Self::Bottom => AxisOrientation::Bottom,
            Self::Left => AxisOrientation::Left,
            Self::Right => AxisOrientation::Right,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ColorbarConfig {
    pub orientation: ColorbarOrientation,
    /// Dimensions of the plot area (width, height) that the colorbar is attached to
    pub dimensions: [f32; 2],
    /// The pattern for the ticks, as for [`AxisConfig::tick_format`].
    pub tick_format: String,
    /// Typst markup for each tick label, where `#tick` is the formatted tick.
    pub tick_label: Option<String>,
    pub style: ColorbarStyle,
}

/// Colorbar sizing and appearance. `ColorbarStyle::default()` is the default look.
#[derive(Debug, Clone)]
pub struct ColorbarStyle {
    /// The gradient's thickness, across the colorbar.
    pub thickness: f32,
    /// The colorbar's length, including its background padding. Without it, the colorbar is as
    /// long as the plot's side, up to 200 pixels.
    pub length: Option<f32>,
    /// Space between the gradient and its axis.
    pub axis_gap: f32,
    pub domain_color: [f32; 4],
    pub tick_color: [f32; 4],
    /// Background, title, and labels.
    pub legend: LegendStyle,
}

impl Default for ColorbarStyle {
    fn default() -> Self {
        Self {
            thickness: 15.0,
            length: None,
            axis_gap: 5.0,
            domain_color: [0.0, 0.0, 0.0, 1.0],
            tick_color: [0.0, 0.0, 0.0, 1.0],
            legend: LegendStyle::default(),
        }
    }
}

/// A rendered colorbar with the gradient that interactions can target.
#[derive(Debug, Clone)]
pub struct ColorbarOutput {
    pub group: SceneGroup,
    /// The scene path of the gradient rect, which takes the colorbar's hits, relative to `group`.
    pub gradient_path: Vec<usize>,
    /// The gradient's [x, y, width, height] relative to `group`.
    pub gradient_bounds: [f32; 4],
}

/// A colorbar at `origin`: a gradient with an axis on the orientation's side, inside its
/// background.
pub fn make_colorbar_legend(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &ColorbarConfig,
    text_engine: &LabelEngine,
) -> Result<ColorbarOutput, AvengerGuidesError> {
    let orientation = config.orientation;
    let horizontal = orientation.is_horizontal();
    let style = &config.style;
    let padding = style.legend.background_padding;
    let available = if horizontal {
        config.dimensions[0]
    } else {
        config.dimensions[1]
    };
    let length = style
        .length
        .unwrap_or(available.min(MAX_DEFAULT_LENGTH))
        .round();

    // The labels at the gradient's ends extend past them by half their width, or half their
    // height on vertical colorbars. The gradient shrinks so that they stay inside the background.
    let overhang = end_label_overhang(scale, config, text_engine)?;
    let label_inset = (overhang - (padding - LABEL_EDGE_MARGIN)).max(0.0).round();
    let gradient_length = (length - 2.0 * padding - 2.0 * label_inset)
        .max(MIN_GRADIENT_LENGTH)
        .round();
    let thickness = style.thickness.round();
    let gap = style.axis_gap.round();

    // The gradient runs left to right, or bottom to top.
    let (gradient, size, range, axis_dimensions) = if horizontal {
        let gradient = [0.0, 0.0, gradient_length, 0.0];
        let range = (0.0, f64::from(gradient_length));
        (
            gradient,
            [gradient_length, thickness],
            range,
            [gradient_length, 0.0],
        )
    } else {
        let gradient = [0.0, gradient_length, 0.0, 0.0];
        let range = (f64::from(gradient_length), 0.0);
        (
            gradient,
            [thickness, gradient_length],
            range,
            [0.0, gradient_length],
        )
    };
    let rect = SceneRectMark {
        len: 1,
        gradients: vec![Gradient::LinearGradient(LinearGradient {
            x0: gradient[0],
            y0: gradient[1],
            x1: gradient[2],
            y1: gradient[3],
            stops: scale.color_range_as_gradient_stops(10)?,
        })],
        x: 0.0.into(),
        x2: Some(size[0].into()),
        y: 0.0.into(),
        y2: Some(size[1].into()),
        fill: ColorOrGradient::GradientIndex(0).into(),
        clip: false,
        ..Default::default()
    };
    let axis_origin = match orientation {
        ColorbarOrientation::Top => [0.0, -gap],
        ColorbarOrientation::Bottom => [0.0, thickness + gap],
        ColorbarOrientation::Left => [-gap, 0.0],
        ColorbarOrientation::Right => [thickness + gap, 0.0],
    };
    let tick_count = horizontal.then_some(HORIZONTAL_TICK_COUNT);
    let axis = make_continuous_axis_marks(
        &scale.clone().with_range_interval(range),
        title,
        axis_origin,
        &axis_config(config, orientation.axis(), axis_dimensions, tick_count),
        text_engine,
    )?;
    let content = vec![rect.into(), SceneMark::Group(axis).with_interactive(false)];

    // An axis above or left of the gradient extends into negative coordinates, so the content
    // shifts by that much and keeps the padding on every side.
    let bounds = SceneGroup {
        marks: content.clone(),
        ..Default::default()
    }
    .bounding_box(text_engine);
    let (content_origin, background) = if horizontal {
        let shift = (-bounds.lower()[1]).max(0.0);
        (
            [label_inset + padding, shift + padding],
            [length, bounds.height() + 2.0 * padding],
        )
    } else {
        let shift = (-bounds.lower()[0]).max(0.0);
        (
            [shift + padding, label_inset + padding],
            [bounds.width() + 2.0 * padding, length],
        )
    };
    let content = SceneGroup {
        origin: content_origin,
        marks: content,
        ..Default::default()
    };
    Ok(ColorbarOutput {
        group: SceneGroup {
            origin,
            marks: vec![
                make_background(&style.legend, background[0], background[1]),
                content.into(),
            ],
            ..Default::default()
        },
        gradient_path: vec![1, 0],
        gradient_bounds: [content_origin[0], content_origin[1], size[0], size[1]],
    })
}

/// How far the colorbar's end labels extend past the gradient's ends: half the wider end label
/// on horizontal colorbars, and half a label's height on vertical ones.
fn end_label_overhang(
    scale: &ConfiguredScale,
    config: &ColorbarConfig,
    text_engine: &LabelEngine,
) -> Result<f32, AvengerGuidesError> {
    let (first, last, syntax) = colorbar_end_labels(scale, config, text_engine)?;
    let horizontal = config.orientation.is_horizontal();
    let mut overhang = 0.0_f32;
    for label in [first, last] {
        let bounds = text_engine.bounds(&text_label(
            &label,
            syntax,
            LabelOptions {
                text: config.style.legend.label_text_style(),
                ..Default::default()
            },
        ))?;
        let extent = if horizontal {
            bounds.width
        } else {
            bounds.height
        };
        overhang = overhang.max(extent / 2.0);
    }
    Ok(overhang.round())
}

/// The first and last tick labels, which can extend past the ends of the colorbar.
fn colorbar_end_labels(
    scale: &ConfiguredScale,
    config: &ColorbarConfig,
    text_engine: &LabelEngine,
) -> Result<(String, String, TextSyntaxMode), AvengerGuidesError> {
    let tick_count = config
        .orientation
        .is_horizontal()
        .then_some(HORIZONTAL_TICK_COUNT);
    let axis_config = axis_config(
        config,
        config.orientation.axis(),
        config.dimensions,
        tick_count,
    );
    let (labels, syntax) = axis_tick_labels(scale, &axis_config, text_engine)?;
    Ok((
        labels.first().cloned().unwrap_or_default(),
        labels.last().cloned().unwrap_or_default(),
        syntax,
    ))
}

/// The colorbar's axis, with its tick format and the legend's title and label styling.
fn axis_config(
    config: &ColorbarConfig,
    orientation: AxisOrientation,
    dimensions: [f32; 2],
    tick_count: Option<f64>,
) -> AxisConfig {
    let style = &config.style;
    let legend = &style.legend;
    AxisConfig {
        orientation,
        dimensions,
        grid: false,
        tick_format: config.tick_format.clone(),
        tick_label: config.tick_label.clone(),
        style: AxisStyle {
            tick_count,
            tick_color: style.tick_color,
            domain_color: style.domain_color,
            label_color: legend.label_color,
            label_font_family: legend.label_font_family.clone(),
            label_font_size: legend.label_font_size,
            label_font_weight: legend.label_font_weight,
            title_syntax: legend.title_syntax,
            title_color: legend.title_color,
            title_font_family: legend.title_font_family.clone(),
            title_font_size: legend.title_font_size,
            title_font_weight: legend.title_font_weight,
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{ArrayRef, Date32Array};
    use avenger_geometry::marks::MarkGeometryUtils;
    use avenger_geometry::rtree::{EnvelopeUtils, SceneGraphRTree};
    use avenger_scales::scales::{linear::LinearScale, time::TimeScale};
    use avenger_scenegraph::{marks::mark::SceneMark, scene_graph::SceneGraph};

    use super::*;
    use crate::test_engine;

    fn color_scale() -> ConfiguredScale {
        LinearScale::configured_color((0.0, 100.0), ["#440154", "#fde725"])
    }

    fn test_config(orientation: ColorbarOrientation) -> ColorbarConfig {
        ColorbarConfig {
            orientation,
            dimensions: [240.0, 220.0],
            tick_format: ",f".into(),
            tick_label: None,
            style: ColorbarStyle {
                thickness: 18.0,
                length: Some(160.0),
                axis_gap: 4.0,
                ..Default::default()
            },
        }
    }

    fn mark_at_path<'a>(marks: &'a [SceneMark], path: &[usize]) -> &'a SceneMark {
        let (first, rest) = path.split_first().expect("non-empty path");
        let mark = marks.get(*first).expect("mark at path segment");
        if rest.is_empty() {
            return mark;
        }
        let SceneMark::Group(group) = mark else {
            panic!("interior path segment should be a group");
        };
        mark_at_path(&group.marks, rest)
    }

    #[test]
    fn colorbar_end_labels_are_the_outer_tick_labels() {
        let scale = LinearScale::configured_color((900_000.0, 1_100_000.0), ["#440154", "#fde725"]);
        let config = ColorbarConfig {
            tick_format: "s".into(),
            tick_label: None,
            ..test_config(ColorbarOrientation::Top)
        };

        let (first, last, syntax_mode) =
            colorbar_end_labels(&scale, &config, &test_engine()).expect("end labels");

        assert_eq!(syntax_mode, TextSyntaxMode::Plain);
        assert_eq!(first, "0.90M");
        assert_eq!(last, "1.10M");
    }

    #[test]
    fn horizontal_time_colorbars_measure_their_date_labels() {
        let start = Arc::new(Date32Array::from(vec![19723])) as ArrayRef;
        let end = Arc::new(Date32Array::from(vec![20088])) as ArrayRef;
        let scale = TimeScale::configured_color((start, end), vec!["#440154", "#fde725"]);
        let config = ColorbarConfig {
            tick_format: String::new(),
            tick_label: None,
            ..test_config(ColorbarOrientation::Bottom)
        };

        let (first, last, _) =
            colorbar_end_labels(&scale, &config, &test_engine()).expect("end labels");
        assert_eq!((first.as_str(), last.as_str()), ("2024", "December"));
        make_colorbar_legend(&scale, "date", [0.0, 0.0], &config, &test_engine())
            .expect("colorbar renders");
    }

    /// End labels are measured at the size the axis draws them, so they stay inside the
    /// background.
    #[test]
    fn horizontal_colorbar_end_labels_stay_inside_the_background() {
        let scale = LinearScale::configured_color((0.0, 1_000_000.0), ["#440154", "#fde725"]);
        let engine = test_engine();
        for orientation in [ColorbarOrientation::Top, ColorbarOrientation::Bottom] {
            let mut config = test_config(orientation);
            config.style.legend.label_font_size = 16.0;
            let output =
                make_colorbar_legend(&scale, "", [0.0, 0.0], &config, &engine).expect("colorbar");
            let SceneMark::Rect(background) = &output.group.marks[0] else {
                panic!("the first mark should be the background");
            };
            let width = background.width.as_ref().unwrap().as_vec(1, None)[0];
            let content = output.group.marks[1].bounding_box(&engine);
            assert!(
                content.lower()[0] >= 0.0 && content.upper()[0] <= width,
                "{orientation:?}: labels span {:?} in a {width} px background",
                [content.lower()[0], content.upper()[0]]
            );
        }
    }

    #[test]
    fn colorbars_keep_their_origin() {
        let output = make_colorbar_legend(
            &color_scale(),
            "temperature",
            [12.0, 34.0],
            &test_config(ColorbarOrientation::Right),
            &test_engine(),
        )
        .expect("colorbar renders");
        assert_eq!(output.group.origin, [12.0, 34.0]);
    }

    #[test]
    fn colorbar_gradient_path_resolves_to_the_gradient_rect_at_its_bounds() {
        for orientation in [
            ColorbarOrientation::Top,
            ColorbarOrientation::Bottom,
            ColorbarOrientation::Left,
            ColorbarOrientation::Right,
        ] {
            let output = make_colorbar_legend(
                &color_scale(),
                "temperature",
                [0.0, 0.0],
                &test_config(orientation),
                &test_engine(),
            )
            .expect("colorbar renders");
            let gradient_mark = mark_at_path(&output.group.marks, &output.gradient_path);
            assert_eq!(gradient_mark.interactive(), None);
            let SceneMark::Rect(rect) = gradient_mark else {
                panic!("colorbar hit path should resolve to the gradient rect");
            };
            let rect_bounds = rect.bounding_box();
            assert_eq!(rect_bounds.width(), output.gradient_bounds[2]);
            assert_eq!(rect_bounds.height(), output.gradient_bounds[3]);
        }
    }

    #[test]
    fn colorbar_rtree_hits_gradient_but_not_noninteractive_chrome() {
        let output = make_colorbar_legend(
            &color_scale(),
            "temperature",
            [0.0, 0.0],
            &test_config(ColorbarOrientation::Right),
            &test_engine(),
        )
        .expect("colorbar renders");
        let bounds = output.gradient_bounds;
        let scene = SceneGraph {
            marks: output.group.marks.clone(),
            width: output.group.bounding_box(&test_engine()).width(),
            height: output.group.bounding_box(&test_engine()).height(),
            origin: output.group.origin,
        };
        let rtree = SceneGraphRTree::from_scene_graph(&scene, &test_engine());

        let gradient_point = [bounds[0] + bounds[2] * 0.5, bounds[1] + bounds[3] * 0.5];
        let hit = rtree
            .pick_top_mark_at_point(&gradient_point)
            .expect("gradient should be interactive");
        assert_eq!(hit.mark_path, output.gradient_path);

        let background_point = [1.0, 1.0];
        assert!(
            rtree.pick_top_mark_at_point(&background_point).is_none(),
            "transparent background/chrome should remain noninteractive"
        );
    }
}
