//! Manual rectangular selection using rendered scene geometry queries.
//!
//! Drag inside the plot to define a screen-space rectangle. The event binding
//! queries rendered symbol anchors inside that rectangle, collects their
//! `point_id` datum values, and replaces a semantic selection with equality
//! clauses. Double-click clears the selection.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example cartesian_rect_scene_query_selection --features winit-wgpu --release
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
    prelude::{SessionContext, col, lit, when},
};
use winit::window::WindowAttributes;

const PLOT_WIDTH: f64 = 620.0;
const PLOT_HEIGHT: f64 = 420.0;
const X_DOMAIN_MIN: f64 = -1.0;
const X_DOMAIN_MAX: f64 = 28.0;
const Y_DOMAIN_MIN: f64 = -3.0;
const Y_DOMAIN_MAX: f64 = 25.0;

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart scene-query rect selection")
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let df = ctx.read_batch(make_points_batch()).expect("read points");

    let picked = Selection::new("picked").empty_selects_nothing();
    let selected = picked.predicate();
    let overlay = Rect::<Cartesian>::new()
        .data_store(StoreData::new("selection_rect"))
        .x_with(col("x_min"), |c| c.exclude_from_scale_domain())
        .x2_with(col("x_max"), |c| c.exclude_from_scale_domain())
        .y_with(col("y_min"), |c| c.exclude_from_scale_domain())
        .y2_with(col("y_max"), |c| c.exclude_from_scale_domain())
        .fill("rgba(37, 99, 235, 0.08)")
        .stroke("#2563eb")
        .stroke_width(1.5)
        .zindex(10_000);

    let plot = Chart::<Cartesian>::new()
        .plot_size(PLOT_WIDTH, PLOT_HEIGHT)
        .data(df)
        .selection(picked)
        .store(rect_overlay_store())
        .mark(
            Symbol::new()
                .id("points")
                .x_with(col("source_x"), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(X_DOMAIN_MIN), lit(X_DOMAIN_MAX)))
                            .nice(false)
                            .zero(false)
                    })
                })
                .y_with(col("source_y"), |c| {
                    c.scale_with::<Linear>(|s| {
                        s.domain((lit(Y_DOMAIN_MIN), lit(Y_DOMAIN_MAX)))
                            .nice(false)
                            .zero(false)
                    })
                })
                .fill_with(lit("#b8beca"), |c| {
                    c.no_scale()
                        .when_value(selected, lit("#2563eb"))
                        .no_legend()
                })
                .stroke("#ffffff")
                .stroke_width(0.7)
                .size(72.0),
        )
        .mark(overlay)
        .event_binding(cursor_binding())
        .event_binding(rect_drag_binding())
        .event_binding(rect_release_binding())
        .event_binding(rect_clear_binding());

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

fn cursor_binding() -> ChartEventBinding {
    let over_plot = ev::event_coord("x")
        .is_not_null()
        .and(ev::event_coord("y").is_not_null());
    let cursor_expr = when(over_plot, ev::cursor(CursorStyle::Crosshair))
        .otherwise(ev::cursor(CursorStyle::Default))
        .expect("valid cursor expression");
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .set_cursor(cursor_expr)
        .preview()
}

fn rect_drag_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .filter(ev::start_coord("x").is_not_null())
        .filter(ev::start_coord("y").is_not_null())
        .filter(ev::x().is_not_null())
        .filter(ev::y().is_not_null())
        .set_cursor(ev::cursor(CursorStyle::Grabbing))
        .set_store_at_start_scope_replacing_scopes("selection_rect", rect_overlay_update())
        .set_selection_at_start_scope(
            "picked",
            SelectionUpdate::replace_all_from_scene_query(rect_query_update()),
        )
        .preview()
}

fn rect_release_binding() -> ChartEventBinding {
    ChartEventBinding::on_between_end(
        ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
        ChartEventStream::on(ChartEventType::MouseUp),
    )
    .filter(ev::start_coord("x").is_not_null())
    .filter(ev::start_coord("y").is_not_null())
    .filter(ev::x().is_not_null())
    .filter(ev::y().is_not_null())
    .set_store_at_start_scope_replacing_scopes("selection_rect", rect_overlay_update())
    .set_selection_at_start_scope(
        "picked",
        SelectionUpdate::replace_all_from_scene_query(rect_query_update()),
    )
    .exact()
}

fn rect_clear_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .set_store_replacing_scopes("selection_rect", StoreUpdate::clear())
        .clear_selection("picked")
        .exact()
}

fn rect_query_update() -> SelectionSceneQuery {
    SelectionSceneQuery::new(
        SceneGeometryQuery::rect(ev::start_x(), ev::start_y(), ev::x(), ev::y())
            .hit_policy(SceneGeometryHitPolicy::AnchorInside)
            .mark("points")
            .datum_field(
                SceneQueryDatumField::new("point_id")
                    .datum("point_id")
                    .field_expr(col("point_id")),
            )
            .unique_by(["point_id"]),
    )
    .sharing(CoordinationScope::Shared)
}

fn rect_overlay_store() -> Store {
    Store::empty("selection_rect")
        .field("id", DataType::Utf8, false)
        .field("x_min", DataType::Float64, false)
        .field("x_max", DataType::Float64, false)
        .field("y_min", DataType::Float64, false)
        .field("y_max", DataType::Float64, false)
        .primary_key(["id"])
        .sharing(CoordinationScope::Shared)
}

fn rect_overlay_update() -> StoreUpdate {
    let x_interval =
        ev::interval_ordered(ev::start_coord("x"), ev::event_at_start_clipped_coord("x"));
    let y_interval =
        ev::interval_ordered(ev::start_coord("y"), ev::event_at_start_clipped_coord("y"));
    StoreUpdate::replace_rows([StoreRow::new()
        .field("id", lit("active"))
        .field("x_min", ev::interval_start(x_interval.clone()))
        .field("x_max", ev::interval_end(x_interval))
        .field("y_min", ev::interval_start(y_interval.clone()))
        .field("y_max", ev::interval_end(y_interval))])
}

fn make_points_batch() -> RecordBatch {
    let mut ids = Vec::new();
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for row in 0..22 {
        for col in 0..28 {
            let idx = row * 28 + col;
            let jitter_x = (((idx * 37 + 17) % 100) as f64 - 50.0) / 145.0;
            let jitter_y = (((idx * 61 + 23) % 100) as f64 - 50.0) / 145.0;
            ids.push(format!("p{idx:04}"));
            xs.push(col as f64 + jitter_x);
            let wave = (col as f64 / 3.8).sin() * 1.6 + (col as f64 / 8.0).cos() * 0.8;
            ys.push(row as f64 + wave + jitter_y);
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("point_id", DataType::Utf8, false),
        Field::new("source_x", DataType::Float64, false),
        Field::new("source_y", DataType::Float64, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(ids)),
            Arc::new(Float64Array::from(xs)),
            Arc::new(Float64Array::from(ys)),
        ],
    )
    .expect("build point batch")
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
