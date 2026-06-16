use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererSelection, Mark,
    MarkRuntimeContext, ScaleTypePreference, apply_opacity_to_color_channel,
    coerce_color_channel_with_renderer, coerce_font_style_channel, coerce_font_weight_channel,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel_with_renderer,
    coerce_text_align_channel, coerce_text_baseline_channel, coerce_text_channel,
    default_scale_type_for_data_type, impl_mark_trait_common, is_continuous_scale,
};
use avenger_chart_marks::{Text, text_channel_defaults};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::marks::{mark::SceneMark, text::SceneTextMark};
use avenger_text::types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline};
use datafusion::{
    arrow::array::RecordBatch,
    arrow::datatypes::DataType,
    common::ScalarValue,
    logical_expr::{Expr, lit},
};
use serde::{Deserialize, Serialize};

use crate::{Cartesian, marks::util};

#[async_trait::async_trait]
impl Mark<Cartesian> for Text<Cartesian> {
    impl_mark_trait_common!(Text);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianText {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianText {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledCartesianText {
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
        "text"
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
                name: "y",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "text",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "align",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "baseline",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "angle",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "color",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font_size",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font_weight",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "font_style",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "limit",
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
        text_channel_defaults(channel)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        if let Some(scale_type) = super::nested_position_scale_type(channel, data_type) {
            return Some(scale_type);
        }

        match (channel, data_type) {
            ("x" | "y", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Point)
            }
            ("color", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
            ("text" | "font" | "align" | "baseline" | "font_weight" | "font_style", _) => None,
            _ => default_scale_type_for_data_type(data_type),
        }
    }

    fn preferred_legend_renderer(
        &self,
        _channel: &str,
        _scale: &ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        None
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        _data_type: &DataType,
    ) -> std::collections::HashMap<String, Expr> {
        let mut options = std::collections::HashMap::new();
        if channel == "color" && is_continuous_scale(scale_impl) {
            options.insert("nice".to_string(), lit(true));
        }
        options
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianText {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mark_context = context.core_view();
        let len = util::scene_len(data);
        let position = util::transform_cartesian_point_channels(
            self, data, scalars, context, coord, "x", "y",
        )?;

        let text = coerce_text_channel(data, scalars, "text", String::new())?;
        let align = coerce_text_align_channel(data, scalars, "align", TextAlign::Left)?;
        let baseline =
            coerce_text_baseline_channel(data, scalars, "baseline", TextBaseline::Alphabetic)?;
        let angle =
            coerce_numeric_channel_with_renderer(self, data, scalars, "angle", &mark_context, 0.0)?;
        let color = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "color",
            &mark_context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let font = coerce_text_channel(data, scalars, "font", "sans-serif".to_string())?;
        let font_size = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "font_size",
            &mark_context,
            10.0,
        )?;
        let font_weight = coerce_font_weight_channel(
            data,
            scalars,
            "font_weight",
            FontWeight::Name(FontWeightNameSpec::Normal),
        )?;
        let font_style = coerce_font_style_channel(data, scalars, "font_style", FontStyle::Normal)?;
        let limit =
            coerce_numeric_channel_with_renderer(self, data, scalars, "limit", &mark_context, 0.0)?;
        let opacity = coerce_opacity_channel_with_renderer(
            self,
            data,
            scalars,
            "opacity",
            &mark_context,
            1.0,
        )?;
        let color = apply_opacity_to_color_channel(color, &opacity, len as usize);

        Ok(vec![
            SceneTextMark {
                name: "text".to_string(),
                clip: true,
                len,
                text,
                x: position.x,
                y: position.y,
                align,
                baseline,
                angle,
                color,
                font,
                font_size,
                font_weight,
                font_style,
                limit,
                indices: None,
                zindex: self.state.zindex,
                interactive: true,
            }
            .into(),
        ])
    }
}
