use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererKind, LegendRendererSelection,
    Mark, MarkAdjustmentSpec, MarkEvaluationFrame, MarkRuntimeContext, PrimitiveMarkEffects,
    RadiusExpression, ScaleTypePreference, apply_opacity_to_color_channel,
    coerce_color_channel_with_renderer, coerce_numeric_channel_with_renderer,
    coerce_opacity_channel_with_renderer, coerce_stroke_cap_channel_values_with_renderer,
    coerce_stroke_dash_channel, default_scale_type_for_data_type, evaluate_item_assignments,
    impl_mark_trait_common, is_continuous_scale, item_bbox_column_name, item_channel_column_name,
    item_data_column_name, serialization::DefaultLogicalExprNodeExt,
};
use avenger_chart_marks::{Rule, rule_channel_defaults};
use avenger_color::ColorOrGradient;
use avenger_common::types::StrokeCap;
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{mark::SceneMark, rule::SceneRuleMark};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float32Array, RecordBatch, StringArray},
        datatypes::{DataType, Field},
    },
    common::ScalarValue,
    logical_expr::{Expr, lit},
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};

use crate::{Cartesian, marks::util};

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<Cartesian> for Rule<Cartesian> {
    impl_mark_trait_common!(Rule);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianRule {
            state: compiled_state,
            effects: self.mark_effects().clone(),
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianRule {
    pub(crate) state: CompiledMarkState,
    pub(crate) effects: PrimitiveMarkEffects,
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

    fn wants_full_data_batch(&self) -> bool {
        self.effects.requires_data_batch()
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
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
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
        let stroke_width = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke_width",
            &mark_context,
            1.0,
        )?;
        let visual = self.coerce_rule_visual_channels(data, scalars, &mark_context)?;
        let (x, y, x2, y2, stroke_width, visual) = self.apply_expression_adjustments(
            start.x,
            start.y,
            end.x,
            end.y,
            stroke_width,
            visual,
            data,
            len,
            context,
            &mark_context,
        )?;

        let stroke = apply_opacity_to_color_channel(visual.stroke, &visual.opacity, len as usize);

        Ok(vec![SceneMark::Rule(SceneRuleMark {
            name: "rule".to_string(),
            clip: true,
            len,
            gradients: vec![],
            stroke_dash: visual.stroke_dash,
            x,
            y,
            x2,
            y2,
            stroke,
            stroke_width,
            stroke_cap: visual.stroke_cap,
            indices: None,
            zindex: self.state.zindex,
            interactive: true,
        })])
    }
}

impl CompiledCartesianRule {
    fn coerce_rule_visual_channels(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        mark_context: &avenger_chart_core::MarkRenderContext<'_>,
    ) -> Result<RuleVisualChannels, AvengerChartError> {
        let stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke",
            mark_context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let stroke_dash =
            util::optional_stroke_dash(coerce_stroke_dash_channel(data, scalars, "stroke_dash")?);
        let stroke_cap = coerce_stroke_cap_channel_values_with_renderer(
            self,
            data,
            scalars,
            "stroke_cap",
            mark_context,
            StrokeCap::Butt,
        )?;
        let opacity = coerce_opacity_channel_with_renderer(
            self,
            data,
            scalars,
            "opacity",
            mark_context,
            1.0,
        )?;
        Ok(RuleVisualChannels {
            stroke,
            stroke_dash,
            stroke_cap,
            opacity,
        })
    }

    fn apply_expression_adjustments(
        &self,
        mut x: ScalarOrArray<f32>,
        mut y: ScalarOrArray<f32>,
        mut x2: ScalarOrArray<f32>,
        mut y2: ScalarOrArray<f32>,
        mut stroke_width: ScalarOrArray<f32>,
        mut visual: RuleVisualChannels,
        data: Option<&RecordBatch>,
        len: u32,
        runtime_context: &dyn MarkRuntimeContext,
        context: &avenger_chart_core::MarkRenderContext<'_>,
    ) -> Result<
        (
            ScalarOrArray<f32>,
            ScalarOrArray<f32>,
            ScalarOrArray<f32>,
            ScalarOrArray<f32>,
            ScalarOrArray<f32>,
            RuleVisualChannels,
        ),
        AvengerChartError,
    > {
        if self.effects.adjustments.is_empty() {
            return Ok((x, y, x2, y2, stroke_width, visual));
        }

        let len = len as usize;
        for adjustment in &self.effects.adjustments {
            let mut frame =
                build_rule_item_frame(&x, &y, &x2, &y2, &stroke_width, &visual, data, len)?;
            let assignments = match adjustment {
                MarkAdjustmentSpec::Expr(spec) => &spec.assignments,
                MarkAdjustmentSpec::Transform(spec) => {
                    let adjustment_context = util::adjustment_transform_context(runtime_context);
                    spec.transform.apply(&mut frame, &adjustment_context)?;
                    &spec.assignments
                }
            };
            if assignments.is_empty() {
                continue;
            }
            for assignment in assignments {
                match assignment.channel.as_str() {
                    "x" | "y" | "x2" | "y2" | "stroke_width" | "stroke" | "stroke_dash"
                    | "stroke_cap" | "opacity" => {}
                    channel => {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "Rule<Cartesian> adjustment channel '{channel}' is not implemented yet"
                        )));
                    }
                }
            }

            let item_batch = frame.record_batch()?;
            let output_batch = evaluate_item_assignments(
                assignments.iter(),
                &item_batch,
                context.session_context().as_ref(),
            )?;
            for (index, assignment) in assignments.iter().enumerate() {
                frame.set_column(
                    item_channel_column_name(&assignment.channel),
                    output_batch.column(index).clone(),
                )?;
            }
            x = ScalarOrArray::new_array(frame.f32_values(&item_channel_column_name("x"))?);
            y = ScalarOrArray::new_array(frame.f32_values(&item_channel_column_name("y"))?);
            x2 = ScalarOrArray::new_array(frame.f32_values(&item_channel_column_name("x2"))?);
            y2 = ScalarOrArray::new_array(frame.f32_values(&item_channel_column_name("y2"))?);
            stroke_width = ScalarOrArray::new_array(
                frame.f32_values(&item_channel_column_name("stroke_width"))?,
            );
            visual.stroke = util::coerce_color_strings(
                &frame.string_values(&item_channel_column_name("stroke"))?,
                "stroke",
            )?;
            visual.stroke_dash = util::coerce_stroke_dash_strings(
                &frame.string_values(&item_channel_column_name("stroke_dash"))?,
                "stroke_dash",
            )?;
            visual.stroke_cap = util::coerce_stroke_cap_strings(
                &frame.string_values(&item_channel_column_name("stroke_cap"))?,
                "stroke_cap",
            )?;
            visual.opacity =
                ScalarOrArray::new_array(frame.f32_values(&item_channel_column_name("opacity"))?);
        }

        Ok((x, y, x2, y2, stroke_width, visual))
    }
}

#[derive(Clone)]
struct RuleVisualChannels {
    stroke: ScalarOrArray<ColorOrGradient>,
    stroke_dash: Option<ScalarOrArray<Vec<f32>>>,
    stroke_cap: ScalarOrArray<StrokeCap>,
    opacity: ScalarOrArray<f32>,
}

fn build_rule_item_frame(
    x: &ScalarOrArray<f32>,
    y: &ScalarOrArray<f32>,
    x2: &ScalarOrArray<f32>,
    y2: &ScalarOrArray<f32>,
    stroke_width: &ScalarOrArray<f32>,
    visual: &RuleVisualChannels,
    data: Option<&RecordBatch>,
    len: usize,
) -> Result<MarkEvaluationFrame, AvengerChartError> {
    let x_values = x.as_vec(len, None);
    let y_values = y.as_vec(len, None);
    let x2_values = x2.as_vec(len, None);
    let y2_values = y2.as_vec(len, None);
    let stroke_width_values = stroke_width.as_vec(len, None);
    let stroke_values = util::color_channel_strings(&visual.stroke, len);
    let stroke_dash_values = util::stroke_dash_strings(&visual.stroke_dash, len);
    let stroke_cap_values = util::stroke_cap_strings(&visual.stroke_cap, len);
    let opacity_values = visual.opacity.as_vec(len, None);
    let mut left = Vec::with_capacity(len);
    let mut right = Vec::with_capacity(len);
    let mut top = Vec::with_capacity(len);
    let mut bottom = Vec::with_capacity(len);
    for (((x, y), x2), y2) in x_values
        .iter()
        .zip(&y_values)
        .zip(&x2_values)
        .zip(&y2_values)
    {
        left.push(x.min(*x2));
        right.push(x.max(*x2));
        top.push(y.min(*y2));
        bottom.push(y.max(*y2));
    }

    let mut columns = vec![
        f32_item_column("x", x_values),
        f32_item_column("y", y_values),
        f32_item_column("x2", x2_values),
        f32_item_column("y2", y2_values),
        f32_item_column("stroke_width", stroke_width_values),
        string_item_column("stroke", stroke_values),
        string_item_column("stroke_dash", stroke_dash_values),
        string_item_column("stroke_cap", stroke_cap_values),
        f32_item_column("opacity", opacity_values),
        (
            Field::new(item_bbox_column_name("left"), DataType::Float32, true),
            Arc::new(Float32Array::from(left)) as ArrayRef,
        ),
        (
            Field::new(item_bbox_column_name("right"), DataType::Float32, true),
            Arc::new(Float32Array::from(right)) as ArrayRef,
        ),
        (
            Field::new(item_bbox_column_name("top"), DataType::Float32, true),
            Arc::new(Float32Array::from(top)) as ArrayRef,
        ),
        (
            Field::new(item_bbox_column_name("bottom"), DataType::Float32, true),
            Arc::new(Float32Array::from(bottom)) as ArrayRef,
        ),
    ];

    if let Some(data) = data {
        if data.num_rows() != len {
            return Err(AvengerChartError::InternalError(format!(
                "Rule adjustment data row count {} did not match item count {len}",
                data.num_rows()
            )));
        }
        for (index, field) in data.schema().fields().iter().enumerate() {
            columns.push((
                Field::new(
                    item_data_column_name(field.name()),
                    field.data_type().clone(),
                    field.is_nullable(),
                ),
                data.column(index).clone(),
            ));
        }
    }

    Ok(MarkEvaluationFrame::new(len, columns))
}

fn f32_item_column(channel: &str, values: Vec<f32>) -> (Field, ArrayRef) {
    (
        Field::new(item_channel_column_name(channel), DataType::Float32, true),
        Arc::new(Float32Array::from(values)) as ArrayRef,
    )
}

fn string_item_column(channel: &str, values: Vec<String>) -> (Field, ArrayRef) {
    (
        Field::new(item_channel_column_name(channel), DataType::Utf8, true),
        Arc::new(StringArray::from(values)) as ArrayRef,
    )
}
