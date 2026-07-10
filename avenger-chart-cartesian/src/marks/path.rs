use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, LegendRendererKind, LegendRendererSelection,
    Mark, MarkAdjustmentSpec, MarkEvaluationFrame, MarkRenderContext, MarkRuntimeContext,
    PrimitiveMarkEffects, RenderedMarkData, ResolvedDomain, ScaleRange, ScaleTypePreference, Theme,
    apply_opacity_to_color_channel, coerce_color_channel_with_renderer,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel_with_renderer,
    coerce_pattern_channel_with_renderer, coerce_stroke_cap_channel_values_with_renderer,
    coerce_stroke_join_channel_values_with_renderer, coerce_text_channel,
    default_scale_type_for_data_type, evaluate_item_assignments, impl_mark_trait_common,
    is_continuous_scale, item_bbox_column_name, item_channel_column_name, item_data_column_name,
};
use avenger_chart_marks::{PathMark, path_channel_defaults};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{PathTransform, StrokeCap, StrokeJoin},
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl, coerce::Coercer};
use avenger_scenegraph::marks::{mark::SceneMark, path::ScenePathMark, pattern::PatternFill};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float32Array, RecordBatch, StringArray},
        datatypes::{DataType, Field},
    },
    common::ScalarValue,
    logical_expr::{Expr, lit},
};
use indexmap::IndexMap;
use lyon_extra::euclid::Vector2D;
use lyon_path::Path;
use serde::{Deserialize, Serialize};

use crate::{Cartesian, marks::util};

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<Cartesian> for PathMark<Cartesian> {
    impl_mark_trait_common!(PathMark);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianPath {
            state: compiled_state,
            effects: self.mark_effects().clone(),
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianPath {
    pub(crate) state: CompiledMarkState,
    #[serde(default)]
    pub(crate) effects: PrimitiveMarkEffects,
}

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
struct PathRenderPartitionKey {
    stroke_width_bits: u32,
    stroke_cap: String,
    stroke_join: String,
}

impl CompiledMarkCore for CompiledCartesianPath {
    avenger_chart_core::impl_mark_with_data_context!();

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
        "path"
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
                name: "path",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "path_transform",
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
                name: "fill_pattern",
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
                allow_column_ref: false,
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
        ]
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        path_channel_defaults(channel)
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
            ("fill_pattern", _) => Some(ScaleTypePreference::Ordinal),
            ("x" | "y", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Point)
            }
            ("fill" | "stroke", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
            ("path" | "path_transform" | "stroke_cap" | "stroke_join", _) => None,
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
            "stroke" | "stroke_width" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line))
            }
            "x" | "y" | "path" | "path_transform" | "stroke_cap" | "stroke_join" => None,
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

    fn default_channel_range(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        domain: &ResolvedDomain,
        _data_type: &DataType,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
    ) -> Option<ScaleRange> {
        let range_kind = scale_impl.range_kind();
        let cardinality = match domain {
            ResolvedDomain::Discrete(count) => Some(*count),
            ResolvedDomain::Interval => None,
        };

        theme.get_range_for_channel("path", channel, range_kind, cardinality, params)
    }
}

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl CompiledMark for CompiledCartesianPath {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        self.render_path_mark_data(data, scalars, context, coord)
            .map(|rendered| rendered.marks)
    }

    async fn render_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        self.render_path_mark_data(data, scalars, context, coord)
    }
}

impl CompiledCartesianPath {
    fn render_path_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        let mark_context = context.core_view();
        let len = util::scene_len(data) as usize;
        let coercer = Coercer::default();
        let position = util::transform_cartesian_point_channels(
            self, data, scalars, context, coord, "x", "y",
        )?;
        let visual =
            self.coerce_path_visual_channels(&coercer, data, scalars, &mark_context, context)?;
        let (x, y, visual) = self.apply_expression_adjustments(
            position.x,
            position.y,
            visual,
            &coercer,
            data,
            scalars,
            len,
            context,
            &mark_context,
        )?;
        let transform = translate_transforms(visual.path_transform, &x, &y, len);
        let fill = apply_opacity_to_color_channel(visual.fill, &visual.opacity, len);
        let stroke = apply_opacity_to_color_channel(visual.stroke, &visual.opacity, len);
        let stroke_width_values = visual.stroke_width.as_vec(len, None);

        let mut groups: IndexMap<PathRenderPartitionKey, Vec<usize>> = IndexMap::new();
        for i in 0..len {
            let key = PathRenderPartitionKey {
                stroke_width_bits: stroke_width_values[i].to_bits(),
                stroke_cap: visual.stroke_cap_strings[i].clone(),
                stroke_join: visual.stroke_join_strings[i].clone(),
            };
            groups.entry(key).or_default().push(i);
        }

        let mut marks = Vec::new();
        let mut source_row_indices = Vec::new();
        for (key, indices) in groups {
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
            .unwrap_or(StrokeJoin::Miter);
            let indices_ref = if indices.len() == len && indices.iter().copied().eq(0..len) {
                None
            } else {
                Some(Arc::new(indices.clone()))
            };
            marks.push(SceneMark::Path(ScenePathMark {
                name: "path".to_string(),
                clip: true,
                len: indices.len() as u32,
                gradients: vec![],
                stroke_cap,
                stroke_join,
                stroke_width: Some(f32::from_bits(key.stroke_width_bits)),
                path: visual.path.clone(),
                fill: fill.clone(),
                fill_pattern: visual.fill_pattern.clone(),
                stroke: stroke.clone(),
                transform: transform.clone(),
                indices: indices_ref,
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

    fn coerce_path_visual_channels(
        &self,
        coercer: &Coercer,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &MarkRenderContext<'_>,
        runtime_context: &dyn MarkRuntimeContext,
    ) -> Result<PathVisualChannels, AvengerChartError> {
        let len = util::scene_len(data) as usize;
        let path = coerce_path_channel(coercer, data, scalars)?;
        let path_transform = coerce_path_transform_channel(coercer, data, scalars)?;
        let fill = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "fill",
            context,
            [0.0, 0.0, 0.0, 0.0],
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
        let fill_pattern = coerce_pattern_channel_with_renderer(
            self,
            data,
            scalars,
            "fill_pattern",
            runtime_context,
        )?;
        let stroke_width = coerce_numeric_channel_with_renderer(
            self,
            None,
            scalars,
            "stroke_width",
            context,
            0.0,
        )?;
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
            StrokeJoin::Miter,
        )?;
        Ok(PathVisualChannels {
            path,
            path_strings: None,
            path_transform,
            path_transform_strings: None,
            fill,
            fill_pattern,
            stroke,
            stroke_width,
            stroke_cap_strings: util::stroke_cap_strings(&stroke_cap, len),
            stroke_join_strings: util::stroke_join_strings(&stroke_join, len),
            opacity,
        })
    }

    fn apply_expression_adjustments(
        &self,
        mut x: ScalarOrArray<f32>,
        mut y: ScalarOrArray<f32>,
        mut visual: PathVisualChannels,
        coercer: &Coercer,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        len: usize,
        runtime_context: &dyn MarkRuntimeContext,
        context: &MarkRenderContext<'_>,
    ) -> Result<(ScalarOrArray<f32>, ScalarOrArray<f32>, PathVisualChannels), AvengerChartError>
    {
        if self.effects.adjustments.is_empty() {
            return Ok((x, y, visual));
        }

        for adjustment in &self.effects.adjustments {
            let assignments = match adjustment {
                MarkAdjustmentSpec::Expr(spec) => &spec.assignments,
                MarkAdjustmentSpec::Transform(spec) => &spec.assignments,
            };
            if visual.path_strings.is_none()
                && assignments_read_channel(
                    assignments,
                    "path",
                    context.session_context().as_ref(),
                )?
            {
                visual.path_strings =
                    Some(coerce_text_channel(data, scalars, "path", String::new())?);
            }
            if visual.path_transform_strings.is_none()
                && assignments_read_channel(
                    assignments,
                    "path_transform",
                    context.session_context().as_ref(),
                )?
            {
                visual.path_transform_strings = Some(coerce_text_channel(
                    data,
                    scalars,
                    "path_transform",
                    String::new(),
                )?);
            }
            let mut frame = build_path_item_frame(&x, &y, &visual, data, len)?;
            if let MarkAdjustmentSpec::Transform(spec) = adjustment {
                let adjustment_context = util::adjustment_transform_context(runtime_context);
                spec.transform.apply(&mut frame, &adjustment_context)?;
            }
            if assignments.is_empty() {
                continue;
            }
            for assignment in assignments {
                match assignment.channel.as_str() {
                    "x" | "y" | "path" | "path_transform" | "fill" | "stroke" | "stroke_width"
                    | "stroke_cap" | "stroke_join" | "opacity" => {}
                    channel => {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "PathMark<Cartesian> adjustment channel '{channel}' is not implemented yet"
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
            let path_column = item_channel_column_name("path");
            if frame.column(&path_column).is_some() {
                let path_strings = ScalarOrArray::new_array(frame.string_values(&path_column)?)
                    .to_scalar_if_len_one();
                visual.path = coerce_path_strings(coercer, &path_strings)?;
                visual.path_strings = Some(path_strings);
            }
            let path_transform_column = item_channel_column_name("path_transform");
            if frame.column(&path_transform_column).is_some() {
                let path_transform_strings =
                    ScalarOrArray::new_array(frame.string_values(&path_transform_column)?)
                        .to_scalar_if_len_one();
                visual.path_transform =
                    coerce_path_transform_strings(coercer, &path_transform_strings)?;
                visual.path_transform_strings = Some(path_transform_strings);
            }
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
            visual.stroke_cap_strings =
                frame.string_values(&item_channel_column_name("stroke_cap"))?;
            visual.stroke_join_strings =
                frame.string_values(&item_channel_column_name("stroke_join"))?;
            visual.opacity =
                ScalarOrArray::new_array(frame.f32_values(&item_channel_column_name("opacity"))?);
        }

        Ok((x, y, visual))
    }
}

#[derive(Clone)]
struct PathVisualChannels {
    path: ScalarOrArray<Path>,
    path_strings: Option<ScalarOrArray<String>>,
    path_transform: ScalarOrArray<PathTransform>,
    path_transform_strings: Option<ScalarOrArray<String>>,
    fill: ScalarOrArray<ColorOrGradient>,
    fill_pattern: ScalarOrArray<Option<PatternFill>>,
    stroke: ScalarOrArray<ColorOrGradient>,
    stroke_width: ScalarOrArray<f32>,
    stroke_cap_strings: Vec<String>,
    stroke_join_strings: Vec<String>,
    opacity: ScalarOrArray<f32>,
}

fn build_path_item_frame(
    x: &ScalarOrArray<f32>,
    y: &ScalarOrArray<f32>,
    visual: &PathVisualChannels,
    data: Option<&RecordBatch>,
    len: usize,
) -> Result<MarkEvaluationFrame, AvengerChartError> {
    let x_values = x.as_vec(len, None);
    let y_values = y.as_vec(len, None);
    let fill_values = util::color_channel_strings(&visual.fill, len);
    let stroke_values = util::color_channel_strings(&visual.stroke, len);
    let stroke_width_values = visual.stroke_width.as_vec(len, None);
    let opacity_values = visual.opacity.as_vec(len, None);
    let mut columns = vec![
        f32_item_column("x", x_values.clone()),
        f32_item_column("y", y_values.clone()),
        string_item_column("fill", fill_values),
        string_item_column("stroke", stroke_values),
        f32_item_column("stroke_width", stroke_width_values),
        string_item_column("stroke_cap", visual.stroke_cap_strings.clone()),
        string_item_column("stroke_join", visual.stroke_join_strings.clone()),
        f32_item_column("opacity", opacity_values),
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
    if let Some(path_strings) = &visual.path_strings {
        columns.push((
            Field::new(item_channel_column_name("path"), DataType::Utf8, true),
            Arc::new(StringArray::from(path_strings.as_vec(len, None))) as ArrayRef,
        ));
    }
    if let Some(path_transform) = &visual.path_transform_strings {
        columns.push((
            Field::new(
                item_channel_column_name("path_transform"),
                DataType::Utf8,
                true,
            ),
            Arc::new(StringArray::from(path_transform.as_vec(len, None))) as ArrayRef,
        ));
    }

    if let Some(data) = data {
        if data.num_rows() != len {
            return Err(AvengerChartError::InternalError(format!(
                "PathMark adjustment data row count {} did not match item count {len}",
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

fn assignments_read_channel(
    assignments: &[avenger_chart_core::ItemChannelAssignment],
    channel: &str,
    ctx: &datafusion::prelude::SessionContext,
) -> Result<bool, AvengerChartError> {
    let item_column = item_channel_column_name(channel);
    for assignment in assignments {
        if assignment
            .expr(ctx)?
            .column_refs()
            .iter()
            .any(|column| column.name == item_column)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn coerce_path_channel(
    coercer: &Coercer,
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
) -> Result<ScalarOrArray<Path>, AvengerChartError> {
    if let Some(array) = data.and_then(|data| data.column_by_name("path")) {
        coercer.to_path(array).map_err(|error| {
            AvengerChartError::InternalError(format!("Error coercing channel 'path': {error}"))
        })
    } else if let Some(array) = scalars.column_by_name("path") {
        coercer
            .to_path(array)
            .map(|values| values.to_scalar_if_len_one())
            .map_err(|error| {
                AvengerChartError::InternalError(format!("Error coercing channel 'path': {error}"))
            })
    } else {
        Ok(ScenePathMark::default().path)
    }
}

fn coerce_path_transform_channel(
    coercer: &Coercer,
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
) -> Result<ScalarOrArray<PathTransform>, AvengerChartError> {
    if let Some(array) = data.and_then(|data| data.column_by_name("path_transform")) {
        coercer.to_path_transform(array).map_err(|error| {
            AvengerChartError::InternalError(format!(
                "Error coercing channel 'path_transform': {error}"
            ))
        })
    } else if let Some(array) = scalars.column_by_name("path_transform") {
        coercer
            .to_path_transform(array)
            .map(|values| values.to_scalar_if_len_one())
            .map_err(|error| {
                AvengerChartError::InternalError(format!(
                    "Error coercing channel 'path_transform': {error}"
                ))
            })
    } else {
        Ok(ScenePathMark::default().transform)
    }
}

fn coerce_path_strings(
    coercer: &Coercer,
    values: &ScalarOrArray<String>,
) -> Result<ScalarOrArray<Path>, AvengerChartError> {
    let values = match values.value() {
        ScalarOrArrayValue::Scalar(value) => ScalarOrArray::new_array(vec![value.clone()]),
        ScalarOrArrayValue::Array(values) => ScalarOrArray::new_array(values.as_ref().clone()),
    };
    let array = Arc::new(StringArray::from(values.as_vec(values.len(), None))) as ArrayRef;
    coercer
        .to_path(&array)
        .map(|values| values.to_scalar_if_len_one())
        .map_err(|error| {
            AvengerChartError::InternalError(format!("Error coercing channel 'path': {error}"))
        })
}

fn coerce_path_transform_strings(
    coercer: &Coercer,
    values: &ScalarOrArray<String>,
) -> Result<ScalarOrArray<PathTransform>, AvengerChartError> {
    let values = match values.value() {
        ScalarOrArrayValue::Scalar(value) => ScalarOrArray::new_array(vec![value.clone()]),
        ScalarOrArrayValue::Array(values) => ScalarOrArray::new_array(values.as_ref().clone()),
    };
    let array = Arc::new(StringArray::from(values.as_vec(values.len(), None))) as ArrayRef;
    coercer
        .to_path_transform(&array)
        .map(|values| values.to_scalar_if_len_one())
        .map_err(|error| {
            AvengerChartError::InternalError(format!(
                "Error coercing channel 'path_transform': {error}"
            ))
        })
}

fn translate_transforms(
    transforms: ScalarOrArray<PathTransform>,
    x: &ScalarOrArray<f32>,
    y: &ScalarOrArray<f32>,
    len: usize,
) -> ScalarOrArray<PathTransform> {
    match (transforms.value(), x.value(), y.value()) {
        (
            ScalarOrArrayValue::Scalar(transform),
            ScalarOrArrayValue::Scalar(x),
            ScalarOrArrayValue::Scalar(y),
        ) => ScalarOrArray::new_scalar(transform.then_translate(Vector2D::new(*x, *y))),
        _ => {
            let transforms = transforms.as_vec(len, None);
            let x = x.as_vec(len, None);
            let y = y.as_vec(len, None);
            ScalarOrArray::new_array(
                transforms
                    .iter()
                    .zip(x.iter())
                    .zip(y.iter())
                    .map(|((transform, x), y)| transform.then_translate(Vector2D::new(*x, *y)))
                    .collect(),
            )
        }
    }
}
