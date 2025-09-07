use crate::define_position_channels;
use crate::impl_mark_trait_common;
use crate::marks::Mark;

use crate::polar::Polar;
use crate::render_context::RenderContext;
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;

// Import Rect for the macro, then re-export it
use crate::error::AvengerChartError;
use crate::marks::rect::Rect;

// Define position channels for Polar Rect using the macro
define_position_channels! {
    Rect<Polar> {
        r: {
            with_config: crate::polar::channels::PolarPositionConfig,
        },
        r2: {
            with_config: crate::polar::channels::PolarPositionConfig,
        },
        theta: {
            with_config: crate::polar::channels::PolarPositionConfig,
        },
        theta2: {
            with_config: crate::polar::channels::PolarPositionConfig,
        }
    }
}

// Implement Mark trait for PolarGeneral Rect with any axis type
impl Mark<Polar> for Rect<Polar> {
    impl_mark_trait_common!(Rect, Polar, "rect");

    fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &RenderContext,
        _coord: &Polar,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Polar rect rendering not yet implemented
        Err(AvengerChartError::InternalError(
            "Polar rect rendering not yet implemented".to_string(),
        ))
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
            "r" | "theta" | "r2" | "theta2" | "corner_radius" | "defined" | "order" => None,
            // For any other channel, default to RectLegendRenderer
            _ => Some(Arc::new(RectLegendRenderer::new())),
        }
    }
}
