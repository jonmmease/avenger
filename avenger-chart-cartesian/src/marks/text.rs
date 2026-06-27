use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererSelection, Mark,
    MarkRuntimeContext, PrimitiveMarkEffects, RenderedMarkData, ScaleTypePreference,
    default_scale_type_for_data_type, impl_mark_trait_common, is_continuous_scale,
    text_rendering::{apply_text_adjustments, build_scene_text_mark},
};
use avenger_chart_marks::{Text, text_channel_defaults};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::marks::{mark::SceneMark, text::SceneTextMark};
use avenger_text::types::TextSyntaxMode;
use datafusion::{
    arrow::{array::RecordBatch, datatypes::DataType},
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
            effects: self.mark_effects().clone(),
            syntax_mode: self.text_syntax_mode(),
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianText {
    pub(crate) state: CompiledMarkState,
    #[serde(default)]
    pub(crate) effects: PrimitiveMarkEffects,
    #[serde(default)]
    pub(crate) syntax_mode: TextSyntaxMode,
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
            ChannelDescriptor {
                name: "defined",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_offset_x",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_offset_y",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_stroke",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_stroke_width",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_stroke_dash",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_stroke_cap",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_stroke_join",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_label_padding",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_target_radius",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_min_length",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_shape",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_arrow",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_arrow_length",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "leader_arrow_width",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        text_channel_defaults(channel)
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
            ("x" | "y", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Point)
            }
            ("color", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
            ("leader_stroke", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
            (
                "text"
                | "font"
                | "align"
                | "baseline"
                | "font_weight"
                | "font_style"
                | "defined"
                | "leader"
                | "leader_offset_x"
                | "leader_offset_y"
                | "leader_label_padding"
                | "leader_target_radius"
                | "leader_min_length"
                | "leader_shape"
                | "leader_arrow"
                | "leader_arrow_length"
                | "leader_arrow_width"
                | "leader_stroke_cap"
                | "leader_stroke_join",
                _,
            ) => None,
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
        if matches!(channel, "color" | "leader_stroke") && is_continuous_scale(scale_impl) {
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
        self.render_mark_data(data, scalars, context, coord)
            .await
            .map(|rendered| rendered.marks)
    }

    async fn render_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        let mark = self.render_text_scene(data, scalars, context, coord)?;
        Ok(RenderedMarkData::new(vec![mark.into()]))
    }
}

impl CompiledCartesianText {
    pub(crate) fn render_text_scene(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<SceneTextMark, AvengerChartError> {
        let mark_context = context.core_view();
        let len = util::scene_len(data);
        let position = util::transform_cartesian_point_channels(
            self, data, scalars, context, coord, "x", "y",
        )?;

        let mut mark = build_scene_text_mark(
            self,
            data,
            scalars,
            &mark_context,
            position.x,
            position.y,
            len,
            self.state.zindex,
            true,
        )?;
        mark.text_syntax = self.syntax_mode;
        apply_text_adjustments(
            self,
            mark,
            data,
            None,
            context,
            &self.effects,
            self.state.zindex,
        )
    }
}
