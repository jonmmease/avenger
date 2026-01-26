use crate::cartesian::Cartesian;
use crate::define_position_channels;
use crate::impl_mark_trait_common;
use crate::marks::{CompiledDataContext, CompiledMark, CompiledMarkState, Mark};
use arrow::array::RecordBatch;
use avenger_scenegraph::marks::mark::SceneMark;
// Import Rect for the macro, then re-export it
use crate::channel::ChannelDescriptor;
use crate::coords::CoordinateSystemTransform;
use crate::error::AvengerChartError;
pub use crate::marks::rect::Rect;
use crate::render::RenderContext;
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};

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
#[async_trait::async_trait]
impl Mark<Cartesian> for Rect<Cartesian> {
    impl_mark_trait_common!(Rect);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<std::sync::Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(std::sync::Arc::new(CompiledCartesianRect {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianRect {
    pub(crate) state: CompiledMarkState,
}

// CompiledMark implementation
#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianRect {
    fn state(&self) -> &CompiledMarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        &mut self.state
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.state.data
    }

    fn mark_type(&self) -> &str {
        "rect"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            // Position channels - rectangles need all four corners
            ChannelDescriptor {
                name: "x",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "x2",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "y",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "y2",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            // Style channels
            ChannelDescriptor {
                name: "fill",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke_width",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "corner_radius",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "opacity",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    async fn measure_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &RenderContext,
        _coord: Box<dyn CoordinateSystemTransform>,
    ) -> Result<
        (
            crate::guide::OverflowSpaceRequirement,
            Box<dyn crate::marks::MarkMeasurement>,
        ),
        AvengerChartError,
    > {
        // Regular marks don't need overflow space or cached data
        Ok((
            crate::guide::OverflowSpaceRequirement::default(),
            Box::new(crate::marks::EmptyMarkMeasurement),
        ))
    }

    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &RenderContext,
        coord: Box<dyn CoordinateSystemTransform>,
        _measurement: &dyn crate::marks::MarkMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        use crate::marks::util::{
            coerce_color_channel_with_renderer, coerce_numeric_channel_with_renderer,
        };
        use avenger_scenegraph::marks::rect::SceneRectMark;

        // Determine number of marks from data batch or default to 1
        let len = data.map_or(1, |data| data.num_rows()) as u32;

        // Extract position channels for the coordinate system
        // For Cartesian rectangles, we need to transform both corners
        let mut position_channels_corner1 = std::collections::HashMap::new();
        let mut position_channels_corner2 = std::collections::HashMap::new();

        // Extract raw position values
        let x_raw = coerce_numeric_channel_with_renderer(self, data, scalars, "x", context, 0.0)?;
        let x2_raw = coerce_numeric_channel_with_renderer(self, data, scalars, "x2", context, 0.0)?;
        let y_raw = coerce_numeric_channel_with_renderer(self, data, scalars, "y", context, 0.0)?;
        let y2_raw = coerce_numeric_channel_with_renderer(self, data, scalars, "y2", context, 0.0)?;

        // Set up position channels for first corner (x, y)
        position_channels_corner1.insert("x", x_raw.clone());
        position_channels_corner1.insert("y", y_raw.clone());

        // Set up position channels for second corner (x2, y2)
        position_channels_corner2.insert("x", x2_raw.clone());
        position_channels_corner2.insert("y", y2_raw.clone());

        // Transform both corners through the coordinate system
        let geometry1 = coord.transform(
            &position_channels_corner1,
            None,
            context.plot_width,
            context.plot_height,
        )?;
        let geometry2 = coord.transform(
            &position_channels_corner2,
            None,
            context.plot_width,
            context.plot_height,
        )?;

        // Extract transformed coordinates as PointGeometry
        let point1 = geometry1
            .as_any()
            .downcast_ref::<crate::coords::PointGeometry>()
            .ok_or_else(|| {
                AvengerChartError::CoordinateSystemError(
                    "Failed to downcast corner1 to PointGeometry".to_string(),
                )
            })?;
        let point2 = geometry2
            .as_any()
            .downcast_ref::<crate::coords::PointGeometry>()
            .ok_or_else(|| {
                AvengerChartError::CoordinateSystemError(
                    "Failed to downcast corner2 to PointGeometry".to_string(),
                )
            })?;

        // Use the transformed coordinates
        let x = point1.x.clone();
        let y = point1.y.clone();
        let x2 = point2.x.clone();
        let y2 = point2.y.clone();

        // Extract style values using Coercer with mark defaults
        let fill = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "fill",
            context,
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0], // Fallback steel blue
        )?;
        let stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke",
            context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let stroke_width = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke_width",
            context,
            1.0,
        )?;
        let corner_radius = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "corner_radius",
            context,
            0.0,
        )?;

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
            zindex: self.state.zindex,
        };

        Ok(vec![SceneMark::Rect(rect_mark)])
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        crate::marks::rect::rect_channel_defaults(channel)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &datafusion::arrow::datatypes::DataType,
    ) -> Option<Box<dyn crate::scales::ScaleSpec>> {
        use crate::scales::spec::{Band, Ordinal};
        use datafusion::arrow::datatypes::DataType;

        match (channel, data_type) {
            // Rect marks use band scales for categorical position data
            (
                "x" | "x2" | "y" | "y2",
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            ) => Some(Box::new(Band::default())),
            // Color channels use ordinal scales for categorical data
            (
                "fill" | "stroke" | "color",
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            ) => Some(Box::new(Ordinal::default())),
            // Fall back to data type-based inference for other channels
            _ => crate::marks::default_scale_for_data_type(data_type),
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<std::sync::Arc<dyn crate::legend::LegendRenderer>> {
        use crate::legend::{CompiledColorbar, renderer::rect::CompiledRectLegend};
        use crate::marks::util::is_continuous_scale;
        use std::sync::Arc;

        // Check if scale is continuous (for colorbar)
        let is_continuous = is_continuous_scale(scale.scale_impl.as_ref());

        match channel {
            // Use colorbar for continuous color scales
            "fill" | "stroke" | "color" if is_continuous => Some(Arc::new(CompiledColorbar::new())),
            // Rect marks use CompiledRectLegend for discrete scales and other visual properties
            "fill" | "stroke" | "color" | "opacity" | "stroke_width" => {
                Some(Arc::new(CompiledRectLegend::new()))
            }
            // No legend for position channels and other non-visual channels
            "x" | "y" | "x2" | "y2" | "width" | "height" | "defined" | "order"
            | "corner_radius" => None,
            // For any other channel, default to CompiledRectLegend
            _ => Some(Arc::new(CompiledRectLegend::new())),
        }
    }
}
