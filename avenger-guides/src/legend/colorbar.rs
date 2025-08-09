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
    _origin: [f32; 2], // Unused - we always start at (0, 0) now
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
            // colorbar_height now refers to the total height including background padding
            let total_height = config
                .colorbar_height
                .unwrap_or(available_height.min(200.0));
            let bg_padding = config.background_padding.unwrap_or(8.0);

            // Calculate the actual gradient height by subtracting padding
            let gradient_height = (total_height - 2.0 * bg_padding).max(10.0);

            let colorbar_width = config.colorbar_width.unwrap_or(15.0);
            let colorbar_margin = config.colorbar_margin.unwrap_or(5.0);

            // Create a gradient for the colorbar rect
            let gradient = Gradient::LinearGradient(LinearGradient {
                x0: 0.0,
                y0: gradient_height,
                x1: 0.0,
                y1: 0.0,
                stops: scale.color_range_as_gradient_stops(10)?,
            });

            // Make colorbar rect
            let rect = SceneRectMark {
                len: 1,
                gradients: vec![gradient],
                x: 0.0.into(),
                x2: Some((colorbar_width).into()),
                y: 0.0.into(),
                y2: Some((gradient_height).into()),
                fill: ColorOrGradient::GradientIndex(0).into(),
                ..Default::default()
            };

            // Make axis positioned to the right of the colorbar with small margin
            let axis_origin = [colorbar_width + colorbar_margin, 0.0];
            let axis_config = AxisConfig {
                orientation: AxisOrientation::Right,
                dimensions: [0.0, gradient_height],
                grid: false,
                format_number: None,
            };
            let axis_config = AxisConfig {
                format_number: config.format_number.clone(),
                ..axis_config
            };

            // Create a new scale with desired range for the axis
            let numeric_scale = scale.clone().with_range_interval((gradient_height, 0.0));
            let axis = make_numeric_axis_marks(&numeric_scale, title, axis_origin, &axis_config)?;

            // Content marks
            let content_marks = vec![rect.clone().into(), axis.clone().into()];
            let content_group = SceneGroup {
                marks: content_marks.clone(),
                ..Default::default()
            };
            let content_bbox = content_group.bounding_box();

            // Calculate total dimensions including padding
            // The background rect always exists and defines our coordinate system
            // total_height already includes the padding
            let bg_width = content_bbox.width() + bg_padding * 2.0;
            let bg_height = total_height;

            // Always create a background rect at origin (0, 0)
            // This provides consistent layout whether visible or not
            let bg_rect = SceneRectMark {
                x: 0.0.into(),
                y: 0.0.into(),
                width: Some(bg_width.into()),
                height: Some(bg_height.into()),
                fill: config
                    .background_fill
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])) // Transparent by default
                    .into(),
                stroke: config
                    .background_stroke
                    .clone()
                    .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])) // No stroke by default
                    .into(),
                stroke_width: if config.background_stroke.is_some() {
                    1.0.into()
                } else {
                    0.0.into()
                },
                corner_radius: config.background_corner_radius.unwrap_or(0.0).into(),
                zindex: Some(0),
                ..Default::default()
            };

            // Create a group for colorbar and axis that places them centered vertically
            let colorbar_axis_group = SceneGroup {
                origin: [bg_padding, bg_padding],
                marks: vec![rect.into(), axis.into()],
                clip: avenger_scenegraph::marks::group::Clip::None,
                ..Default::default()
            };

            // Insert background rect first, then legend content
            let marks = vec![bg_rect.into(), colorbar_axis_group.into()];

            Ok(SceneGroup {
                origin: [0.0, 0.0],
                marks,
                clip: avenger_scenegraph::marks::group::Clip::None,
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
    /// Total height of the colorbar including background padding,
    /// if None will use plot height limited to 200px
    pub colorbar_height: Option<f32>,
    /// Margin between plot area and colorbar
    pub colorbar_margin: Option<f32>,
    /// Optional numeric formatting string for colorbar tick labels
    pub format_number: Option<String>,
    /// Optional background rect styling
    pub background_fill: Option<ColorOrGradient>,
    pub background_stroke: Option<ColorOrGradient>,
    pub background_corner_radius: Option<f32>,
    pub background_padding: Option<f32>,
}

impl Default for ColorbarConfig {
    fn default() -> Self {
        Self {
            orientation: ColorbarOrientation::Right,
            dimensions: [100.0, 100.0],
            colorbar_width: None,
            colorbar_height: None,
            colorbar_margin: None,
            format_number: None,
            background_fill: None,
            background_stroke: None,
            background_corner_radius: None,
            background_padding: None,
        }
    }
}
