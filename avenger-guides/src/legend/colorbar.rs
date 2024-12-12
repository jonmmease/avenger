use avenger_common::types::{ColorOrGradient, Gradient, LinearGradient};
use avenger_scales::{
    color::continuous_color::{ColorSpace, ContinuousColorScale},
    numeric::ContinuousNumericScale,
};
use avenger_scenegraph::marks::{group::SceneGroup, rect::SceneRectMark};

use crate::axis::{
    numeric::make_numeric_axis_marks,
    opts::{AxisConfig, AxisOrientation},
};

pub fn make_colorbar_marks<C, S>(
    scale: &ContinuousColorScale<C, S, f32>,
    title: &str,
    origin: [f32; 2],
    config: &ColorbarConfig,
) -> SceneGroup
where
    C: ColorSpace,
    S: ContinuousNumericScale<f32>,
{
    match config.orientation {
        ColorbarOrientation::Top => todo!(),
        ColorbarOrientation::Bottom => todo!(),
        ColorbarOrientation::Left => todo!(),
        ColorbarOrientation::Right => {
            let scale_x_offset = 30.0;
            let colorbar_width = 10.0;

            // Make axis
            let axis_config = AxisConfig {
                orientation: AxisOrientation::Right,
                dimensions: [config.dimensions[0] + scale_x_offset, config.dimensions[1]],
                grid: false,
            };
            let mut numeric_scale = scale.get_numeric_scale().clone();
            numeric_scale.set_range((config.dimensions[1], 0.0));
            let axis = make_numeric_axis_marks(&numeric_scale, title, origin, &axis_config);

            let gradient = Gradient::LinearGradient(LinearGradient {
                x0: 0.0,
                y0: config.dimensions[1],
                x1: 0.0,
                y1: 0.0,
                stops: scale.to_gradient_stops(),
            });

            // Make colorbar rect
            let rect = SceneRectMark {
                len: 1,
                gradients: vec![gradient.into()],
                x: (config.dimensions[0] + scale_x_offset - colorbar_width).into(),
                x2: Some((config.dimensions[0] + scale_x_offset).into()),
                y: (-0.5).into(),
                y2: Some((config.dimensions[1] + 0.5).into()),
                fill: ColorOrGradient::GradientIndex(0).into(),
                ..Default::default()
            };

            // // Shift axis to the left
            // axis.origin[0] -= config.dimensions[0];

            // // Make rect
            // let rect = SceneRectMark {
            //     origin,
            //     dimensions: config.dimensions,
            //     ..Default::default()
            // };
            SceneGroup {
                origin,
                marks: vec![rect.into(), axis.into()],
                ..Default::default()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum ColorbarOrientation {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub struct ColorbarConfig {
    pub orientation: ColorbarOrientation,
    pub dimensions: [f32; 2],
}

impl Default for ColorbarConfig {
    fn default() -> Self {
        Self {
            orientation: ColorbarOrientation::Right,
            dimensions: [100.0, 100.0],
        }
    }
}
