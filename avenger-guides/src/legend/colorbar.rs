use avenger_common::types::{ColorOrGradient, Gradient, LinearGradient};
use avenger_geometry::{marks::MarkGeometryUtils, rtree::EnvelopeUtils};
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
            // config.dimensions represents available space for the colorbar
            let _available_width = config.dimensions[0];
            let available_height = config.dimensions[1];

            // Colorbar properties
            let colorbar_width = config.colorbar_width.unwrap_or(15.0);
            let colorbar_height = config
                .colorbar_height
                .unwrap_or(available_height.min(200.0));
            let colorbar_margin = config.colorbar_margin.unwrap_or(5.0);

            // Create a gradient for the colorbar rect
            let gradient = Gradient::LinearGradient(LinearGradient {
                x0: 0.0,
                y0: colorbar_height,
                x1: 0.0,
                y1: 0.0,
                stops: scale.color_range_as_gradient_stops(10)?,
            });

            // Position colorbar at origin with optional additional left padding
            let left_padding = config.left_padding.unwrap_or(8.0);
            let colorbar_x = origin[0] + left_padding;
            let colorbar_y = origin[1];

            // Make colorbar rect
            let rect = SceneRectMark {
                len: 1,
                gradients: vec![gradient],
                x: colorbar_x.into(),
                x2: Some((colorbar_x + colorbar_width).into()),
                y: colorbar_y.into(),
                y2: Some((colorbar_y + colorbar_height).into()),
                fill: ColorOrGradient::GradientIndex(0).into(),
                ..Default::default()
            };

            // Make axis positioned to the right of the colorbar with small margin
            let axis_origin = [colorbar_x + colorbar_width + colorbar_margin, colorbar_y];
            let axis_config = AxisConfig {
                orientation: AxisOrientation::Right,
                dimensions: [0.0, colorbar_height],
                grid: false,
                format_number: None,
            };
            let axis_config = AxisConfig { format_number: config.format_number.clone(), ..axis_config };

            // Create a new scale with desired range for the axis
            let numeric_scale = scale.clone().with_range_interval((colorbar_height, 0.0));
            let axis = make_numeric_axis_marks(&numeric_scale, title, axis_origin, &axis_config)?;

            // Measure the overall bounds to create a clip rect
            let marks = vec![rect.into(), axis.into()];
            let temp_group = SceneGroup {
                marks: marks.clone(),
                ..Default::default()
            };
            let bbox = temp_group.bounding_box();

            // Use actual bounding box coordinates for clip rect to avoid clipping content
            // Add a bit more outer padding so the colorbar legend has breathing room
            // Respect asymmetric left padding as well
            let outer_padding = 6.0;
            let clip_x = bbox.lower()[0] - outer_padding - left_padding;
            let clip_y = bbox.lower()[1] - outer_padding;
            let clip_width = bbox.width() + 2.0 * outer_padding + left_padding;
            let clip_height = bbox.height() + 2.0 * outer_padding;

            Ok(SceneGroup {
                origin: [0.0, 0.0], // Group at root since we're positioning elements absolutely
                marks,
                clip: avenger_scenegraph::marks::group::Clip::Rect {
                    x: clip_x,
                    y: clip_y,
                    width: clip_width,
                    height: clip_height,
                },
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
    /// Dimensions of the plot area (width, height) that the colorbar is attached to
    pub dimensions: [f32; 2],
    /// Width of the colorbar (thickness)
    pub colorbar_width: Option<f32>,
    /// Height of the colorbar (length),
    /// if None will use plot height limited to 200px
    pub colorbar_height: Option<f32>,
    /// Margin between plot area and colorbar
    pub colorbar_margin: Option<f32>,
    /// Extra left padding before the colorbar rectangle (gap from plot edge)
    pub left_padding: Option<f32>,
    /// Optional numeric formatting string for colorbar tick labels
    pub format_number: Option<String>,
}

impl Default for ColorbarConfig {
    fn default() -> Self {
        Self {
            orientation: ColorbarOrientation::Right,
            dimensions: [100.0, 100.0],
            colorbar_width: None,
            colorbar_height: None,
            colorbar_margin: None,
            left_padding: None,
            format_number: None,
        }
    }
}
