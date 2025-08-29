use crate::cartesian::Cartesian;
use crate::define_position_channels;
use crate::impl_mark_trait_common;
use crate::marks::{ChannelType, Mark};
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
// Import Rect for the macro, then re-export it
use crate::error::AvengerChartError;
pub use crate::marks::rect::Rect;
use crate::marks::util::{coerce_color_channel, coerce_numeric_channel};

// Define position channels for Cartesian Rect using the macro
define_position_channels! {
    Rect<Cartesian> {
        x: {
            type: ChannelType::Numeric,
            with_config: crate::cartesian::channels::CartesianPositionConfig
        },
        x2: {
            type: ChannelType::Numeric,
            with_config: crate::cartesian::channels::CartesianPositionConfig
        },
        y: {
            type: ChannelType::Numeric,
            with_config: crate::cartesian::channels::CartesianPositionConfig
        },
        y2: {
            type: ChannelType::Numeric,
            with_config: crate::cartesian::channels::CartesianPositionConfig
        }
    }
}

// Implement Mark trait for Cartesian Rect with any axis type
impl Mark<Cartesian> for Rect<Cartesian> {
    impl_mark_trait_common!(Rect, Cartesian, "rect");

    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Determine number of marks from data batch or default to 1
        let len = data.map_or(1, |data| data.num_rows()) as u32;

        // Extract position values using Coercer
        let x = coerce_numeric_channel(data, scalars, "x", 0.0)?;
        let x2 = coerce_numeric_channel(data, scalars, "x2", 0.0)?;
        let y = coerce_numeric_channel(data, scalars, "y", 0.0)?;
        let y2 = coerce_numeric_channel(data, scalars, "y2", 0.0)?;

        // Extract style values using Coercer
        let fill = coerce_color_channel(data, scalars, "fill", [0.27, 0.51, 0.71, 1.0])?;
        let stroke = coerce_color_channel(data, scalars, "stroke", [0.0, 0.0, 0.0, 1.0])?;
        let stroke_width = coerce_numeric_channel(data, scalars, "stroke_width", 1.0)?;
        let corner_radius = coerce_numeric_channel(data, scalars, "corner_radius", 0.0)?;

        // Create SceneRectMark
        let rect_mark = SceneRectMark {
            name: "rect".to_string(),
            clip: true,
            len,
            gradients: vec![],
            x,
            y,
            width: None,
            height: None,
            x2: Some(x2),
            y2: Some(y2),
            fill,
            stroke,
            stroke_width,
            corner_radius,
            indices: None,
            zindex: self.get_zindex(),
        };

        Ok(vec![SceneMark::Rect(rect_mark)])
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<std::sync::Arc<dyn crate::legend_renderer::LegendRenderer>> {
        use crate::legend_renderer::{ColorbarRenderer, RectLegendRenderer};
        use std::sync::Arc;

        // Check if scale is continuous (for colorbar)
        let scale_type = scale.scale_impl.scale_type();
        let is_continuous = matches!(
            scale_type,
            "linear" | "log" | "pow" | "sqrt" | "symlog" | "time"
        );

        match channel {
            // Use colorbar for continuous color scales
            "fill" | "stroke" | "color" if is_continuous => Some(Arc::new(ColorbarRenderer::new())),
            // Rect marks use RectLegendRenderer for discrete scales and other visual properties
            "fill" | "stroke" | "color" | "opacity" | "stroke_width" => {
                Some(Arc::new(RectLegendRenderer::new()))
            }
            // No legend for position channels and other non-visual channels
            "x" | "y" | "x2" | "y2" | "width" | "height" | "defined" | "order"
            | "corner_radius" => None,
            // For any other channel, default to RectLegendRenderer
            _ => Some(Arc::new(RectLegendRenderer::new())),
        }
    }
}
