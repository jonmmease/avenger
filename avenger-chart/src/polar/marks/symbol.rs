use crate::define_position_channels;
use crate::error::AvengerChartError;
use crate::impl_mark_trait_common;
use crate::marks::{Mark, RadiusExpression};

use crate::polar::Polar;
use crate::render_context::RenderContext;
use crate::scales::ScaleRange;
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::arrow::datatypes::DataType;
use datafusion::logical_expr::{Expr, lit};
use datafusion_common::ScalarValue;
// Import Symbol for the macro, then re-export it
use crate::marks::symbol::Symbol;

// Define position channels for Polar Symbol using the macro
define_position_channels! {
    Symbol<Polar> {
        r: {
            with_config: crate::polar::channels::PolarPositionConfig,
        },
        theta: {
            with_config: crate::polar::channels::PolarPositionConfig,
        }
    }
}

// Implement Mark trait for PolarGeneral Symbol with any axis type
impl Mark<Polar> for Symbol<Polar> {
    impl_mark_trait_common!(Symbol, Polar, "symbol");

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        Symbol::<Polar>::common_mark_specific_default(channel)
    }

    fn radius_expression(
        &self,
        dimension: &str,
        resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        match dimension {
            "r" | "theta" => {
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
        coord: &Polar,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        self.render_from_data_common(data, scalars, context, coord)
    }

    fn default_channel_range(
        &self,
        channel: &str,
        scale_impl: &dyn avenger_scales::scales::ScaleImpl,
        domain: &crate::scales::ResolvedDomain,
        _data_type: &DataType,
        theme: &crate::theme::Theme,
    ) -> Option<ScaleRange> {
        Symbol::<Polar>::common_default_channel_range(channel, scale_impl, domain, theme)
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<std::sync::Arc<dyn crate::legend_renderer::LegendRenderer>> {
        Symbol::<Polar>::common_preferred_legend_renderer(channel, scale, &["r", "theta"])
    }
}
