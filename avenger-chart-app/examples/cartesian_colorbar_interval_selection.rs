//! Scatter plot with colorbar interval selection.
//!
//! Drag on the continuous colorbar to select a temperature interval. Points
//! start grey; points whose `temperature` falls inside the selected interval
//! render through the continuous fill scale. The selected interval is drawn as
//! an ordinary `Rect<Cartesian>` overlay on top of the colorbar.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example cartesian_colorbar_interval_selection --features winit-wgpu --release
//! ```

use std::sync::Arc;

use avenger_chart::{event as ev, prelude::*};
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::{
    arrow::{
        array::{Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::{Expr, SessionContext, col, lit},
};
use winit::window::WindowAttributes;

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart colorbar interval selection")
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let df = ctx.read_batch(make_points_batch()).expect("read points");

    let temperature_selection = Selection::new("temperature_brush").empty_selects_nothing();
    let selected = temperature_selection.predicate();

    let colorbar_drag = ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .filter(ev::start_coord("y").is_not_null())
        .filter(ev::event_at_start_clipped_coord("y").is_not_null())
        .set_selection(
            "temperature_brush",
            SelectionUpdate::replace_clause(temperature_clause()),
        )
        .set_store_replacing_scopes("temperature_interval", temperature_overlay_update())
        .preview();

    let colorbar_release = ChartEventBinding::on_between_end(
        ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
        ChartEventStream::on(ChartEventType::MouseUp),
    )
    .filter(ev::start_coord("y").is_not_null())
    .filter(ev::event_at_start_clipped_coord("y").is_not_null())
    .set_selection(
        "temperature_brush",
        SelectionUpdate::replace_clause(temperature_clause()),
    )
    .set_store_replacing_scopes("temperature_interval", temperature_overlay_update())
    .exact();

    let colorbar_clear = ChartEventBinding::on(ChartEventType::DoubleClick)
        .set_selection("temperature_brush", SelectionUpdate::clear())
        .set_store_replacing_scopes("temperature_interval", StoreUpdate::clear())
        .exact();

    let colorbar_overlay = ColorbarOverlay::new().mark(
        Rect::<Cartesian>::new()
            .data_store(StoreData::new("temperature_interval"))
            .exclude_from_scale_domains()
            .x(lit(0.0))
            .x2(lit(1.0))
            .y(col("temperature_min"))
            .y2(col("temperature_max"))
            .fill("rgba(255, 255, 255, 0.35)")
            .stroke("#000000")
            .stroke_width(1.75)
            .zindex(10_000),
    );

    let plot = Plot::<Cartesian>::new()
        .canvas_size(860.0, 560.0)
        .title("Colorbar interval selection")
        .data(df)
        .add_selection(temperature_selection)
        .add_store(temperature_interval_store())
        .mark(
            Symbol::new()
                .id("all_points")
                .x(col("source_x"))
                .y(col("source_y"))
                .fill("#aeb6c4")
                .stroke("#ffffff")
                .stroke_width(0.6)
                .size(96.0),
        )
        .mark(
            Symbol::new()
                .id("selected_points")
                .x(col("source_x"))
                .y(col("source_y"))
                .fill_with(col("temperature"), |c| {
                    c.legend(|l| {
                        l.title("Temperature")
                            .event_binding(colorbar_drag)
                            .event_binding(colorbar_release)
                            .event_binding(colorbar_clear)
                            .colorbar_overlay(colorbar_overlay)
                    })
                })
                .opacity_with(lit(0.0), |c| {
                    c.no_scale().when_value(selected, lit(1.0)).no_legend()
                })
                .stroke("#ffffff")
                .stroke_width(0.8)
                .size(112.0),
        );

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

fn temperature_clause() -> SelectionClauseUpdate {
    let interval = temperature_interval();
    SelectionClauseUpdate::interval(lit("active"))
        .facet_scope(Sharing::Shared)
        .dimension(col("temperature"))
        .endpoints(
            ev::interval_start(interval.clone()),
            ev::interval_end(interval),
        )
        .build()
}

fn temperature_overlay_update() -> StoreUpdate {
    let interval = temperature_interval();
    StoreUpdate::replace_rows([StoreRow::new()
        .field("id", lit("active"))
        .field("temperature_min", ev::interval_start(interval.clone()))
        .field("temperature_max", ev::interval_end(interval))])
}

fn temperature_interval() -> Expr {
    ev::interval_ordered(ev::start_coord("y"), ev::event_at_start_clipped_coord("y"))
}

fn temperature_interval_store() -> Store {
    Store::empty("temperature_interval")
        .field("id", DataType::Utf8, false)
        .field("temperature_min", DataType::Float64, false)
        .field("temperature_max", DataType::Float64, false)
        .primary_key(["id"])
        .sharing(Sharing::Shared)
}

fn make_points_batch() -> RecordBatch {
    let mut labels = Vec::new();
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut temperatures = Vec::new();
    for row in 0..12 {
        for col_index in 0..14 {
            let idx = row * 14 + col_index;
            let wave = ((idx as f64 * 0.73).sin() + (col_index as f64 * 0.31).cos()) * 5.0;
            let source_x = col_index as f64 + (row as f64 * 0.17).sin() * 0.22;
            let source_y = row as f64 + (col_index as f64 * 0.41).cos() * 0.24;
            let temperature = 18.0 + row as f64 * 2.6 + col_index as f64 * 1.15 + wave;
            labels.push(format!("p{idx:03}"));
            xs.push(source_x);
            ys.push(source_y);
            temperatures.push(temperature);
        }
    }
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("point_id", DataType::Utf8, false),
            Field::new("source_x", DataType::Float64, false),
            Field::new("source_y", DataType::Float64, false),
            Field::new("temperature", DataType::Float64, false),
        ])),
        vec![
            Arc::new(StringArray::from(labels)),
            Arc::new(Float64Array::from(xs)),
            Arc::new(Float64Array::from(ys)),
            Arc::new(Float64Array::from(temperatures)),
        ],
    )
    .expect("points batch")
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
