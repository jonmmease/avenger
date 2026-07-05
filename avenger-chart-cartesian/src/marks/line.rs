use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererKind, LegendRendererSelection,
    Mark, MarkAdjustmentSpec, MarkEvaluationFrame, MarkRenderContext, MarkRuntimeContext,
    PointGeometry, PrimitiveMarkEffects, RadiusExpression, RenderedMarkData, ScaleTypePreference,
    apply_opacity_to_color, coerce_bool_channel_with_renderer, coerce_color_channel_with_renderer,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel_with_renderer,
    coerce_stroke_cap_channel_values_with_renderer, coerce_stroke_dash_channel,
    coerce_stroke_join_channel_values_with_renderer, default_scale_type_for_data_type,
    evaluate_item_assignments, impl_mark_trait_common, is_continuous_scale, item_bbox_column_name,
    item_channel_column_name, item_data_column_name, serialization::DefaultLogicalExprNodeExt,
};
use avenger_chart_marks::{Line, line_channel_defaults};
use avenger_color::ColorOrGradient;
use avenger_common::types::{StrokeCap, StrokeJoin};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::{line::SceneLineMark, mark::SceneMark};
use datafusion::{
    arrow::{
        array::{ArrayRef, BooleanArray, Float32Array, RecordBatch, StringArray},
        datatypes::{DataType, Field},
    },
    common::ScalarValue,
    logical_expr::{Expr, lit},
};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    Cartesian,
    marks::{detail::DetailColumns, util},
};

pub use avenger_chart_marks::ensure_dictionary_array as ensure_dictionary_array_fn;

// Implement Mark trait for Cartesian Line with any axis type
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<Cartesian> for Line<Cartesian> {
    impl_mark_trait_common!(Line);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianLine {
            state: compiled_state,
            effects: self.mark_effects().clone(),
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianLine {
    pub(crate) state: CompiledMarkState,
    #[serde(default)]
    pub(crate) effects: PrimitiveMarkEffects,
}

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
struct LineRenderPartitionKey {
    details: Vec<ScalarValue>,
    stroke: String,
    stroke_width_bits: u32,
    stroke_dash: String,
    opacity_bits: u32,
    stroke_cap: String,
    stroke_join: String,
}

impl CompiledMarkCore for CompiledCartesianLine {
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
        "line"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            // Position channels
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
            // Style channels
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
                name: "opacity",
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
                name: "stroke_join",
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
                name: "order",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn supports_order(&self) -> bool {
        true
    }

    fn details_partition_continuous_geometry(&self) -> bool {
        true
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        line_channel_defaults(channel)
    }

    fn wants_full_data_batch(&self) -> bool {
        self.effects.requires_data_batch() || !self.effects.adjustments.is_empty()
    }

    fn radius_expression(
        &self,
        dimension: &str,
        resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        match dimension {
            "y" => {
                let stroke_width_expr = resolve_channel("stroke_width");
                let radius_expr = stroke_width_expr * lit(2.0);
                let radius_expr_node = LogicalExprNode::from_default_expr(radius_expr)
                    .expect("Failed to serialize expr");
                Some(RadiusExpression::Symmetric(radius_expr_node))
            }
            "x" => None,
            _ => None,
        }
    }

    fn preferred_scale_type(
        &self,
        _channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        default_scale_type_for_data_type(data_type)
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        // Check if scale is continuous (for colorbar)
        let is_continuous = is_continuous_scale(scale.scale_impl.as_ref());

        match channel {
            // Use colorbar for continuous color scales
            "stroke" if is_continuous => Some(LegendRendererSelection::BuiltIn(
                LegendRendererKind::Colorbar,
            )),
            // Line marks use line legend for stroke properties
            "stroke" | "stroke_width" | "stroke_dash" | "opacity" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line))
            }
            // No legend for position channels
            "x" | "y" | "defined" | "order" | "stroke_cap" | "stroke_join" | "interpolate" => None,
            // For any other channel, default to line legend rendering.
            _ => Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line)),
        }
    }
}

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl CompiledMark for CompiledCartesianLine {
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
        // For lines, we need array data for positions
        let data = data.ok_or_else(|| {
            AvengerChartError::InternalError(
                "Line mark requires array data for x and y positions".to_string(),
            )
        })?;

        let mark_context = context.core_view();
        let len = data.num_rows();

        // Extract position channels
        let mut position_channels = std::collections::HashMap::new();
        for channel_name in coord.required_channels() {
            let value = coerce_numeric_channel_with_renderer(
                self,
                Some(data),
                scalars,
                channel_name,
                &mark_context,
                0.0,
            )?;
            position_channels.insert(*channel_name, value);
        }

        // Transform position channels to plot coordinates
        let geometry = coord.transform(
            &position_channels,
            None,
            context.plot_width(),
            context.plot_height(),
        )?;
        let geometry = geometry
            .as_any()
            .downcast_ref::<PointGeometry>()
            .ok_or_else(|| {
                AvengerChartError::CoordinateSystemError(
                    "Failed to downcast to PointGeometry".to_string(),
                )
            })?;

        let stroke_width = coerce_numeric_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "stroke_width",
            &mark_context,
            2.0,
        )?;
        let visual = self.coerce_line_visual_channels(Some(data), scalars, &mark_context, len)?;
        let (x, y, visual) = self.apply_expression_adjustments(
            geometry.x.clone(),
            geometry.y.clone(),
            LineVisualChannels {
                stroke_width,
                ..visual
            },
            Some(data),
            len,
            context,
            &mark_context,
        )?;

        let detail_columns = DetailColumns::from_mark_data(self, data)?;

        // Build partition map
        let mut partition_groups: IndexMap<LineRenderPartitionKey, Vec<usize>> = IndexMap::new();
        let stroke_values = visual.stroke.as_vec(len, None);
        let stroke_strings = util::color_channel_strings(&visual.stroke, len);
        let stroke_width_values = visual.stroke_width.as_vec(len, None);
        let opacity_values = visual.opacity.as_vec(len, None);

        for i in 0..len {
            let key = LineRenderPartitionKey {
                details: detail_columns.key_for_row(i)?,
                stroke: stroke_strings[i].clone(),
                stroke_width_bits: stroke_width_values[i].to_bits(),
                stroke_dash: visual.stroke_dash_strings[i].clone(),
                opacity_bits: opacity_values[i].clamp(0.0, 1.0).to_bits(),
                stroke_cap: visual.stroke_cap_strings[i].clone(),
                stroke_join: visual.stroke_join_strings[i].clone(),
            };
            partition_groups.entry(key).or_default().push(i);
        }

        // Create a line mark for each partition
        let mut scene_marks = Vec::new();
        let mut source_row_indices = Vec::new();

        for (partition_key, indices) in partition_groups {
            if indices.is_empty() {
                continue;
            }
            let first_index = indices[0];

            // Get the values for this partition
            let stroke_color = stroke_values[first_index].clone();
            let opacity_value = f32::from_bits(partition_key.opacity_bits);
            let stroke_color = apply_opacity_to_color(&stroke_color, opacity_value);
            let stroke_dash_value = stroke_dash_from_name(&partition_key.stroke_dash)?;
            let stroke_cap = util::coerce_stroke_cap_strings(
                std::slice::from_ref(&partition_key.stroke_cap),
                "stroke_cap",
            )?
            .first()
            .copied()
            .unwrap_or(StrokeCap::Round);
            let stroke_join = util::coerce_stroke_join_strings(
                std::slice::from_ref(&partition_key.stroke_join),
                "stroke_join",
            )?
            .first()
            .copied()
            .unwrap_or(StrokeJoin::Miter);

            let line_mark = SceneLineMark {
                name: "line".to_string(),
                clip: true,
                len: indices.len() as u32,
                x: util::gather_by_indices(&x, len, &indices),
                y: util::gather_by_indices(&y, len, &indices),
                gradients: vec![],
                stroke: stroke_color,
                stroke_width: f32::from_bits(partition_key.stroke_width_bits),
                stroke_dash: stroke_dash_value,
                stroke_cap,
                stroke_join,
                defined: util::gather_by_indices(&visual.defined, len, &indices),
                zindex: self.state.zindex,
                interactive: true,
            };

            scene_marks.push(SceneMark::Line(line_mark));
            source_row_indices.push(indices);
        }

        Ok(RenderedMarkData::with_source_row_indices(
            scene_marks,
            source_row_indices,
        ))
    }
}

impl CompiledCartesianLine {
    fn coerce_line_visual_channels(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &MarkRenderContext<'_>,
        len: usize,
    ) -> Result<LineVisualChannels, AvengerChartError> {
        let stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke",
            context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let opacity =
            coerce_opacity_channel_with_renderer(self, data, scalars, "opacity", context, 1.0)?;
        let defined =
            coerce_bool_channel_with_renderer(self, data, scalars, "defined", context, true)?;
        let stroke_dash =
            util::optional_stroke_dash(coerce_stroke_dash_channel(data, scalars, "stroke_dash")?);
        let stroke_cap = coerce_stroke_cap_channel_values_with_renderer(
            self,
            data,
            scalars,
            "stroke_cap",
            context,
            StrokeCap::Round,
        )?;
        let stroke_join = coerce_stroke_join_channel_values_with_renderer(
            self,
            data,
            scalars,
            "stroke_join",
            context,
            StrokeJoin::Miter,
        )?;
        Ok(LineVisualChannels {
            stroke,
            stroke_width: ScalarOrArray::new_scalar(2.0),
            stroke_dash_strings: util::stroke_dash_strings(&stroke_dash, len),
            stroke_cap_strings: util::stroke_cap_strings(&stroke_cap, len),
            stroke_join_strings: util::stroke_join_strings(&stroke_join, len),
            opacity,
            defined,
        })
    }

    fn apply_expression_adjustments(
        &self,
        mut x: ScalarOrArray<f32>,
        mut y: ScalarOrArray<f32>,
        mut visual: LineVisualChannels,
        data: Option<&RecordBatch>,
        len: usize,
        runtime_context: &dyn MarkRuntimeContext,
        context: &MarkRenderContext<'_>,
    ) -> Result<(ScalarOrArray<f32>, ScalarOrArray<f32>, LineVisualChannels), AvengerChartError>
    {
        if self.effects.adjustments.is_empty() {
            return Ok((x, y, visual));
        }

        for adjustment in &self.effects.adjustments {
            let mut frame = build_line_item_frame(&x, &y, &visual, data, len)?;
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
                    "x" | "y" | "stroke" | "stroke_width" | "stroke_dash" | "stroke_cap"
                    | "stroke_join" | "opacity" | "defined" => {}
                    channel => {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "Line<Cartesian> adjustment channel '{channel}' is not implemented yet"
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
            visual.stroke = util::coerce_color_strings(
                &frame.string_values(&item_channel_column_name("stroke"))?,
                "stroke",
            )?;
            visual.stroke_width = ScalarOrArray::new_array(
                frame.f32_values(&item_channel_column_name("stroke_width"))?,
            );
            visual.stroke_dash_strings =
                frame.string_values(&item_channel_column_name("stroke_dash"))?;
            visual.stroke_cap_strings =
                frame.string_values(&item_channel_column_name("stroke_cap"))?;
            visual.stroke_join_strings =
                frame.string_values(&item_channel_column_name("stroke_join"))?;
            visual.opacity =
                ScalarOrArray::new_array(frame.f32_values(&item_channel_column_name("opacity"))?);
            visual.defined =
                ScalarOrArray::new_array(frame.bool_values(&item_channel_column_name("defined"))?);
        }

        Ok((x, y, visual))
    }
}

#[derive(Clone)]
struct LineVisualChannels {
    stroke: ScalarOrArray<ColorOrGradient>,
    stroke_width: ScalarOrArray<f32>,
    stroke_dash_strings: Vec<String>,
    stroke_cap_strings: Vec<String>,
    stroke_join_strings: Vec<String>,
    opacity: ScalarOrArray<f32>,
    defined: ScalarOrArray<bool>,
}

fn build_line_item_frame(
    x: &ScalarOrArray<f32>,
    y: &ScalarOrArray<f32>,
    visual: &LineVisualChannels,
    data: Option<&RecordBatch>,
    len: usize,
) -> Result<MarkEvaluationFrame, AvengerChartError> {
    let x_values = x.as_vec(len, None);
    let y_values = y.as_vec(len, None);
    let stroke_values = util::color_channel_strings(&visual.stroke, len);
    let stroke_width_values = visual.stroke_width.as_vec(len, None);
    let opacity_values = visual.opacity.as_vec(len, None);
    let defined_values = visual.defined.as_vec(len, None);
    let mut columns = vec![
        f32_item_column("x", x_values.clone()),
        f32_item_column("y", y_values.clone()),
        string_item_column("stroke", stroke_values),
        f32_item_column("stroke_width", stroke_width_values),
        string_item_column("stroke_dash", visual.stroke_dash_strings.clone()),
        string_item_column("stroke_cap", visual.stroke_cap_strings.clone()),
        string_item_column("stroke_join", visual.stroke_join_strings.clone()),
        f32_item_column("opacity", opacity_values),
        bool_item_column("defined", defined_values),
        (
            Field::new(item_bbox_column_name("left"), DataType::Float32, true),
            Arc::new(Float32Array::from(x_values.clone())) as ArrayRef,
        ),
        (
            Field::new(item_bbox_column_name("right"), DataType::Float32, true),
            Arc::new(Float32Array::from(x_values)) as ArrayRef,
        ),
        (
            Field::new(item_bbox_column_name("top"), DataType::Float32, true),
            Arc::new(Float32Array::from(y_values.clone())) as ArrayRef,
        ),
        (
            Field::new(item_bbox_column_name("bottom"), DataType::Float32, true),
            Arc::new(Float32Array::from(y_values)) as ArrayRef,
        ),
    ];

    if let Some(data) = data {
        if data.num_rows() != len {
            return Err(AvengerChartError::InternalError(format!(
                "Line adjustment data row count {} did not match vertex count {len}",
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

fn bool_item_column(channel: &str, values: Vec<bool>) -> (Field, ArrayRef) {
    (
        Field::new(item_channel_column_name(channel), DataType::Boolean, true),
        Arc::new(BooleanArray::from(values)) as ArrayRef,
    )
}

fn string_item_column(channel: &str, values: Vec<String>) -> (Field, ArrayRef) {
    (
        Field::new(item_channel_column_name(channel), DataType::Utf8, true),
        Arc::new(StringArray::from(values)) as ArrayRef,
    )
}

fn stroke_dash_from_name(value: &str) -> Result<Option<Vec<f32>>, AvengerChartError> {
    Ok(
        util::coerce_stroke_dash_strings(&[value.to_string()], "stroke_dash")?
            .and_then(|dash| dash.first().cloned())
            .filter(|dash| !dash.is_empty() && dash.as_slice() != [f32::MAX]),
    )
}
