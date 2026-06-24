use std::{collections::HashMap, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererKind, LegendRendererSelection,
    Mark, MarkRenderContext, MarkRuntimeContext, PointGeometry, PrimitiveMarkEffects,
    RenderedMarkData, ScaleTypePreference, apply_opacity_to_color_channel,
    coerce_color_channel_with_renderer, coerce_numeric_channel_with_renderer,
    coerce_opacity_channel_with_renderer, default_scale_type_for_data_type, impl_mark_trait_common,
    is_continuous_scale,
};
use avenger_chart_marks::{Rect, rect_channel_defaults};
use avenger_color::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{mark::SceneMark, rect::SceneRectMark};
use datafusion::{
    arrow::{datatypes::DataType, record_batch::RecordBatch},
    common::ScalarValue,
};
use serde::{Deserialize, Serialize};

use crate::WebMercator;

#[async_trait::async_trait]
impl Mark<WebMercator> for Rect<WebMercator> {
    impl_mark_trait_common!(Rect);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledWebMercatorRect {
            state: compiled_state,
            effects: self.mark_effects().clone(),
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledWebMercatorRect {
    pub(crate) state: CompiledMarkState,
    #[serde(default)]
    pub(crate) effects: PrimitiveMarkEffects,
}

impl CompiledMarkCore for CompiledWebMercatorRect {
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

    fn wants_full_data_batch(&self) -> bool {
        self.effects.requires_data_batch()
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        match (channel, data_type) {
            (
                "fill" | "stroke" | "color",
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            ) => Some(ScaleTypePreference::Ordinal),
            _ => default_scale_type_for_data_type(data_type),
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        let is_continuous = is_continuous_scale(scale.scale_impl.as_ref());
        match channel {
            "fill" | "stroke" | "color" if is_continuous => Some(LegendRendererSelection::BuiltIn(
                LegendRendererKind::Colorbar,
            )),
            "fill" | "stroke" | "color" | "opacity" | "stroke_width" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Rect))
            }
            _ => None,
        }
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledWebMercatorRect {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        self.render_rect_scene(data, scalars, context, coord)
            .map(|mark| vec![mark])
    }

    async fn render_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        self.render_from_data(data, scalars, context, coord)
            .await
            .map(RenderedMarkData::new)
    }
}

impl CompiledWebMercatorRect {
    fn render_rect_scene(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<SceneMark, AvengerChartError> {
        if !self.effects.is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "Rect<WebMercator> does not support render-stage adjustments or derived marks yet"
                    .to_string(),
            ));
        }

        let mark_context = context.core_view();
        let len = data.map_or(1, RecordBatch::num_rows);

        let x_raw =
            coerce_numeric_channel_with_renderer(self, data, scalars, "x", &mark_context, 0.0)?;
        let x2_raw =
            coerce_numeric_channel_with_renderer(self, data, scalars, "x2", &mark_context, 0.0)?;
        let y_raw =
            coerce_numeric_channel_with_renderer(self, data, scalars, "y", &mark_context, 0.0)?;
        let y2_raw =
            coerce_numeric_channel_with_renderer(self, data, scalars, "y2", &mark_context, 0.0)?;

        let point1 = transform_rect_corner(coord, context, x_raw, y_raw)?;
        let point2 = transform_rect_corner(coord, context, x2_raw, y2_raw)?;
        let visual = self.coerce_rect_visual_channels(data, scalars, &mark_context)?;
        let fill = apply_opacity_to_color_channel(visual.fill, &visual.opacity, len);
        let stroke = apply_opacity_to_color_channel(visual.stroke, &visual.opacity, len);
        let corner_radius = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "corner_radius",
            &mark_context,
            0.0,
        )?;

        Ok(SceneMark::Rect(SceneRectMark {
            name: "rect".to_string(),
            interactive: true,
            clip: true,
            len: len as u32,
            gradients: vec![],
            x: point1.x,
            y: point1.y,
            width: None,
            height: None,
            x2: Some(point2.x),
            y2: Some(point2.y),
            fill,
            stroke,
            stroke_width: visual.stroke_width,
            corner_radius,
            indices: None,
            zindex: self.state.zindex,
        }))
    }

    fn coerce_rect_visual_channels(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        mark_context: &MarkRenderContext<'_>,
    ) -> Result<RectVisualChannels, AvengerChartError> {
        let fill = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "fill",
            mark_context,
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
        )?;
        let stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke",
            mark_context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let stroke_width = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke_width",
            mark_context,
            1.0,
        )?;
        let opacity = coerce_opacity_channel_with_renderer(
            self,
            data,
            scalars,
            "opacity",
            mark_context,
            1.0,
        )?;
        Ok(RectVisualChannels {
            fill,
            stroke,
            stroke_width,
            opacity,
        })
    }
}

fn transform_rect_corner(
    coord: &dyn CoordinateSystemTransformCore,
    context: &dyn MarkRuntimeContext,
    x: ScalarOrArray<f32>,
    y: ScalarOrArray<f32>,
) -> Result<PointGeometry, AvengerChartError> {
    let mut position_channels = HashMap::new();
    position_channels.insert("x", x);
    position_channels.insert("y", y);
    let geometry = coord.transform(
        &position_channels,
        None,
        context.plot_width(),
        context.plot_height(),
    )?;
    geometry
        .as_any()
        .downcast_ref::<PointGeometry>()
        .cloned()
        .ok_or_else(|| {
            AvengerChartError::CoordinateSystemError(
                "Failed to downcast transformed WebMercator rect corner to PointGeometry"
                    .to_string(),
            )
        })
}

#[derive(Clone)]
struct RectVisualChannels {
    fill: ScalarOrArray<ColorOrGradient>,
    stroke: ScalarOrArray<ColorOrGradient>,
    stroke_width: ScalarOrArray<f32>,
    opacity: ScalarOrArray<f32>,
}
