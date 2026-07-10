use std::{marker::PhantomData, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, IntoExpr, LegendRendererKind,
    LegendRendererSelection, Mark, MarkRuntimeContext, PositionConfig, RenderedMarkData,
    apply_opacity_to_color, coerce_bool_channel_with_renderer, coerce_color_channel_with_renderer,
    coerce_numeric_channel_with_renderer, define_common_mark_channels, impl_mark_base,
    impl_mark_trait_common,
};
use avenger_chart_marks::line_channel_defaults;
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{StrokeCap, StrokeJoin},
    value::{ScalarOrArray, ScalarOrArrayValue},
};
use avenger_scales::scales::coerce::Coercer;
use avenger_scenegraph::marks::{line::SceneLineMark, mark::SceneMark};
use datafusion::{
    arrow::{array::RecordBatch, datatypes::DataType},
    common::ScalarValue,
};
use serde::{Deserialize, Serialize};

use crate::{Parallel, ParallelDimensionBinding, ParallelDimensionConfig};

/// Polyline mark for wide-form parallel-coordinate rows.
///
/// `ParallelLine` is coordinate-specific: it implements `Mark<Parallel>`, not
/// the generic Cartesian line-mark contract.
///
/// ```compile_fail
/// use avenger_chart_cartesian::Cartesian;
/// use avenger_chart_core::Mark;
/// use avenger_chart_parallel::ParallelLine;
///
/// fn requires_cartesian_mark<M: Mark<Cartesian>>(_: M) {}
///
/// requires_cartesian_mark(ParallelLine::new());
/// ```
pub struct ParallelLine<C = Parallel> {
    pub(crate) state: avenger_chart_core::MarkState,
    pub(crate) _phantom: PhantomData<C>,
}

impl_mark_base!(ParallelLine);

impl ParallelLine<Parallel> {
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
    ParallelLine {
        stroke: {
            allow_column: true,
            with_config: avenger_chart_core::ColorChannelConfig,
        },
        stroke_width: {
            allow_column: true,
            with_config: avenger_chart_core::StrokeWidthChannelConfig,
        },
        stroke_dash: {
            allow_column: true,
            with_config: avenger_chart_core::StrokeDashChannelConfig,
        },
        stroke_cap: {
            allow_column: false,
        },
        stroke_join: {
            allow_column: false,
        },
        opacity: {
            allow_column: true,
            with_config: avenger_chart_core::OpacityChannelConfig,
        },
        defined: {
            allow_column: true,
        },
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<Parallel> for ParallelLine<Parallel> {
    impl_mark_trait_common!(ParallelLine);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledParallelLine {
            state: compiled_state,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledParallelLine {
    pub(crate) state: CompiledMarkState,
}

impl CompiledMarkCore for CompiledParallelLine {
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
        "parallel_line"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
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
                allow_column_ref: false,
            },
            ChannelDescriptor {
                name: "stroke_join",
                required: false,
                default_value: None,
                allow_column_ref: false,
            },
            ChannelDescriptor {
                name: "defined",
                required: false,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    fn mark_specific_default(&self, channel: &str) -> Option<ScalarValue> {
        line_channel_defaults(channel)
    }

    fn preferred_scale_type(
        &self,
        _channel: &str,
        data_type: &DataType,
    ) -> Option<avenger_chart_core::ScaleTypePreference> {
        avenger_chart_core::default_scale_type_for_data_type(data_type)
    }

    fn preferred_legend_renderer(
        &self,
        channel: &str,
        scale: &avenger_scales::scales::ConfiguredScale,
    ) -> Option<LegendRendererSelection> {
        let is_continuous = avenger_chart_core::is_continuous_scale(scale.scale_impl.as_ref());
        match channel {
            "stroke" if is_continuous => Some(LegendRendererSelection::BuiltIn(
                LegendRendererKind::Colorbar,
            )),
            "stroke" | "stroke_width" | "stroke_dash" | "opacity" => {
                Some(LegendRendererSelection::BuiltIn(LegendRendererKind::Line))
            }
            _ => None,
        }
    }
}

#[typetag::serde]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl CompiledMark for CompiledParallelLine {
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
                "ParallelLine requires inherited or explicit row data".to_string(),
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
                "ParallelLine requires a Parallel coordinate system with at least one dimension"
                    .to_string(),
            ));
        }

        let row_count = data.num_rows();
        let dimension_count = slots.len();
        let x_positions = slots.iter().map(|slot| slot.display_x).collect::<Vec<_>>();

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
        let defined_values = coerce_bool_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "defined",
            &mark_context,
            true,
        )?;
        let strokes = coerce_color_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "stroke",
            &mark_context,
            [0.0, 0.0, 0.0, 1.0],
        )?;
        let stroke_widths = coerce_numeric_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "stroke_width",
            &mark_context,
            2.0,
        )?;
        let opacities = coerce_numeric_channel_with_renderer(
            self,
            Some(data),
            scalars,
            "opacity",
            &mark_context,
            1.0,
        )?;
        let stroke_cap = scalar_stroke_cap(self, &mark_context);
        let stroke_join = scalar_stroke_join(self, &mark_context);
        let stroke_dashes = stroke_dash_values(data, scalars)?;

        let mut marks = Vec::with_capacity(row_count);
        let mut source_row_indices = Vec::with_capacity(row_count);
        for row in 0..row_count {
            let row_defined = bool_at(&defined_values, row, true);
            let mut y_values = Vec::with_capacity(dimension_count);
            let mut vertex_defined = Vec::with_capacity(dimension_count);
            for values in &dimension_values {
                let value = f32_at(values, row, f32::NAN);
                y_values.push(value);
                vertex_defined.push(row_defined && value.is_finite());
            }

            let stroke = color_at(&strokes, row);
            let opacity = f32_at(&opacities, row, 1.0).clamp(0.0, 1.0);
            let stroke = apply_opacity_to_color(&stroke, opacity);
            let stroke_width = f32_at(&stroke_widths, row, 2.0);
            let stroke_dash = stroke_dashes
                .as_ref()
                .map(|values| stroke_dash_at(values, row))
                .filter(|dash| !dash.is_empty());

            marks.push(SceneMark::Line(SceneLineMark {
                name: "parallel_line".to_string(),
                clip: true,
                len: dimension_count as u32,
                x: ScalarOrArray::from(x_positions.clone()),
                y: ScalarOrArray::from(y_values),
                gradients: vec![],
                stroke,
                stroke_width,
                stroke_dash,
                stroke_cap,
                stroke_join,
                defined: ScalarOrArray::from(vertex_defined),
                zindex: self.state.zindex,
                interactive: true,
            }));
            source_row_indices.push(vec![row]);
        }

        Ok(RenderedMarkData::with_source_row_indices(
            marks,
            source_row_indices,
        ))
    }
}

fn f32_at(values: &ScalarOrArray<f32>, row: usize, default: f32) -> f32 {
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => *value,
        ScalarOrArrayValue::Array(values) => values.get(row).copied().unwrap_or(default),
    }
}

fn bool_at(values: &ScalarOrArray<bool>, row: usize, default: bool) -> bool {
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

fn scalar_stroke_cap(
    mark: &CompiledParallelLine,
    context: &avenger_chart_core::MarkRenderContext<'_>,
) -> StrokeCap {
    mark.default_channel_value("stroke_cap", context)
        .and_then(|value| match value {
            ScalarValue::Utf8(Some(value)) => match value.as_str() {
                "butt" => Some(StrokeCap::Butt),
                "round" => Some(StrokeCap::Round),
                "square" => Some(StrokeCap::Square),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or(StrokeCap::Round)
}

fn scalar_stroke_join(
    mark: &CompiledParallelLine,
    context: &avenger_chart_core::MarkRenderContext<'_>,
) -> StrokeJoin {
    mark.default_channel_value("stroke_join", context)
        .and_then(|value| match value {
            ScalarValue::Utf8(Some(value)) => match value.as_str() {
                "miter" => Some(StrokeJoin::Miter),
                "round" => Some(StrokeJoin::Round),
                "bevel" => Some(StrokeJoin::Bevel),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or(StrokeJoin::Round)
}

fn stroke_dash_values(
    data: &RecordBatch,
    scalars: &RecordBatch,
) -> Result<Option<ScalarOrArray<Vec<f32>>>, AvengerChartError> {
    let coercer = Coercer::default();
    if let Some(array) = data.column_by_name("stroke_dash") {
        return Ok(Some(coercer.to_stroke_dash(array)?));
    }
    if let Some(array) = scalars.column_by_name("stroke_dash") {
        return Ok(Some(coercer.to_stroke_dash(array)?));
    }
    Ok(None)
}

fn stroke_dash_at(values: &ScalarOrArray<Vec<f32>>, row: usize) -> Vec<f32> {
    match values.value() {
        ScalarOrArrayValue::Scalar(value) => value.clone(),
        ScalarOrArrayValue::Array(values) => values.get(row).cloned().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use avenger_chart_core::{
        CompiledDataContext, CompiledMarkState, CoordinateSystem, EmptyCoordMeasurement,
        EvaluationContext, FacetDataScope, MarkDataMode, MarkRenderContext, MarkRuntimeContext,
        Theme,
    };
    use datafusion::{
        arrow::{
            array::{ArrayRef, Float32Array, RecordBatch, StringArray},
            datatypes::{DataType, Field, Schema},
        },
        common::ScalarValue,
        prelude::{SessionContext, col},
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
            details: Some(vec!["id".to_string()]),
            zindex: Some(8),
            geometry_space: None,
            axis_configs: HashMap::new(),
        }
    }

    fn scalar_batch() -> RecordBatch {
        RecordBatch::new_empty(Arc::new(Schema::empty()))
    }

    fn prepared_data() -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("id", DataType::Utf8, false),
                Field::new(
                    generated_dimension_channel("alpha"),
                    DataType::Float32,
                    true,
                ),
                Field::new(generated_dimension_channel("beta"), DataType::Float32, true),
                Field::new("stroke", DataType::Utf8, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["row0", "row1"])) as ArrayRef,
                Arc::new(Float32Array::from(vec![Some(10.0), Some(20.0)])),
                Arc::new(Float32Array::from(vec![Some(30.0), None])),
                Arc::new(StringArray::from(vec!["red", "blue"])),
            ],
        )
        .expect("prepared parallel line data")
    }

    fn render_test_line() -> avenger_chart_core::RenderedMarkData {
        render_test_line_with_coord_and_context(
            Parallel::new()
                .dimension("alpha")
                .dimension("beta")
                .create_transform(),
            TestRuntimeContext::new(100.0, 50.0),
        )
    }

    fn render_test_line_with_coord_and_context(
        coord: Box<dyn avenger_chart_core::CoordinateSystemTransform>,
        context: TestRuntimeContext,
    ) -> avenger_chart_core::RenderedMarkData {
        let mark = CompiledParallelLine {
            state: compiled_state(),
        };
        block_on(mark.render_mark_data(
            Some(&prepared_data()),
            &scalar_batch(),
            &context,
            coord.as_ref(),
        ))
        .expect("render parallel line")
    }

    #[test]
    fn parallel_line_dimension_adds_mark_owned_hidden_channel() {
        let mark = ParallelLine::new().dimension("alpha", col("alpha"));
        let channel = generated_dimension_channel("alpha");
        let value = mark
            .state()
            .data
            .channels()
            .get(&channel)
            .expect("hidden dimension channel");
        assert_eq!(value.get_scale_name(&channel), Some("alpha".to_string()));
    }

    #[test]
    fn parallel_line_accepts_details_for_event_datum_retention() {
        let mark = ParallelLine::<Parallel>::new().details(["id"]);
        assert_eq!(mark.state.details.as_deref(), Some(&["id".to_string()][..]));
    }

    #[test]
    fn parallel_line_renders_one_line_per_source_row_and_preserves_source_rows() {
        let rendered = render_test_line();
        assert_eq!(rendered.marks.len(), 2);
        assert_eq!(
            rendered
                .source_row_indices
                .as_ref()
                .expect("source row indices"),
            &vec![vec![0], vec![1]]
        );

        let first = match &rendered.marks[0] {
            SceneMark::Line(mark) => mark,
            _ => panic!("expected first line mark"),
        };
        assert_eq!(first.len, 2);
        assert_eq!(first.zindex, Some(8));
        match first.x.value() {
            ScalarOrArrayValue::Array(values) => assert_eq!(values.as_slice(), &[0.0, 100.0]),
            ScalarOrArrayValue::Scalar(_) => panic!("expected x array"),
        }
        match first.y.value() {
            ScalarOrArrayValue::Array(values) => assert_eq!(values.as_slice(), &[10.0, 30.0]),
            ScalarOrArrayValue::Scalar(_) => panic!("expected y array"),
        }
        match first.defined.value() {
            ScalarOrArrayValue::Array(values) => assert_eq!(values.as_slice(), &[true, true]),
            ScalarOrArrayValue::Scalar(_) => panic!("expected defined array"),
        }
    }

    #[test]
    fn parallel_line_marks_missing_dimension_vertices_undefined() {
        let rendered = render_test_line();
        let second = match &rendered.marks[1] {
            SceneMark::Line(mark) => mark,
            _ => panic!("expected second line mark"),
        };
        match second.y.value() {
            ScalarOrArrayValue::Array(values) => {
                assert_eq!(values[0], 20.0);
                assert!(values[1].is_nan());
            }
            ScalarOrArrayValue::Scalar(_) => panic!("expected y array"),
        }
        match second.defined.value() {
            ScalarOrArrayValue::Array(values) => assert_eq!(values.as_slice(), &[true, false]),
            ScalarOrArrayValue::Scalar(_) => panic!("expected defined array"),
        }
    }

    #[test]
    fn parallel_line_uses_display_x_for_geometry() {
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
        let rendered = render_test_line_with_coord_and_context(coord, context);
        let first = match &rendered.marks[0] {
            SceneMark::Line(mark) => mark,
            _ => panic!("expected first line mark"),
        };
        match first.x.value() {
            ScalarOrArrayValue::Array(values) => assert_eq!(values.as_slice(), &[0.0, 72.0]),
            ScalarOrArrayValue::Scalar(_) => panic!("expected x array"),
        }
    }

    #[test]
    fn parallel_line_style_channels_can_vary_by_source_row() {
        let rendered = render_test_line();
        let first = match &rendered.marks[0] {
            SceneMark::Line(mark) => mark,
            _ => panic!("expected first line mark"),
        };
        let second = match &rendered.marks[1] {
            SceneMark::Line(mark) => mark,
            _ => panic!("expected second line mark"),
        };
        assert_eq!(first.stroke, ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(second.stroke, ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0]));
    }
}
