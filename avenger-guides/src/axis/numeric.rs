use std::sync::Arc;

use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};
use avenger_geometry::rtree::MarkRTree;
use avenger_scales::numeric::ContinuousNumericScale;
use avenger_scenegraph::marks::{group::SceneGroup, rule::SceneRuleMark, text::SceneTextMark};
use avenger_text::{
    rasterization::cosmic::CosmicTextRasterizer,
    types::{FontWeightNameSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec},
};

use super::opts::{AxisConfig, AxisOrientation};

pub fn make_numeric_axis_marks(
    scale: &impl ContinuousNumericScale<f32>,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
) -> SceneGroup {
    match config.orientation {
        AxisOrientation::Top => todo!(),
        AxisOrientation::Bottom { .. } => todo!(),
        AxisOrientation::Left => {
            let mut group = SceneGroup {
                origin,
                ..Default::default()
            };

            let y_offset = 0.5;

            // ticks rule mark
            let ticks = scale.ticks(None);
            let tick_y0 = scale.scale(&ticks).map(|y| y + y_offset);
            let tick_x0 = ScalarOrArray::Scalar(0.0);
            let tick_x1 = ScalarOrArray::Scalar(-5.0);

            group.marks.push(
                SceneRuleMark {
                    len: ticks.len() as u32,
                    clip: false,
                    x0: tick_x0,
                    x1: tick_x1.clone(),
                    y0: tick_y0.clone(),
                    y1: tick_y0.clone(),
                    stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
                    stroke_width: 1.0.into(),
                    ..Default::default()
                }
                .into(),
            );

            let rasterizer = Arc::new(CosmicTextRasterizer::<()>::new());

            // Axis line rule mark
            // Offset bottom by a half pixel to not overlay with tick at zero
            let y_upper = f32::min(scale.range().1, scale.range().0);
            let y_lower = f32::max(scale.range().0, scale.range().1) + y_offset;
            let height = y_lower - y_upper;
            let y_mid = (y_lower + y_upper) / 2.0;

            let axis_y0 = ScalarOrArray::Scalar(y_lower);
            let axis_y1 = ScalarOrArray::Scalar(y_upper);

            group.marks.push(
                SceneRuleMark {
                    x0: 0.0.into(),
                    x1: 0.0.into(),
                    y0: axis_y0,
                    y1: axis_y1,
                    stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
                    stroke_width: 1.0.into(),
                    ..Default::default()
                }
                .into(),
            );

            // Add tick text
            let tick_text = ticks.iter().map(|t| t.to_string()).collect::<Vec<String>>();
            group.marks.push(
                SceneTextMark {
                    len: ticks.len() as u32,
                    text: tick_text.clone().into(),
                    x: tick_x1.map(|x| -8.0),
                    y: tick_y0,
                    align: TextAlignSpec::Right.into(),
                    baseline: TextBaselineSpec::Middle.into(),
                    angle: 0.0.into(),
                    color: [0.0, 0.0, 0.0, 1.0].into(),
                    font_size: 8.0.into(),
                    ..Default::default()
                }
                .into(),
            );

            // make rtree
            let rtree = MarkRTree::from_scene_group(&group);

            // Add axis label, offset to avoid overlap with ticks
            let x_offset = rtree.envelope().lower()[0];
            let title_margin = 2.0;

            group.marks.push(
                SceneTextMark {
                    len: 1,
                    text: title.to_string().into(),
                    x: (x_offset - title_margin).into(),
                    y: y_mid.into(),
                    align: TextAlignSpec::Center.into(),
                    baseline: TextBaselineSpec::LineBottom.into(),
                    angle: (-90.0).into(),
                    color: [0.0, 0.0, 0.0, 1.0].into(),
                    font_size: 10.0.into(),
                    font_weight: FontWeightSpec::Name(FontWeightNameSpec::Bold).into(),
                    ..Default::default()
                }
                .into(),
            );

            group
        }
        AxisOrientation::Right { width } => todo!(),
    }
}
