//! Parallel coordinates with axis brushing.
//!
//! Drag near a vertical axis to brush a value interval on that dimension.
//! Shift-drag another axis to add another brush; selected rows are the
//! intersection of active dimension intervals. Selected lines render above the
//! grey context lines and use a continuous color scale.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example parallel_axis_brush_selection --features winit-wgpu
//! ```

use std::{marker::PhantomData, sync::Arc};

use avenger_chart::{event as ev, prelude::*};
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use avenger_chart_core::{
    AvengerChartError, ChannelDescriptor, CompiledDataContext, CompiledMark, CompiledMarkCore,
    CompiledMarkState, CoordinateSystemTransformCore, DataContext, IntoPlotMark, Mark,
    MarkDataMode, MarkRuntimeContext, MarkState, PlotMark, RenderedMarkData,
};
use avenger_color::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::{mark::SceneMark, rect::SceneRectMark};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    prelude::{Expr, SessionContext, col, lit, when},
};
use serde::{Deserialize, Serialize};
use winit::{dpi::LogicalSize, window::WindowAttributes};

const CANVAS_SIZE: [f32; 2] = [1040.0, 620.0];
const PLOT_SIZE: [f32; 2] = [800.0, 390.0];
const AXIS_HIT_WIDTH: f64 = 12.0;
const BRUSH_WIDTH: f32 = 13.0;

const DIMENSIONS: [ParallelBrushDimension; 5] = [
    ParallelBrushDimension {
        id: "speed",
        field: "speed",
        title: "Speed",
    },
    ParallelBrushDimension {
        id: "efficiency",
        field: "efficiency",
        title: "Efficiency",
    },
    ParallelBrushDimension {
        id: "stability",
        field: "stability",
        title: "Stability",
    },
    ParallelBrushDimension {
        id: "cost",
        field: "cost",
        title: "Cost",
    },
    ParallelBrushDimension {
        id: "quality",
        field: "quality",
        title: "Quality",
    },
];

#[derive(Clone, Copy)]
struct ParallelBrushDimension {
    id: &'static str,
    field: &'static str,
    title: &'static str,
}

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart parallel axis brush selection")
            .with_inner_size(LogicalSize::new(
                f64::from(CANVAS_SIZE[0]),
                f64::from(CANVAS_SIZE[1]),
            ))
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let brush = Selection::new("axis_brush")
        .combine(SelectionCombine::Intersect)
        .empty_selects_nothing();
    let selected = brush.predicate();

    let coord = DIMENSIONS.iter().fold(Parallel::new(), |coord, dimension| {
        coord.dimension_with(dimension.id, col(dimension.field), |d| {
            d.axis(|axis| axis.title(dimension.title))
        })
    });

    let mut plot = Plot::with_coord(coord)
        .canvas_size(CANVAS_SIZE[0], CANVAS_SIZE[1])
        .plot_size(PLOT_SIZE[0], PLOT_SIZE[1])
        .title("Drag an axis to brush; shift-drag to combine dimensions")
        .data(ctx.read_batch(source_batch()).expect("read data"))
        .add_selection(brush)
        .add_store(brush_store())
        .mark(
            ParallelLine::new()
                .id("context_lines")
                .stroke("#c4cbd5")
                .stroke_width(1.1)
                .opacity(0.42)
                .zindex(1),
        )
        .mark(
            ParallelLine::new()
                .id("selected_lines")
                .transform_no_output(Filter::new(selected), |mark| mark)
                .stroke_with(quality_color_expr(), |stroke| stroke.no_scale().no_legend())
                .stroke_width(2.35)
                .opacity(0.95)
                .zindex(20),
        )
        .mark(
            ParallelBrushOverlay::new(DIMENSIONS.len())
                .data_store(StoreData::new("axis_brush_boxes"))
                .width_px(BRUSH_WIDTH)
                .zindex(10_000),
        )
        .event_binding(clear_brush_binding());

    for (axis_index, dimension) in DIMENSIONS.iter().enumerate() {
        plot = plot
            .event_binding(axis_drag_binding(*dimension, axis_index, false))
            .event_binding(axis_drag_binding(*dimension, axis_index, true))
            .event_binding(axis_release_binding(*dimension, axis_index, false))
            .event_binding(axis_release_binding(*dimension, axis_index, true));
    }

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app(
        compiled,
        ctx,
        ChartAppOptions {
            resize_binding: ChartResizeBinding::none(),
            resize_throttle_ms: None,
            exact_on_resize_settle: true,
            log_metrics: true,
        },
    )
    .await
    .expect("build chart app")
}

fn axis_drag_binding(
    dimension: ParallelBrushDimension,
    axis_index: usize,
    additive: bool,
) -> ChartEventBinding {
    let update = if additive {
        SelectionUpdate::upsert_clause(axis_selection_clause(dimension))
    } else {
        SelectionUpdate::replace_clause(axis_selection_clause(dimension))
    };
    let store_update = if additive {
        upsert_store_update(dimension, axis_index)
    } else {
        replace_store_update(dimension, axis_index)
    };
    let mut binding = ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .filter(axis_hit_filter(axis_index))
        .filter(dimension_drag_values_exist(dimension))
        .filter(ev::shift().eq(lit(additive)))
        .set_selection_at_start_scope("axis_brush", update);
    binding = if additive {
        binding.set_store_at_start_scope("axis_brush_boxes", store_update)
    } else {
        binding.set_store_at_start_scope_replacing_scopes("axis_brush_boxes", store_update)
    };
    binding.preview()
}

fn quality_color_expr() -> Expr {
    when(col("quality").gt(lit(72.0)), lit("#4f46e5"))
        .when(col("quality").gt(lit(66.0)), lit("#2563eb"))
        .when(col("quality").gt(lit(60.0)), lit("#0891b2"))
        .otherwise(lit("#16a34a"))
        .expect("valid quality color expression")
}

fn axis_release_binding(
    dimension: ParallelBrushDimension,
    axis_index: usize,
    additive: bool,
) -> ChartEventBinding {
    let update = if additive {
        SelectionUpdate::upsert_clause(axis_selection_clause(dimension))
    } else {
        SelectionUpdate::replace_clause(axis_selection_clause(dimension))
    };
    let store_update = if additive {
        upsert_store_update(dimension, axis_index)
    } else {
        replace_store_update(dimension, axis_index)
    };
    let mut binding = ChartEventBinding::on_between_end(
        ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
        ChartEventStream::on(ChartEventType::MouseUp),
    )
    .filter(axis_hit_filter(axis_index))
    .filter(dimension_drag_values_exist(dimension))
    .filter(ev::shift().eq(lit(additive)))
    .set_selection_at_start_scope("axis_brush", update);
    binding = if additive {
        binding.set_store_at_start_scope("axis_brush_boxes", store_update)
    } else {
        binding.set_store_at_start_scope_replacing_scopes("axis_brush_boxes", store_update)
    };
    binding.exact()
}

fn clear_brush_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .clear_selection("axis_brush")
        .set_store_replacing_scopes("axis_brush_boxes", StoreUpdate::clear())
        .exact()
}

fn dimension_drag_values_exist(dimension: ParallelBrushDimension) -> Expr {
    ev::start_coord(dimension.id)
        .is_not_null()
        .and(ev::event_at_start_clipped_coord(dimension.id).is_not_null())
        .and(ev::start_coord(PARALLEL_LOCAL_Y_CHANNEL).is_not_null())
        .and(ev::event_at_start_clipped_coord(PARALLEL_LOCAL_Y_CHANNEL).is_not_null())
}

fn axis_hit_filter(axis_index: usize) -> Expr {
    let axis_x = axis_local_x(axis_index);
    ev::start_coord(PARALLEL_LOCAL_X_CHANNEL)
        .gt(lit(axis_x - AXIS_HIT_WIDTH))
        .and(ev::start_coord(PARALLEL_LOCAL_X_CHANNEL).lt(lit(axis_x + AXIS_HIT_WIDTH)))
}

fn axis_local_x(axis_index: usize) -> f64 {
    if DIMENSIONS.len() <= 1 {
        f64::from(PLOT_SIZE[0]) / 2.0
    } else {
        axis_index as f64 * f64::from(PLOT_SIZE[0]) / (DIMENSIONS.len() - 1) as f64
    }
}

fn axis_selection_clause(dimension: ParallelBrushDimension) -> SelectionClauseUpdate {
    let interval = ev::interval_ordered(
        ev::start_coord(dimension.id),
        ev::event_at_start_clipped_coord(dimension.id),
    );
    SelectionClauseUpdate::interval(lit(dimension.id))
        .facet_scope(CoordinationScope::Shared)
        .dimension_named(dimension.id, col(dimension.field))
        .endpoints(
            ev::interval_start(interval.clone()),
            ev::interval_end(interval),
        )
        .build()
}

fn brush_store_row(dimension: ParallelBrushDimension, axis_index: usize) -> StoreRow {
    let y_interval = ev::interval_ordered(
        ev::start_coord(PARALLEL_LOCAL_Y_CHANNEL),
        ev::event_at_start_clipped_coord(PARALLEL_LOCAL_Y_CHANNEL),
    );
    StoreRow::new()
        .field("id", lit(dimension.id))
        .field("axis_index", lit(axis_index as f64))
        .field("y_min_px", ev::interval_start(y_interval.clone()))
        .field("y_max_px", ev::interval_end(y_interval))
}

fn replace_store_update(dimension: ParallelBrushDimension, axis_index: usize) -> StoreUpdate {
    StoreUpdate::replace_rows([brush_store_row(dimension, axis_index)])
}

fn upsert_store_update(dimension: ParallelBrushDimension, axis_index: usize) -> StoreUpdate {
    StoreUpdate::upsert_rows([brush_store_row(dimension, axis_index)])
}

fn brush_store() -> Store {
    Store::empty("axis_brush_boxes")
        .field("id", DataType::Utf8, false)
        .field("axis_index", DataType::Float64, false)
        .field("y_min_px", DataType::Float64, false)
        .field("y_max_px", DataType::Float64, false)
        .primary_key(["id"])
        .sharing(CoordinationScope::Shared)
}

fn source_batch() -> RecordBatch {
    let mut sample_id = Vec::new();
    let mut speed = Vec::new();
    let mut efficiency = Vec::new();
    let mut stability = Vec::new();
    let mut cost = Vec::new();
    let mut quality = Vec::new();

    for i in 0..90 {
        let t = i as f64;
        let group_wave = ((i % 9) as f64 - 4.0) * 0.9;
        let speed_value = 42.0 + (t * 0.41).sin() * 18.0 + group_wave;
        let efficiency_value = 0.58 + (t * 0.23).cos() * 0.18 - group_wave * 0.006;
        let stability_value = 72.0 + (t * 0.17).sin() * 16.0 + (t * 0.07).cos() * 7.0;
        let cost_value =
            110.0 + speed_value * 1.35 - stability_value * 0.42 + (t * 0.31).sin() * 8.0;
        let quality_value = stability_value * 0.54 + efficiency_value * 52.0 - cost_value * 0.08
            + speed_value * 0.12;

        sample_id.push(format!("s{i:03}"));
        speed.push(speed_value);
        efficiency.push(efficiency_value);
        stability.push(stability_value);
        cost.push(cost_value);
        quality.push(quality_value);
    }

    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("sample_id", DataType::Utf8, false),
            Field::new("speed", DataType::Float64, false),
            Field::new("efficiency", DataType::Float64, false),
            Field::new("stability", DataType::Float64, false),
            Field::new("cost", DataType::Float64, false),
            Field::new("quality", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(sample_id)) as ArrayRef,
            Arc::new(Float64Array::from(speed)),
            Arc::new(Float64Array::from(efficiency)),
            Arc::new(Float64Array::from(stability)),
            Arc::new(Float64Array::from(cost)),
            Arc::new(Float64Array::from(quality)),
        ],
    )
    .expect("parallel axis brush example data")
}

#[derive(Clone)]
struct ParallelBrushOverlay<C = Parallel> {
    state: MarkState,
    dimension_count: usize,
    width_px: f32,
    _phantom: PhantomData<C>,
}

impl ParallelBrushOverlay<Parallel> {
    fn new(dimension_count: usize) -> Self {
        Self {
            state: MarkState {
                id: None,
                data: DataContext::default(),
                data_mode: MarkDataMode::Inherit,
                facet_data_scope: avenger_chart_core::FacetDataScope::FILTERED,
                exclude_from_scale_domains: true,
                visible: None,
                details: None,
                zindex: None,
                axis_configs: Default::default(),
            },
            dimension_count,
            width_px: BRUSH_WIDTH,
            _phantom: PhantomData,
        }
    }

    fn data_store(mut self, data: StoreData) -> Self {
        self.state.data = DataContext::store_data(data);
        self
    }

    fn width_px(mut self, width_px: f32) -> Self {
        self.width_px = width_px;
        self
    }

    fn zindex(mut self, zindex: i32) -> Self {
        self.state.zindex = Some(zindex);
        self
    }
}

impl IntoPlotMark<Parallel> for ParallelBrushOverlay<Parallel> {
    fn into_plot_marks(self) -> Vec<PlotMark<Parallel>> {
        vec![PlotMark::from_mark(self)]
    }
}

#[async_trait::async_trait]
impl Mark<Parallel> for ParallelBrushOverlay<Parallel> {
    fn state(&self) -> &MarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    fn data_context(&self) -> &DataContext {
        &self.state.data
    }

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledParallelBrushOverlay {
            state: compiled_state,
            dimension_count: self.dimension_count,
            width_px: self.width_px,
        }))
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct CompiledParallelBrushOverlay {
    state: CompiledMarkState,
    dimension_count: usize,
    width_px: f32,
}

impl CompiledMarkCore for CompiledParallelBrushOverlay {
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
        "parallel_brush_overlay"
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        Vec::new()
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledParallelBrushOverlay {
    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Ok(self
            .render_mark_data(data, _scalars, context, _coord)
            .await?
            .marks)
    }

    async fn render_mark_data(
        &self,
        data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<RenderedMarkData, AvengerChartError> {
        let Some(data) = data else {
            return Ok(RenderedMarkData::new(Vec::new()));
        };
        if data.num_rows() == 0 {
            return Ok(RenderedMarkData::new(Vec::new()));
        }

        let mut xs = Vec::new();
        let mut x2s = Vec::new();
        let mut ys = Vec::new();
        let mut y2s = Vec::new();
        for row in 0..data.num_rows() {
            let Some(axis_index) = f64_column_value(data, "axis_index", row) else {
                continue;
            };
            let Some(y_min) = f64_column_value(data, "y_min_px", row) else {
                continue;
            };
            let Some(y_max) = f64_column_value(data, "y_max_px", row) else {
                continue;
            };
            let axis_index = axis_index.round().max(0.0) as usize;
            if axis_index >= self.dimension_count {
                continue;
            }
            let x = local_axis_x(axis_index, self.dimension_count, context.plot_width());
            xs.push(x - self.width_px / 2.0);
            x2s.push(x + self.width_px / 2.0);
            ys.push(y_min as f32);
            y2s.push(y_max as f32);
        }

        if xs.is_empty() {
            return Ok(RenderedMarkData::new(Vec::new()));
        }

        Ok(RenderedMarkData::new(vec![SceneMark::Rect(
            SceneRectMark {
                name: "parallel_axis_brush".to_string(),
                interactive: false,
                clip: true,
                len: xs.len() as u32,
                gradients: Vec::new(),
                x: ScalarOrArray::from(xs),
                y: ScalarOrArray::from(ys),
                width: None,
                height: None,
                x2: Some(ScalarOrArray::from(x2s)),
                y2: Some(ScalarOrArray::from(y2s)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([
                    0.145, 0.388, 0.922, 0.14,
                ])),
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([
                    0.145, 0.388, 0.922, 0.85,
                ])),
                stroke_width: ScalarOrArray::new_scalar(1.4),
                corner_radius: ScalarOrArray::new_scalar(0.0),
                indices: None,
                zindex: self.state.zindex,
            },
        )]))
    }
}

fn local_axis_x(axis_index: usize, dimension_count: usize, plot_width: f32) -> f32 {
    if dimension_count <= 1 {
        plot_width / 2.0
    } else {
        axis_index as f32 * plot_width / (dimension_count - 1) as f32
    }
}

fn f64_column_value(batch: &RecordBatch, column_name: &str, row: usize) -> Option<f64> {
    let array = batch.column_by_name(column_name)?;
    match ScalarValue::try_from_array(array.as_ref(), row).ok()? {
        ScalarValue::Float64(value) => value,
        ScalarValue::Float32(value) => value.map(f64::from),
        ScalarValue::Int64(value) => value.map(|value| value as f64),
        ScalarValue::Int32(value) => value.map(f64::from),
        ScalarValue::UInt64(value) => value.map(|value| value as f64),
        ScalarValue::UInt32(value) => value.map(f64::from),
        _ => None,
    }
}

fn init_diagnostics() {
    if std::env::var_os("RUST_LOG").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
            .try_init();
    } else {
        let _ = env_logger::try_init();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parallel_axis_brush_selection_app_builds() {
        let _ = build_app().await;
    }
}
