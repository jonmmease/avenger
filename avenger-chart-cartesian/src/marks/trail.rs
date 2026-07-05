use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererKind, LegendRendererSelection,
    Mark, MarkAdjustmentSpec, MarkEvaluationFrame, MarkRenderContext, MarkRuntimeContext,
    PrimitiveMarkEffects, RenderedMarkData, ResolvedDomain, ScaleRange, ScaleTypePreference, Theme,
    apply_opacity_to_color, coerce_bool_channel_with_renderer, coerce_color_channel_with_renderer,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel_with_renderer,
    default_scale_type_for_data_type, evaluate_item_assignments, impl_mark_trait_common,
    is_continuous_scale, item_bbox_column_name, item_channel_column_name, item_data_column_name,
};
use avenger_chart_marks::{Trail, trail_channel_defaults};
use avenger_color::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::marks::{mark::SceneMark, trail::SceneTrailMark};
use datafusion::{
    arrow::{
        array::{ArrayRef, BooleanArray, Float32Array, RecordBatch, StringArray},
        datatypes::{DataType, Field},
    },
    common::ScalarValue,
    logical_expr::lit,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    Cartesian,
    marks::{detail::DetailColumns, util},
};

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<Cartesian> for Trail<Cartesian> {
    impl_mark_trait_common!(Trail);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianTrail {
            state: compiled_state,
            effects: self.mark_effects().clone(),
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianTrail {
    pub(crate) state: CompiledMarkState,
    #[serde(default)]
    pub(crate) effects: PrimitiveMarkEffects,
}

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
struct TrailRenderPartitionKey {
    details: Vec<ScalarValue>,
    stroke: String,
    opacity_bits: u32,
}

impl CompiledMarkCore for CompiledCartesianTrail {
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
        "trail"
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
                name: "size",
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
        trail_channel_defaults(channel)
    }

    fn wants_full_data_batch(&self) -> bool {
        self.effects.requires_data_batch() || !self.effects.adjustments.is_empty()
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
            (
                "size",
                DataType::Float32
                | DataType::Float64
                | DataType::Int8
                | DataType::Int16
                | DataType::Int32
                | DataType::Int64
                | DataType::UInt8
                | DataType::UInt16
                | DataType::UInt32
                | DataType::UInt64,
            ) => Some(ScaleTypePreference::Sqrt),
            ("size", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
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
            "stroke" => Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line)),
            "size" | "opacity" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Symbol))
            }
            "x" | "y" | "defined" | "order" => None,
            _ => Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line)),
        }
    }

    fn default_channel_range(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        domain: &ResolvedDomain,
        _data_type: &DataType,
        theme: &Theme,
        params: &indexmap::IndexMap<String, ScalarValue>,
    ) -> Option<ScaleRange> {
        let range_kind = scale_impl.range_kind();
        let cardinality = match domain {
            ResolvedDomain::Discrete(count) => Some(*count),
            ResolvedDomain::Interval => None,
        };

        if let Some(theme_range) =
            theme.get_range_for_channel("trail", channel, range_kind, cardinality, params)
        {
            return Some(theme_range);
        }

        match channel {
            "size" => match domain {
                ResolvedDomain::Discrete(count) => {
                    if *count == 1 {
                        Some(ScaleRange::new_discrete(vec![ScalarValue::Float32(Some(
                            8.0,
                        ))]))
                    } else {
                        Some(ScaleRange::new_linspace_discrete(2.0, 18.0, *count))
                    }
                }
                ResolvedDomain::Interval => Some(ScaleRange::new_interval(lit(2.0), lit(18.0))),
            },
            "opacity" => Some(domain.make_interval_or_linspaced_range(0.0, 1.0)),
            _ => None,
        }
    }
}

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl CompiledMark for CompiledCartesianTrail {
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
        let data = data.ok_or_else(|| {
            AvengerChartError::InternalError(
                "Trail mark requires array data for x and y positions".to_string(),
            )
        })?;
        let mark_context = context.core_view();
        let len = data.num_rows();

        let position = util::transform_cartesian_point_channels(
            self,
            Some(data),
            scalars,
            context,
            coord,
            "x",
            "y",
        )?;
        let size = coerce_numeric_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "size",
            &mark_context,
            1.0,
        )?;
        let visual = self.coerce_trail_visual_channels(Some(data), scalars, &mark_context)?;
        let (x, y, size, visual) = self.apply_expression_adjustments(
            position.x.clone(),
            position.y.clone(),
            size,
            visual,
            Some(data),
            len,
            context,
            &mark_context,
        )?;
        let detail_columns = DetailColumns::from_mark_data(self, data)?;

        let stroke_strings = util::color_channel_strings(&visual.stroke, len);
        let opacity_values = visual.opacity.as_vec(len, None);
        let stroke_values = visual.stroke.as_vec(len, None);

        let mut groups: IndexMap<TrailRenderPartitionKey, Vec<usize>> = IndexMap::new();
        for i in 0..len {
            let key = TrailRenderPartitionKey {
                details: detail_columns.key_for_row(i)?,
                stroke: stroke_strings[i].clone(),
                opacity_bits: opacity_values[i].clamp(0.0, 1.0).to_bits(),
            };
            groups.entry(key).or_default().push(i);
        }

        let mut marks = Vec::new();
        let mut source_row_indices = Vec::new();
        for (key, indices) in groups {
            let first_index = indices.first().copied().ok_or_else(|| {
                AvengerChartError::InternalError("Trail render partition was empty".to_string())
            })?;
            let stroke = stroke_values[first_index].clone();
            let opacity = f32::from_bits(key.opacity_bits);

            marks.push(SceneMark::Trail(SceneTrailMark {
                name: "trail".to_string(),
                clip: true,
                len: indices.len() as u32,
                gradients: vec![],
                stroke: apply_opacity_to_color(&stroke, opacity),
                x: util::gather_by_indices(&x, len, &indices),
                y: util::gather_by_indices(&y, len, &indices),
                size: util::gather_by_indices(&size, len, &indices),
                defined: util::gather_by_indices(&visual.defined, len, &indices),
                zindex: self.state.zindex,
                interactive: true,
            }));
            source_row_indices.push(indices);
        }

        Ok(RenderedMarkData::with_source_row_indices(
            marks,
            source_row_indices,
        ))
    }
}

impl CompiledCartesianTrail {
    fn coerce_trail_visual_channels(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &MarkRenderContext<'_>,
    ) -> Result<TrailVisualChannels, AvengerChartError> {
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
        Ok(TrailVisualChannels {
            stroke,
            opacity,
            defined,
        })
    }

    fn apply_expression_adjustments(
        &self,
        mut x: ScalarOrArray<f32>,
        mut y: ScalarOrArray<f32>,
        mut size: ScalarOrArray<f32>,
        mut visual: TrailVisualChannels,
        data: Option<&RecordBatch>,
        len: usize,
        runtime_context: &dyn MarkRuntimeContext,
        context: &MarkRenderContext<'_>,
    ) -> Result<
        (
            ScalarOrArray<f32>,
            ScalarOrArray<f32>,
            ScalarOrArray<f32>,
            TrailVisualChannels,
        ),
        AvengerChartError,
    > {
        if self.effects.adjustments.is_empty() {
            return Ok((x, y, size, visual));
        }

        for adjustment in &self.effects.adjustments {
            let mut frame = build_trail_item_frame(&x, &y, &size, &visual, data, len)?;
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
                    "x" | "y" | "size" | "stroke" | "opacity" | "defined" => {}
                    channel => {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "Trail<Cartesian> adjustment channel '{channel}' is not implemented yet"
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
            size = ScalarOrArray::new_array(frame.f32_values(&item_channel_column_name("size"))?);
            visual.stroke = util::coerce_color_strings(
                &frame.string_values(&item_channel_column_name("stroke"))?,
                "stroke",
            )?;
            visual.opacity =
                ScalarOrArray::new_array(frame.f32_values(&item_channel_column_name("opacity"))?);
            visual.defined =
                ScalarOrArray::new_array(frame.bool_values(&item_channel_column_name("defined"))?);
        }

        Ok((x, y, size, visual))
    }
}

#[derive(Clone)]
struct TrailVisualChannels {
    stroke: ScalarOrArray<ColorOrGradient>,
    opacity: ScalarOrArray<f32>,
    defined: ScalarOrArray<bool>,
}

fn build_trail_item_frame(
    x: &ScalarOrArray<f32>,
    y: &ScalarOrArray<f32>,
    size: &ScalarOrArray<f32>,
    visual: &TrailVisualChannels,
    data: Option<&RecordBatch>,
    len: usize,
) -> Result<MarkEvaluationFrame, AvengerChartError> {
    let x_values = x.as_vec(len, None);
    let y_values = y.as_vec(len, None);
    let size_values = size.as_vec(len, None);
    let stroke_values = util::color_channel_strings(&visual.stroke, len);
    let opacity_values = visual.opacity.as_vec(len, None);
    let defined_values = visual.defined.as_vec(len, None);
    let mut columns = vec![
        f32_item_column("x", x_values.clone()),
        f32_item_column("y", y_values.clone()),
        f32_item_column("size", size_values),
        string_item_column("stroke", stroke_values),
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
                "Trail adjustment data row count {} did not match vertex count {len}",
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
