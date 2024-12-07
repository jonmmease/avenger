use avenger_common::value::{ColorOrGradient, ScalarOrArray};
use avenger_scales::numeric::ContinuousNumericScale;
use avenger_scenegraph::marks::{
    group::SceneGroup,
    mark::SceneMark,
    rule::SceneRuleMark,
    text::{SceneTextMark, TextAlignSpec, TextBaselineSpec},
};

use super::opts::{AxisConfig, AxisOrientation};

pub fn make_numeric_axis_marks(
    scale: &impl ContinuousNumericScale<f32>,
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
            marks.push(
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

            // Axis line rule mark
            // Offset bottom by a half pixel to not overlay with tick at zero
            let y_upper = f32::min(scale.range().1, scale.range().0);
            let y_lower = f32::max(scale.range().0, scale.range().1) + y_offset;

            let axis_y0 = ScalarOrArray::Scalar(y_lower);
            let axis_y1 = ScalarOrArray::Scalar(y_upper);
            marks.push(
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
            marks.push(
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

            SceneGroup {
                marks,
                origin,
                ..Default::default()
            }
        }
        AxisOrientation::Right { width } => todo!(),
    }
}