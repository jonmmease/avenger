use std::sync::Arc;

use avenger_common::{
    canvas::CanvasDimensions,
    value::{ColorOrGradient, ScalarOrArray},
};
use avenger_geometry::rtree::MarkRTree;
use avenger_scales::band::BandScale;
use avenger_scenegraph::marks::{
    group::SceneGroup, mark::SceneMark, rule::SceneRuleMark, text::SceneTextMark,
};

use avenger_text::{
    rasterization::cosmic::CosmicTextRasterizer,
    types::{FontWeightNameSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec},
};

use super::opts::{AxisConfig, AxisOrientation};

pub fn make_band_axis_marks(
    scale: &BandScale<String>,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
) -> SceneGroup {
    // Make sure ticks end up centered in the band
    // Unwrap is safe because this band value is always valid
    let scale = scale.clone().with_band(0.5).unwrap();
    let tick_length = 5.0;

    match config.orientation {
        AxisOrientation::Top => todo!(),
        AxisOrientation::Left => todo!(),
        AxisOrientation::Bottom { height } => {
            let mut marks: Vec<SceneMark> = vec![];

            // Push down by a half pixel to not overlay with mark at zero
            let height = height + 0.5;

            // ticks rule mark
            let tick_x = scale.scale(scale.domain());
            let tick_y0 = ScalarOrArray::Scalar(height);
            let tick_y1 = ScalarOrArray::Scalar(height + tick_length);
            let tick_x_mark = SceneRuleMark {
                len: scale.domain().len() as u32,
                clip: false,
                x0: tick_x.clone(),
                x1: tick_x.clone(),
                y0: tick_y0,
                y1: tick_y1,
                stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
                stroke_width: 1.0.into(),
                ..Default::default()
            };

            let mut rtree = MarkRTree::new(tick_x_mark.geometry_iter(0).collect());
            let rasterizer = Arc::new(CosmicTextRasterizer::<()>::new());
            marks.push(tick_x_mark.into());

            println!("ticks envelope: {:?}", rtree.envelope());

            // Axis line rule mark
            let axis_x0 = ScalarOrArray::Scalar(scale.range().0);
            let axis_x1 = ScalarOrArray::Scalar(scale.range().1);
            let width = scale.range().1 - scale.range().0;
            let x_mid = (scale.range().0 + scale.range().1) / 2.0;
            let axis_rule_mark = SceneRuleMark {
                x0: axis_x0.clone(),
                x1: axis_x1.clone(),
                y0: height.into(),
                y1: height.into(),
                stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
                stroke_width: 1.0.into(),
                ..Default::default()
            };
            rtree.insert_all(axis_rule_mark.geometry_iter(1).collect());
            marks.push(axis_rule_mark.into());

            println!("ticks+rule envelope: {:?}", rtree.envelope());

            // Add tick text
            let tick_text_mark = SceneTextMark {
                len: scale.domain().len() as u32,
                text: scale.domain().clone().into(),
                x: tick_x,
                y: (height + 8.0).into(),
                align: TextAlignSpec::Center.into(),
                baseline: TextBaselineSpec::Top.into(),
                angle: 0.0.into(),
                color: [0.0, 0.0, 0.0, 1.0].into(),
                font_size: 8.0.into(),
                ..Default::default()
            };
            rtree.insert_all(
                tick_text_mark
                    .geometry_iter(
                        2,
                        rasterizer,
                        &CanvasDimensions {
                            size: [width, width],
                            scale: 1.0,
                        },
                    )
                    .unwrap()
                    .collect(),
            );
            marks.push(tick_text_mark.into());

            println!("ticks+rule+label envelope: {:?}", rtree.envelope());
            let y_offset = rtree.envelope().upper()[1];
            let title_margin = 2.0;

            let label_text_mark = SceneTextMark {
                len: 1,
                text: title.to_string().into(),
                x: x_mid.into(),
                y: (y_offset + title_margin).into(),
                align: TextAlignSpec::Center.into(),
                baseline: TextBaselineSpec::Top.into(),
                angle: (0.0).into(),
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
