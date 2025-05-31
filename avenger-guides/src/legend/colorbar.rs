use avenger_common::types::{ColorOrGradient, Gradient, LinearGradient};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::SceneGroup, rect::SceneRectMark};

use crate::{
    axis::{
        numeric::make_numeric_axis_marks,
        opts::{AxisConfig, AxisOrientation},
    },
    error::AvengerGuidesError,
};

pub fn make_colorbar_marks(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &ColorbarConfig,
) -> Result<SceneGroup, AvengerGuidesError> {
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

            // Create a new scale with desired range for the axis
            let numeric_scale = scale
                .clone()
                .with_range_interval((config.dimensions[1], 0.0));
            let axis = make_numeric_axis_marks(&numeric_scale, title, origin, &axis_config)?;

            // Create a gradient for the colorbar rect
            let gradient = Gradient::LinearGradient(LinearGradient {
                x0: 0.0,
                y0: config.dimensions[1],
                x1: 0.0,
                y1: 0.0,
                stops: scale.color_range_as_gradient_stops(10)?,
            });

            // Make colorbar rect
            let rect = SceneRectMark {
                len: 1,
                gradients: vec![gradient],
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
            Ok(SceneGroup {
                origin,
                marks: vec![rect.into(), axis.into()],
                ..Default::default()
            })
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
