use std::sync::Arc;

use arrow::array::RecordBatch;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{mark::SceneMark, rect::SceneRectMark};
use datafusion::arrow::datatypes::DataType;
use datafusion_common::ScalarValue;
use serde::{Deserialize, Serialize};

pub use avenger_chart_cartesian::CartesianRectPositionChannels;

use crate::{
    cartesian::Cartesian,
    channel::ChannelDescriptor,
    chart_core::{
        LegendRendererKind, ScaleTypePreference, coerce_color_channel_with_renderer,
        coerce_numeric_channel_with_renderer, default_scale_type_for_data_type,
        is_continuous_scale,
    },
    coords::{CoordinateSystemTransformCore, PointGeometry},
    error::AvengerChartError,
    impl_mark_trait_common,
    marks::{
        CompiledDataContext, CompiledMark, CompiledMarkCore, CompiledMarkState, Mark,
        rect::{Rect, rect_channel_defaults},
    },
    render::RenderContext,
};

// Implement Mark trait for Cartesian Rect with any axis type
#[async_trait::async_trait]
impl Mark<Cartesian> for Rect<Cartesian> {
    impl_mark_trait_common!(Rect);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianRect {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianRect {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledCartesianRect {
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

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        rect_channel_defaults(channel)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        match (channel, data_type) {
            // Rect marks use band scales for categorical position data
            (
                "x" | "x2" | "y" | "y2",
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            ) => Some(ScaleTypePreference::Band),
            // Color channels use ordinal scales for categorical data
            (
                "fill" | "stroke" | "color",
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            ) => Some(ScaleTypePreference::Ordinal),
            // Fall back to data type-based inference for other channels
            _ => default_scale_type_for_data_type(data_type),
        }
    }

    fn preferred_legend_renderer_kind(
        &self,
        channel: &str,
        scale: &ConfiguredScale,
    ) -> Option<LegendRendererKind> {
        // Check if scale is continuous (for colorbar)
        let is_continuous = is_continuous_scale(scale.scale_impl.as_ref());

        match channel {
            // Use colorbar for continuous color scales
            "fill" | "stroke" | "color" if is_continuous => Some(LegendRendererKind::Colorbar),
            // Rect marks use rect legend rendering for discrete scales and other visual properties.
            "fill" | "stroke" | "color" | "opacity" | "stroke_width" => {
                Some(LegendRendererKind::Rect)
            }
            // No legend for position channels and other non-visual channels
            "x" | "y" | "x2" | "y2" | "width" | "height" | "defined" | "order"
            | "corner_radius" => None,
            // For any other channel, default to rect legend rendering.
            _ => Some(LegendRendererKind::Rect),
        }
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianRect {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &RenderContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mark_context = context.core_view();

        // Determine number of marks from data batch or default to 1
        let len = data.map_or(1, |data| data.num_rows()) as u32;

        // Extract position channels for the coordinate system
        // For Cartesian rectangles, we need to transform both corners
        let mut position_channels_corner1 = std::collections::HashMap::new();
        let mut position_channels_corner2 = std::collections::HashMap::new();

        // Extract raw position values
        let x_raw =
            coerce_numeric_channel_with_renderer(self, data, scalars, "x", &mark_context, 0.0)?;
        let x2_raw =
            coerce_numeric_channel_with_renderer(self, data, scalars, "x2", &mark_context, 0.0)?;
        let y_raw =
            coerce_numeric_channel_with_renderer(self, data, scalars, "y", &mark_context, 0.0)?;
        let y2_raw =
            coerce_numeric_channel_with_renderer(self, data, scalars, "y2", &mark_context, 0.0)?;

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
            context.plot_width(),
            context.plot_height(),
        )?;
        let geometry2 = coord.transform(
            &position_channels_corner2,
            None,
            context.plot_width(),
            context.plot_height(),
        )?;

        // Extract transformed coordinates as PointGeometry
        let point1 = geometry1
            .as_any()
            .downcast_ref::<PointGeometry>()
            .ok_or_else(|| {
                AvengerChartError::CoordinateSystemError(
                    "Failed to downcast corner1 to PointGeometry".to_string(),
                )
            })?;
        let point2 = geometry2
            .as_any()
            .downcast_ref::<PointGeometry>()
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
            &mark_context,
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0], // Fallback steel blue
        )?;
        let stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke",
            &mark_context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let stroke_width = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke_width",
            &mark_context,
            1.0,
        )?;
        let corner_radius = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "corner_radius",
            &mark_context,
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
}
