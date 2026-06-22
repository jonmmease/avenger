use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, Mark, MarkAdjustmentSpec,
    MarkEvaluationFrame, MarkRenderContext, MarkRuntimeContext, PrimitiveMarkEffects,
    RenderedMarkData, ScaleTypePreference, coerce_bool_channel_with_renderer,
    coerce_image_align_channel, coerce_image_baseline_channel,
    coerce_numeric_channel_with_renderer, coerce_text_channel, default_scale_type_for_data_type,
    evaluate_item_assignments, impl_mark_trait_common, item_bbox_column_name,
    item_channel_column_name, item_data_column_name,
};
use avenger_chart_marks::{Image, image_channel_defaults};
use avenger_common::{
    types::{ImageAlign, ImageBaseline},
    value::ScalarOrArray,
};
use avenger_image::RgbaImage;
use avenger_scales::scales::coerce::Coercer;
use avenger_scenegraph::marks::{image::SceneImageMark, mark::SceneMark};
use datafusion::{
    arrow::{
        array::{ArrayRef, BooleanArray, Float32Array, RecordBatch, StringArray},
        datatypes::{DataType, Field},
    },
    common::ScalarValue,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{Cartesian, marks::util};

#[async_trait::async_trait]
impl Mark<Cartesian> for Image<Cartesian> {
    impl_mark_trait_common!(Image);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianImage {
            state: compiled_state,
            effects: self.mark_effects().clone(),
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianImage {
    pub(crate) state: CompiledMarkState,
    #[serde(default)]
    pub(crate) effects: PrimitiveMarkEffects,
}

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
struct ImageRenderPartitionKey {
    aspect: bool,
    smooth: bool,
}

impl CompiledMarkCore for CompiledCartesianImage {
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
        "image"
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
                name: "image",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "width",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "height",
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
                name: "aspect",
                required: false,
                default_value: None,
                allow_column_ref: false,
            },
            ChannelDescriptor {
                name: "smooth",
                required: false,
                default_value: None,
                allow_column_ref: false,
            },
        ]
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        image_channel_defaults(channel)
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
            ("image" | "align" | "baseline" | "aspect" | "smooth", _) => None,
            _ => default_scale_type_for_data_type(data_type),
        }
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianImage {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        self.render_image_mark_data(data, scalars, context, coord)
            .map(|rendered| rendered.marks)
    }

    async fn render_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        self.render_image_mark_data(data, scalars, context, coord)
    }
}

impl CompiledCartesianImage {
    fn render_image_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        let mark_context = context.core_view();
        let len = util::scene_len(data) as usize;
        let position = util::transform_cartesian_point_channels(
            self, data, scalars, context, coord, "x", "y",
        )?;
        let coercer = Coercer::default();

        let image = coerce_image_channel(&coercer, data, scalars)?;
        let width =
            coerce_numeric_channel_with_renderer(self, data, scalars, "width", &mark_context, 0.0)?;
        let height = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "height",
            &mark_context,
            0.0,
        )?;
        let align = coerce_image_align_channel(data, scalars, "align", ImageAlign::Left)?;
        let baseline =
            coerce_image_baseline_channel(data, scalars, "baseline", ImageBaseline::Top)?;
        let aspect =
            coerce_bool_channel_with_renderer(self, None, scalars, "aspect", &mark_context, true)?;
        let smooth =
            coerce_bool_channel_with_renderer(self, None, scalars, "smooth", &mark_context, true)?;
        let (x, y, image, width, height, align, baseline, aspect, smooth) = self
            .apply_expression_adjustments(
                position.x,
                position.y,
                image,
                None,
                width,
                height,
                align,
                baseline,
                aspect,
                smooth,
                &coercer,
                scalars,
                data,
                len,
                context,
                &mark_context,
            )?;

        let aspect_values = aspect.as_vec(len, None);
        let smooth_values = smooth.as_vec(len, None);
        let mut groups: IndexMap<ImageRenderPartitionKey, Vec<usize>> = IndexMap::new();
        for i in 0..len {
            groups
                .entry(ImageRenderPartitionKey {
                    aspect: aspect_values[i],
                    smooth: smooth_values[i],
                })
                .or_default()
                .push(i);
        }

        let mut marks = Vec::new();
        let mut source_row_indices = Vec::new();
        for (key, indices) in groups {
            let indices_ref = if indices.len() == len && indices.iter().copied().eq(0..len) {
                None
            } else {
                Some(Arc::new(indices.clone()))
            };
            marks.push(
                SceneImageMark {
                    name: "image".to_string(),
                    clip: true,
                    len: indices.len() as u32,
                    aspect: key.aspect,
                    smooth: key.smooth,
                    image: image.clone(),
                    x: x.clone(),
                    y: y.clone(),
                    width: width.clone(),
                    height: height.clone(),
                    align: align.clone(),
                    baseline: baseline.clone(),
                    indices: indices_ref,
                    zindex: self.state.zindex,
                    interactive: true,
                }
                .into(),
            );
            source_row_indices.push(indices);
        }

        Ok(RenderedMarkData::with_source_row_indices(
            marks,
            source_row_indices,
        ))
    }

    fn apply_expression_adjustments(
        &self,
        mut x: ScalarOrArray<f32>,
        mut y: ScalarOrArray<f32>,
        mut image: ScalarOrArray<RgbaImage>,
        mut image_strings: Option<ScalarOrArray<String>>,
        mut width: ScalarOrArray<f32>,
        mut height: ScalarOrArray<f32>,
        mut align: ScalarOrArray<ImageAlign>,
        mut baseline: ScalarOrArray<ImageBaseline>,
        mut aspect: ScalarOrArray<bool>,
        mut smooth: ScalarOrArray<bool>,
        coercer: &Coercer,
        scalars: &RecordBatch,
        data: Option<&RecordBatch>,
        len: usize,
        runtime_context: &dyn MarkRuntimeContext,
        context: &MarkRenderContext<'_>,
    ) -> Result<
        (
            ScalarOrArray<f32>,
            ScalarOrArray<f32>,
            ScalarOrArray<RgbaImage>,
            ScalarOrArray<f32>,
            ScalarOrArray<f32>,
            ScalarOrArray<ImageAlign>,
            ScalarOrArray<ImageBaseline>,
            ScalarOrArray<bool>,
            ScalarOrArray<bool>,
        ),
        AvengerChartError,
    > {
        if self.effects.adjustments.is_empty() {
            return Ok((x, y, image, width, height, align, baseline, aspect, smooth));
        }

        for adjustment in &self.effects.adjustments {
            let assignments = match adjustment {
                MarkAdjustmentSpec::Expr(spec) => &spec.assignments,
                MarkAdjustmentSpec::Transform(spec) => &spec.assignments,
            };
            if image_strings.is_none()
                && assignments_read_channel(
                    assignments,
                    "image",
                    context.session_context().as_ref(),
                )?
            {
                image_strings = Some(coerce_text_channel(data, scalars, "image", String::new())?);
            }
            let mut frame = build_image_item_frame(
                &x,
                &y,
                image_strings.as_ref(),
                &width,
                &height,
                &align,
                &baseline,
                &aspect,
                &smooth,
                data,
                len,
            )?;
            if let MarkAdjustmentSpec::Transform(spec) = adjustment {
                let adjustment_context = util::adjustment_transform_context(runtime_context);
                spec.transform.apply(&mut frame, &adjustment_context)?;
            }
            if assignments.is_empty() {
                continue;
            }
            for assignment in assignments {
                match assignment.channel.as_str() {
                    "image" | "x" | "y" | "width" | "height" | "align" | "baseline" | "aspect"
                    | "smooth" => {}
                    channel => {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "Image<Cartesian> adjustment channel '{channel}' is not implemented yet"
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
            let image_column = item_channel_column_name("image");
            if frame.column(&image_column).is_some() {
                let strings = ScalarOrArray::new_array(frame.string_values(&image_column)?)
                    .to_scalar_if_len_one();
                image = coerce_image_strings(coercer, &strings)?;
                image_strings = Some(strings);
            }
            width = ScalarOrArray::new_array(frame.f32_values(&item_channel_column_name("width"))?);
            height =
                ScalarOrArray::new_array(frame.f32_values(&item_channel_column_name("height"))?);
            align = frame
                .string_values(&item_channel_column_name("align"))?
                .iter()
                .map(|value| parse_image_align(value))
                .collect::<Result<Vec<_>, _>>()
                .map(ScalarOrArray::new_array)?;
            baseline = frame
                .string_values(&item_channel_column_name("baseline"))?
                .iter()
                .map(|value| parse_image_baseline(value))
                .collect::<Result<Vec<_>, _>>()
                .map(ScalarOrArray::new_array)?;
            aspect =
                ScalarOrArray::new_array(frame.bool_values(&item_channel_column_name("aspect"))?);
            smooth =
                ScalarOrArray::new_array(frame.bool_values(&item_channel_column_name("smooth"))?);
        }

        Ok((x, y, image, width, height, align, baseline, aspect, smooth))
    }
}

fn build_image_item_frame(
    x: &ScalarOrArray<f32>,
    y: &ScalarOrArray<f32>,
    image_strings: Option<&ScalarOrArray<String>>,
    width: &ScalarOrArray<f32>,
    height: &ScalarOrArray<f32>,
    align: &ScalarOrArray<ImageAlign>,
    baseline: &ScalarOrArray<ImageBaseline>,
    aspect: &ScalarOrArray<bool>,
    smooth: &ScalarOrArray<bool>,
    data: Option<&RecordBatch>,
    len: usize,
) -> Result<MarkEvaluationFrame, AvengerChartError> {
    let x_values = x.as_vec(len, None);
    let y_values = y.as_vec(len, None);
    let width_values = width.as_vec(len, None);
    let height_values = height.as_vec(len, None);
    let align_values = align.as_vec(len, None);
    let baseline_values = baseline.as_vec(len, None);
    let aspect_values = aspect.as_vec(len, None);
    let smooth_values = smooth.as_vec(len, None);
    let mut left = Vec::with_capacity(len);
    let mut right = Vec::with_capacity(len);
    let mut top = Vec::with_capacity(len);
    let mut bottom = Vec::with_capacity(len);

    for ((((x, y), width), height), (align, baseline)) in x_values
        .iter()
        .zip(&y_values)
        .zip(&width_values)
        .zip(&height_values)
        .zip(align_values.iter().zip(&baseline_values))
    {
        let item_left = match align {
            ImageAlign::Left => *x,
            ImageAlign::Center => *x - *width / 2.0,
            ImageAlign::Right => *x - *width,
        };
        let item_top = match baseline {
            ImageBaseline::Top => *y,
            ImageBaseline::Middle => *y - *height / 2.0,
            ImageBaseline::Bottom => *y - *height,
        };
        left.push(item_left);
        right.push(item_left + *width);
        top.push(item_top);
        bottom.push(item_top + *height);
    }

    let mut columns = vec![
        f32_item_column("x", x_values),
        f32_item_column("y", y_values),
        f32_item_column("width", width_values),
        f32_item_column("height", height_values),
        (
            Field::new(item_channel_column_name("align"), DataType::Utf8, true),
            Arc::new(StringArray::from(
                align_values
                    .iter()
                    .map(image_align_name)
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("baseline"), DataType::Utf8, true),
            Arc::new(StringArray::from(
                baseline_values
                    .iter()
                    .map(image_baseline_name)
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("aspect"), DataType::Boolean, true),
            Arc::new(BooleanArray::from(aspect_values)) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("smooth"), DataType::Boolean, true),
            Arc::new(BooleanArray::from(smooth_values)) as ArrayRef,
        ),
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
    if let Some(image_strings) = image_strings {
        columns.push((
            Field::new(item_channel_column_name("image"), DataType::Utf8, true),
            Arc::new(StringArray::from(image_strings.as_vec(len, None))) as ArrayRef,
        ));
    }

    if let Some(data) = data {
        if data.num_rows() != len {
            return Err(AvengerChartError::InternalError(format!(
                "Image adjustment data row count {} did not match item count {len}",
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

fn coerce_image_channel(
    coercer: &Coercer,
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
) -> Result<ScalarOrArray<RgbaImage>, AvengerChartError> {
    if let Some(array) = data.and_then(|data| data.column_by_name("image")) {
        coercer.to_image(array).map_err(|error| {
            AvengerChartError::InternalError(format!("Error coercing channel 'image': {error}"))
        })
    } else if let Some(array) = scalars.column_by_name("image") {
        coercer
            .to_image(array)
            .map(|values| values.to_scalar_if_len_one())
            .map_err(|error| {
                AvengerChartError::InternalError(format!("Error coercing channel 'image': {error}"))
            })
    } else {
        Ok(SceneImageMark::default().image)
    }
}

fn coerce_image_strings(
    coercer: &Coercer,
    values: &ScalarOrArray<String>,
) -> Result<ScalarOrArray<RgbaImage>, AvengerChartError> {
    let array = Arc::new(StringArray::from(values.as_vec(values.len(), None))) as ArrayRef;
    coercer
        .to_image(&array)
        .map(|values| values.to_scalar_if_len_one())
        .map_err(|error| {
            AvengerChartError::InternalError(format!("Error coercing channel 'image': {error}"))
        })
}

fn image_align_name(align: &ImageAlign) -> &'static str {
    match align {
        ImageAlign::Left => "left",
        ImageAlign::Center => "center",
        ImageAlign::Right => "right",
    }
}

fn image_baseline_name(baseline: &ImageBaseline) -> &'static str {
    match baseline {
        ImageBaseline::Top => "top",
        ImageBaseline::Middle => "middle",
        ImageBaseline::Bottom => "bottom",
    }
}

fn parse_image_align(value: &str) -> Result<ImageAlign, AvengerChartError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "left" => Ok(ImageAlign::Left),
        "center" => Ok(ImageAlign::Center),
        "right" => Ok(ImageAlign::Right),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "Invalid image align '{other}'. Expected one of: left, center, right"
        ))),
    }
}

fn parse_image_baseline(value: &str) -> Result<ImageBaseline, AvengerChartError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "top" => Ok(ImageBaseline::Top),
        "middle" => Ok(ImageBaseline::Middle),
        "bottom" => Ok(ImageBaseline::Bottom),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "Invalid image baseline '{other}'. Expected one of: top, middle, bottom"
        ))),
    }
}
