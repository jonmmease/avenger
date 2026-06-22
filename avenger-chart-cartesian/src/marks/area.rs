use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererKind, LegendRendererSelection,
    Mark, MarkAdjustmentSpec, MarkEvaluationFrame, MarkRenderContext, MarkRuntimeContext,
    PrimitiveMarkEffects, RenderedMarkData, ScaleTypePreference, apply_opacity_to_color,
    coerce_area_orientation_channel, coerce_bool_channel_with_renderer,
    coerce_color_channel_with_renderer, coerce_numeric_channel_with_renderer,
    coerce_opacity_channel_with_renderer, coerce_stroke_cap_channel_values_with_renderer,
    coerce_stroke_dash_channel, coerce_stroke_join_channel_values_with_renderer,
    default_scale_type_for_data_type, evaluate_item_assignments, impl_mark_trait_common,
    is_continuous_scale, item_bbox_column_name, item_channel_column_name, item_data_column_name,
};
use avenger_chart_marks::{Area, area_channel_defaults};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{AreaOrientation, StrokeCap, StrokeJoin},
    value::ScalarOrArray,
};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::marks::{area::SceneAreaMark, mark::SceneMark};
use datafusion::{
    arrow::{
        array::{ArrayRef, BooleanArray, Float32Array, RecordBatch, StringArray},
        datatypes::{DataType, Field},
    },
    common::ScalarValue,
    logical_expr::{Expr, lit},
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    Cartesian,
    marks::{detail::DetailColumns, util},
};

#[async_trait::async_trait]
impl Mark<Cartesian> for Area<Cartesian> {
    impl_mark_trait_common!(Area);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianArea {
            state: compiled_state,
            effects: self.mark_effects().clone(),
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianArea {
    pub(crate) state: CompiledMarkState,
    #[serde(default)]
    pub(crate) effects: PrimitiveMarkEffects,
}

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
struct AreaRenderPartitionKey {
    details: Vec<ScalarValue>,
    orientation: String,
    fill: String,
    stroke: String,
    stroke_width_bits: u32,
    stroke_dash: String,
    opacity_bits: u32,
    stroke_cap: String,
    stroke_join: String,
}

impl CompiledMarkCore for CompiledCartesianArea {
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
        "area"
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
                name: "orientation",
                required: false,
                default_value: None,
                allow_column_ref: false,
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
                name: "stroke_dash",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "stroke_cap",
                required: false,
                default_value: None,
                allow_column_ref: false,
            },
            ChannelDescriptor {
                name: "stroke_join",
                required: false,
                default_value: None,
                allow_column_ref: false,
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
        area_channel_defaults(channel)
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
            (
                "x" | "x2" | "y" | "y2",
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            ) => Some(ScaleTypePreference::Point),
            ("fill" | "stroke", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
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
            "fill" if is_continuous_scale(scale.scale_impl.as_ref()) => Some(
                LegendRendererSelection::BuiltIn(LegendRendererKind::Colorbar),
            ),
            "fill" | "opacity" => Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Rect)),
            "stroke" if is_continuous_scale(scale.scale_impl.as_ref()) => Some(
                LegendRendererSelection::BuiltIn(LegendRendererKind::Colorbar),
            ),
            "stroke" | "stroke_width" | "stroke_dash" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line))
            }
            "x" | "y" | "x2" | "y2" | "defined" | "order" | "orientation" | "stroke_cap"
            | "stroke_join" => None,
            _ => Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Rect)),
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        _data_type: &DataType,
    ) -> std::collections::HashMap<String, Expr> {
        let mut options = std::collections::HashMap::new();
        if matches!(channel, "fill" | "stroke") && is_continuous_scale(scale_impl) {
            options.insert("nice".to_string(), lit(true));
        }
        options
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianArea {
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
                "Area mark requires array data for x and y positions".to_string(),
            )
        })?;
        let mark_context = context.core_view();
        let len = data.num_rows();

        let start = util::transform_cartesian_point_channels(
            self,
            Some(data),
            scalars,
            context,
            coord,
            "x",
            "y",
        )?;
        let end = util::transform_cartesian_point_channels(
            self,
            Some(data),
            scalars,
            context,
            coord,
            "x2",
            "y2",
        )?;
        let stroke_width = coerce_numeric_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "stroke_width",
            &mark_context,
            0.0,
        )?;
        let visual = self.coerce_area_visual_channels(Some(data), scalars, &mark_context, len)?;
        let (x, y, x2, y2, visual) = self.apply_expression_adjustments(
            start.x.clone(),
            start.y.clone(),
            end.x.clone(),
            end.y.clone(),
            AreaVisualChannels {
                stroke_width,
                ..visual
            },
            Some(data),
            len,
            context,
            &mark_context,
        )?;

        let detail_columns = DetailColumns::from_mark_data(self, data)?;
        let mut groups: IndexMap<AreaRenderPartitionKey, Vec<usize>> = IndexMap::new();
        let fill_values = visual.fill.as_vec(len, None);
        let fill_strings = util::color_channel_strings(&visual.fill, len);
        let stroke_values = visual.stroke.as_vec(len, None);
        let stroke_strings = util::color_channel_strings(&visual.stroke, len);
        let stroke_width_values = visual.stroke_width.as_vec(len, None);
        let opacity_values = visual.opacity.as_vec(len, None);
        for i in 0..len {
            let key = AreaRenderPartitionKey {
                details: detail_columns.key_for_row(i)?,
                orientation: visual.orientation_strings[i].clone(),
                fill: fill_strings[i].clone(),
                stroke: stroke_strings[i].clone(),
                stroke_width_bits: stroke_width_values[i].to_bits(),
                stroke_dash: visual.stroke_dash_strings[i].clone(),
                opacity_bits: opacity_values[i].clamp(0.0, 1.0).to_bits(),
                stroke_cap: visual.stroke_cap_strings[i].clone(),
                stroke_join: visual.stroke_join_strings[i].clone(),
            };
            groups.entry(key).or_default().push(i);
        }

        let mut marks = Vec::new();
        let mut source_row_indices = Vec::new();
        for (key, indices) in groups {
            let first_index = indices.first().copied().ok_or_else(|| {
                AvengerChartError::InternalError("Area render partition was empty".to_string())
            })?;
            let orientation = area_orientation_from_name(&key.orientation)?;
            let opacity = f32::from_bits(key.opacity_bits);
            let fill = fill_values[first_index].clone();
            let stroke = stroke_values[first_index].clone();
            let stroke_dash = stroke_dash_from_name(&key.stroke_dash)?;
            let stroke_cap = util::coerce_stroke_cap_strings(
                std::slice::from_ref(&key.stroke_cap),
                "stroke_cap",
            )?
            .first()
            .copied()
            .unwrap_or(StrokeCap::Butt);
            let stroke_join = util::coerce_stroke_join_strings(
                std::slice::from_ref(&key.stroke_join),
                "stroke_join",
            )?
            .first()
            .copied()
            .unwrap_or(StrokeJoin::Round);

            marks.push(SceneMark::Area(SceneAreaMark {
                name: "area".to_string(),
                clip: true,
                len: indices.len() as u32,
                orientation,
                gradients: vec![],
                x: util::gather_by_indices(&x, len, &indices),
                y: util::gather_by_indices(&y, len, &indices),
                x2: util::gather_by_indices(&x2, len, &indices),
                y2: util::gather_by_indices(&y2, len, &indices),
                defined: util::gather_by_indices(&visual.defined, len, &indices),
                fill: apply_opacity_to_color(&fill, opacity),
                stroke: apply_opacity_to_color(&stroke, opacity),
                stroke_width: f32::from_bits(key.stroke_width_bits),
                stroke_cap,
                stroke_join,
                stroke_dash,
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

impl CompiledCartesianArea {
    fn coerce_area_visual_channels(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &MarkRenderContext<'_>,
        len: usize,
    ) -> Result<AreaVisualChannels, AvengerChartError> {
        let orientation = coerce_area_orientation_channel(
            None,
            scalars,
            "orientation",
            AreaOrientation::Vertical,
        )?;
        let fill = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "fill",
            context,
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
        )?;
        let stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke",
            context,
            [0.0, 0.0, 0.0, 0.0],
        )?;
        let opacity =
            coerce_opacity_channel_with_renderer(self, data, scalars, "opacity", context, 1.0)?;
        let defined =
            coerce_bool_channel_with_renderer(self, data, scalars, "defined", context, true)?;
        let stroke_dash =
            util::optional_stroke_dash(coerce_stroke_dash_channel(data, scalars, "stroke_dash")?);
        let stroke_cap = coerce_stroke_cap_channel_values_with_renderer(
            self,
            None,
            scalars,
            "stroke_cap",
            context,
            StrokeCap::Butt,
        )?;
        let stroke_join = coerce_stroke_join_channel_values_with_renderer(
            self,
            None,
            scalars,
            "stroke_join",
            context,
            StrokeJoin::Round,
        )?;
        Ok(AreaVisualChannels {
            orientation_strings: area_orientation_strings(&orientation, len),
            fill,
            stroke,
            stroke_width: ScalarOrArray::new_scalar(0.0),
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
        mut x2: ScalarOrArray<f32>,
        mut y2: ScalarOrArray<f32>,
        mut visual: AreaVisualChannels,
        data: Option<&RecordBatch>,
        len: usize,
        runtime_context: &dyn MarkRuntimeContext,
        context: &MarkRenderContext<'_>,
    ) -> Result<
        (
            ScalarOrArray<f32>,
            ScalarOrArray<f32>,
            ScalarOrArray<f32>,
            ScalarOrArray<f32>,
            AreaVisualChannels,
        ),
        AvengerChartError,
    > {
        if self.effects.adjustments.is_empty() {
            return Ok((x, y, x2, y2, visual));
        }

        for adjustment in &self.effects.adjustments {
            let mut frame = build_area_item_frame(&x, &y, &x2, &y2, &visual, data, len)?;
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
                    "orientation" | "x" | "y" | "x2" | "y2" | "fill" | "stroke"
                    | "stroke_width" | "stroke_dash" | "stroke_cap" | "stroke_join" | "opacity"
                    | "defined" => {}
                    channel => {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "Area<Cartesian> adjustment channel '{channel}' is not implemented yet"
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
            visual.orientation_strings =
                frame.string_values(&item_channel_column_name("orientation"))?;
            visual.fill = util::coerce_color_strings(
                &frame.string_values(&item_channel_column_name("fill"))?,
                "fill",
            )?;
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

        Ok((x, y, x2, y2, visual))
    }
}

#[derive(Clone)]
struct AreaVisualChannels {
    orientation_strings: Vec<String>,
    fill: ScalarOrArray<ColorOrGradient>,
    stroke: ScalarOrArray<ColorOrGradient>,
    stroke_width: ScalarOrArray<f32>,
    stroke_dash_strings: Vec<String>,
    stroke_cap_strings: Vec<String>,
    stroke_join_strings: Vec<String>,
    opacity: ScalarOrArray<f32>,
    defined: ScalarOrArray<bool>,
}

fn build_area_item_frame(
    x: &ScalarOrArray<f32>,
    y: &ScalarOrArray<f32>,
    x2: &ScalarOrArray<f32>,
    y2: &ScalarOrArray<f32>,
    visual: &AreaVisualChannels,
    data: Option<&RecordBatch>,
    len: usize,
) -> Result<MarkEvaluationFrame, AvengerChartError> {
    let x_values = x.as_vec(len, None);
    let y_values = y.as_vec(len, None);
    let x2_values = x2.as_vec(len, None);
    let y2_values = y2.as_vec(len, None);
    let fill_values = util::color_channel_strings(&visual.fill, len);
    let stroke_values = util::color_channel_strings(&visual.stroke, len);
    let stroke_width_values = visual.stroke_width.as_vec(len, None);
    let opacity_values = visual.opacity.as_vec(len, None);
    let defined_values = visual.defined.as_vec(len, None);
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
        string_item_column("orientation", visual.orientation_strings.clone()),
        string_item_column("fill", fill_values),
        string_item_column("stroke", stroke_values),
        f32_item_column("stroke_width", stroke_width_values),
        string_item_column("stroke_dash", visual.stroke_dash_strings.clone()),
        string_item_column("stroke_cap", visual.stroke_cap_strings.clone()),
        string_item_column("stroke_join", visual.stroke_join_strings.clone()),
        f32_item_column("opacity", opacity_values),
        bool_item_column("defined", defined_values),
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
                "Area adjustment data row count {} did not match vertex count {len}",
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

fn area_orientation_strings(values: &ScalarOrArray<AreaOrientation>, len: usize) -> Vec<String> {
    values
        .as_vec(len, None)
        .into_iter()
        .map(area_orientation_name)
        .collect()
}

fn area_orientation_name(value: AreaOrientation) -> String {
    match value {
        AreaOrientation::Vertical => "vertical",
        AreaOrientation::Horizontal => "horizontal",
    }
    .to_string()
}

fn area_orientation_from_name(value: &str) -> Result<AreaOrientation, AvengerChartError> {
    match value {
        "vertical" => Ok(AreaOrientation::Vertical),
        "horizontal" => Ok(AreaOrientation::Horizontal),
        _ => Err(AvengerChartError::InvalidArgument(format!(
            "Error coercing adjusted area orientation channel 'orientation': invalid value '{value}'"
        ))),
    }
}

fn stroke_dash_from_name(value: &str) -> Result<Option<Vec<f32>>, AvengerChartError> {
    Ok(
        util::coerce_stroke_dash_strings(&[value.to_string()], "stroke_dash")?
            .and_then(|dash| dash.first().cloned())
            .filter(|dash| !dash.is_empty() && dash.as_slice() != [f32::MAX]),
    )
}
