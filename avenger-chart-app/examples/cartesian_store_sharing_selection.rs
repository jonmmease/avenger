//! Compare store sharing levels for box-selection chrome.
//!
//! Both faceted views receive the same drag updates. The left view reads a
//! free-scoped store, so each facet owns its own boxes. The right view reads a
//! shared store, so the same boxes appear in every facet.

use std::sync::Arc;

use avenger_chart::event as ev;
use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::{
    arrow::datatypes::DataType,
    prelude::{Expr, SessionContext, col, lit, when},
};
use winit::window::WindowAttributes;

const FREE_STORE: &str = "free_brush_boxes";
const SHARED_STORE: &str = "shared_brush_boxes";

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart store sharing selection")
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
                ('Alpha', 0.8, 1.2), ('Alpha', 1.6, 2.0), ('Alpha', 2.4, 1.8), ('Alpha', 3.2, 2.8),
                ('Beta',  1.0, 3.8), ('Beta',  1.8, 4.6), ('Beta',  2.6, 3.9), ('Beta',  3.4, 5.0),
                ('Gamma', 0.9, 6.2), ('Gamma', 1.7, 5.5), ('Gamma', 2.5, 6.8), ('Gamma', 3.3, 6.0)
            ) AS t(group_name, x, y)",
        )
        .await
        .expect("build data");

    let free_brush = Selection::new("free_brush")
        .empty_selects_nothing()
        .facet_context_field("group_name", col("group_name"));
    let free_selected = free_brush.predicate();
    let shared_brush = Selection::new("shared_brush").empty_selects_nothing();
    let shared_selected = shared_brush.predicate();
    let cursor = Param::cursor("brush_cursor", CursorStyle::Default);

    let free_leaf = Plot::<Cartesian>::new()
        .mark(selection_points(free_selected, "#2563eb", 92.0))
        .mark(selection_overlay(StoreData::new(FREE_STORE), "#2563eb"));
    let free_facets = Plot::<FacetColumn>::new()
        .data(df.clone())
        .mark(
            Subplot::new(free_leaf)
                .column(col("group_name"))
                .label("Store sharing: Free"),
        )
        .event_binding(selection_drag_binding(
            &cursor,
            FREE_STORE,
            "free_brush",
            CoordinationScope::Free,
        ))
        .event_binding(selection_add_drag_binding(
            &cursor,
            FREE_STORE,
            "free_brush",
            CoordinationScope::Free,
        ))
        .event_binding(selection_release_binding(
            FREE_STORE,
            "free_brush",
            CoordinationScope::Free,
        ))
        .event_binding(selection_add_release_binding(
            FREE_STORE,
            "free_brush",
            CoordinationScope::Free,
        ))
        .event_binding(selection_clear_binding(FREE_STORE, "free_brush"));

    let shared_leaf = Plot::<Cartesian>::new()
        .mark(selection_points(shared_selected, "#d97706", 92.0))
        .mark(selection_overlay(StoreData::new(SHARED_STORE), "#d97706"));
    let shared_facets = Plot::<FacetColumn>::new()
        .data(df)
        .mark(
            Subplot::new(shared_leaf)
                .column(col("group_name"))
                .label("Store sharing: Shared"),
        )
        .event_binding(selection_drag_binding(
            &cursor,
            SHARED_STORE,
            "shared_brush",
            CoordinationScope::Shared,
        ))
        .event_binding(selection_add_drag_binding(
            &cursor,
            SHARED_STORE,
            "shared_brush",
            CoordinationScope::Shared,
        ))
        .event_binding(selection_release_binding(
            SHARED_STORE,
            "shared_brush",
            CoordinationScope::Shared,
        ))
        .event_binding(selection_add_release_binding(
            SHARED_STORE,
            "shared_brush",
            CoordinationScope::Shared,
        ))
        .event_binding(selection_clear_binding(SHARED_STORE, "shared_brush"));

    let plot = Chart::<HConcat>::new()
        .canvas_size(1440.0, 520.0)
        .store(brush_box_store(FREE_STORE, CoordinationScope::Free))
        .store(brush_box_store(SHARED_STORE, CoordinationScope::Shared))
        .selection(free_brush)
        .selection(shared_brush)
        .param(cursor.clone())
        .cursor_param(cursor.name.clone())
        .mark(Subplot::new(free_facets).name("free"))
        .mark(Subplot::new(shared_facets).name("shared"))
        .event_binding(cursor_binding(&cursor));

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

fn selection_points(selected: Expr, selected_fill: &str, size: f64) -> Symbol<Cartesian> {
    Symbol::new()
        .x(col("x"))
        .y(col("y"))
        .fill_with(lit("#b8beca"), |c| {
            c.no_scale()
                .when_value(selected, lit(selected_fill))
                .no_legend()
        })
        .stroke("#ffffff")
        .stroke_width(0.75)
        .size(size)
}

fn selection_overlay(data: StoreData, stroke: &str) -> Rect<Cartesian> {
    Rect::<Cartesian>::new()
        .data_store(data)
        .exclude_from_scale_domains()
        .x(col("x_min"))
        .x2(col("x_max"))
        .y(col("y_min"))
        .y2(col("y_max"))
        .fill("rgba(37, 99, 235, 0.08)")
        .stroke(stroke)
        .stroke_width(1.5)
        .zindex(10_000)
}

fn selectable_scope() -> Expr {
    ev::event_facet_value(0).is_not_null()
}

fn selectable_start_scope() -> Expr {
    ev::start_facet_value(0).is_not_null()
}

fn cursor_binding(cursor: &Param) -> ChartEventBinding {
    let over_selectable = ev::event_coord("x")
        .is_not_null()
        .and(ev::event_coord("y").is_not_null())
        .and(selectable_scope());
    let cursor_expr = when(over_selectable, ev::cursor(CursorStyle::Crosshair))
        .otherwise(ev::cursor(CursorStyle::Default))
        .expect("valid cursor case expression");
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .set_param(cursor, cursor_expr)
        .preview()
}

fn selection_drag_binding(
    cursor: &Param,
    store_name: &str,
    selection_id: &str,
    facet_scope: CoordinationScope,
) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .filter(selectable_start_scope())
        .filter(ev::start_coord("x").is_not_null())
        .filter(ev::start_coord("y").is_not_null())
        .filter(ev::event_at_start_clipped_coord("x").is_not_null())
        .filter(ev::event_at_start_clipped_coord("y").is_not_null())
        .filter(ev::shift().eq(lit(false)))
        .set_param(cursor, ev::cursor(CursorStyle::Grabbing))
        .set_selection_at_start_scope(selection_id, replace_selection_update(facet_scope))
        .set_store_at_start_scope_replacing_scopes(store_name, replace_store_update())
        .preview()
}

fn selection_add_drag_binding(
    cursor: &Param,
    store_name: &str,
    selection_id: &str,
    facet_scope: CoordinationScope,
) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .filter(selectable_start_scope())
        .filter(ev::start_coord("x").is_not_null())
        .filter(ev::start_coord("y").is_not_null())
        .filter(ev::event_at_start_clipped_coord("x").is_not_null())
        .filter(ev::event_at_start_clipped_coord("y").is_not_null())
        .filter(ev::shift().eq(lit(true)))
        .set_param(cursor, ev::cursor(CursorStyle::Grabbing))
        .set_selection_at_start_scope(selection_id, upsert_selection_update(facet_scope))
        .set_store_at_start_scope(store_name, upsert_store_update())
        .preview()
}

fn selection_release_binding(
    store_name: &str,
    selection_id: &str,
    facet_scope: CoordinationScope,
) -> ChartEventBinding {
    ChartEventBinding::on_between_end(
        ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
        ChartEventStream::on(ChartEventType::MouseUp),
    )
    .filter(selectable_start_scope())
    .filter(ev::start_coord("x").is_not_null())
    .filter(ev::start_coord("y").is_not_null())
    .filter(ev::event_at_start_clipped_coord("x").is_not_null())
    .filter(ev::event_at_start_clipped_coord("y").is_not_null())
    .filter(ev::shift().eq(lit(false)))
    .set_selection_at_start_scope(selection_id, replace_selection_update(facet_scope))
    .set_store_at_start_scope_replacing_scopes(store_name, replace_store_update())
    .exact()
}

fn selection_add_release_binding(
    store_name: &str,
    selection_id: &str,
    facet_scope: CoordinationScope,
) -> ChartEventBinding {
    ChartEventBinding::on_between_end(
        ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
        ChartEventStream::on(ChartEventType::MouseUp),
    )
    .filter(selectable_start_scope())
    .filter(ev::start_coord("x").is_not_null())
    .filter(ev::start_coord("y").is_not_null())
    .filter(ev::event_at_start_clipped_coord("x").is_not_null())
    .filter(ev::event_at_start_clipped_coord("y").is_not_null())
    .filter(ev::shift().eq(lit(true)))
    .set_selection_at_start_scope(selection_id, upsert_selection_update(facet_scope))
    .set_store_at_start_scope(store_name, upsert_store_update())
    .exact()
}

fn selection_clear_binding(store_name: &str, selection_id: &str) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .filter(selectable_scope())
        .clear_selection(selection_id)
        .set_store_replacing_scopes(store_name, StoreUpdate::clear())
        .exact()
}

fn brush_box_store(name: &str, sharing: CoordinationScope) -> Store {
    Store::empty(name)
        .field("id", DataType::Utf8, false)
        .field("x_min", DataType::Float64, false)
        .field("x_max", DataType::Float64, false)
        .field("y_min", DataType::Float64, false)
        .field("y_max", DataType::Float64, false)
        .primary_key(["id"])
        .sharing(sharing)
}

fn brush_selection_clause(id: Expr, facet_scope: CoordinationScope) -> SelectionClauseUpdate {
    SelectionClauseUpdate::interval(id)
        .facet_scope(facet_scope)
        .dimension(col("x"))
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
        .dimension(col("y"))
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

fn replace_selection_update(facet_scope: CoordinationScope) -> SelectionUpdate {
    SelectionUpdate::replace_all_clauses([brush_selection_clause(lit("active"), facet_scope)])
}

fn upsert_selection_update(facet_scope: CoordinationScope) -> SelectionUpdate {
    SelectionUpdate::upsert_clauses([brush_selection_clause(ev::start_event_id(), facet_scope)])
}

fn brush_box_row(id: Expr) -> StoreRow {
    let x_interval =
        ev::interval_ordered(ev::start_coord("x"), ev::event_at_start_clipped_coord("x"));
    let y_interval =
        ev::interval_ordered(ev::start_coord("y"), ev::event_at_start_clipped_coord("y"));
    StoreRow::new()
        .field("id", id)
        .field("x_min", ev::interval_start(x_interval.clone()))
        .field("x_max", ev::interval_end(x_interval))
        .field("y_min", ev::interval_start(y_interval.clone()))
        .field("y_max", ev::interval_end(y_interval))
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
