use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, CompiledGuide, CompiledMarkCore, CoordMeasurement, CoordinateGuide,
    EventDatumFieldSpec, GuideEventDatumRows, GuideRenderContext, GuideSharingContext,
    LayoutBounds, OverflowSpaceRequirement, ScalarValueHelpers, Theme, eval_to_scalars,
    evaluate_bool_expr, evaluate_f32_expr, evaluate_string_expr, params_to_datafusion,
    serialization::DefaultLogicalExprNodeExt,
};
use avenger_color::{ColorOrGradient, parse_color_string_strict};
use avenger_common::value::ScalarOrArray;
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_guides::axis::{
    band::make_band_axis_marks,
    numeric::make_numeric_axis_marks,
    opts::{AxisConfig, AxisOrientation, AxisTickSpacing},
    point::make_point_axis_marks,
};
use avenger_scales::scales::{ConfiguredScale, DomainKind, band::BandScale};
use avenger_scenegraph::marks::{
    group::Clip, mark::SceneMark, pattern::default_no_fill_pattern, rect::SceneRectMark,
    text::SceneTextMark,
};
use avenger_text::{
    TextEngine, default_text_engine,
    measurement::TextMeasurementConfig,
    types::{FontStyle, FontWeight, FontWeightNameSpec, TextAlign, TextBaseline},
};
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, Float64Array, Int64Array, StringArray, StructArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    dataframe::DataFrame,
    logical_expr::Expr,
    prelude::SessionContext,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::ParallelAxis;
use crate::event::{
    PARALLEL_DIMENSION_ID_FIELD, PARALLEL_DISPLACEMENT_PX_FIELD, PARALLEL_DISPLACEMENT_SLOTS_FIELD,
    PARALLEL_DISPLAY_X_FIELD, PARALLEL_EQUILIBRIUM_X_FIELD, PARALLEL_ORDER_INDEX_FIELD,
    PARALLEL_SCALE_NAME_FIELD, PARALLEL_SURFACE_KIND_DIMENSION_TITLE, PARALLEL_SURFACE_KIND_FIELD,
    PARALLEL_TITLE_FIELD,
};
use crate::frame::{
    ParallelFrameDimension, resolve_display_state, resolve_order_state,
    resolve_parallel_frame_dimensions,
};

const TITLE_FONT_SIZE: f32 = 12.0;
const TITLE_Y_OFFSET: f32 = -12.0;
const MIN_TITLE_OVERFLOW_TOP: f32 = 34.0;

/// Parallel-coordinate guide configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ParallelGuide {
    pub axes: HashMap<String, ParallelAxis>,
}

impl CoordinateGuide for ParallelGuide {
    type Axis = ParallelAxis;

    fn set_axes(&mut self, axes: HashMap<String, Self::Axis>) {
        self.axes = axes;
    }

    fn set_compiled_marks<M>(
        &mut self,
        _compiled_marks: &[Arc<M>],
        _session_context: &SessionContext,
    ) where
        M: CompiledMarkCore + ?Sized,
    {
    }

    fn update(&mut self, other: Self) {
        for (channel, axis) in other.axes {
            match self.axes.get_mut(&channel) {
                Some(existing) => *existing = existing.clone().update(axis),
                None => {
                    self.axes.insert(channel, axis);
                }
            }
        }
    }

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(CompiledParallelGuide { axes: self.axes })
    }
}

/// Compiled guide for parallel-coordinate axes and dimension headers.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompiledParallelGuide {
    pub axes: HashMap<String, ParallelAxis>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParallelAxisGuideDatum {
    pub dimension_id: String,
    pub scale_name: String,
    pub title: String,
    pub order_index: usize,
    pub equilibrium_x: f32,
    pub display_x: f32,
    pub displacement_px: f32,
    pub displacement_slots: f32,
}

#[derive(Clone, Debug)]
struct EvaluatedParallelAxisGuideDatum {
    datum: ParallelAxisGuideDatum,
    visible: bool,
    title_visible: bool,
    title: String,
    title_font_family: String,
    title_font_size: f32,
    title_font_weight: FontWeight,
    title_color: [f32; 4],
    axis_config: AxisConfig,
}

impl CompiledParallelGuide {
    pub fn axis_guide_datums(
        &self,
        plot_width: f32,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<Vec<ParallelAxisGuideDatum>, AvengerChartError> {
        let axes = ordered_axes(&self.axes);
        if axes.is_empty() {
            return Ok(Vec::new());
        }

        let base_order = axes
            .iter()
            .map(|(channel, axis)| {
                axis.dimension_id
                    .clone()
                    .unwrap_or_else(|| (*channel).to_string())
            })
            .collect::<Vec<_>>();
        let dimensions = axes
            .iter()
            .map(|(channel, axis)| {
                let dimension_id = axis
                    .dimension_id
                    .clone()
                    .unwrap_or_else(|| (*channel).to_string());
                ParallelFrameDimension::new(dimension_id.clone(), (*channel).clone(), dimension_id)
            })
            .collect::<Vec<_>>();
        let order_state = axes.iter().find_map(|(_, axis)| axis.order_state.as_ref());
        let display_state = axes
            .iter()
            .find_map(|(_, axis)| axis.display_state.as_ref());
        let param_order = resolve_order_state(order_state, params, &base_order)?;
        let display_overrides = resolve_display_state(display_state, params, &base_order)?;
        let frame = resolve_parallel_frame_dimensions(
            &dimensions,
            param_order.as_deref().or(Some(base_order.as_slice())),
            display_overrides.as_ref(),
            plot_width,
        );
        let axis_by_id = axes
            .into_iter()
            .map(|(channel, axis)| {
                let dimension_id = axis
                    .dimension_id
                    .clone()
                    .unwrap_or_else(|| channel.to_string());
                (dimension_id, axis)
            })
            .collect::<HashMap<_, _>>();
        frame
            .slots
            .into_iter()
            .map(|slot| {
                let axis = axis_by_id.get(&slot.id).ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Missing parallel axis metadata for dimension '{}'",
                        slot.id
                    ))
                })?;
                Ok(ParallelAxisGuideDatum {
                    scale_name: slot.scale_name,
                    title: axis_title(axis, ctx),
                    dimension_id: slot.id,
                    order_index: slot.equilibrium_index,
                    equilibrium_x: slot.equilibrium_x,
                    display_x: slot.display_x,
                    displacement_px: slot.displacement_px,
                    displacement_slots: slot.displacement_slots,
                })
            })
            .collect()
    }

    async fn evaluated_axis_guide_datums(
        &self,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
    ) -> Result<Vec<EvaluatedParallelAxisGuideDatum>, AvengerChartError> {
        let datums = self.axis_guide_datums(plot_width, params, ctx)?;
        let axis_by_id = self
            .axes
            .iter()
            .map(|(channel, axis)| {
                let dimension_id = axis
                    .dimension_id
                    .clone()
                    .unwrap_or_else(|| channel.to_string());
                (dimension_id, axis)
            })
            .collect::<HashMap<_, _>>();
        let mut evaluated = Vec::with_capacity(datums.len());
        for datum in datums {
            let axis = axis_by_id.get(&datum.dimension_id).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing parallel axis metadata for dimension '{}'",
                    datum.dimension_id
                ))
            })?;
            evaluated.push(
                evaluate_parallel_axis_datum(
                    axis,
                    datum,
                    plot_width,
                    plot_height,
                    theme,
                    params,
                    ctx,
                )
                .await?,
            );
        }
        Ok(evaluated)
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for CompiledParallelGuide {
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        _data_override: Option<&DataFrame>,
        ctx: &SessionContext,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        measure_parallel_guide_overflow(self, scales, plot_width, plot_height, theme, params, ctx)
            .await
    }

    async fn evaluate(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        _guide_overflow: &OverflowSpaceRequirement,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
        _data_override: Option<&DataFrame>,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: &dyn CoordMeasurement,
        _render_context: GuideRenderContext<'_>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let datums = self
            .evaluated_axis_guide_datums(plot_width, plot_height, theme, params, ctx)
            .await?;
        if datums.is_empty() {
            return Ok(Vec::new());
        }
        let count = datums.len();
        let xs = datums
            .iter()
            .map(|datum| datum.datum.display_x)
            .collect::<Vec<_>>();
        let titles = datums
            .iter()
            .map(|datum| {
                if datum.visible && datum.title_visible {
                    datum.title.clone()
                } else {
                    String::new()
                }
            })
            .collect::<Vec<_>>();
        let title_fonts = datums
            .iter()
            .map(|datum| datum.title_font_family.clone())
            .collect::<Vec<_>>();
        let title_font_sizes = datums
            .iter()
            .map(|datum| datum.title_font_size)
            .collect::<Vec<_>>();
        let title_font_weights = datums
            .iter()
            .map(|datum| datum.title_font_weight)
            .collect::<Vec<_>>();
        let title_colors = datums
            .iter()
            .map(|datum| ColorOrGradient::Color(datum.title_color))
            .collect::<Vec<_>>();

        let mut marks = Vec::with_capacity(count + 2);
        for datum in &datums {
            if !datum.visible {
                continue;
            }
            let scale = scales.get(&datum.datum.scale_name).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing configured scale for parallel axis '{}'",
                    datum.datum.scale_name
                ))
            })?;
            marks.push(make_axis_mark(
                scale,
                plot_bounds.x + datum.datum.display_x,
                plot_bounds.y,
                &datum.axis_config,
            )?);
        }

        let hit_half_width = 54.0_f32;
        let title_hit_rect = SceneRectMark {
            name: "parallel_axis_title_hit".to_string(),
            interactive: true,
            clip: false,
            len: count as u32,
            gradients: Vec::new(),
            x: ScalarOrArray::from(
                xs.iter()
                    .map(|x| plot_bounds.x + *x - hit_half_width)
                    .collect::<Vec<_>>(),
            ),
            y: ScalarOrArray::new_scalar(plot_bounds.y - 34.0),
            width: None,
            height: None,
            x2: Some(ScalarOrArray::from(
                xs.iter()
                    .map(|x| plot_bounds.x + *x + hit_half_width)
                    .collect::<Vec<_>>(),
            )),
            y2: Some(ScalarOrArray::new_scalar(plot_bounds.y + 2.0)),
            fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            fill_pattern: default_no_fill_pattern(),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            stroke_width: ScalarOrArray::new_scalar(0.0),
            corner_radius: ScalarOrArray::new_scalar(0.0),
            indices: None,
            zindex: Some(3),
        };

        let title_mark = SceneTextMark {
            name: "parallel_axis_title".to_string(),
            interactive: true,
            clip: false,
            len: count as u32,
            text: ScalarOrArray::from(titles),
            x: ScalarOrArray::from(xs.iter().map(|x| plot_bounds.x + *x).collect::<Vec<_>>()),
            y: ScalarOrArray::new_scalar(plot_bounds.y + TITLE_Y_OFFSET),
            align: ScalarOrArray::new_scalar(TextAlign::Center),
            baseline: ScalarOrArray::new_scalar(TextBaseline::Bottom),
            angle: ScalarOrArray::new_scalar(0.0),
            color: scalar_or_array_if_uniform(title_colors),
            font: scalar_or_array_if_uniform(title_fonts),
            font_size: scalar_or_array_if_uniform(title_font_sizes),
            font_weight: scalar_or_array_if_uniform(title_font_weights),
            font_style: ScalarOrArray::new_scalar(FontStyle::Normal),
            limit: ScalarOrArray::new_scalar(0.0),
            indices: None,
            zindex: Some(4),
            ..SceneTextMark::default()
        };

        marks.push(SceneMark::Rect(title_hit_rect));
        marks.push(SceneMark::from(title_mark));
        Ok(marks)
    }

    fn event_datum_field_specs(&self) -> Vec<EventDatumFieldSpec> {
        parallel_axis_title_event_datum_field_specs()
    }

    fn event_datum_rows(
        &self,
        guide_marks: &[SceneMark],
        plot_width: f32,
        _plot_height: f32,
        params: &IndexMap<String, ScalarValue>,
        ctx: &SessionContext,
        _coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<GuideEventDatumRows>, AvengerChartError> {
        let axis_datums = self.axis_guide_datums(plot_width, params, ctx)?;
        if axis_datums.is_empty() {
            return Ok(Vec::new());
        }
        let rows = parallel_axis_title_event_datum_batch(axis_datums)?;
        Ok(guide_marks
            .iter()
            .enumerate()
            .filter_map(|(guide_mark_index, mark)| {
                let name = scene_mark_name(mark)?;
                matches!(name, "parallel_axis_title_hit" | "parallel_axis_title").then_some(
                    GuideEventDatumRows {
                        guide_mark_index,
                        rows: rows.clone(),
                    },
                )
            })
            .collect())
    }

    fn get_clip(
        &self,
        _plot_width: f32,
        _plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip {
        Clip::None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn scalar_or_array_if_uniform<T>(values: Vec<T>) -> ScalarOrArray<T>
where
    T: Sync + Clone + PartialEq,
{
    let first = values.first().cloned();
    if let Some(first) = first
        && values.iter().all(|value| *value == first)
    {
        return ScalarOrArray::new_scalar(first);
    }
    ScalarOrArray::from(values)
}

fn parallel_axis_title_event_datum_field_specs() -> Vec<EventDatumFieldSpec> {
    vec![
        EventDatumFieldSpec {
            name: PARALLEL_SURFACE_KIND_FIELD.to_string(),
            data_type: DataType::Utf8,
        },
        EventDatumFieldSpec {
            name: PARALLEL_DIMENSION_ID_FIELD.to_string(),
            data_type: DataType::Utf8,
        },
        EventDatumFieldSpec {
            name: PARALLEL_SCALE_NAME_FIELD.to_string(),
            data_type: DataType::Utf8,
        },
        EventDatumFieldSpec {
            name: PARALLEL_TITLE_FIELD.to_string(),
            data_type: DataType::Utf8,
        },
        EventDatumFieldSpec {
            name: PARALLEL_ORDER_INDEX_FIELD.to_string(),
            data_type: DataType::Int64,
        },
        EventDatumFieldSpec {
            name: PARALLEL_EQUILIBRIUM_X_FIELD.to_string(),
            data_type: DataType::Float64,
        },
        EventDatumFieldSpec {
            name: PARALLEL_DISPLAY_X_FIELD.to_string(),
            data_type: DataType::Float64,
        },
        EventDatumFieldSpec {
            name: PARALLEL_DISPLACEMENT_PX_FIELD.to_string(),
            data_type: DataType::Float64,
        },
        EventDatumFieldSpec {
            name: PARALLEL_DISPLACEMENT_SLOTS_FIELD.to_string(),
            data_type: DataType::Float64,
        },
    ]
}

fn parallel_axis_title_event_datum_batch(
    datums: Vec<ParallelAxisGuideDatum>,
) -> Result<RecordBatch, AvengerChartError> {
    let len = datums.len();
    let mut dimension_id = Vec::with_capacity(len);
    let mut scale_name = Vec::with_capacity(len);
    let mut title = Vec::with_capacity(len);
    let mut order_index = Vec::with_capacity(len);
    let mut equilibrium_x = Vec::with_capacity(len);
    let mut display_x = Vec::with_capacity(len);
    let mut displacement_px = Vec::with_capacity(len);
    let mut displacement_slots = Vec::with_capacity(len);
    for datum in datums {
        dimension_id.push(datum.dimension_id);
        scale_name.push(datum.scale_name);
        title.push(datum.title);
        order_index.push(datum.order_index as i64);
        equilibrium_x.push(f64::from(datum.equilibrium_x));
        display_x.push(f64::from(datum.display_x));
        displacement_px.push(f64::from(datum.displacement_px));
        displacement_slots.push(f64::from(datum.displacement_slots));
    }
    let schema = Arc::new(Schema::new(vec![
        Field::new(PARALLEL_SURFACE_KIND_FIELD, DataType::Utf8, false),
        Field::new(PARALLEL_DIMENSION_ID_FIELD, DataType::Utf8, false),
        Field::new(PARALLEL_SCALE_NAME_FIELD, DataType::Utf8, false),
        Field::new(PARALLEL_TITLE_FIELD, DataType::Utf8, false),
        Field::new(PARALLEL_ORDER_INDEX_FIELD, DataType::Int64, false),
        Field::new(PARALLEL_EQUILIBRIUM_X_FIELD, DataType::Float64, false),
        Field::new(PARALLEL_DISPLAY_X_FIELD, DataType::Float64, false),
        Field::new(PARALLEL_DISPLACEMENT_PX_FIELD, DataType::Float64, false),
        Field::new(PARALLEL_DISPLACEMENT_SLOTS_FIELD, DataType::Float64, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                PARALLEL_SURFACE_KIND_DIMENSION_TITLE;
                len
            ])) as ArrayRef,
            Arc::new(StringArray::from(dimension_id)),
            Arc::new(StringArray::from(scale_name)),
            Arc::new(StringArray::from(title)),
            Arc::new(Int64Array::from(order_index)),
            Arc::new(Float64Array::from(equilibrium_x)),
            Arc::new(Float64Array::from(display_x)),
            Arc::new(Float64Array::from(displacement_px)),
            Arc::new(Float64Array::from(displacement_slots)),
        ],
    )
    .map_err(AvengerChartError::ArrowError)
}

fn scene_mark_name(mark: &SceneMark) -> Option<&str> {
    match mark {
        SceneMark::Arc(mark) => Some(mark.name.as_str()),
        SceneMark::Area(mark) => Some(mark.name.as_str()),
        SceneMark::Path(mark) => Some(mark.name.as_str()),
        SceneMark::Symbol(mark) => Some(mark.name.as_str()),
        SceneMark::Line(mark) => Some(mark.name.as_str()),
        SceneMark::Trail(mark) => Some(mark.name.as_str()),
        SceneMark::Rect(mark) => Some(mark.name.as_str()),
        SceneMark::Rule(mark) => Some(mark.name.as_str()),
        SceneMark::Text(mark) => Some(mark.name.as_str()),
        SceneMark::Image(mark) => Some(mark.name.as_str()),
        SceneMark::WarpedImage(mark) => Some(mark.name.as_str()),
        SceneMark::Group(mark) => Some(mark.name.as_str()),
    }
}

async fn measure_parallel_guide_overflow(
    guide: &CompiledParallelGuide,
    scales: &HashMap<String, ConfiguredScale>,
    plot_width: f32,
    plot_height: f32,
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
    ctx: &SessionContext,
) -> Result<OverflowSpaceRequirement, AvengerChartError> {
    let text_engine = default_text_engine();
    let datums = guide
        .evaluated_axis_guide_datums(plot_width, plot_height, theme, params, ctx)
        .await?;
    if datums.is_empty() {
        return Ok(OverflowSpaceRequirement::default());
    }

    let mut min_x = 0.0_f32;
    let mut max_x = plot_width;
    let mut min_y = 0.0_f32;
    let mut max_y = plot_height;

    for datum in &datums {
        if !datum.visible {
            continue;
        }
        let scale = scales.get(&datum.datum.scale_name).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Missing configured scale for parallel axis '{}'",
                datum.datum.scale_name
            ))
        })?;
        let axis_mark = make_axis_mark(scale, datum.datum.display_x, 0.0, &datum.axis_config)?;
        let bbox = axis_mark.bounding_box();
        let lower = bbox.lower();
        let upper = bbox.upper();
        min_x = min_x.min(lower[0]);
        max_x = max_x.max(upper[0]);
        min_y = min_y.min(lower[1]);
        max_y = max_y.max(upper[1]);
        measure_parallel_categorical_tick_labels(
            scale,
            datum,
            &mut min_x,
            &mut max_x,
            &mut min_y,
            &mut max_y,
            &text_engine,
        )?;
    }

    measure_parallel_axis_titles(
        &datums,
        &mut min_x,
        &mut max_x,
        &mut min_y,
        &mut max_y,
        &text_engine,
    )?;

    Ok(OverflowSpaceRequirement {
        top: (0.0 - min_y).max(MIN_TITLE_OVERFLOW_TOP),
        bottom: (max_y - plot_height).max(0.0),
        left: (0.0 - min_x).max(0.0),
        right: (max_x - plot_width).max(0.0),
    })
}

fn measure_parallel_categorical_tick_labels(
    scale: &ConfiguredScale,
    datum: &EvaluatedParallelAxisGuideDatum,
    min_x: &mut f32,
    max_x: &mut f32,
    min_y: &mut f32,
    max_y: &mut f32,
    text_engine: &TextEngine,
) -> Result<(), AvengerChartError> {
    if !datum.axis_config.labels_visible.unwrap_or(true)
        || !matches!(scale.scale_impl.domain_kind(), DomainKind::Categorical)
    {
        return Ok(());
    }

    let labels = scale
        .format(scale.domain())
        .map_err(|err| AvengerChartError::InternalError(err.to_string()))?
        .as_vec(scale.domain().len(), None);
    let font_family = datum
        .axis_config
        .label_font_family
        .as_deref()
        .unwrap_or("sans-serif");
    let font_size = datum.axis_config.label_font_size.unwrap_or(12.0);
    let font_weight = FontWeight::Number(datum.axis_config.label_font_weight.unwrap_or(400.0));
    let tick_length = datum.axis_config.tick_length.unwrap_or(5.0);
    let text_x = datum.datum.display_x - tick_length - 3.0;
    for label in labels {
        if label.trim().is_empty() {
            continue;
        }
        let bounds =
            text_engine.measure_bounds_with_plain_fallback_or_approx(&TextMeasurementConfig {
                text: &label,
                font: font_family,
                font_size,
                font_weight,
                font_style: FontStyle::Normal,
                syntax_mode: avenger_text::types::TextSyntaxMode::Plain,
                params: avenger_text::empty_label_params(),
                number_locale: None,
                number_locale_specs: None,
                datetime_locale: None,
                datetime_timezone: None,
                datetime_locale_specs: None,
            });
        let origin =
            bounds.calculate_origin([text_x, 0.0], &TextAlign::Right, &TextBaseline::Middle);
        *min_x = (*min_x).min(origin[0]);
        *max_x = (*max_x).max(origin[0] + bounds.width);
        *min_y = (*min_y).min(origin[1]);
        *max_y = (*max_y).max(origin[1] + bounds.height);
    }
    Ok(())
}

fn measure_parallel_axis_titles(
    datums: &[EvaluatedParallelAxisGuideDatum],
    min_x: &mut f32,
    max_x: &mut f32,
    min_y: &mut f32,
    max_y: &mut f32,
    text_engine: &TextEngine,
) -> Result<(), AvengerChartError> {
    for datum in datums {
        if !datum.visible || !datum.title_visible || datum.title.trim().is_empty() {
            continue;
        }
        let bounds = text_engine.measure_bounds(&TextMeasurementConfig {
            text: &datum.title,
            font: &datum.title_font_family,
            font_size: datum.title_font_size,
            font_weight: datum.title_font_weight,
            font_style: FontStyle::Normal,
            syntax_mode: avenger_text::types::TextSyntaxMode::Plain,
            params: avenger_text::empty_label_params(),
            number_locale: None,
            number_locale_specs: None,
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: None,
        })?;
        let origin = bounds.calculate_origin(
            [datum.datum.display_x, TITLE_Y_OFFSET],
            &TextAlign::Center,
            &TextBaseline::Bottom,
        );
        *min_x = (*min_x).min(origin[0]);
        *max_x = (*max_x).max(origin[0] + bounds.width);
        *min_y = (*min_y).min(origin[1]);
        *max_y = (*max_y).max(origin[1] + bounds.height);
    }
    Ok(())
}

fn make_axis_mark(
    scale: &ConfiguredScale,
    display_x: f32,
    display_y: f32,
    axis_config: &AxisConfig,
) -> Result<SceneMark, AvengerChartError> {
    let mut group = match scale.scale_impl.domain_kind() {
        DomainKind::Categorical => match scale.scale_impl.scale_type() {
            "band" => make_band_axis_marks(scale, "", [display_x, display_y], axis_config)?,
            "point" => {
                make_point_axis_marks(scale.clone(), "", [display_x, display_y], axis_config)?
            }
            "ordinal" => {
                let band_scale = BandScale::from_point_scale(scale);
                make_band_axis_marks(&band_scale, "", [display_x, display_y], axis_config)?
            }
            scale_type => {
                return Err(AvengerChartError::InternalError(format!(
                    "Unsupported parallel categorical axis scale type '{scale_type}'"
                )));
            }
        },
        DomainKind::NestedCategorical => {
            return Err(AvengerChartError::InternalError(
                "Nested categorical scales are not supported on parallel axes".to_string(),
            ));
        }
        DomainKind::Numeric | DomainKind::Temporal => {
            make_numeric_axis_marks(scale, "", [display_x, display_y], axis_config)?
        }
    };
    group.name = "parallel_axis".to_string();
    Ok(SceneMark::Group(group))
}

async fn evaluate_parallel_axis_datum(
    axis: &ParallelAxis,
    datum: ParallelAxisGuideDatum,
    plot_width: f32,
    plot_height: f32,
    theme: &Theme,
    params: &IndexMap<String, ScalarValue>,
    ctx: &SessionContext,
) -> Result<EvaluatedParallelAxisGuideDatum, AvengerChartError> {
    let axis_ctx =
        theme.axis_context_with_params(Some("parallel"), Some("dimension"), params.clone());
    let label_ctx = axis_ctx.child("label");
    let title_ctx = axis_ctx.child("title");

    let visible = if let Some(visible_node) = axis.visible.as_option().and_then(|o| o.as_ref()) {
        let visible_expr = visible_node.to_default_expr(ctx)?;
        evaluate_bool_expr(&visible_expr, ctx, params).await?
    } else {
        true
    };
    let grid = if let Some(grid_node) = axis.grid.as_option().and_then(|o| o.as_ref()) {
        let grid_expr = grid_node.to_default_expr(ctx)?;
        evaluate_bool_expr(&grid_expr, ctx, params).await?
    } else {
        false
    };
    let title_visible =
        if let Some(show_title_node) = axis.show_title.as_option().and_then(|o| o.as_ref()) {
            let show_title_expr = show_title_node.to_default_expr(ctx)?;
            evaluate_bool_expr(&show_title_expr, ctx, params).await?
        } else {
            true
        };
    let title = if let Some(title_node) = axis.title.as_option().and_then(|o| o.as_ref()) {
        let title_expr = title_node.to_default_expr(ctx)?;
        evaluate_string_expr(&title_expr, ctx, params).await?
    } else {
        datum.title.clone()
    };
    let tick_count =
        if let Some(tick_count_node) = axis.tick_count.as_option().and_then(|o| o.as_ref()) {
            let tick_count_expr = tick_count_node.to_default_expr(ctx)?;
            Some(evaluate_f32_expr(&tick_count_expr, ctx, params).await?)
        } else {
            None
        };
    let tick_start_step = evaluate_tick_spacing(axis, ctx, params).await?;
    let label_angle =
        if let Some(angle_node) = axis.label_angle.as_option().and_then(|o| o.as_ref()) {
            let angle_expr = angle_node.to_default_expr(ctx)?;
            Some(evaluate_f32_expr(&angle_expr, ctx, params).await?)
        } else {
            None
        };
    let format_number =
        if let Some(format_node) = axis.format_number.as_option().and_then(|o| o.as_ref()) {
            let format_expr = format_node.to_default_expr(ctx)?;
            Some(evaluate_string_expr(&format_expr, ctx, params).await?)
        } else {
            None
        };
    let label_font_family =
        if let Some(font_node) = axis.label_font_family.as_option().and_then(|o| o.as_ref()) {
            let font_expr = font_node.to_default_expr(ctx)?;
            Some(evaluate_string_expr(&font_expr, ctx, params).await?)
        } else {
            theme.font_family(&label_ctx)
        };
    let title_font_family =
        if let Some(font_node) = axis.title_font_family.as_option().and_then(|o| o.as_ref()) {
            let font_expr = font_node.to_default_expr(ctx)?;
            evaluate_string_expr(&font_expr, ctx, params).await?
        } else {
            "sans-serif".to_string()
        };
    let title_color =
        if let Some(color_node) = axis.title_color.as_option().and_then(|o| o.as_ref()) {
            let color_expr = color_node.to_default_expr(ctx)?;
            let color = evaluate_string_expr(&color_expr, ctx, params).await?;
            Some(parse_color_string_strict(&color).map_err(|err| {
                AvengerChartError::InvalidArgument(format!(
                    "Invalid parallel axis title_color '{color}': {err}"
                ))
            })?)
        } else {
            theme.text_color(&title_ctx)
        };
    let title_font_size = TITLE_FONT_SIZE;
    let title_font_weight = FontWeight::Name(FontWeightNameSpec::Normal);

    let axis_config = AxisConfig {
        orientation: AxisOrientation::Left,
        dimensions: [plot_width, plot_height],
        grid,
        format_number,
        title_font_size: Some(title_font_size),
        domain_color: theme.stroke_color(&axis_ctx.child("domain")),
        tick_color: theme.stroke_color(&axis_ctx.child("tick")),
        grid_color: theme
            .stroke_color(&axis_ctx.child("grid"))
            .map(|mut color| {
                if let Some(opacity) = theme.opacity(&axis_ctx.child("grid")) {
                    color[3] = opacity;
                }
                color
            }),
        grid_width: theme.axis_grid_width(&axis_ctx),
        label_color: theme.text_color(&label_ctx),
        title_color,
        tick_length: theme.axis_tick_length(&axis_ctx),
        label_font_size: theme.font_size(&label_ctx),
        label_font_weight: theme.font_weight(&label_ctx),
        label_angle,
        title_font_weight: theme.font_weight(&title_ctx),
        label_font_family,
        title_font_family: Some(title_font_family.clone()),
        title_visible: Some(false),
        tick_count,
        tick_start_step,
        ..AxisConfig::default()
    };

    Ok(EvaluatedParallelAxisGuideDatum {
        datum,
        visible,
        title_visible,
        title,
        title_font_family,
        title_font_size,
        title_font_weight,
        title_color: title_color.unwrap_or([0.12, 0.12, 0.12, 1.0]),
        axis_config,
    })
}

async fn evaluate_tick_spacing(
    axis: &ParallelAxis,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<Option<AxisTickSpacing>, AvengerChartError> {
    let Some(spacing_node) = axis.tick_spacing.as_option().and_then(|o| o.as_ref()) else {
        return Ok(None);
    };
    let spacing_expr = spacing_node.to_default_expr(ctx)?;
    let spacing = evaluate_scalar_expr(&spacing_expr, ctx, params).await?;
    Ok(Some(extract_tick_spacing(spacing)?))
}

async fn evaluate_scalar_expr(
    expr: &Expr,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
) -> Result<ScalarValue, AvengerChartError> {
    let scalars = eval_to_scalars(
        vec![expr.clone()],
        Some(ctx),
        params_to_datafusion(params).as_ref(),
    )
    .await
    .map_err(|err| {
        AvengerChartError::InternalError(format!("Failed to evaluate scalar expression: {err}"))
    })?;

    scalars
        .into_iter()
        .next()
        .ok_or_else(|| AvengerChartError::InternalError("No value returned".to_string()))
}

fn extract_tick_spacing(spacing: ScalarValue) -> Result<AxisTickSpacing, AvengerChartError> {
    let ScalarValue::Struct(struct_array) = spacing else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Parallel axis tick_spacing must evaluate to a struct with start and step fields, got {spacing}"
        )));
    };

    if struct_array.len() != 1 {
        return Err(AvengerChartError::InvalidArgument(
            "Parallel axis tick_spacing struct must contain exactly one row".to_string(),
        ));
    }

    let start = tick_spacing_field(&struct_array, "start")?;
    let step = tick_spacing_field(&struct_array, "step")?;
    if let (Ok(start), Ok(step)) = (start.as_f32(), step.as_f32()) {
        return Ok(AxisTickSpacing::Numeric { start, step });
    }

    let start_millis = tick_spacing_start_millis(&start)?;
    let (months, days, nanos) = tick_spacing_interval_parts(&step)?;
    Ok(AxisTickSpacing::Temporal {
        start_millis,
        months,
        days,
        nanos,
    })
}

fn tick_spacing_field(
    struct_array: &StructArray,
    name: &str,
) -> Result<ScalarValue, AvengerChartError> {
    let (field_index, _) = struct_array
        .fields()
        .iter()
        .enumerate()
        .find(|(_, field)| field.name() == name)
        .ok_or_else(|| {
            AvengerChartError::InvalidArgument(format!(
                "Parallel axis tick_spacing struct is missing required field '{name}'"
            ))
        })?;
    let value =
        ScalarValue::try_from_array(struct_array.column(field_index), 0).map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "Failed to read parallel axis tick_spacing field '{name}': {err}"
            ))
        })?;
    if value.is_null() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Parallel axis tick_spacing field '{name}' must not be null"
        )));
    }
    Ok(value)
}

fn tick_spacing_start_millis(start: &ScalarValue) -> Result<i64, AvengerChartError> {
    match start {
        ScalarValue::Date32(Some(days)) => Ok(i64::from(*days) * 86_400_000),
        ScalarValue::Date64(Some(millis)) => Ok(*millis),
        ScalarValue::TimestampSecond(Some(value), _) => Ok(*value * 1_000),
        ScalarValue::TimestampMillisecond(Some(value), _) => Ok(*value),
        ScalarValue::TimestampMicrosecond(Some(value), _) => Ok(*value / 1_000),
        ScalarValue::TimestampNanosecond(Some(value), _) => Ok(*value / 1_000_000),
        _ => Err(AvengerChartError::InvalidArgument(format!(
            "Parallel axis temporal tick_spacing start must be a date or timestamp, got {start}"
        ))),
    }
}

fn tick_spacing_interval_parts(step: &ScalarValue) -> Result<(i32, i32, i64), AvengerChartError> {
    let ScalarValue::IntervalMonthDayNano(Some(value)) = step else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Parallel axis tick_spacing step must be numeric or an IntervalMonthDayNano scalar, got {step}"
        )));
    };
    Ok(datafusion::arrow::array::types::IntervalMonthDayNanoType::to_parts(*value))
}

fn ordered_axes(axes: &HashMap<String, ParallelAxis>) -> Vec<(&String, &ParallelAxis)> {
    let mut axes = axes.iter().collect::<Vec<_>>();
    axes.sort_by_key(|(channel, axis)| {
        (
            axis.order_index.unwrap_or(usize::MAX),
            axis.dimension_id
                .as_deref()
                .unwrap_or(channel.as_str())
                .to_string(),
        )
    });
    axes
}

fn axis_title(axis: &ParallelAxis, ctx: &SessionContext) -> String {
    if let Some(title) = axis.title.as_option().and_then(|title| title.as_ref())
        && let Ok(expr) = title.to_default_expr(ctx)
    {
        return match expr {
            Expr::Literal(ScalarValue::Utf8(Some(value)), _)
            | Expr::Literal(ScalarValue::LargeUtf8(Some(value)), _) => value,
            other => other.to_string(),
        };
    }
    axis.dimension_id
        .clone()
        .unwrap_or_else(|| "dimension".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use avenger_chart_core::{
        AxisGuideVisibilityConfig, AxisPosition, AxisVisibility, ChildFrameGuideSharingView,
        CompiledGuide, CoordinationAxis, EmptyCoordMeasurement, FacetGuideSharingView,
        SharingLevel, guide_sharing::AxisOwnershipMode,
    };
    use avenger_common::value::ScalarOrArrayValue;
    use avenger_scales::scales::{linear::LinearScale, point::PointScale};
    use datafusion::arrow::array::{Float64Array, Int64Array, StringArray};

    #[test]
    fn ordered_axes_use_parallel_dimension_order() {
        let mut axes = HashMap::new();
        axes.insert(
            "generated_b".to_string(),
            ParallelAxis::new().with_dimension_metadata("b", 1),
        );
        axes.insert(
            "generated_a".to_string(),
            ParallelAxis::new().with_dimension_metadata("a", 0),
        );

        let ordered_ids = ordered_axes(&axes)
            .into_iter()
            .map(|(_, axis)| axis.dimension_id.as_deref().unwrap().to_string())
            .collect::<Vec<_>>();

        assert_eq!(ordered_ids, vec!["a", "b"]);
    }

    #[test]
    fn axis_title_uses_configured_title_then_dimension_id() {
        let ctx = SessionContext::new();
        let titled = ParallelAxis::new()
            .title("Miles Per Gallon")
            .with_dimension_metadata("mpg", 0);
        assert_eq!(axis_title(&titled, &ctx), "Miles Per Gallon");

        let defaulted = ParallelAxis::new().with_dimension_metadata("mpg", 0);
        assert_eq!(axis_title(&defaulted, &ctx), "mpg");
    }

    #[test]
    fn axis_guide_datums_include_title_and_positions() {
        let ctx = SessionContext::new();
        let mut axes = HashMap::new();
        axes.insert(
            "generated_speed".to_string(),
            ParallelAxis::new()
                .title("Speed")
                .with_dimension_metadata("speed", 0),
        );
        axes.insert(
            "generated_cost".to_string(),
            ParallelAxis::new()
                .title("Cost")
                .with_dimension_metadata("cost", 1),
        );
        let guide = CompiledParallelGuide { axes };
        let datums = guide
            .axis_guide_datums(300.0, &IndexMap::new(), &ctx)
            .expect("guide datums");

        assert_eq!(datums.len(), 2);
        assert_eq!(datums[0].dimension_id, "speed");
        assert_eq!(datums[0].scale_name, "speed");
        assert_eq!(datums[0].title, "Speed");
        assert_eq!(datums[0].equilibrium_x, 0.0);
        assert_eq!(datums[0].display_x, 0.0);
        assert_eq!(datums[1].dimension_id, "cost");
        assert_eq!(datums[1].equilibrium_x, 300.0);
    }

    #[test]
    fn axis_guide_datums_use_order_and_display_state_params() {
        let ctx = SessionContext::new();
        let order_state = crate::ParallelOrderState::param("axis_order");
        let display_state =
            crate::ParallelDisplayState::active_axis("drag_dimension", "drag_display_x");
        let mut axes = HashMap::new();
        axes.insert(
            "generated_speed".to_string(),
            ParallelAxis::new()
                .title("Speed")
                .with_dimension_metadata("speed", 0)
                .with_frame_state(Some(order_state.clone()), Some(display_state.clone())),
        );
        axes.insert(
            "generated_cost".to_string(),
            ParallelAxis::new()
                .title("Cost")
                .with_dimension_metadata("cost", 1)
                .with_frame_state(Some(order_state), Some(display_state)),
        );
        let guide = CompiledParallelGuide { axes };
        let params = IndexMap::from([
            (
                "axis_order".to_string(),
                ScalarValue::List(ScalarValue::new_list(
                    &[
                        ScalarValue::Utf8(Some("cost".to_string())),
                        ScalarValue::Utf8(Some("speed".to_string())),
                    ],
                    &datafusion::arrow::datatypes::DataType::Utf8,
                    true,
                )),
            ),
            (
                "drag_dimension".to_string(),
                ScalarValue::Utf8(Some("speed".to_string())),
            ),
            (
                "drag_display_x".to_string(),
                ScalarValue::Float64(Some(210.0)),
            ),
        ]);
        let datums = guide
            .axis_guide_datums(300.0, &params, &ctx)
            .expect("guide datums");

        assert_eq!(
            datums
                .iter()
                .map(|datum| datum.dimension_id.as_str())
                .collect::<Vec<_>>(),
            vec!["cost", "speed"]
        );
        assert_eq!(datums[0].equilibrium_x, 0.0);
        assert_eq!(datums[0].display_x, 0.0);
        assert_eq!(datums[1].equilibrium_x, 300.0);
        assert_eq!(datums[1].display_x, 210.0);
        assert_eq!(datums[1].displacement_px, -90.0);
        assert_eq!(datums[1].displacement_slots, -0.3);
    }

    #[test]
    fn guide_renders_axis_title_hit_rects() {
        let ctx = SessionContext::new();
        let mut axes = HashMap::new();
        axes.insert(
            "generated_speed".to_string(),
            ParallelAxis::new()
                .title("Speed")
                .with_dimension_metadata("speed", 0),
        );
        let guide = CompiledParallelGuide { axes };
        let facet = TestFacetGuideSharingView;
        let child = TestChildFrameGuideSharingView;
        let scales = HashMap::from([(
            "speed".to_string(),
            LinearScale::configured((0.0, 100.0), (200.0, 0.0)),
        )]);
        let marks = futures::executor::block_on(guide.evaluate(
            &scales,
            300.0,
            200.0,
            &LayoutBounds {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 200.0,
            },
            &OverflowSpaceRequirement::default(),
            &Theme::light(),
            &IndexMap::new(),
            &ctx,
            None,
            GuideSharingContext::new(&facet, &[], &child),
            &EmptyCoordMeasurement,
            GuideRenderContext::without_resource_sink(300.0, 200.0),
        ))
        .expect("evaluate guide");

        assert!(matches!(&marks[0], SceneMark::Group(group) if group.name == "parallel_axis"));
        assert!(
            matches!(&marks[1], SceneMark::Rect(rect) if rect.name == "parallel_axis_title_hit" && rect.interactive)
        );
        assert!(
            matches!(&marks[2], SceneMark::Text(text) if text.name == "parallel_axis_title" && text.interactive)
        );
    }

    #[test]
    fn guide_event_datum_rows_retain_title_rows() {
        let ctx = SessionContext::new();
        let mut axes = HashMap::new();
        axes.insert(
            "generated_speed".to_string(),
            ParallelAxis::new()
                .title("Speed")
                .with_dimension_metadata("speed", 0),
        );
        axes.insert(
            "generated_cost".to_string(),
            ParallelAxis::new()
                .title("Cost")
                .with_dimension_metadata("cost", 1),
        );
        let guide = CompiledParallelGuide { axes };
        let facet = TestFacetGuideSharingView;
        let child = TestChildFrameGuideSharingView;
        let scales = HashMap::from([
            (
                "speed".to_string(),
                LinearScale::configured((0.0, 100.0), (200.0, 0.0)),
            ),
            (
                "cost".to_string(),
                LinearScale::configured((0.0, 100.0), (200.0, 0.0)),
            ),
        ]);
        let guide_marks = futures::executor::block_on(guide.evaluate(
            &scales,
            300.0,
            200.0,
            &LayoutBounds {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 200.0,
            },
            &OverflowSpaceRequirement::default(),
            &Theme::light(),
            &IndexMap::new(),
            &ctx,
            None,
            GuideSharingContext::new(&facet, &[], &child),
            &EmptyCoordMeasurement,
            GuideRenderContext::without_resource_sink(300.0, 200.0),
        ))
        .expect("evaluate guide");

        let event_rows = guide
            .event_datum_rows(
                &guide_marks,
                300.0,
                200.0,
                &IndexMap::new(),
                &ctx,
                &EmptyCoordMeasurement,
            )
            .expect("parallel guide event datums");
        let title_indices = guide_marks
            .iter()
            .enumerate()
            .filter_map(|(index, mark)| {
                let name = scene_mark_name(mark)?;
                matches!(name, "parallel_axis_title_hit" | "parallel_axis_title").then_some(index)
            })
            .collect::<Vec<_>>();

        assert_eq!(event_rows.len(), title_indices.len());
        assert_eq!(
            event_rows
                .iter()
                .map(|rows| rows.guide_mark_index)
                .collect::<Vec<_>>(),
            title_indices
        );
        for rows in &event_rows {
            assert_eq!(rows.rows.num_rows(), 2);
            assert_eq!(
                rows.rows.schema().field(0).name(),
                PARALLEL_SURFACE_KIND_FIELD
            );
        }

        let retained = &event_rows[0].rows;
        let surface_kind = retained
            .column_by_name(PARALLEL_SURFACE_KIND_FIELD)
            .and_then(|array| array.as_any().downcast_ref::<StringArray>())
            .expect("surface kind column");
        let dimensions = retained
            .column_by_name(PARALLEL_DIMENSION_ID_FIELD)
            .and_then(|array| array.as_any().downcast_ref::<StringArray>())
            .expect("dimension id column");
        let titles = retained
            .column_by_name(PARALLEL_TITLE_FIELD)
            .and_then(|array| array.as_any().downcast_ref::<StringArray>())
            .expect("title column");
        let order_indices = retained
            .column_by_name(PARALLEL_ORDER_INDEX_FIELD)
            .and_then(|array| array.as_any().downcast_ref::<Int64Array>())
            .expect("order index column");
        let display_x = retained
            .column_by_name(PARALLEL_DISPLAY_X_FIELD)
            .and_then(|array| array.as_any().downcast_ref::<Float64Array>())
            .expect("display x column");

        assert_eq!(surface_kind.value(0), PARALLEL_SURFACE_KIND_DIMENSION_TITLE);
        assert_eq!(surface_kind.value(1), PARALLEL_SURFACE_KIND_DIMENSION_TITLE);
        assert_eq!(dimensions.value(0), "speed");
        assert_eq!(dimensions.value(1), "cost");
        assert_eq!(titles.value(0), "Speed");
        assert_eq!(titles.value(1), "Cost");
        assert_eq!(order_indices.value(0), 0);
        assert_eq!(order_indices.value(1), 1);
        assert_eq!(display_x.value(0), 0.0);
        assert_eq!(display_x.value(1), 300.0);
    }

    #[test]
    fn guide_marks_are_offset_by_plot_bounds() {
        let ctx = SessionContext::new();
        let mut axes = HashMap::new();
        axes.insert(
            "generated_speed".to_string(),
            ParallelAxis::new()
                .title("Speed")
                .with_dimension_metadata("speed", 0),
        );
        axes.insert(
            "generated_cost".to_string(),
            ParallelAxis::new()
                .title("Cost")
                .with_dimension_metadata("cost", 1),
        );
        let guide = CompiledParallelGuide { axes };
        let facet = TestFacetGuideSharingView;
        let child = TestChildFrameGuideSharingView;
        let scales = HashMap::from([
            (
                "speed".to_string(),
                LinearScale::configured((0.0, 100.0), (200.0, 0.0)),
            ),
            (
                "cost".to_string(),
                LinearScale::configured((0.0, 100.0), (200.0, 0.0)),
            ),
        ]);
        let marks = futures::executor::block_on(guide.evaluate(
            &scales,
            300.0,
            200.0,
            &LayoutBounds {
                x: 25.0,
                y: 40.0,
                width: 300.0,
                height: 200.0,
            },
            &OverflowSpaceRequirement::default(),
            &Theme::light(),
            &IndexMap::new(),
            &ctx,
            None,
            GuideSharingContext::new(&facet, &[], &child),
            &EmptyCoordMeasurement,
            GuideRenderContext::without_resource_sink(300.0, 200.0),
        ))
        .expect("evaluate guide");

        match &marks[0] {
            SceneMark::Group(group) => assert_eq!(group.origin, [25.0, 40.0]),
            _ => panic!("expected first parallel axis group"),
        }
        let rect = match &marks[2] {
            SceneMark::Rect(rect) => rect,
            _ => panic!("expected title hit rect"),
        };
        match rect.x.value() {
            ScalarOrArrayValue::Array(values) => assert_eq!(values.as_slice(), &[-29.0, 271.0]),
            ScalarOrArrayValue::Scalar(_) => panic!("expected title hit rect x array"),
        }
        match rect.y.value() {
            ScalarOrArrayValue::Scalar(value) => assert_eq!(*value, 6.0),
            ScalarOrArrayValue::Array(_) => panic!("expected title hit rect scalar y"),
        }
    }

    #[test]
    fn guide_dispatches_numeric_and_point_axes() {
        let ctx = SessionContext::new();
        let mut axes = HashMap::new();
        axes.insert(
            "generated_speed".to_string(),
            ParallelAxis::new()
                .title("Speed")
                .with_dimension_metadata("speed", 0),
        );
        axes.insert(
            "generated_origin".to_string(),
            ParallelAxis::new()
                .title("Origin")
                .with_dimension_metadata("origin", 1),
        );
        let guide = CompiledParallelGuide { axes };
        let scales = HashMap::from([
            (
                "speed".to_string(),
                LinearScale::configured((0.0, 100.0), (200.0, 0.0)),
            ),
            (
                "origin".to_string(),
                PointScale::configured(
                    Arc::new(StringArray::from(vec!["EU", "JP", "US"])),
                    (200.0, 0.0),
                ),
            ),
        ]);
        let facet = TestFacetGuideSharingView;
        let child = TestChildFrameGuideSharingView;
        let marks = futures::executor::block_on(guide.evaluate(
            &scales,
            300.0,
            200.0,
            &LayoutBounds {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 200.0,
            },
            &OverflowSpaceRequirement::default(),
            &Theme::light(),
            &IndexMap::new(),
            &ctx,
            None,
            GuideSharingContext::new(&facet, &[], &child),
            &EmptyCoordMeasurement,
            GuideRenderContext::without_resource_sink(300.0, 200.0),
        ))
        .expect("evaluate guide");

        assert_eq!(marks.len(), 4);
        assert!(matches!(&marks[0], SceneMark::Group(group) if group.name == "parallel_axis"));
        assert!(matches!(&marks[1], SceneMark::Group(group) if group.name == "parallel_axis"));
        assert!(
            matches!(&marks[2], SceneMark::Rect(rect) if rect.name == "parallel_axis_title_hit")
        );
        assert!(matches!(&marks[3], SceneMark::Text(text) if text.name == "parallel_axis_title"));
    }

    #[test]
    fn parallel_axis_config_evaluates_visible_grid_tick_format_and_font_options() {
        let ctx = SessionContext::new();
        let axis = ParallelAxis::new()
            .visible(true)
            .title("Custom Speed")
            .grid(true)
            .tick_count(4.0)
            .ticks_start_step(1.0, 2.0)
            .label_angle(-45.0)
            .format(".1f")
            .title_font_family("serif")
            .title_color("#2563eb")
            .label_font_family("mono")
            .show_title(false)
            .with_dimension_metadata("speed", 0);
        let guide = CompiledParallelGuide {
            axes: HashMap::from([("generated_speed".to_string(), axis)]),
        };
        let datums = futures::executor::block_on(guide.evaluated_axis_guide_datums(
            300.0,
            200.0,
            &Theme::light(),
            &IndexMap::new(),
            &ctx,
        ))
        .expect("evaluate axis config");

        let datum = &datums[0];
        assert!(datum.visible);
        assert!(!datum.title_visible);
        assert_eq!(datum.title, "Custom Speed");
        assert_eq!(datum.title_font_family, "serif");
        assert_eq!(
            datum.axis_config.title_font_family.as_deref(),
            Some("serif")
        );
        assert_eq!(
            datum.axis_config.title_color,
            Some([37.0 / 255.0, 99.0 / 255.0, 235.0 / 255.0, 1.0])
        );
        assert_eq!(datum.axis_config.label_font_family.as_deref(), Some("mono"));
        assert!(datum.axis_config.grid);
        assert_eq!(datum.axis_config.tick_count, Some(4.0));
        assert_eq!(
            datum.axis_config.tick_start_step,
            Some(AxisTickSpacing::Numeric {
                start: 1.0,
                step: 2.0,
            })
        );
        assert_eq!(datum.axis_config.label_angle, Some(-45.0));
        assert_eq!(datum.axis_config.format_number.as_deref(), Some(".1f"));
        assert_eq!(datum.axis_config.title_visible, Some(false));
    }

    #[test]
    fn parallel_axis_grid_defaults_to_false() {
        let ctx = SessionContext::new();
        let guide = CompiledParallelGuide {
            axes: HashMap::from([(
                "generated_speed".to_string(),
                ParallelAxis::new().with_dimension_metadata("speed", 0),
            )]),
        };
        let datums = futures::executor::block_on(guide.evaluated_axis_guide_datums(
            300.0,
            200.0,
            &Theme::light(),
            &IndexMap::new(),
            &ctx,
        ))
        .expect("evaluate default axis config");

        assert!(!datums[0].axis_config.grid);
    }

    #[test]
    fn visible_false_suppresses_axis_group_and_header_text() {
        let ctx = SessionContext::new();
        let mut axes = HashMap::new();
        axes.insert(
            "generated_speed".to_string(),
            ParallelAxis::new()
                .title("Speed")
                .visible(false)
                .with_dimension_metadata("speed", 0),
        );
        let guide = CompiledParallelGuide { axes };
        let facet = TestFacetGuideSharingView;
        let child = TestChildFrameGuideSharingView;
        let scales = HashMap::from([(
            "speed".to_string(),
            LinearScale::configured((0.0, 100.0), (200.0, 0.0)),
        )]);
        let marks = futures::executor::block_on(guide.evaluate(
            &scales,
            300.0,
            200.0,
            &LayoutBounds {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 200.0,
            },
            &OverflowSpaceRequirement::default(),
            &Theme::light(),
            &IndexMap::new(),
            &ctx,
            None,
            GuideSharingContext::new(&facet, &[], &child),
            &EmptyCoordMeasurement,
            GuideRenderContext::without_resource_sink(300.0, 200.0),
        ))
        .expect("evaluate guide");

        assert_eq!(marks.len(), 2);
        assert!(
            matches!(&marks[0], SceneMark::Rect(rect) if rect.name == "parallel_axis_title_hit")
        );
        let text = match &marks[1] {
            SceneMark::Text(text) => text,
            _ => panic!("expected title text mark"),
        };
        match text.text.value() {
            ScalarOrArrayValue::Array(values) => assert_eq!(values.as_slice(), &["".to_string()]),
            ScalarOrArrayValue::Scalar(value) => assert!(value.is_empty()),
        }
    }

    #[test]
    fn guide_renders_axes_at_display_x() {
        let ctx = SessionContext::new();
        let display_state =
            crate::ParallelDisplayState::active_axis("drag_dimension", "drag_display_x");
        let mut axes = HashMap::new();
        axes.insert(
            "generated_speed".to_string(),
            ParallelAxis::new()
                .title("Speed")
                .with_dimension_metadata("speed", 0)
                .with_frame_state(None, Some(display_state.clone())),
        );
        axes.insert(
            "generated_cost".to_string(),
            ParallelAxis::new()
                .title("Cost")
                .with_dimension_metadata("cost", 1)
                .with_frame_state(None, Some(display_state)),
        );
        let guide = CompiledParallelGuide { axes };
        let facet = TestFacetGuideSharingView;
        let child = TestChildFrameGuideSharingView;
        let scales = HashMap::from([
            (
                "speed".to_string(),
                LinearScale::configured((0.0, 100.0), (200.0, 0.0)),
            ),
            (
                "cost".to_string(),
                LinearScale::configured((0.0, 100.0), (200.0, 0.0)),
            ),
        ]);
        let params = IndexMap::from([
            (
                "drag_dimension".to_string(),
                ScalarValue::Utf8(Some("speed".to_string())),
            ),
            (
                "drag_display_x".to_string(),
                ScalarValue::Float64(Some(210.0)),
            ),
        ]);
        let marks = futures::executor::block_on(guide.evaluate(
            &scales,
            300.0,
            200.0,
            &LayoutBounds {
                x: 25.0,
                y: 40.0,
                width: 300.0,
                height: 200.0,
            },
            &OverflowSpaceRequirement::default(),
            &Theme::light(),
            &params,
            &ctx,
            None,
            GuideSharingContext::new(&facet, &[], &child),
            &EmptyCoordMeasurement,
            GuideRenderContext::without_resource_sink(300.0, 200.0),
        ))
        .expect("evaluate guide");

        match &marks[0] {
            SceneMark::Group(group) => assert_eq!(group.origin, [235.0, 40.0]),
            _ => panic!("expected first parallel axis group"),
        }
        match &marks[1] {
            SceneMark::Group(group) => assert_eq!(group.origin, [325.0, 40.0]),
            _ => panic!("expected second parallel axis group"),
        }
    }

    #[test]
    fn guide_overflow_increases_for_long_categorical_labels() {
        let ctx = SessionContext::new();
        let mut axes = HashMap::new();
        axes.insert(
            "generated_category".to_string(),
            ParallelAxis::new()
                .title("Category")
                .with_dimension_metadata("category", 0),
        );
        let guide = CompiledParallelGuide { axes };
        let facet = TestFacetGuideSharingView;
        let child = TestChildFrameGuideSharingView;
        let short_scales = HashMap::from([(
            "category".to_string(),
            PointScale::configured(Arc::new(StringArray::from(vec!["A", "B"])), (200.0, 0.0)),
        )]);
        let long_scales = HashMap::from([(
            "category".to_string(),
            PointScale::configured(
                Arc::new(StringArray::from(vec![
                    "International growth markets and enterprise platform operations",
                    "North America enterprise",
                ])),
                (200.0, 0.0),
            ),
        )]);
        let short = futures::executor::block_on(guide.measure_overflow(
            &short_scales,
            80.0,
            200.0,
            &Theme::light(),
            &IndexMap::new(),
            None,
            &ctx,
            GuideSharingContext::new(&facet, &[], &child),
            None,
        ))
        .expect("measure short labels");
        let long = futures::executor::block_on(guide.measure_overflow(
            &long_scales,
            80.0,
            200.0,
            &Theme::light(),
            &IndexMap::new(),
            None,
            &ctx,
            GuideSharingContext::new(&facet, &[], &child),
            None,
        ))
        .expect("measure long labels");

        assert!(
            long.left > short.left + 20.0,
            "expected long labels to increase left overflow: short={short:?}, long={long:?}"
        );
    }

    struct TestFacetGuideSharingView;

    impl FacetGuideSharingView for TestFacetGuideSharingView {
        fn channel_axis_visibility_for_path_checked(
            &self,
            _path: &[ScalarValue],
            _axis_position: AxisPosition,
            _sharing_level: u8,
        ) -> Option<AxisVisibility> {
            None
        }

        fn channel_axis_visibility_for_path_checked_with_mode(
            &self,
            _path: &[ScalarValue],
            _axis_position: AxisPosition,
            _sharing_level: u8,
            _ownership_mode: AxisOwnershipMode,
        ) -> Option<AxisVisibility> {
            None
        }

        fn is_jagged_for_axis(&self, _axis_position: AxisPosition) -> bool {
            false
        }

        fn channel_domain_sharing_level(&self, _channel: &str) -> SharingLevel {
            SharingLevel::FREE
        }

        fn effective_edge_indices_for_values_at_path(
            &self,
            _facet_path: &[ScalarValue],
            _values: &[ScalarValue],
        ) -> Option<(usize, usize)> {
            None
        }
    }

    struct TestChildFrameGuideSharingView;

    impl ChildFrameGuideSharingView for TestChildFrameGuideSharingView {
        fn position_indices(&self) -> Vec<usize> {
            Vec::new()
        }

        fn level_counts(&self) -> Vec<usize> {
            Vec::new()
        }

        fn level_axes(&self) -> Vec<CoordinationAxis> {
            Vec::new()
        }

        fn axis_guide_visibility_config_for_axis(
            &self,
            _axis: CoordinationAxis,
        ) -> AxisGuideVisibilityConfig {
            AxisGuideVisibilityConfig::auto()
        }
    }
}
