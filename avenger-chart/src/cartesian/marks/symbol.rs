use crate::cartesian::Cartesian;
use crate::define_position_channels;
use crate::impl_mark_trait_common;
use crate::marks::{Mark, RadiusExpression};
use crate::scales::ScaleRange;
use arrow::array::RecordBatch;
use avenger_scales::scales::{ScaleImpl, ordinal::OrdinalScale, point::PointScale, pow::PowScale};
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::datatypes::DataType;
use datafusion::logical_expr::{Expr, lit};
use datafusion_common::ScalarValue;
use std::collections::HashMap;
// Import Symbol for the macro, then re-export it
use crate::error::AvengerChartError;
pub use crate::marks::symbol::Symbol;
use crate::render_context::RenderContext;
use std::sync::Arc;

// Define position channels for Cartesian Symbol using the macro
define_position_channels! {
    Symbol<Cartesian> {
        x: {
            with_config: crate::cartesian::channels::CartesianPositionConfig,
        },
        y: {
            with_config: crate::cartesian::channels::CartesianPositionConfig,
        }
    }
}

// Implement Mark trait for Cartesian Symbol
impl Mark<Cartesian> for Symbol<Cartesian> {
    impl_mark_trait_common!(Symbol, Cartesian, "symbol");

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        Symbol::<Cartesian>::common_mark_specific_default(channel)
    }

    fn radius_expression(
        &self,
        dimension: &str,
        resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        match dimension {
            "x" | "y" => {
                // Get size and stroke_width expressions (either mapped or default)
                let size_expr = resolve_channel("size");
                let stroke_width_expr = resolve_channel("stroke_width");

                // For symbols: radius = sqrt(area) * 0.5 + stroke_width / 2
                // The size channel represents the area of the bounding square
                // The base circle SVG path has radius 0.5 for a unit square (size=1)
                // Add half the stroke width since stroke extends both inward and outward
                use datafusion::functions::expr_fn::sqrt;
                let radius_expr = sqrt(size_expr) * lit(0.5) + stroke_width_expr / lit(2.0);

                Some(RadiusExpression::Symmetric(radius_expr))
            }
            _ => None,
        }
    }

    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &RenderContext,
        coord: &Cartesian,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Use common rendering logic - it will extract x and y channels
        self.render_from_data_common(data, scalars, context, coord)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<Arc<dyn ScaleImpl>> {
        match (channel, data_type) {
            // Symbol marks use point scales for categorical position data
            ("x" | "y", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(Arc::new(PointScale))
            }
            // Size uses sqrt scale for numeric data (better for area perception)
            (
                "size",
                DataType::Float32
                | DataType::Float64
                | DataType::Int8
                | DataType::Int16
                | DataType::Int32
                | DataType::Int64
                | DataType::UInt8
                | DataType::UInt16
                | DataType::UInt32
                | DataType::UInt64,
            ) => {
                // Use PowScale as a Sqrt scale (it will be configured with exponent 0.5 later)
                Some(Arc::new(PowScale))
            }
            // Size uses ordinal for categorical data
            ("size", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(Arc::new(OrdinalScale))
            }
            // Color and shape channels use ordinal scales for categorical data
            (
                "fill" | "stroke" | "color" | "shape",
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            ) => Some(Arc::new(OrdinalScale)),
            // Stroke width always uses ordinal scale for discrete mapping
            ("stroke_width", _) => Some(Arc::new(OrdinalScale)),
            // Fall back to data type-based inference for other channels
            _ => crate::marks::default_scale_for_data_type(data_type),
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_type: &str,
        _data_type: &DataType,
    ) -> HashMap<String, Expr> {
        let mut options = HashMap::new();

        // Configure PowScale as Sqrt scale for size channel
        if channel == "size" && scale_type == "pow" {
            options.insert("exponent".to_string(), lit(0.5f32));
        }

        options
    }

    fn default_channel_range(
        &self,
        channel: &str,
        scale_type: &str,
        _data_type: &DataType,
        theme: &crate::theme::Theme,
    ) -> Option<ScaleRange> {
        Symbol::<Cartesian>::common_default_channel_range(channel, scale_type, theme)
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<std::sync::Arc<dyn crate::legend_renderer::LegendRenderer>> {
        Symbol::<Cartesian>::common_preferred_legend_renderer(
            channel,
            scale,
            &["x", "y", "x2", "y2"],
        )
    }
}
