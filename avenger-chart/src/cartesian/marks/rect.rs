use crate::cartesian::Cartesian;
use crate::define_position_channels;
use crate::impl_mark_trait_common;
use crate::marks::Mark;
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
// Import Rect for the macro, then re-export it
use crate::error::AvengerChartError;
pub use crate::marks::rect::Rect;
use crate::marks::util::{coerce_color_channel_with_mark, coerce_numeric_channel_with_mark};
use crate::render_context::RenderContext;
use crate::scales::ScaleRange;
use avenger_scales::scales::{ScaleImpl, band::BandScale, ordinal::OrdinalScale};
use datafusion::arrow::datatypes::DataType;
use datafusion_common::ScalarValue;
use std::sync::Arc;

// Define position channels for Cartesian Rect using the macro
define_position_channels! {
    Rect<Cartesian> {
        x: {
            with_config: crate::cartesian::channels::CartesianPositionConfig,
        },
        x2: {
            with_config: crate::cartesian::channels::CartesianPositionConfig,
        },
        y: {
            with_config: crate::cartesian::channels::CartesianPositionConfig,
        },
        y2: {
            with_config: crate::cartesian::channels::CartesianPositionConfig,
        }
    }
}

// Implement Mark trait for Cartesian Rect with any axis type
impl Mark<Cartesian> for Rect<Cartesian> {
    impl_mark_trait_common!(Rect, Cartesian, "rect");

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        match channel {
            "fill" => Some(ScalarValue::Utf8(Some("#4682b4".to_string()))), // Default steel blue
            "stroke" => Some(ScalarValue::Utf8(Some("#000000".to_string()))), // Default black
            "stroke_width" => Some(ScalarValue::Float32(Some(1.0))),        // Default stroke width
            "corner_radius" => Some(ScalarValue::Float32(Some(0.0))),       // Default no rounding
            "opacity" => Some(ScalarValue::Float32(Some(1.0))),             // Fully opaque
            _ => None,
        }
    }

    fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &RenderContext,
        _coord: &Cartesian,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Determine number of marks from data batch or default to 1
        let len = data.map_or(1, |data| data.num_rows()) as u32;

        // Extract position values using Coercer with mark defaults
        let x = coerce_numeric_channel_with_mark(self, data, scalars, "x", context, 0.0)?;
        let x2 = coerce_numeric_channel_with_mark(self, data, scalars, "x2", context, 0.0)?;
        let y = coerce_numeric_channel_with_mark(self, data, scalars, "y", context, 0.0)?;
        let y2 = coerce_numeric_channel_with_mark(self, data, scalars, "y2", context, 0.0)?;

        // Extract style values using Coercer with mark defaults
        let fill = coerce_color_channel_with_mark(
            self,
            data,
            scalars,
            "fill",
            context,
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0], // Fallback steel blue
        )?;
        let stroke = coerce_color_channel_with_mark(
            self,
            data,
            scalars,
            "stroke",
            context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let stroke_width =
            coerce_numeric_channel_with_mark(self, data, scalars, "stroke_width", context, 1.0)?;
        let corner_radius =
            coerce_numeric_channel_with_mark(self, data, scalars, "corner_radius", context, 0.0)?;

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

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<Arc<dyn ScaleImpl>> {
        match (channel, data_type) {
            // Rect marks use band scales for categorical position data
            ("x" | "y", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(Arc::new(BandScale))
            }
            // Color channels use ordinal scales for categorical data
            (
                "fill" | "stroke" | "color",
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            ) => Some(Arc::new(OrdinalScale)),
            // Stroke width uses ordinal scale only for categorical data
            ("stroke_width", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(Arc::new(OrdinalScale))
            }
            // Fall back to data type-based inference for other channels
            _ => crate::marks::default_scale_for_data_type(data_type),
        }
    }

    fn default_channel_range(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        domain: &crate::scales::ResolvedDomain,
        _data_type: &DataType,
        theme: &dyn crate::theme::Theme,
    ) -> Option<ScaleRange> {
        match channel {
            "corner_radius" => Some(domain.make_interval_or_linspaced_range(0.0, 10.0)),
            "opacity" => Some(domain.make_interval_or_linspaced_range(0.1, 1.0)),
            "stroke_width" => Some(domain.make_interval_or_linspaced_range(0.5, 5.0)),
            "fill" | "stroke" | "color" => {
                // Use theme color system
                let range_kind = scale_impl.range_kind();
                Some(theme.get_range_for_channel("rect", channel, range_kind, None))
            }
            _ => None,
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<std::sync::Arc<dyn crate::legend::LegendRenderer>> {
        use crate::legend::{ColorbarRenderer, RectLegendRenderer};
        use crate::marks::util::is_continuous_scale;
        use std::sync::Arc;

        // Check if scale is continuous (for colorbar)
        let is_continuous = is_continuous_scale(scale.scale_impl.as_ref());

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
