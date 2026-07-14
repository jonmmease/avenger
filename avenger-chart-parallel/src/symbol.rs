use std::{collections::HashMap, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, EventDatumFieldSpec, IntoExpr,
    LegendRendererSelection, Mark, MarkRuntimeContext, PositionConfig, RenderedMarkData,
    ResolvedDomain, ScalarValueHelpers, ScaleRange, ScaleTypePreference, Theme,
    apply_opacity_to_color_channel, coerce_color_channel_with_renderer,
    coerce_numeric_channel_with_renderer, coerce_opacity_channel_with_renderer,
    default_scale_type_for_data_type, define_common_mark_channels, impl_mark_base,
    impl_mark_trait_common, is_continuous_scale,
};
use avenger_chart_marks::{symbol_channel_defaults, symbol_legend_renderer_kind};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::SymbolShape,
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl, coerce::Coercer};
use avenger_scenegraph::marks::{
    mark::SceneMark, pattern::default_no_fill_pattern, symbol::SceneSymbolMark,
};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    logical_expr::{Expr, lit},
};
use serde::{Deserialize, Serialize};

use crate::{
    Parallel, ParallelDimensionBinding, ParallelDimensionConfig,
    event::{
        PARALLEL_DIMENSION_ID_FIELD, PARALLEL_DISPLACEMENT_PX_FIELD,
        PARALLEL_DISPLACEMENT_SLOTS_FIELD, PARALLEL_DISPLAY_X_FIELD, PARALLEL_EQUILIBRIUM_X_FIELD,
        PARALLEL_ORDER_INDEX_FIELD, PARALLEL_SCALE_NAME_FIELD, PARALLEL_SURFACE_KIND_FIELD,
        PARALLEL_SURFACE_KIND_POINT,
    },
};

/// Parallel-coordinate point overlay mark.
///
/// It renders one symbol for each finite source-row/dimension intersection.
pub struct ParallelSymbol<C = Parallel> {
    pub(crate) state: avenger_chart_core::MarkState,
    pub(crate) _phantom: std::marker::PhantomData<C>,
}

impl_mark_base!(ParallelSymbol);

impl ParallelSymbol<Parallel> {
    pub fn dimension(self, id: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.dimension_with(id, expr, |dimension| dimension)
    }

    pub fn dimension_with<F>(self, id: impl Into<String>, expr: impl IntoExpr, configure: F) -> Self
    where
        F: FnOnce(ParallelDimensionConfig) -> ParallelDimensionConfig,
    {
        let binding = ParallelDimensionBinding::new(id, expr);
        let generated_channel = binding.generated_channel.clone();
        let id = binding.id.clone();
        let config = ParallelDimensionConfig::new(binding.channel_value);
        let (channel_value, axis_config) = configure(config).take_axis_config();
        let mut mark =
            self.with_channel_value(&generated_channel, channel_value.with_scale_name(id));
        if let Some(axis_config) = axis_config {
            mark.state
                .axis_configs
                .insert(generated_channel, Arc::new(axis_config));
        }
        mark
    }
}

define_common_mark_channels! {
    ParallelSymbol {
        size: {
            allow_column: true,
            with_config: avenger_chart_core::SizeChannelConfig,
        },
        fill: {
            allow_column: true,
            with_config: avenger_chart_core::ColorChannelConfig,
        },
        stroke: {
            allow_column: true,
            with_config: avenger_chart_core::ColorChannelConfig,
        },
        stroke_width: {
            allow_column: false,
            with_config: avenger_chart_core::StrokeWidthChannelConfig,
        },
        shape: {
            allow_column: true,
            with_config: avenger_chart_core::ShapeChannelConfig,
        },
        angle: {
            allow_column: true,
            with_config: avenger_chart_core::AngleChannelConfig,
        },
        opacity: {
            allow_column: true,
            with_config: avenger_chart_core::OpacityChannelConfig,
        },
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<Parallel> for ParallelSymbol<Parallel> {
    impl_mark_trait_common!(ParallelSymbol);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledParallelSymbol {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledParallelSymbol {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledParallelSymbol {
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
        "parallel_symbol"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "size",
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
                name: "shape",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "angle",
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
        symbol_channel_defaults(channel)
    }

    fn event_datum_field_specs(&self) -> Vec<EventDatumFieldSpec> {
        parallel_point_event_datum_field_specs()
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        symbol_legend_renderer_kind(channel, scale, &[]).map(LegendRendererSelection::BuiltIn)
    }

    fn preferred_scale_type(
        &self,
        channel: &str,
        data_type: &DataType,
    ) -> Option<ScaleTypePreference> {
        match (channel, data_type) {
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
            (
                "fill" | "stroke" | "color" | "shape",
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            ) => Some(ScaleTypePreference::Ordinal),
            ("stroke_width", DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View) => {
                Some(ScaleTypePreference::Ordinal)
            }
            _ => default_scale_type_for_data_type(data_type),
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
        _data_type: &DataType,
    ) -> HashMap<String, Expr> {
        let mut options = HashMap::new();
        if channel == "size" && scale_impl.scale_type() == "pow" {
            options.insert("exponent".to_string(), lit(0.5f32));
        }
        if matches!(channel, "fill" | "stroke" | "color") && is_continuous_scale(scale_impl) {
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
        params: &indexmap::IndexMap<String, ScalarValue>,
    ) -> Option<ScaleRange> {
        let range_kind = scale_impl.range_kind();
        let cardinality = match domain {
            ResolvedDomain::Discrete(count) => Some(*count),
            ResolvedDomain::Interval => None,
        };
        if let Some(theme_range) =
            theme.get_range_for_channel("symbol", channel, range_kind, cardinality, params)
        {
            return Some(theme_range);
        }

        match channel {
            "size" => match domain {
                ResolvedDomain::Discrete(count) => {
                    let min = 40.0;
                    let max = 400.0;
                    if *count == 1 {
                        Some(ScaleRange::new_discrete(vec![ScalarValue::Float32(Some(
                            max,
                        ))]))
                    } else {
                        Some(ScaleRange::new_linspace_discrete(min, max, *count))
                    }
                }
                ResolvedDomain::Interval => Some(ScaleRange::new_interval(lit(0.0), lit(400.0))),
            },
            "angle" => Some(domain.make_interval_or_linspaced_range(0.0, 360.0)),
            "opacity" => Some(domain.make_interval_or_linspaced_range(0.0, 1.0)),
            "stroke_width" => Some(domain.make_interval_or_linspaced_range(0.5, 5.0)),
            _ => None,
        }
    }
}

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl CompiledMark for CompiledParallelSymbol {
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
                "ParallelSymbol requires inherited or explicit row data".to_string(),
            )
        })?;
        let mark_context = context.core_view();
        let mark_channels = self.state.data.channels();
        let slots = coord
            .generated_position_slots(context.plot_width(), mark_context.params())?
            .into_iter()
            .filter(|slot| mark_channels.contains_key(&slot.channel))
            .collect::<Vec<_>>();
        if slots.is_empty() {
            return Err(AvengerChartError::CoordinateSystemError(
                "ParallelSymbol requires a Parallel coordinate system with at least one dimension"
                    .to_string(),
            ));
        }

        let row_count = data.num_rows();
        let dimension_count = slots.len();

        let dimension_values = slots
            .iter()
            .map(|slot| {
                coerce_numeric_channel_with_renderer(
                    self,
                    Some(data),
                    scalars,
                    &slot.channel,
                    &mark_context,
                    f32::NAN,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        let size = coerce_numeric_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "size",
            &mark_context,
            64.0,
        )?;
        let fill = coerce_color_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "fill",
            &mark_context,
            [70.0 / 255.0, 130.0 / 255.0, 180.0 / 255.0, 1.0],
        )?;
        let stroke = coerce_color_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "stroke",
            &mark_context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let angle = coerce_numeric_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "angle",
            &mark_context,
            0.0,
        )?;
        let opacity = coerce_opacity_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "opacity",
            &mark_context,
            1.0,
        )?;
        let fill = apply_opacity_to_color_channel(fill, &opacity, row_count);
        let stroke = apply_opacity_to_color_channel(stroke, &opacity, row_count);

        let coercer = Coercer::default();
        let shape_default = self
            .default_channel_value("shape", &mark_context)
            .and_then(|scalar| match scalar {
                ScalarValue::Utf8(Some(value)) => SymbolShape::from_vega_str(&value).ok(),
                _ => None,
            })
            .unwrap_or(SymbolShape::Circle);
        let (shapes, shape_index) = if let Some(shape_array) = data.column_by_name("shape") {
            coercer.to_symbol_shape(shape_array, Some(shape_default))?
        } else if let Some(shape_scalar) = scalars.column_by_name("shape") {
            coercer.to_symbol_shape(shape_scalar, Some(shape_default))?
        } else {
            (vec![shape_default], ScalarOrArray::new_scalar(0))
        };

        let stroke_width_default = self
            .default_channel_value("stroke_width", &mark_context)
            .and_then(|scalar| ScalarValueHelpers::as_f32(&scalar).ok())
            .unwrap_or(1.0);
        let stroke_width = if let Some(width_scalar) = scalars.column_by_name("stroke_width") {
            let value = *coercer
                .to_numeric(width_scalar, Some(stroke_width_default))?
                .first()
                .unwrap_or(&stroke_width_default);
            Some(value)
        } else {
            Some(stroke_width_default)
        };

        let mut marks = Vec::with_capacity(dimension_count);
        let mut source_row_indices = Vec::with_capacity(dimension_count);
        let mut event_datum_rows = Vec::with_capacity(dimension_count);
        for (dimension_index, slot) in slots.iter().enumerate() {
            let x = slot.display_x;
            let values = &dimension_values[dimension_index];
            let mut y = Vec::new();
            let mut source_rows = Vec::new();
            for row in 0..row_count {
                let value = f32_at(values, row, f32::NAN);
                if value.is_finite() {
                    y.push(value);
                    source_rows.push(row);
                }
            }

            let len = y.len();
            marks.push(SceneMark::Symbol(SceneSymbolMark {
                name: "parallel_symbol".to_string(),
                clip: true,
                len: len as u32,
                gradients: vec![],
                shapes: shapes.clone(),
                stroke_width,
                shape_index: gather_usize(&shape_index, &source_rows),
                x: ScalarOrArray::new_scalar(x),
                y: ScalarOrArray::from(y),
                fill: gather_color(&fill, &source_rows),
                fill_pattern: default_no_fill_pattern(),
                size: gather_f32(&size, &source_rows),
                stroke: gather_color(&stroke, &source_rows),
                angle: gather_f32(&angle, &source_rows),
                indices: None,
                zindex: self.state.zindex,
                x_adjustment: None,
                y_adjustment: None,
                interactive: true,
            }));
            event_datum_rows.push(parallel_point_event_datum_batch(len, slot)?);
            source_row_indices.push(source_rows);
        }

        Ok(
            RenderedMarkData::with_source_row_indices_and_event_datum_rows(
                marks,
                source_row_indices,
                event_datum_rows,
            ),
        )
    }
}

fn parallel_point_event_datum_field_specs() -> Vec<EventDatumFieldSpec> {
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

fn parallel_point_event_datum_batch(
    len: usize,
    slot: &avenger_chart_core::GeneratedPositionSlot,
) -> Result<RecordBatch, AvengerChartError> {
    let schema = Arc::new(Schema::new(vec![
        Field::new(PARALLEL_SURFACE_KIND_FIELD, DataType::Utf8, false),
        Field::new(PARALLEL_DIMENSION_ID_FIELD, DataType::Utf8, false),
        Field::new(PARALLEL_SCALE_NAME_FIELD, DataType::Utf8, false),
        Field::new(PARALLEL_ORDER_INDEX_FIELD, DataType::Int64, false),
        Field::new(PARALLEL_EQUILIBRIUM_X_FIELD, DataType::Float64, false),
        Field::new(PARALLEL_DISPLAY_X_FIELD, DataType::Float64, false),
        Field::new(PARALLEL_DISPLACEMENT_PX_FIELD, DataType::Float64, false),
        Field::new(PARALLEL_DISPLACEMENT_SLOTS_FIELD, DataType::Float64, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![PARALLEL_SURFACE_KIND_POINT; len])) as ArrayRef,
            Arc::new(StringArray::from(vec![slot.id.as_str(); len])),
            Arc::new(StringArray::from(vec![slot.scale_name.as_str(); len])),
            Arc::new(Int64Array::from(vec![slot.order_index as i64; len])),
            Arc::new(Float64Array::from(vec![f64::from(slot.equilibrium_x); len])),
            Arc::new(Float64Array::from(vec![f64::from(slot.display_x); len])),
            Arc::new(Float64Array::from(vec![
                f64::from(slot.displacement_px);
                len
            ])),
            Arc::new(Float64Array::from(vec![
                f64::from(slot.displacement_slots);
                len
            ])),
        ],
    )
    .map_err(AvengerChartError::ArrowError)
}

fn f32_at(values: &ScalarOrArray<f32>, row: usize, default: f32) -> f32 {
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => *value,
        ScalarOrArrayValue::Array(values) => values.get(row).copied().unwrap_or(default),
    }
}

fn usize_at(values: &ScalarOrArray<usize>, row: usize, default: usize) -> usize {
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => *value,
        ScalarOrArrayValue::Array(values) => values.get(row).copied().unwrap_or(default),
    }
}

fn color_at(values: &ScalarOrArray<ColorOrGradient>, row: usize) -> ColorOrGradient {
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => value.clone(),
        ScalarOrArrayValue::Array(values) => values
            .get(row)
            .cloned()
            .unwrap_or(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
    }
}

fn gather_f32(values: &ScalarOrArray<f32>, source_rows: &[usize]) -> ScalarOrArray<f32> {
    if matches!(values.value(), ScalarOrArrayValue::Scalar(_)) {
        return values.clone();
    }
    ScalarOrArray::from(
        source_rows
            .iter()
            .map(|row| f32_at(values, *row, 0.0))
            .collect::<Vec<_>>(),
    )
}

fn gather_usize(values: &ScalarOrArray<usize>, source_rows: &[usize]) -> ScalarOrArray<usize> {
    if matches!(values.value(), ScalarOrArrayValue::Scalar(_)) {
        return values.clone();
    }
    ScalarOrArray::from(
        source_rows
            .iter()
            .map(|row| usize_at(values, *row, 0))
            .collect::<Vec<_>>(),
    )
}

fn gather_color(
    values: &ScalarOrArray<ColorOrGradient>,
    source_rows: &[usize],
) -> ScalarOrArray<ColorOrGradient> {
    if matches!(values.value(), ScalarOrArrayValue::Scalar(_)) {
        return values.clone();
    }
    ScalarOrArray::from(
        source_rows
            .iter()
            .map(|row| color_at(values, *row))
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use avenger_chart_core::{
        CompiledDataContext, CompiledMark, CompiledMarkState, CoordinateSystem,
        EmptyCoordMeasurement, EvaluationContext, FacetDataScope, MarkDataMode, MarkRenderContext,
        MarkRuntimeContext, Theme,
    };
    use avenger_color::ColorOrGradient;
    use avenger_common::value::ScalarOrArrayValue;
    use avenger_scenegraph::marks::mark::SceneMark;
    use datafusion::{
        arrow::{
            array::{ArrayRef, Float32Array, Float64Array, StringArray},
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        common::ScalarValue,
        prelude::SessionContext,
    };

    use crate::event::{
        PARALLEL_DIMENSION_ID_FIELD, PARALLEL_DISPLACEMENT_PX_FIELD, PARALLEL_DISPLAY_X_FIELD,
        PARALLEL_EQUILIBRIUM_X_FIELD, PARALLEL_SURFACE_KIND_FIELD,
    };
    use futures::executor::block_on;
    use indexmap::IndexMap;

    use super::*;
    use crate::{Parallel, generated_dimension_channel};

    struct TestRuntimeContext {
        eval: EvaluationContext,
        measurement: EmptyCoordMeasurement,
        plot_width: f32,
        plot_height: f32,
        facet_path: Vec<ScalarValue>,
    }

    impl TestRuntimeContext {
        fn new(plot_width: f32, plot_height: f32) -> Self {
            Self {
                eval: EvaluationContext::new(
                    Arc::new(Theme::light()),
                    Arc::new(SessionContext::new()),
                    IndexMap::new(),
                ),
                measurement: EmptyCoordMeasurement,
                plot_width,
                plot_height,
                facet_path: Vec::new(),
            }
        }

        fn with_params(mut self, params: IndexMap<String, ScalarValue>) -> Self {
            self.eval = self.eval.with_params(params);
            self
        }
    }

    impl MarkRuntimeContext for TestRuntimeContext {
        fn core_view(&self) -> MarkRenderContext<'_> {
            MarkRenderContext::new(&self.eval, self.plot_width, self.plot_height)
        }

        fn coord_measurement(&self) -> &dyn avenger_chart_core::CoordMeasurement {
            &self.measurement
        }

        fn facet_path(&self) -> &[ScalarValue] {
            &self.facet_path
        }
    }

    fn dimension_channels() -> IndexMap<String, avenger_chart_core::ChannelValue> {
        IndexMap::from([
            (
                generated_dimension_channel("alpha"),
                avenger_chart_core::ChannelValue::from(0.0).with_scale_name("alpha"),
            ),
            (
                generated_dimension_channel("beta"),
                avenger_chart_core::ChannelValue::from(0.0).with_scale_name("beta"),
            ),
        ])
    }

    fn compiled_state() -> CompiledMarkState {
        CompiledMarkState {
            id: None,
            public_target_path: None,
            data: CompiledDataContext::new(None, Vec::new(), dimension_channels()),
            view: None,
            data_mode: MarkDataMode::Inherit,
            mark_index: 0,
            facet_data_scope: FacetDataScope::default(),
            exclude_from_scale_domains: false,
            visible: None,
            details: None,
            zindex: Some(12),
            geometry_space: None,
            axis_configs: HashMap::new(),
            widget_theme: None,
        }
    }

    fn scalar_batch() -> RecordBatch {
        RecordBatch::new_empty(Arc::new(Schema::empty()))
    }

    fn prepared_data() -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new(
                    generated_dimension_channel("alpha"),
                    DataType::Float32,
                    true,
                ),
                Field::new(generated_dimension_channel("beta"), DataType::Float32, true),
                Field::new("fill", DataType::Utf8, false),
            ])),
            vec![
                Arc::new(Float32Array::from(vec![Some(10.0), Some(20.0)])) as ArrayRef,
                Arc::new(Float32Array::from(vec![Some(30.0), None])),
                Arc::new(StringArray::from(vec!["red", "blue"])),
            ],
        )
        .expect("prepared parallel symbol data")
    }

    fn render_test_symbol() -> avenger_chart_core::RenderedMarkData {
        render_test_symbol_with_coord_and_context(
            Parallel::new()
                .dimension("alpha")
                .dimension("beta")
                .create_transform(),
            TestRuntimeContext::new(100.0, 50.0),
        )
    }

    fn render_test_symbol_with_coord_and_context(
        coord: Box<dyn avenger_chart_core::CoordinateSystemTransform>,
        context: TestRuntimeContext,
    ) -> avenger_chart_core::RenderedMarkData {
        let mark = CompiledParallelSymbol {
            state: compiled_state(),
        };
        block_on(mark.render_mark_data(
            Some(&prepared_data()),
            &scalar_batch(),
            &context,
            coord.as_ref(),
        ))
        .expect("render parallel symbol")
    }

    #[test]
    fn parallel_symbol_renders_one_point_per_finite_dimension_value() {
        let rendered = render_test_symbol();
        assert_eq!(rendered.marks.len(), 2);

        let first = match &rendered.marks[0] {
            SceneMark::Symbol(mark) => mark,
            _ => panic!("expected first dimension symbol mark"),
        };
        let second = match &rendered.marks[1] {
            SceneMark::Symbol(mark) => mark,
            _ => panic!("expected second dimension symbol mark"),
        };
        assert_eq!(first.len, 2);
        assert_eq!(second.len, 1);
        assert_eq!(first.zindex, Some(12));
        assert_eq!(second.zindex, Some(12));

        match first.x.value() {
            ScalarOrArrayValue::Scalar(value) => assert_eq!(*value, 0.0),
            ScalarOrArrayValue::Array(_) => panic!("expected scalar x for first dimension"),
        }
        match second.x.value() {
            ScalarOrArrayValue::Scalar(value) => assert_eq!(*value, 100.0),
            ScalarOrArrayValue::Array(_) => panic!("expected scalar x for second dimension"),
        }
        match first.y.value() {
            ScalarOrArrayValue::Array(values) => assert_eq!(values.as_slice(), &[10.0, 20.0]),
            ScalarOrArrayValue::Scalar(_) => panic!("expected y array for first dimension"),
        }
        match second.y.value() {
            ScalarOrArrayValue::Array(values) => assert_eq!(values.as_slice(), &[30.0]),
            ScalarOrArrayValue::Scalar(_) => panic!("expected y array for second dimension"),
        }
    }

    #[test]
    fn parallel_symbol_preserves_source_rows_and_dimension_event_datums() {
        let rendered = render_test_symbol();
        let source_rows = rendered
            .source_row_indices
            .as_ref()
            .expect("source row indices");
        assert_eq!(source_rows, &vec![vec![0, 1], vec![0]]);

        let event_rows = rendered
            .event_datum_rows
            .as_ref()
            .expect("generated event datum rows");
        let alpha_dimension = event_rows[0]
            .column_by_name(PARALLEL_DIMENSION_ID_FIELD)
            .expect("alpha dimension id")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("alpha dimension string");
        assert_eq!(alpha_dimension.value(0), "alpha");
        assert_eq!(alpha_dimension.value(1), "alpha");

        let beta_dimension = event_rows[1]
            .column_by_name(PARALLEL_DIMENSION_ID_FIELD)
            .expect("beta dimension id")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("beta dimension string");
        assert_eq!(beta_dimension.value(0), "beta");

        let surface_kind = event_rows[1]
            .column_by_name(PARALLEL_SURFACE_KIND_FIELD)
            .expect("surface kind")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("surface kind string");
        assert_eq!(surface_kind.value(0), PARALLEL_SURFACE_KIND_POINT);
    }

    #[test]
    fn parallel_symbol_uses_display_slots_and_event_datum_geometry() {
        let coord = Parallel::new()
            .dimension("alpha")
            .dimension("beta")
            .active_axis_display_params("drag_dimension", "drag_display_x")
            .create_transform();
        let context = TestRuntimeContext::new(100.0, 50.0).with_params(IndexMap::from([
            (
                "drag_dimension".to_string(),
                ScalarValue::Utf8(Some("beta".to_string())),
            ),
            (
                "drag_display_x".to_string(),
                ScalarValue::Float64(Some(72.0)),
            ),
        ]));
        let rendered = render_test_symbol_with_coord_and_context(coord, context);
        let second = match &rendered.marks[1] {
            SceneMark::Symbol(mark) => mark,
            _ => panic!("expected second dimension symbol mark"),
        };
        match second.x.value() {
            ScalarOrArrayValue::Scalar(value) => assert_eq!(*value, 72.0),
            ScalarOrArrayValue::Array(_) => panic!("expected scalar x for second dimension"),
        }

        let event_rows = rendered
            .event_datum_rows
            .as_ref()
            .expect("generated event datum rows");
        let beta_rows = &event_rows[1];
        let equilibrium_x = beta_rows
            .column_by_name(PARALLEL_EQUILIBRIUM_X_FIELD)
            .expect("equilibrium x")
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("equilibrium x f64");
        let display_x = beta_rows
            .column_by_name(PARALLEL_DISPLAY_X_FIELD)
            .expect("display x")
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("display x f64");
        let displacement_px = beta_rows
            .column_by_name(PARALLEL_DISPLACEMENT_PX_FIELD)
            .expect("displacement")
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("displacement f64");
        assert_eq!(equilibrium_x.value(0), 100.0);
        assert_eq!(display_x.value(0), 72.0);
        assert_eq!(displacement_px.value(0), -28.0);
    }

    #[test]
    fn parallel_symbol_style_channels_vary_by_source_row() {
        let rendered = render_test_symbol();
        let first = match &rendered.marks[0] {
            SceneMark::Symbol(mark) => mark,
            _ => panic!("expected first dimension symbol mark"),
        };
        match first.fill.value() {
            ScalarOrArrayValue::Array(values) => {
                assert_eq!(
                    values.as_slice(),
                    &[
                        ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0]),
                        ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]),
                    ]
                );
            }
            ScalarOrArrayValue::Scalar(_) => panic!("expected array fill"),
        }
    }
}
