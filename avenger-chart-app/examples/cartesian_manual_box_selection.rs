//! Manual Cartesian box selection prototype.
//!
//! This example intentionally uses the public low-level pieces directly:
//! `Selection`, cursor params, ordinary event bindings, and a unit `Rect`
//! overlay mark. The bundled `BoxSelection` tool will be a convenience wrapper
//! over this shape.

use std::sync::Arc;

use avenger_chart::event as ev;
use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::{
    arrow::datatypes::DataType,
    prelude::{Expr, SessionContext, lit, when},
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
            .with_title("avenger-chart manual box selection")
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
                (0.8, 1.2, 'A'), (1.6, 2.0, 'A'), (2.5, 1.8, 'A'),
                (3.4, 3.8, 'B'), (4.2, 4.5, 'B'), (5.1, 3.7, 'B'),
                (6.0, 6.4, 'C'), (6.8, 5.4, 'C'), (7.7, 6.1, 'C'),
                (8.5, 7.5, 'D'), (9.2, 8.4, 'D'), (9.8, 7.8, 'D')
            ) AS t(source_a, source_b, group_name)",
        )
        .await
        .expect("build data");

    let brush = Selection::new("brush").empty_selects_nothing();
    let selected = brush.predicate();

    let cursor = Param::cursor("brush_cursor", CursorStyle::Default);

    let overlay = Rect::<Cartesian>::new()
        .data_store(StoreData::new("brush_boxes"))
        .exclude_from_scale_domains()
        .x(col("box_left"))
        .x2(col("box_right"))
        .y(col("box_bottom"))
        .y2(col("box_top"))
        .fill("rgba(37, 99, 235, 0.08)")
        .stroke("#2563eb")
        .stroke_width(1.5)
        .zindex(10_000);

    let plot = Plot::<Cartesian>::new()
        .canvas_size(760.0, 520.0)
        .data(df)
        .add_selection(brush)
        .add_store(brush_box_store(Sharing::Shared))
        .add_param(cursor.clone())
        .cursor_param(cursor.name.clone())
        .mark(
            Symbol::new()
                .x(col("source_a"))
                .y(col("source_b"))
                .fill_with(lit("#b8beca"), |c| {
                    c.no_scale()
                        .when_value(selected, lit("#2563eb"))
                        .no_legend()
                })
                .stroke("#ffffff")
                .stroke_width(0.75)
                .size(180.0),
        )
        .mark(overlay)
        .event_binding(cursor_binding(&cursor))
        .event_binding(selection_drag_binding(&cursor))
        .event_binding(selection_add_drag_binding(&cursor))
        .event_binding(selection_release_binding())
        .event_binding(selection_add_release_binding())
        .event_binding(selection_clear_binding());

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

fn cursor_binding(cursor: &Param) -> ChartEventBinding {
    let over_plot = ev::event_coord("x")
        .is_not_null()
        .and(ev::event_coord("y").is_not_null());
    let cursor_expr = when(over_plot, ev::cursor(CursorStyle::Crosshair))
        .otherwise(ev::cursor(CursorStyle::Default))
        .expect("valid cursor case expression");
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .set_param(cursor, cursor_expr)
        .preview()
}

fn selection_drag_binding(cursor: &Param) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .filter(ev::start_coord("x").is_not_null())
        .filter(ev::start_coord("y").is_not_null())
        .filter(ev::event_at_start_clipped_coord("x").is_not_null())
        .filter(ev::event_at_start_clipped_coord("y").is_not_null())
        .filter(ev::shift().eq(lit(false)))
        .set_param(cursor, ev::cursor(CursorStyle::Grabbing))
        .set_selection_at_start_scope("brush", replace_selection_update(Sharing::Shared))
        .set_store_at_start_scope_replacing_scopes("brush_boxes", replace_store_update())
        .preview()
}

fn selection_add_drag_binding(cursor: &Param) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .filter(ev::start_coord("x").is_not_null())
        .filter(ev::start_coord("y").is_not_null())
        .filter(ev::event_at_start_clipped_coord("x").is_not_null())
        .filter(ev::event_at_start_clipped_coord("y").is_not_null())
        .filter(ev::shift().eq(lit(true)))
        .set_param(cursor, ev::cursor(CursorStyle::Grabbing))
        .set_selection_at_start_scope("brush", upsert_selection_update(Sharing::Shared))
        .set_store_at_start_scope("brush_boxes", upsert_store_update())
        .preview()
}

fn selection_release_binding() -> ChartEventBinding {
    ChartEventBinding::on_between_end(
        ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
        ChartEventStream::on(ChartEventType::MouseUp),
    )
    .filter(ev::start_coord("x").is_not_null())
    .filter(ev::start_coord("y").is_not_null())
    .filter(ev::event_at_start_clipped_coord("x").is_not_null())
    .filter(ev::event_at_start_clipped_coord("y").is_not_null())
    .filter(ev::shift().eq(lit(false)))
    .set_selection_at_start_scope("brush", replace_selection_update(Sharing::Shared))
    .set_store_at_start_scope_replacing_scopes("brush_boxes", replace_store_update())
    .exact()
}

fn selection_add_release_binding() -> ChartEventBinding {
    ChartEventBinding::on_between_end(
        ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
        ChartEventStream::on(ChartEventType::MouseUp),
    )
    .filter(ev::start_coord("x").is_not_null())
    .filter(ev::start_coord("y").is_not_null())
    .filter(ev::event_at_start_clipped_coord("x").is_not_null())
    .filter(ev::event_at_start_clipped_coord("y").is_not_null())
    .filter(ev::shift().eq(lit(true)))
    .set_selection_at_start_scope("brush", upsert_selection_update(Sharing::Shared))
    .set_store_at_start_scope("brush_boxes", upsert_store_update())
    .exact()
}

fn selection_clear_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .clear_selection("brush")
        .set_store_replacing_scopes("brush_boxes", StoreUpdate::clear())
        .exact()
}

fn brush_box_store(sharing: Sharing) -> Store {
    Store::empty("brush_boxes")
        .field("id", DataType::Utf8, false)
        .field("box_left", DataType::Float64, false)
        .field("box_right", DataType::Float64, false)
        .field("box_bottom", DataType::Float64, false)
        .field("box_top", DataType::Float64, false)
        .primary_key(["id"])
        .sharing(sharing)
}

fn brush_selection_clause(id: Expr, facet_scope: Sharing) -> SelectionClauseUpdate {
    SelectionClauseUpdate::interval(id)
        .facet_scope(facet_scope)
        .dimension(col("source_a"))
        .endpoints(
            ev::interval_start(ev::interval_ordered(
                ev::start_coord("x"),
                ev::event_at_start_clipped_coord("x"),
            )),
            ev::interval_end(ev::interval_ordered(
                ev::start_coord("x"),
                ev::event_at_start_clipped_coord("x"),
            )),
        )
        .dimension(col("source_b"))
        .endpoints(
            ev::interval_start(ev::interval_ordered(
                ev::start_coord("y"),
                ev::event_at_start_clipped_coord("y"),
            )),
            ev::interval_end(ev::interval_ordered(
                ev::start_coord("y"),
                ev::event_at_start_clipped_coord("y"),
            )),
        )
        .build()
}

fn replace_selection_update(facet_scope: Sharing) -> SelectionUpdate {
    SelectionUpdate::replace_all_clauses([brush_selection_clause(lit("active"), facet_scope)])
}

fn upsert_selection_update(facet_scope: Sharing) -> SelectionUpdate {
    SelectionUpdate::upsert_clauses([brush_selection_clause(ev::start_event_id(), facet_scope)])
}

fn brush_box_row(id: Expr) -> StoreRow {
    let x_interval =
        ev::interval_ordered(ev::start_coord("x"), ev::event_at_start_clipped_coord("x"));
    let y_interval =
        ev::interval_ordered(ev::start_coord("y"), ev::event_at_start_clipped_coord("y"));
    StoreRow::new()
        .field("id", id)
        .field("box_left", ev::interval_start(x_interval.clone()))
        .field("box_right", ev::interval_end(x_interval))
        .field("box_bottom", ev::interval_start(y_interval.clone()))
        .field("box_top", ev::interval_end(y_interval))
}

fn replace_store_update() -> StoreUpdate {
    StoreUpdate::replace_rows([brush_box_row(lit("active"))])
}

fn upsert_store_update() -> StoreUpdate {
    StoreUpdate::upsert_rows([brush_box_row(ev::start_event_id())])
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
