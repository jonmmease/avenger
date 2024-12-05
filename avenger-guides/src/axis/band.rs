use avenger_common::value::{ColorOrGradient, ScalarOrArray};
use avenger_scales::band::BandScale;
use avenger_scenegraph::marks::{
    group::SceneGroup,
    mark::SceneMark,
    rule::SceneRuleMark,
    text::{SceneTextMark, TextAlignSpec, TextBaselineSpec},
};

use super::opts::{AxisConfig, AxisOrientation};

pub fn make_band_axis_marks(
    scale: &BandScale<String>,
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
            marks.push(
                SceneRuleMark {
                    len: scale.domain().len() as u32,
                    clip: false,
                    x0: tick_x.clone(),
                    x1: tick_x.clone(),
                    y0: tick_y0,
                    y1: tick_y1,
                    stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
                    stroke_width: 1.0.into(),
                    ..Default::default()
                }
                .into(),
            );

            // Axis line rule mark
            let axis_x0 = ScalarOrArray::Scalar(scale.range().0);
            let axis_x1 = ScalarOrArray::Scalar(scale.range().1);
            marks.push(
                SceneRuleMark {
                    x0: axis_x0.clone(),
                    x1: axis_x1.clone(),
                    y0: height.into(),
                    y1: height.into(),
                    stroke: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
                    stroke_width: 1.0.into(),
                    ..Default::default()
                }
                .into(),
            );

            // Add tick text
            marks.push(
                SceneTextMark {
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
