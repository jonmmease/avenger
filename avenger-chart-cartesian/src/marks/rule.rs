use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererKind, LegendRendererSelection,
    Mark, MarkRuntimeContext, RadiusExpression, ScaleTypePreference,
    apply_opacity_to_color_channel, coerce_color_channel_with_renderer,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel_with_renderer,
    coerce_stroke_cap_channel_values_with_renderer, coerce_stroke_dash_channel,
    default_scale_type_for_data_type, impl_mark_trait_common, is_continuous_scale,
    serialization::DefaultLogicalExprNodeExt,
};
use avenger_chart_marks::{Rule, rule_channel_defaults};
use avenger_common::types::StrokeCap;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{mark::SceneMark, rule::SceneRuleMark};
use datafusion::{
    arrow::array::RecordBatch,
    arrow::datatypes::DataType,
    common::ScalarValue,
    logical_expr::{Expr, lit},
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};

use crate::{Cartesian, marks::util};

#[async_trait::async_trait]
impl Mark<Cartesian> for Rule<Cartesian> {
    impl_mark_trait_common!(Rule);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianRule {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianRule {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledCartesianRule {
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
        "rule"
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
                name: "x2",
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
                name: "stroke_dash",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke_cap",
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
        rule_channel_defaults(channel)
    }

    fn radius_expression(
        &self,
        dimension: &str,
        resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        match dimension {
            "x" | "y" => {
                let stroke_width_expr = resolve_channel("stroke_width");
                let radius_expr = stroke_width_expr / lit(2.0);
                let radius_expr_node = LogicalExprNode::from_default_expr(radius_expr)
                    .expect("Failed to serialize expr");
                Some(RadiusExpression::Symmetric(radius_expr_node))
            }
            _ => None,
        }
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        match (channel, data_type) {
            (
                "x" | "x2" | "y" | "y2",
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            ) => Some(ScaleTypePreference::Point),
            ("stroke", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
            _ => default_scale_type_for_data_type(data_type),
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        match channel {
            "stroke" if is_continuous_scale(scale.scale_impl.as_ref()) => Some(
                LegendRendererSelection::BuiltIn(LegendRendererKind::Colorbar),
            ),
            "stroke" | "stroke_width" | "stroke_dash" | "opacity" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line))
            }
            "x" | "y" | "x2" | "y2" | "stroke_cap" => None,
            _ => Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line)),
        }
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianRule {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let mark_context = context.core_view();
        let len = util::scene_len(data);
        let start = util::transform_cartesian_point_channels(
            self, data, scalars, context, coord, "x", "y",
        )?;
        let end = util::transform_cartesian_point_channels(
            self, data, scalars, context, coord, "x2", "y2",
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
        let stroke_dash =
            util::optional_stroke_dash(coerce_stroke_dash_channel(data, scalars, "stroke_dash")?);
        let stroke_cap = coerce_stroke_cap_channel_values_with_renderer(
            self,
            data,
            scalars,
            "stroke_cap",
            &mark_context,
            StrokeCap::Butt,
        )?;
        let opacity = coerce_opacity_channel_with_renderer(
            self,
            data,
            scalars,
            "opacity",
            &mark_context,
            1.0,
        )?;
        let stroke = apply_opacity_to_color_channel(stroke, &opacity, len as usize);

        Ok(vec![SceneMark::Rule(SceneRuleMark {
            name: "rule".to_string(),
            clip: true,
            len,
            gradients: vec![],
            stroke_dash,
            x: start.x,
            y: start.y,
            x2: end.x,
            y2: end.y,
            stroke,
            stroke_width,
            stroke_cap,
            indices: None,
            zindex: self.state.zindex,
            interactive: true,
        })])
    }
}
