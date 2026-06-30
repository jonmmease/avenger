use std::sync::Arc;

use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{SceneTextLeaderArrow, SceneTextLeaderShape, StrokeCap, StrokeJoin},
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_text::types::{
    FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline, TextSyntaxMode,
};
use datafusion::arrow::{
    array::{ArrayRef, BooleanArray, Float32Array, RecordBatch, StringArray},
    datatypes::{DataType, Field, Schema},
};

use crate::{
    AdjustmentTransformContext, AvengerChartError, CompiledMarkCore, MarkAdjustmentSpec,
    MarkEvaluationFrame, MarkRenderContext, MarkRuntimeContext, PlotAreaInfo, PrimitiveMarkEffects,
    apply_opacity_to_color_channel, coerce_bool_channel_with_renderer,
    coerce_color_channel_with_renderer, coerce_font_style_channel, coerce_font_weight_channel,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel_with_renderer,
    coerce_stroke_cap_channel_values_with_renderer, coerce_stroke_dash_channel,
    coerce_stroke_join_channel_values_with_renderer, coerce_text_align_channel,
    coerce_text_baseline_channel, coerce_text_channel, evaluate_item_assignments,
    item_bbox_column_name, item_channel_column_name, item_channel_name_from_column,
    item_data_column_name, item_data_name_from_column, scalar_params_for_label_sources_lenient,
    stroke_rendering,
};

#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn build_scene_text_mark<M>(
    mark: &M,
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    mark_context: &MarkRenderContext<'_>,
    x: ScalarOrArray<f32>,
    y: ScalarOrArray<f32>,
    len: u32,
    zindex: Option<i32>,
    apply_opacity: bool,
) -> Result<SceneTextMark, AvengerChartError>
where
    M: CompiledMarkCore + ?Sized,
{
    build_scene_text_mark_with_angle(
        mark,
        data,
        scalars,
        mark_context,
        x,
        y,
        None,
        len,
        zindex,
        apply_opacity,
    )
}

#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn build_scene_text_mark_with_angle<M>(
    mark: &M,
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    mark_context: &MarkRenderContext<'_>,
    x: ScalarOrArray<f32>,
    y: ScalarOrArray<f32>,
    angle_override: Option<ScalarOrArray<f32>>,
    len: u32,
    zindex: Option<i32>,
    apply_opacity: bool,
) -> Result<SceneTextMark, AvengerChartError>
where
    M: CompiledMarkCore + ?Sized,
{
    let text = coerce_text_channel(data, scalars, "text", String::new())?;
    let align = coerce_text_align_channel(data, scalars, "align", TextAlign::Left)?;
    let baseline =
        coerce_text_baseline_channel(data, scalars, "baseline", TextBaseline::Alphabetic)?;
    let angle = if let Some(angle) = angle_override {
        angle
    } else {
        coerce_numeric_channel_with_renderer(mark, data, scalars, "angle", mark_context, 0.0)?
    };
    let color = coerce_color_channel_with_renderer(
        mark,
        data,
        scalars,
        "color",
        mark_context,
        [0.0, 0.0, 0.0, 1.0],
    )?;
    let font = coerce_text_channel(data, scalars, "font", "sans-serif".to_string())?;
    let font_size =
        coerce_numeric_channel_with_renderer(mark, data, scalars, "font_size", mark_context, 10.0)?;
    let font_weight = coerce_font_weight_channel(
        data,
        scalars,
        "font_weight",
        FontWeight::Name(FontWeightNameSpec::Normal),
    )?;
    let font_style = coerce_font_style_channel(data, scalars, "font_style", FontStyle::Normal)?;
    let limit =
        coerce_numeric_channel_with_renderer(mark, data, scalars, "limit", mark_context, 0.0)?;
    let opacity =
        coerce_opacity_channel_with_renderer(mark, data, scalars, "opacity", mark_context, 1.0)?;
    let rendered_color = if apply_opacity {
        apply_opacity_to_color_channel(color.clone(), &opacity, len as usize)
    } else {
        color
    };
    let defined =
        coerce_bool_channel_with_renderer(mark, data, scalars, "defined", mark_context, true)?;
    let leader =
        coerce_bool_channel_with_renderer(mark, data, scalars, "leader", mark_context, false)?;
    let leader_offset_x = coerce_numeric_channel_with_renderer(
        mark,
        data,
        scalars,
        "leader_offset_x",
        mark_context,
        0.0,
    )?;
    let leader_offset_y = coerce_numeric_channel_with_renderer(
        mark,
        data,
        scalars,
        "leader_offset_y",
        mark_context,
        0.0,
    )?;
    let leader_stroke = coerce_color_channel_with_renderer(
        mark,
        data,
        scalars,
        "leader_stroke",
        mark_context,
        [0.0, 0.0, 0.0, 0.7],
    )?;
    let rendered_leader_stroke = if apply_opacity {
        apply_opacity_to_color_channel(leader_stroke.clone(), &opacity, len as usize)
    } else {
        leader_stroke
    };
    let leader_stroke_width = coerce_numeric_channel_with_renderer(
        mark,
        data,
        scalars,
        "leader_stroke_width",
        mark_context,
        1.0,
    )?;
    let leader_stroke_dash = stroke_rendering::optional_stroke_dash(coerce_stroke_dash_channel(
        data,
        scalars,
        "leader_stroke_dash",
    )?);
    let leader_stroke_cap = coerce_stroke_cap_channel_values_with_renderer(
        mark,
        data,
        scalars,
        "leader_stroke_cap",
        mark_context,
        StrokeCap::Round,
    )?;
    let leader_stroke_join = coerce_stroke_join_channel_values_with_renderer(
        mark,
        data,
        scalars,
        "leader_stroke_join",
        mark_context,
        StrokeJoin::Round,
    )?;
    let leader_label_padding = coerce_numeric_channel_with_renderer(
        mark,
        data,
        scalars,
        "leader_label_padding",
        mark_context,
        2.0,
    )?;
    let leader_target_radius = coerce_numeric_channel_with_renderer(
        mark,
        data,
        scalars,
        "leader_target_radius",
        mark_context,
        0.0,
    )?;
    let leader_min_length = coerce_numeric_channel_with_renderer(
        mark,
        data,
        scalars,
        "leader_min_length",
        mark_context,
        1.0,
    )?;
    let leader_shape = coerce_text_leader_shape_channel(
        data,
        scalars,
        "leader_shape",
        SceneTextLeaderShape::Straight,
    )?;
    let leader_arrow = coerce_text_leader_arrow_channel(
        data,
        scalars,
        "leader_arrow",
        SceneTextLeaderArrow::None,
    )?;
    let leader_arrow_length = coerce_numeric_channel_with_renderer(
        mark,
        data,
        scalars,
        "leader_arrow_length",
        mark_context,
        6.0,
    )?;
    let leader_arrow_width = coerce_numeric_channel_with_renderer(
        mark,
        data,
        scalars,
        "leader_arrow_width",
        mark_context,
        5.0,
    )?;

    Ok(SceneTextMark {
        name: "text".to_string(),
        clip: true,
        len,
        text,
        text_syntax: TextSyntaxMode::Plain,
        text_params: avenger_text::empty_label_params().clone(),
        number_locale: None,
        number_locale_specs: avenger_text::NumberLocaleSpecs::default(),
        datetime_locale: None,
        datetime_timezone: None,
        datetime_locale_specs: avenger_text::DateTimeLocaleSpecs::default(),
        x,
        y,
        defined,
        dx: leader_offset_x,
        dy: leader_offset_y,
        align,
        baseline,
        angle,
        color: rendered_color,
        opacity,
        font,
        font_size,
        font_weight,
        font_style,
        limit,
        leader,
        leader_stroke: rendered_leader_stroke,
        leader_stroke_width,
        leader_stroke_cap,
        leader_stroke_join,
        leader_stroke_dash,
        leader_label_padding,
        leader_target_radius,
        leader_min_length,
        leader_shape,
        leader_arrow,
        leader_arrow_length,
        leader_arrow_width,
        indices: None,
        zindex,
        interactive: true,
    })
}

#[doc(hidden)]
pub fn apply_text_syntax_and_params(
    mark: &mut SceneTextMark,
    syntax_mode: TextSyntaxMode,
    mark_context: &MarkRenderContext<'_>,
) {
    mark.text_syntax = syntax_mode;
    if syntax_mode == TextSyntaxMode::Plain {
        mark.text_params = avenger_text::empty_label_params().clone();
        mark.number_locale = None;
        mark.number_locale_specs = avenger_text::NumberLocaleSpecs::default();
        mark.datetime_locale = None;
        mark.datetime_timezone = None;
        mark.datetime_locale_specs = avenger_text::DateTimeLocaleSpecs::default();
        return;
    }

    let text_values = mark.text.as_vec(mark.len as usize, None);
    mark.text_params = scalar_params_for_label_sources_lenient(
        text_values.iter().map(String::as_str),
        syntax_mode,
        mark_context.params(),
    );
    mark.number_locale = Some(
        mark_context
            .eval()
            .formatting_context()
            .resolved_number_locale()
            .to_string(),
    );
    mark.number_locale_specs = mark_context
        .eval()
        .formatting_context()
        .number_locale_specs()
        .clone();
    mark.datetime_locale = Some(
        mark_context
            .eval()
            .formatting_context()
            .resolved_datetime_locale()
            .to_string(),
    );
    mark.datetime_timezone = Some(
        mark_context
            .eval()
            .formatting_context()
            .resolved_datetime_timezone()
            .to_string(),
    );
    mark.datetime_locale_specs = mark_context
        .eval()
        .formatting_context()
        .datetime_locale_specs()
        .clone();
}

#[doc(hidden)]
pub fn apply_text_adjustments<M>(
    mark: &M,
    text_mark: SceneTextMark,
    source_data: Option<&RecordBatch>,
    source_frame: Option<&MarkEvaluationFrame>,
    context: &dyn MarkRuntimeContext,
    effects: &PrimitiveMarkEffects,
    zindex: Option<i32>,
) -> Result<SceneTextMark, AvengerChartError>
where
    M: CompiledMarkCore + ?Sized,
{
    if effects.adjustments.is_empty() {
        return Ok(text_mark);
    }

    let len = text_mark.len as usize;
    let mut frame = build_text_item_frame(&text_mark, source_data, source_frame)?;
    for adjustment in &effects.adjustments {
        let assignments = match adjustment {
            MarkAdjustmentSpec::Expr(spec) => &spec.assignments,
            MarkAdjustmentSpec::Transform(spec) => {
                let requirements = spec.transform.requirements();
                let mut adjustment_context =
                    AdjustmentTransformContext::new().with_plot_area(PlotAreaInfo {
                        facet_path: context.facet_path(),
                        width: context.plot_width(),
                        height: context.plot_height(),
                        origin: context.plot_area_origin(),
                        clip: context.plot_area_clip(),
                    });
                if requirements.source_frame
                    && let Some(source_frame) = source_frame
                {
                    adjustment_context = adjustment_context.with_source(source_frame);
                }
                if requirements.base_scene
                    && let Some(base_scene) = context.base_plot_area_scene()
                {
                    adjustment_context = adjustment_context.with_base_scene(base_scene);
                }
                if requirements.text_measurement
                    && let Some(text_measurement) = context.text_measurement_service()
                {
                    adjustment_context = adjustment_context.with_text_measurement(text_measurement);
                }
                spec.transform.apply(&mut frame, &adjustment_context)?;
                &spec.assignments
            }
        };
        if assignments.is_empty() {
            continue;
        }

        let item_batch = frame.record_batch()?;
        let output_batch = evaluate_item_assignments(
            assignments.iter(),
            &item_batch,
            context.core_view().session_context().as_ref(),
        )?;
        for (index, assignment) in assignments.iter().enumerate() {
            frame.set_column(
                item_channel_column_name(&assignment.channel),
                output_batch.column(index).clone(),
            )?;
        }
        refresh_text_bbox_columns(&mut frame)?;
    }

    let adjusted_channels = text_channel_batch_from_item_frame(&frame)?;
    let mark_context = context.core_view();
    let x = coerce_numeric_channel_with_renderer(
        mark,
        None,
        &adjusted_channels,
        "x",
        &mark_context,
        0.0,
    )?;
    let y = coerce_numeric_channel_with_renderer(
        mark,
        None,
        &adjusted_channels,
        "y",
        &mark_context,
        0.0,
    )?;
    let mut adjusted_mark = build_scene_text_mark(
        mark,
        None,
        &adjusted_channels,
        &mark_context,
        x,
        y,
        len as u32,
        zindex,
        true,
    )?;
    apply_text_syntax_and_params(&mut adjusted_mark, text_mark.text_syntax, &mark_context);
    Ok(adjusted_mark)
}

fn build_text_item_frame(
    text_mark: &SceneTextMark,
    source_data: Option<&RecordBatch>,
    source_frame: Option<&MarkEvaluationFrame>,
) -> Result<MarkEvaluationFrame, AvengerChartError> {
    let len = text_mark.len as usize;
    let x_values = text_mark.x.as_vec(len, None);
    let y_values = text_mark.y.as_vec(len, None);
    let leader_offset_x_values = text_mark.dx.as_vec(len, None);
    let leader_offset_y_values = text_mark.dy.as_vec(len, None);
    let label_x_values = add_f32_vecs(&x_values, &leader_offset_x_values);
    let label_y_values = add_f32_vecs(&y_values, &leader_offset_y_values);
    let mut columns: Vec<(Field, ArrayRef)> = vec![
        f32_item_channel("x", x_values.clone()),
        f32_item_channel("y", y_values.clone()),
        f32_item_channel("leader_offset_x", leader_offset_x_values),
        f32_item_channel("leader_offset_y", leader_offset_y_values),
        f32_item_channel("angle", text_mark.angle.as_vec(len, None)),
        f32_item_channel("font_size", text_mark.font_size.as_vec(len, None)),
        f32_item_channel("limit", text_mark.limit.as_vec(len, None)),
        f32_item_channel("opacity", text_mark.opacity.as_vec(len, None)),
        f32_item_channel(
            "leader_stroke_width",
            text_mark.leader_stroke_width.as_vec(len, None),
        ),
        f32_item_channel(
            "leader_label_padding",
            text_mark.leader_label_padding.as_vec(len, None),
        ),
        f32_item_channel(
            "leader_target_radius",
            text_mark.leader_target_radius.as_vec(len, None),
        ),
        f32_item_channel(
            "leader_min_length",
            text_mark.leader_min_length.as_vec(len, None),
        ),
        f32_item_channel(
            "leader_arrow_length",
            text_mark.leader_arrow_length.as_vec(len, None),
        ),
        f32_item_channel(
            "leader_arrow_width",
            text_mark.leader_arrow_width.as_vec(len, None),
        ),
        bool_item_channel("defined", text_mark.defined.as_vec(len, None)),
        bool_item_channel("leader", text_mark.leader.as_vec(len, None)),
        string_item_channel(
            "color",
            stroke_rendering::color_channel_strings(
                &text_raw_color_channel(&text_mark.color, &text_mark.opacity, len),
                len,
            ),
        ),
        string_item_channel(
            "leader_stroke",
            stroke_rendering::color_channel_strings(
                &text_raw_color_channel(&text_mark.leader_stroke, &text_mark.opacity, len),
                len,
            ),
        ),
        string_item_channel(
            "leader_stroke_dash",
            stroke_rendering::stroke_dash_strings(&text_mark.leader_stroke_dash, len),
        ),
        string_item_channel(
            "leader_stroke_cap",
            stroke_rendering::stroke_cap_strings(&text_mark.leader_stroke_cap, len),
        ),
        string_item_channel(
            "leader_stroke_join",
            stroke_rendering::stroke_join_strings(&text_mark.leader_stroke_join, len),
        ),
        string_item_channel(
            "leader_shape",
            text_mark
                .leader_shape
                .as_vec(len, None)
                .into_iter()
                .map(leader_shape_name)
                .collect(),
        ),
        string_item_channel(
            "leader_arrow",
            text_mark
                .leader_arrow
                .as_vec(len, None)
                .into_iter()
                .map(leader_arrow_name)
                .collect(),
        ),
        string_item_channel("text", text_mark.text.as_vec(len, None)),
        string_item_channel("font", text_mark.font.as_vec(len, None)),
        string_item_channel(
            "align",
            text_mark
                .align
                .as_vec(len, None)
                .into_iter()
                .map(text_align_name)
                .collect(),
        ),
        string_item_channel(
            "baseline",
            text_mark
                .baseline
                .as_vec(len, None)
                .into_iter()
                .map(text_baseline_name)
                .collect(),
        ),
        string_item_channel(
            "font_weight",
            text_mark
                .font_weight
                .as_vec(len, None)
                .into_iter()
                .map(font_weight_name)
                .collect(),
        ),
        string_item_channel(
            "font_style",
            text_mark
                .font_style
                .as_vec(len, None)
                .into_iter()
                .map(font_style_name)
                .collect(),
        ),
        f32_item_bbox("left", label_x_values.clone()),
        f32_item_bbox("right", label_x_values),
        f32_item_bbox("top", label_y_values.clone()),
        f32_item_bbox("bottom", label_y_values),
    ];

    if let Some(data) = source_data {
        if data.num_rows() != len {
            return Err(AvengerChartError::InternalError(format!(
                "Text adjustment data row count {} did not match item count {len}",
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

    if let Some(source_frame) = source_frame {
        let source_batch = source_frame.record_batch()?;
        if source_batch.num_rows() != len {
            return Err(AvengerChartError::InternalError(format!(
                "Derived text source-frame row count {} did not match item count {len}",
                source_batch.num_rows()
            )));
        }
        for (index, field) in source_batch.schema().fields().iter().enumerate() {
            if item_data_name_from_column(field.name()).is_some() {
                columns.push((field.as_ref().clone(), source_batch.column(index).clone()));
            }
        }
    }

    Ok(MarkEvaluationFrame::new(len, columns))
}

fn f32_item_channel(channel: &str, values: Vec<f32>) -> (Field, ArrayRef) {
    let name = item_channel_column_name(channel);
    (
        Field::new(name, DataType::Float32, true),
        Arc::new(Float32Array::from(values)),
    )
}

fn f32_item_bbox(channel: &str, values: Vec<f32>) -> (Field, ArrayRef) {
    let name = item_bbox_column_name(channel);
    (
        Field::new(name, DataType::Float32, true),
        Arc::new(Float32Array::from(values)),
    )
}

fn add_f32_vecs(lhs: &[f32], rhs: &[f32]) -> Vec<f32> {
    lhs.iter()
        .zip(rhs.iter())
        .map(|(lhs, rhs)| lhs + rhs)
        .collect()
}

fn text_raw_color_channel(
    rendered: &ScalarOrArray<ColorOrGradient>,
    opacity: &ScalarOrArray<f32>,
    len: usize,
) -> ScalarOrArray<ColorOrGradient> {
    let colors = rendered.as_vec(len, None);
    let opacities = opacity.as_vec(len, None);
    ScalarOrArray::new_array(
        colors
            .into_iter()
            .zip(opacities)
            .map(|(color, opacity)| text_raw_color(color, opacity))
            .collect(),
    )
    .to_scalar_if_len_one()
}

fn text_raw_color(color: ColorOrGradient, opacity: f32) -> ColorOrGradient {
    match color {
        ColorOrGradient::Color(mut rgba) => {
            if opacity > f32::EPSILON {
                rgba[3] = (rgba[3] / opacity).clamp(0.0, 1.0);
            }
            ColorOrGradient::Color(rgba)
        }
        ColorOrGradient::GradientIndex(index) => ColorOrGradient::GradientIndex(index),
    }
}

fn refresh_text_bbox_columns(frame: &mut MarkEvaluationFrame) -> Result<(), AvengerChartError> {
    let x = frame.f32_values(&item_channel_column_name("x"))?;
    let y = frame.f32_values(&item_channel_column_name("y"))?;
    let leader_offset_x = frame.f32_values(&item_channel_column_name("leader_offset_x"))?;
    let leader_offset_y = frame.f32_values(&item_channel_column_name("leader_offset_y"))?;
    let label_x = add_f32_vecs(&x, &leader_offset_x);
    let label_y = add_f32_vecs(&y, &leader_offset_y);
    frame.set_column(
        item_bbox_column_name("left"),
        Arc::new(Float32Array::from(label_x.clone())) as ArrayRef,
    )?;
    frame.set_column(
        item_bbox_column_name("right"),
        Arc::new(Float32Array::from(label_x)) as ArrayRef,
    )?;
    frame.set_column(
        item_bbox_column_name("top"),
        Arc::new(Float32Array::from(label_y.clone())) as ArrayRef,
    )?;
    frame.set_column(
        item_bbox_column_name("bottom"),
        Arc::new(Float32Array::from(label_y)) as ArrayRef,
    )?;
    Ok(())
}

fn bool_item_channel(channel: &str, values: Vec<bool>) -> (Field, ArrayRef) {
    let name = item_channel_column_name(channel);
    (
        Field::new(name, DataType::Boolean, true),
        Arc::new(BooleanArray::from(values)),
    )
}

fn string_item_channel(channel: &str, values: Vec<String>) -> (Field, ArrayRef) {
    let name = item_channel_column_name(channel);
    (
        Field::new(name, DataType::Utf8, true),
        Arc::new(StringArray::from(values)),
    )
}

fn text_channel_batch_from_item_frame(
    frame: &MarkEvaluationFrame,
) -> Result<RecordBatch, AvengerChartError> {
    let item_batch = frame.record_batch()?;
    let mut fields = Vec::new();
    let mut arrays = Vec::new();
    for (index, field) in item_batch.schema().fields().iter().enumerate() {
        if let Some(channel) = item_channel_name_from_column(field.name()) {
            fields.push(Field::new(
                channel.to_string(),
                field.data_type().clone(),
                field.is_nullable(),
            ));
            arrays.push(item_batch.column(index).clone());
        }
    }
    Ok(RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)?)
}

fn text_align_name(value: TextAlign) -> String {
    match value {
        TextAlign::Left => "left",
        TextAlign::Center => "center",
        TextAlign::Right => "right",
    }
    .to_string()
}

fn text_baseline_name(value: TextBaseline) -> String {
    match value {
        TextBaseline::Alphabetic => "alphabetic",
        TextBaseline::Top => "top",
        TextBaseline::Middle => "middle",
        TextBaseline::Bottom => "bottom",
        TextBaseline::LineTop => "line-top",
        TextBaseline::LineBottom => "line-bottom",
    }
    .to_string()
}

fn font_weight_name(value: FontWeight) -> String {
    match value {
        FontWeight::Name(FontWeightNameSpec::Normal) => "normal".to_string(),
        FontWeight::Name(FontWeightNameSpec::Bold) => "bold".to_string(),
        FontWeight::Number(value) => value.to_string(),
    }
}

fn font_style_name(value: FontStyle) -> String {
    match value {
        FontStyle::Normal => "normal",
        FontStyle::Italic => "italic",
    }
    .to_string()
}

fn coerce_text_leader_shape_channel(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    default: SceneTextLeaderShape,
) -> Result<ScalarOrArray<SceneTextLeaderShape>, AvengerChartError> {
    let values = coerce_text_channel(data, scalars, channel, leader_shape_name(default))?;
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => {
            Ok(ScalarOrArray::new_scalar(parse_text_leader_shape(value)?))
        }
        ScalarOrArrayValue::Array(values) => values
            .iter()
            .map(|value| parse_text_leader_shape(value))
            .collect::<Result<Vec<_>, _>>()
            .map(ScalarOrArray::new_array),
    }
}

fn coerce_text_leader_arrow_channel(
    data: Option<&RecordBatch>,
    scalars: &RecordBatch,
    channel: &str,
    default: SceneTextLeaderArrow,
) -> Result<ScalarOrArray<SceneTextLeaderArrow>, AvengerChartError> {
    let values = coerce_text_channel(data, scalars, channel, leader_arrow_name(default))?;
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => {
            Ok(ScalarOrArray::new_scalar(parse_text_leader_arrow(value)?))
        }
        ScalarOrArrayValue::Array(values) => values
            .iter()
            .map(|value| parse_text_leader_arrow(value))
            .collect::<Result<Vec<_>, _>>()
            .map(ScalarOrArray::new_array),
    }
}

fn parse_text_leader_shape(value: &str) -> Result<SceneTextLeaderShape, AvengerChartError> {
    match value.to_ascii_lowercase().as_str() {
        "straight" => Ok(SceneTextLeaderShape::Straight),
        "elbow" => Ok(SceneTextLeaderShape::Elbow),
        "curved" => Ok(SceneTextLeaderShape::Curved),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "Invalid text leader shape '{other}'. Expected one of: straight, elbow, curved"
        ))),
    }
}

fn parse_text_leader_arrow(value: &str) -> Result<SceneTextLeaderArrow, AvengerChartError> {
    match value.to_ascii_lowercase().as_str() {
        "none" => Ok(SceneTextLeaderArrow::None),
        "open" => Ok(SceneTextLeaderArrow::Open),
        "triangle" => Ok(SceneTextLeaderArrow::Triangle),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "Invalid text leader arrow '{other}'. Expected one of: none, open, triangle"
        ))),
    }
}

fn leader_shape_name(value: SceneTextLeaderShape) -> String {
    match value {
        SceneTextLeaderShape::Straight => "straight",
        SceneTextLeaderShape::Elbow => "elbow",
        SceneTextLeaderShape::Curved => "curved",
    }
    .to_string()
}

fn leader_arrow_name(value: SceneTextLeaderArrow) -> String {
    match value {
        SceneTextLeaderArrow::None => "none",
        SceneTextLeaderArrow::Open => "open",
        SceneTextLeaderArrow::Triangle => "triangle",
    }
    .to_string()
}
