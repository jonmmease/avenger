//! Lasso cross-highlighting through rendered scene geometry queries.
//!
//! Drag a lasso in the left plot. The binding queries rendered symbols inside
//! the lasso, collects their `point_id` values, and updates a semantic
//! selection. Both the lasso source plot and the sibling plot use the same
//! selection predicate for highlighting.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example concat_lasso_scene_query_cross_highlight --features winit-wgpu --release
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
    prelude::{Expr, SessionContext, col, lit, when},
};
use winit::window::WindowAttributes;

const LEFT_WIDTH: f64 = 470.0;
const RIGHT_WIDTH: f64 = 470.0;
const PLOT_HEIGHT: f64 = 360.0;

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart scene-query lasso cross-highlight")
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
    let cursor = Param::cursor("lasso_cursor", CursorStyle::Default);

    let source = Plot::<Cartesian>::new()
        .data(df.clone())
        .mark(selection_points(
            "source_points",
            col("source_x"),
            col("source_y"),
            selected.clone(),
            "#2563eb",
        ))
        .mark(lasso_overlay("source_lasso", "#2563eb"))
        .event_binding(cursor_binding(&cursor))
        .event_binding(lasso_drag_binding(&cursor))
        .event_binding(lasso_clear_binding());

    let sibling = Plot::<Cartesian>::new().data(df).mark(selection_points(
        "sibling_points",
        col("sibling_x"),
        col("sibling_y"),
        selected,
        "#d97706",
    ));

    let plot = Chart::<HConcat>::new()
        .canvas_size(1080.0, 480.0)
        .selection(picked)
        .store(lasso_overlay_store(
            "source_lasso",
            CoordinationScope::Shared,
        ))
        .param(cursor.clone())
        .cursor_param(cursor.name.clone())
        .mark(
            Subplot::new(source)
                .caption("Lasso source")
                .size(LEFT_WIDTH, PLOT_HEIGHT)
                .id("source")
                .name("source")
                .label("Query rendered marks"),
        )
        .mark(
            Subplot::new(sibling)
                .caption("Sibling view")
                .size(RIGHT_WIDTH, PLOT_HEIGHT)
                .id("sibling")
                .name("sibling")
                .label("Same semantic selection"),
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

fn selection_points(
    mark_id: &str,
    x: Expr,
    y: Expr,
    selected: Expr,
    selected_fill: &str,
) -> Symbol<Cartesian> {
    Symbol::new()
        .id(mark_id)
        .x(x)
        .y(y)
        .fill_with(lit("#b8beca"), |c| {
            c.no_scale()
                .when_value(selected, lit(selected_fill))
                .no_legend()
        })
        .stroke("#ffffff")
        .stroke_width(0.75)
        .size(92.0)
}

fn cursor_binding(cursor: &Param) -> ChartEventBinding {
    let over_plot = ev::event_coord("x")
        .is_not_null()
        .and(ev::event_coord("y").is_not_null());
    let cursor_expr = when(over_plot, ev::cursor(CursorStyle::Crosshair))
        .otherwise(ev::cursor(CursorStyle::Default))
        .expect("valid cursor expression");
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .set_param(cursor, cursor_expr)
        .preview()
}

fn lasso_drag_binding(cursor: &Param) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .filter(ev::start_coord("x").is_not_null())
        .filter(ev::start_coord("y").is_not_null())
        .filter(ev::event_path_svg().is_not_null())
        .set_param(cursor, ev::cursor(CursorStyle::Grabbing))
        .set_store_at_start_scope_replacing_scopes("source_lasso", lasso_overlay_update("active"))
        .set_selection_at_start_scope(
            "picked",
            SelectionUpdate::replace_all_from_scene_query(lasso_query_update()),
        )
        .event_path_min_distance_px(6.0)
        .preview()
        .settle_exact()
}

fn lasso_clear_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .set_store_replacing_scopes("source_lasso", StoreUpdate::clear())
        .clear_selection("picked")
        .exact()
}

fn lasso_query_update() -> SelectionSceneQuery {
    SelectionSceneQuery::new(
        SceneGeometryQuery::polygon(ev::event_path())
            .hit_policy(SceneGeometryHitPolicy::AnchorInside)
            .mark("source_points")
            .within_subplot("source")
            .datum_field(
                SceneQueryDatumField::new("point_id")
                    .datum("point_id")
                    .field_expr(col("point_id")),
            )
            .unique_by(["point_id"]),
    )
    .sharing(CoordinationScope::Shared)
}

fn lasso_overlay(store: &str, stroke: &str) -> PathMark<Cartesian> {
    PathMark::<Cartesian>::new()
        .data_store(StoreData::new(store))
        .exclude_from_scale_domains()
        .x(col("anchor_x"))
        .y(col("anchor_y"))
        .path_with(col("path"), |c| c.no_scale())
        .fill("rgba(37, 99, 235, 0.08)")
        .stroke(stroke)
        .stroke_width(1.5)
        .zindex(10_000)
}

fn lasso_overlay_store(name: &str, sharing: CoordinationScope) -> Store {
    Store::empty(name)
        .field("id", DataType::Utf8, false)
        .field("anchor_x", DataType::Float64, false)
        .field("anchor_y", DataType::Float64, false)
        .field("path", DataType::Utf8, false)
        .primary_key(["id"])
        .sharing(sharing)
}

fn lasso_overlay_update(id: &str) -> StoreUpdate {
    StoreUpdate::replace_rows([StoreRow::new()
        .field("id", lit(id.to_string()))
        .field("anchor_x", ev::start_coord("x"))
        .field("anchor_y", ev::start_coord("y"))
        .field("path", ev::event_path_svg())])
}

fn make_points_batch() -> RecordBatch {
    let mut ids = Vec::new();
    let mut source_x = Vec::new();
    let mut source_y = Vec::new();
    let mut sibling_x = Vec::new();
    let mut sibling_y = Vec::new();
    for index in 0..42 {
        ids.push(format!("p{index:03}"));
        let column = index % 7;
        let row = index / 7;
        let wave = ((index as f64) * 0.63).sin();
        source_x.push(column as f64 + 0.22 * wave);
        source_y.push(row as f64 + 0.34 * ((index as f64) * 0.41).cos());
        sibling_x.push(row as f64 + 0.3 * ((index as f64) * 0.29).sin());
        sibling_y.push(column as f64 + 0.35 * ((index as f64) * 0.51).cos());
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("point_id", DataType::Utf8, false),
        Field::new("source_x", DataType::Float64, false),
        Field::new("source_y", DataType::Float64, false),
        Field::new("sibling_x", DataType::Float64, false),
        Field::new("sibling_y", DataType::Float64, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(ids)),
            Arc::new(Float64Array::from(source_x)),
            Arc::new(Float64Array::from(source_y)),
            Arc::new(Float64Array::from(sibling_x)),
            Arc::new(Float64Array::from(sibling_y)),
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
