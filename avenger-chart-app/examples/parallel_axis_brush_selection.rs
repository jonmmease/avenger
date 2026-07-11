// Parallel coordinates with axis brushing.
//
// Drag near a vertical axis to brush a value interval on that dimension.
// Shift-drag another axis to add another brush; selected rows are the
// intersection of active dimension intervals. Selected lines render above the
// grey context lines and use a quality color scale.
//
// Run with:
// ```bash
// cargo run --release -p avenger-chart-app --example parallel_axis_brush_selection --features winit-wgpu
// ```

use std::sync::Arc;

use avenger_chart::{event as ev, prelude::*};
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::{Expr, SessionContext, col, lit},
};
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

    let coord = parallel_coord();

    let mut plot = Chart::with_coord(coord)
        .canvas_size(CANVAS_SIZE[0], CANVAS_SIZE[1])
        .plot_size(PLOT_SIZE[0], PLOT_SIZE[1])
        .title("Drag an axis to brush; shift-drag to combine dimensions")
        .data(ctx.read_batch(source_batch()).expect("read data"))
        .selection(brush)
        .store(brush_store())
        // Seed the selected-line color scale from all rows, even when the
        // current selection is empty.
        .mark(
            parallel_line()
                .id("quality_scale_seed")
                .stroke_with(col("quality"), |stroke| stroke.no_legend())
                .stroke_width(0.0)
                .opacity(0.0)
                .zindex(0),
        )
        .mark(
            parallel_line()
                .id("context_lines")
                .stroke("#c4cbd5")
                .stroke_width(1.1)
                .opacity(0.42)
                .zindex(1),
        )
        .mark(
            parallel_line()
                .id("selected_lines")
                .transform_no_output(Filter::new(selected), |mark| mark)
                .stroke_with(col("quality"), |stroke| stroke.no_legend())
                .stroke_width(2.35)
                .opacity(0.95)
                .zindex(20),
        )
        .event_binding(clear_brush_binding());

    for (axis_index, dimension) in DIMENSIONS.iter().enumerate() {
        plot = plot
            .mark(axis_brush_overlay(*dimension))
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

fn parallel_coord() -> Parallel {
    DIMENSIONS.iter().fold(Parallel::new(), |coord, dimension| {
        coord.dimension_with(dimension.id, |d| d.axis(|axis| axis.title(dimension.title)))
    })
}

fn parallel_line() -> ParallelLine {
    DIMENSIONS
        .iter()
        .fold(ParallelLine::new(), |mark, dimension| {
            mark.dimension(dimension.id, col(dimension.field))
        })
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
    let value_interval = ev::interval_ordered(
        ev::start_coord(dimension.id),
        ev::event_at_start_clipped_coord(dimension.id),
    );
    StoreRow::new()
        .field("id", lit(dimension.id))
        .field("axis_index", lit(axis_index as f64))
        .field("value_min", ev::interval_start(value_interval.clone()))
        .field("value_max", ev::interval_end(value_interval))
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
        .field("value_min", DataType::Float64, false)
        .field("value_max", DataType::Float64, false)
        .primary_key(["id"])
        .sharing(CoordinationScope::Shared)
}

fn axis_brush_overlay(dimension: ParallelBrushDimension) -> ParallelAxisOverlay<Parallel> {
    let brush_rect = Rect::<Cartesian>::new()
        .data_store(StoreData::new("axis_brush_boxes"))
        .transform_no_output(Filter::new(col("id").eq(lit(dimension.id))), |mark| mark)
        .exclude_from_scale_domains()
        .x(lit(0.0))
        .x2(lit(1.0))
        .y(col("value_min"))
        .y2(col("value_max"))
        .fill("rgba(37, 99, 235, 0.14)")
        .stroke("#2563eb")
        .stroke_width(1.4)
        .zindex(10_000);

    ParallelAxisOverlay::new(dimension.id, Plot::<Cartesian>::new().mark(brush_rect))
        .width_px(BRUSH_WIDTH)
        .zindex(10_000)
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
