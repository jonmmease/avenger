use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use avenger_chart_core::{
    AdjustmentTransformRequirements, AvengerChartError, ChannelDescriptor, CompiledDataContext,
    CompiledMark, CompiledMarkCore, CompiledMarkState, CompiledScalarExpressionProgram,
    CoordinateSystemTransformCore, DerivedPrimitiveMarkSpec, DerivedRectMarkSpec,
    ItemChannelAssignment, LegendRendererKind, LegendRendererSelection, Mark, MarkAdjustmentSpec,
    MarkEvaluationFrame, MarkRenderContext, MarkRuntimeContext, PhysicalScalarExpressionSpec,
    PhysicalScalarProgramOptions, PointGeometry, PrimitiveMarkEffects, RenderedMarkData,
    ScaleTypePreference, apply_opacity_to_color_channel, coerce_color_channel_with_renderer,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel_with_renderer,
    default_scale_type_for_data_type, impl_mark_trait_common, is_continuous_scale,
    item_bbox_column_name, item_channel_column_name, item_data_column_name,
};
use avenger_chart_marks::{Rect, rect_channel_defaults};
use avenger_color::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{mark::SceneMark, rect::SceneRectMark};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float32Array, RecordBatch, StringArray},
        datatypes::{DataType, Field, Schema},
    },
    common::ScalarValue,
    prelude::SessionContext,
};
use serde::{Deserialize, Serialize};

use crate::{Cartesian, marks::util};

// Implement Mark trait for Cartesian Rect with any axis type
#[async_trait::async_trait]
impl Mark<Cartesian> for Rect<Cartesian> {
    impl_mark_trait_common!(Rect);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledCartesianRect {
            state: compiled_state,
            effects: self.mark_effects().clone(),
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianRect {
    pub(crate) state: CompiledMarkState,
    #[serde(default)]
    pub(crate) effects: PrimitiveMarkEffects,
}

impl CompiledMarkCore for CompiledCartesianRect {
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
        "rect"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            // Position channels - rectangles need all four corners
            ChannelDescriptor {
                name: "x",
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
                name: "y",
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
            // Style channels
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
                name: "corner_radius",
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
        rect_channel_defaults(channel)
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
            // Rect marks use band scales for categorical position data
            (
                "x" | "x2" | "y" | "y2",
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            ) => Some(ScaleTypePreference::Band),
            // Color channels use ordinal scales for categorical data
            (
                "fill" | "stroke" | "color",
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            ) => Some(ScaleTypePreference::Ordinal),
            // Fall back to data type-based inference for other channels
            _ => default_scale_type_for_data_type(data_type),
        }
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        // Check if scale is continuous (for colorbar)
        let is_continuous = is_continuous_scale(scale.scale_impl.as_ref());

        match channel {
            // Use colorbar for continuous color scales
            "fill" | "stroke" | "color" if is_continuous => Some(LegendRendererSelection::BuiltIn(
                LegendRendererKind::Colorbar,
            )),
            // Rect marks use rect legend rendering for discrete scales and other visual properties.
            "fill" | "stroke" | "color" | "opacity" | "stroke_width" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Rect))
            }
            // No legend for position channels and other non-visual channels
            "x" | "y" | "x2" | "y2" | "width" | "height" | "defined" | "order"
            | "corner_radius" => None,
            // For any other channel, default to rect legend rendering.
            _ => Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Rect)),
        }
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianRect {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let (mark, _) = self.render_rect_scene(data, scalars, context, coord, false)?;
        Ok(vec![mark])
    }

    async fn render_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        let include_item_frame = self.effects.has_derived();
        let (base_mark, source_frame) =
            self.render_rect_scene(data, scalars, context, coord, include_item_frame)?;
        let mut marks = vec![base_mark];
        if include_item_frame {
            let source_frame = source_frame.ok_or_else(|| {
                AvengerChartError::InternalError(
                    "Derived Rect rendering expected a source item frame".to_string(),
                )
            })?;
            for derived in &self.effects.derived {
                match derived {
                    DerivedPrimitiveMarkSpec::Rect(spec) => {
                        marks.push(self.render_derived_rect(spec, &source_frame, context)?);
                    }
                    other => {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "Rect<Cartesian> cannot derive {other:?} yet"
                        )));
                    }
                }
            }
        }
        Ok(RenderedMarkData::new(marks))
    }

    fn has_render_stage_derived(&self) -> bool {
        self.effects.has_derived()
    }

    fn derived_adjustment_requirements(&self) -> AdjustmentTransformRequirements {
        let mut requirements = AdjustmentTransformRequirements::default();
        for derived in &self.effects.derived {
            requirements.merge(derived.transform_requirements());
        }
        requirements
    }

    async fn render_base_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        let (base_mark, _) = self.render_rect_scene(data, scalars, context, coord, false)?;
        Ok(RenderedMarkData::new(vec![base_mark]))
    }

    async fn render_derived_mark_data(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        if !self.effects.has_derived() {
            return Ok(RenderedMarkData::new(Vec::new()));
        }
        let (_, source_frame) = self.render_rect_scene(data, scalars, context, coord, true)?;
        let source_frame = source_frame.ok_or_else(|| {
            AvengerChartError::InternalError(
                "Derived Rect rendering expected a source item frame".to_string(),
            )
        })?;
        let mut marks = Vec::new();
        for derived in &self.effects.derived {
            match derived {
                DerivedPrimitiveMarkSpec::Rect(spec) => {
                    marks.push(self.render_derived_rect(spec, &source_frame, context)?);
                }
                other => {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Rect<Cartesian> cannot derive {other:?} yet"
                    )));
                }
            }
        }
        Ok(RenderedMarkData::new(marks))
    }
}

impl CompiledCartesianRect {
    fn coerce_rect_visual_channels(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        mark_context: &MarkRenderContext<'_>,
    ) -> Result<RectVisualChannels, AvengerChartError> {
        let fill = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "fill",
            mark_context,
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
        )?;
        let stroke = coerce_color_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke",
            mark_context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let stroke_width = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "stroke_width",
            mark_context,
            1.0,
        )?;
        let opacity = coerce_opacity_channel_with_renderer(
            self,
            data,
            scalars,
            "opacity",
            mark_context,
            1.0,
        )?;
        Ok(RectVisualChannels {
            fill,
            stroke,
            stroke_width,
            opacity,
        })
    }

    fn render_rect_scene(
        &self,
        data: Option<&RecordBatch>,
        scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        coord: &dyn CoordinateSystemTransformCore,
        include_item_frame: bool,
    ) -> Result<(SceneMark, Option<MarkEvaluationFrame>), AvengerChartError> {
        let mark_context = context.core_view();

        // Determine number of marks from data batch or default to 1
        let len = data.map_or(1, |data| data.num_rows());

        // Extract position channels for the coordinate system
        // For Cartesian rectangles, we need to transform both corners
        let mut position_channels_corner1 = HashMap::new();
        let mut position_channels_corner2 = HashMap::new();

        // Extract raw position values
        let x_raw =
            coerce_numeric_channel_with_renderer(self, data, scalars, "x", &mark_context, 0.0)?;
        let x2_raw =
            coerce_numeric_channel_with_renderer(self, data, scalars, "x2", &mark_context, 0.0)?;
        let y_raw =
            coerce_numeric_channel_with_renderer(self, data, scalars, "y", &mark_context, 0.0)?;
        let y2_raw =
            coerce_numeric_channel_with_renderer(self, data, scalars, "y2", &mark_context, 0.0)?;

        // Set up position channels for first corner (x, y)
        position_channels_corner1.insert("x", x_raw.clone());
        position_channels_corner1.insert("y", y_raw.clone());

        // Set up position channels for second corner (x2, y2)
        position_channels_corner2.insert("x", x2_raw.clone());
        position_channels_corner2.insert("y", y2_raw.clone());

        // Transform both corners through the coordinate system
        let geometry1 = coord.transform(
            &position_channels_corner1,
            None,
            context.plot_width(),
            context.plot_height(),
        )?;
        let geometry2 = coord.transform(
            &position_channels_corner2,
            None,
            context.plot_width(),
            context.plot_height(),
        )?;

        // Extract transformed coordinates as PointGeometry
        let point1 = geometry1
            .as_any()
            .downcast_ref::<PointGeometry>()
            .ok_or_else(|| {
                AvengerChartError::CoordinateSystemError(
                    "Failed to downcast corner1 to PointGeometry".to_string(),
                )
            })?;
        let point2 = geometry2
            .as_any()
            .downcast_ref::<PointGeometry>()
            .ok_or_else(|| {
                AvengerChartError::CoordinateSystemError(
                    "Failed to downcast corner2 to PointGeometry".to_string(),
                )
            })?;

        // Use the transformed coordinates
        let x = point1.x.clone();
        let y = point1.y.clone();
        let x2 = point2.x.clone();
        let y2 = point2.y.clone();
        let corner_radius = coerce_numeric_channel_with_renderer(
            self,
            data,
            scalars,
            "corner_radius",
            &mark_context,
            0.0,
        )?;
        let visual = self.coerce_rect_visual_channels(data, scalars, &mark_context)?;
        let (x, y, x2, y2, corner_radius, visual) = self.apply_expression_adjustments(
            x,
            y,
            x2,
            y2,
            corner_radius,
            visual,
            data,
            len,
            context,
            &mark_context,
        )?;

        let source_frame = if include_item_frame {
            Some(build_rect_item_frame(
                &x,
                &y,
                &x2,
                &y2,
                &corner_radius,
                &visual,
                data,
                len,
            )?)
        } else {
            None
        };

        let rect_mark = self.build_scene_rect_mark(
            x,
            y,
            x2,
            y2,
            corner_radius,
            visual,
            len,
            self.state.zindex,
        )?;

        Ok((SceneMark::Rect(rect_mark), source_frame))
    }

    fn build_scene_rect_mark(
        &self,
        x: ScalarOrArray<f32>,
        y: ScalarOrArray<f32>,
        x2: ScalarOrArray<f32>,
        y2: ScalarOrArray<f32>,
        corner_radius: ScalarOrArray<f32>,
        visual: RectVisualChannels,
        len: usize,
        zindex: Option<i32>,
    ) -> Result<SceneRectMark, AvengerChartError> {
        let fill = apply_opacity_to_color_channel(visual.fill, &visual.opacity, len);
        let stroke = apply_opacity_to_color_channel(visual.stroke, &visual.opacity, len);

        Ok(SceneRectMark {
            name: "rect".to_string(),
            clip: true,
            len: len as u32,
            gradients: vec![],
            x,
            y,
            width: None,
            height: None,
            x2: Some(x2),
            y2: Some(y2),
            fill,
            stroke,
            stroke_width: visual.stroke_width,
            corner_radius,
            indices: None,
            zindex,
            interactive: true,
        })
    }

    fn apply_expression_adjustments(
        &self,
        mut x: ScalarOrArray<f32>,
        mut y: ScalarOrArray<f32>,
        mut x2: ScalarOrArray<f32>,
        mut y2: ScalarOrArray<f32>,
        mut corner_radius: ScalarOrArray<f32>,
        mut visual: RectVisualChannels,
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
            ScalarOrArray<f32>,
            RectVisualChannels,
        ),
        AvengerChartError,
    > {
        if self.effects.adjustments.is_empty() {
            return Ok((x, y, x2, y2, corner_radius, visual));
        }

        for adjustment in &self.effects.adjustments {
            let mut frame =
                build_rect_item_frame(&x, &y, &x2, &y2, &corner_radius, &visual, data, len)?;
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
                    "x" | "y" | "x2" | "y2" | "corner_radius" | "fill" | "stroke"
                    | "stroke_width" | "opacity" => {}
                    channel => {
                        return Err(AvengerChartError::InvalidArgument(format!(
                            "Rect<Cartesian> adjustment channel '{channel}' is not implemented yet"
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
            corner_radius = ScalarOrArray::new_array(
                frame.f32_values(&item_channel_column_name("corner_radius"))?,
            );
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
            visual.opacity =
                ScalarOrArray::new_array(frame.f32_values(&item_channel_column_name("opacity"))?);
        }

        Ok((x, y, x2, y2, corner_radius, visual))
    }

    fn render_derived_rect(
        &self,
        spec: &DerivedRectMarkSpec,
        source_frame: &MarkEvaluationFrame,
        context: &dyn MarkRuntimeContext,
    ) -> Result<SceneMark, AvengerChartError> {
        let item_batch = source_frame.record_batch()?;
        let mark_context = context.core_view();
        let derived_scalars = evaluate_item_assignments(
            spec.assignments.iter(),
            &item_batch,
            mark_context.session_context().as_ref(),
        )?;
        let len = source_frame.len();
        let x = coerce_numeric_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "x",
            &mark_context,
            0.0,
        )?;
        let y = coerce_numeric_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "y",
            &mark_context,
            0.0,
        )?;
        let x2 = coerce_numeric_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "x2",
            &mark_context,
            0.0,
        )?;
        let y2 = coerce_numeric_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "y2",
            &mark_context,
            0.0,
        )?;
        let corner_radius = coerce_numeric_channel_with_renderer(
            self,
            None,
            &derived_scalars,
            "corner_radius",
            &mark_context,
            0.0,
        )?;
        let visual = self.coerce_rect_visual_channels(None, &derived_scalars, &mark_context)?;
        Ok(SceneMark::Rect(self.build_scene_rect_mark(
            x,
            y,
            x2,
            y2,
            corner_radius,
            visual,
            len,
            spec.zindex,
        )?))
    }
}

#[derive(Clone)]
struct RectVisualChannels {
    fill: ScalarOrArray<ColorOrGradient>,
    stroke: ScalarOrArray<ColorOrGradient>,
    stroke_width: ScalarOrArray<f32>,
    opacity: ScalarOrArray<f32>,
}

fn build_rect_item_frame(
    x: &ScalarOrArray<f32>,
    y: &ScalarOrArray<f32>,
    x2: &ScalarOrArray<f32>,
    y2: &ScalarOrArray<f32>,
    corner_radius: &ScalarOrArray<f32>,
    visual: &RectVisualChannels,
    data: Option<&RecordBatch>,
    len: usize,
) -> Result<MarkEvaluationFrame, AvengerChartError> {
    let x_values = x.as_vec(len, None);
    let y_values = y.as_vec(len, None);
    let x2_values = x2.as_vec(len, None);
    let y2_values = y2.as_vec(len, None);
    let corner_radius_values = corner_radius.as_vec(len, None);
    let fill_values = util::color_channel_strings(&visual.fill, len);
    let stroke_values = util::color_channel_strings(&visual.stroke, len);
    let stroke_width_values = visual.stroke_width.as_vec(len, None);
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
        (
            Field::new(item_channel_column_name("x"), DataType::Float32, true),
            Arc::new(Float32Array::from(x_values)) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("y"), DataType::Float32, true),
            Arc::new(Float32Array::from(y_values)) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("x2"), DataType::Float32, true),
            Arc::new(Float32Array::from(x2_values)) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("y2"), DataType::Float32, true),
            Arc::new(Float32Array::from(y2_values)) as ArrayRef,
        ),
        (
            Field::new(
                item_channel_column_name("corner_radius"),
                DataType::Float32,
                true,
            ),
            Arc::new(Float32Array::from(corner_radius_values)) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("fill"), DataType::Utf8, true),
            Arc::new(StringArray::from(fill_values)) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("stroke"), DataType::Utf8, true),
            Arc::new(StringArray::from(stroke_values)) as ArrayRef,
        ),
        (
            Field::new(
                item_channel_column_name("stroke_width"),
                DataType::Float32,
                true,
            ),
            Arc::new(Float32Array::from(stroke_width_values)) as ArrayRef,
        ),
        (
            Field::new(item_channel_column_name("opacity"), DataType::Float32, true),
            Arc::new(Float32Array::from(opacity_values)) as ArrayRef,
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
    if let Some(data) = data {
        if data.num_rows() != len {
            return Err(AvengerChartError::InternalError(format!(
                "Rect derived data row count {} did not match item count {len}",
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

fn evaluate_item_assignments<'a>(
    assignments: impl Iterator<Item = &'a ItemChannelAssignment>,
    item_batch: &RecordBatch,
    ctx: &SessionContext,
) -> Result<RecordBatch, AvengerChartError> {
    let assignments = assignments.collect::<Vec<_>>();
    if assignments.is_empty() {
        return Ok(RecordBatch::new_empty(Arc::new(Schema::empty())));
    }
    let allowed_columns = item_batch
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<HashSet<_>>();
    let expression_specs = assignments
        .iter()
        .map(|assignment| {
            Ok(PhysicalScalarExpressionSpec::new(
                assignment.channel.clone(),
                assignment.expr(ctx)?,
            ))
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;
    let program = CompiledScalarExpressionProgram::compile(
        ctx,
        item_batch.schema(),
        expression_specs,
        PhysicalScalarProgramOptions::default().with_allowed_columns(allowed_columns),
    )?;
    Ok(program.evaluate_batch(item_batch)?)
}
