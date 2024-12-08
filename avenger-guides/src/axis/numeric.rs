use std::sync::Arc;

use avenger_common::{canvas::CanvasDimensions, types::ColorOrGradient, value::ScalarOrArray};
use avenger_geometry::rtree::MarkRTree;
use avenger_scales::numeric::ContinuousNumericScale;
use avenger_scenegraph::marks::{
    group::SceneGroup, mark::SceneMark, rule::SceneRuleMark, text::SceneTextMark,
};
use avenger_text::{
    rasterization::{cosmic::CosmicTextRasterizer, TextRasterizer},
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
        AxisOrientation::Bottom { height } => todo!(),
        AxisOrientation::Left => {
            let mut marks: Vec<SceneMark> = vec![];

            let y_offset = 0.5;

            // ticks rule mark
            let ticks = scale.ticks(None);
            let tick_y0 = scale.scale(&ticks).map(|y| y + y_offset);
            let tick_x0 = ScalarOrArray::Scalar(0.0);
            let tick_x1 = ScalarOrArray::Scalar(-5.0);
            let ticks_mark = SceneRuleMark {
                len: ticks.len() as u32,
                clip: false,
                x0: tick_x0,
                x1: tick_x1.clone(),
                y0: tick_y0.clone(),
                y1: tick_y0.clone(),
                stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
                stroke_width: 1.0.into(),
                ..Default::default()
            };

            let ticks_geometry: Vec<_> = ticks_mark.geometry_iter(0).collect();
            marks.push(ticks_mark.into());

            let mut rtree = MarkRTree::new(ticks_geometry);
            let rasterizer = Arc::new(CosmicTextRasterizer::<()>::new());

            // Axis line rule mark
            // Offset bottom by a half pixel to not overlay with tick at zero
            let y_upper = f32::min(scale.range().1, scale.range().0);
            let y_lower = f32::max(scale.range().0, scale.range().1) + y_offset;
            let height = y_lower - y_upper;
            let y_mid = (y_lower + y_upper) / 2.0;

            let axis_y0 = ScalarOrArray::Scalar(y_lower);
            let axis_y1 = ScalarOrArray::Scalar(y_upper);
            let axis_rule_mark = SceneRuleMark {
                x0: 0.0.into(),
                x1: 0.0.into(),
                y0: axis_y0,
                y1: axis_y1,
                stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
                stroke_width: 1.0.into(),
                ..Default::default()
            };

            rtree.insert_all(axis_rule_mark.geometry_iter(1).collect());
            marks.push(axis_rule_mark.into());

            // Add tick text
            let tick_text = ticks.iter().map(|t| t.to_string()).collect::<Vec<String>>();
            let tick_text_mark = SceneTextMark {
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
            };

            rtree.insert_all(
                tick_text_mark
                    .geometry_iter(
                        2,
                        rasterizer.clone(),
                        &CanvasDimensions {
                            size: [height, height],
                            scale: 1.0,
                        },
                    )
                    .unwrap()
                    .collect(),
            );
            marks.push(tick_text_mark.into());

            println!("{:?}", rtree.envelope());

            // Add axis label, offset to avoid overlap with ticks
            let x_offset = rtree.envelope().lower()[0];
            let title_margin = 2.0;

            let label_text_mark = SceneTextMark {
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
            };
            marks.push(label_text_mark.into());

            SceneGroup {
                marks,
                origin,
                ..Default::default()
            }
        }
        AxisOrientation::Right { width } => todo!(),
    }
}
