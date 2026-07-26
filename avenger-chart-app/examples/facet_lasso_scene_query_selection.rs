//! Faceted lasso selection with free versus shared scene-query sharing.
//!
//! The left faceted plot uses free sharing, so a lasso in one facet queries and
//! highlights only that facet. The right faceted plot uses shared sharing, so
//! the same scene-space lasso is replicated across the right-hand facets and
//! highlights the unique rows inside each visible lasso region.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example facet_lasso_scene_query_selection --features winit-wgpu --release
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

const FREE_LASSO_STORE: &str = "free_lasso_path";
const SHARED_LASSO_STORE: &str = "shared_lasso_path";

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart faceted scene-query lasso selection")
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let df = ctx.read_batch(make_points_batch()).expect("read points");

    let free_pick = Selection::new("free_pick")
        .empty_selects_nothing()
        .facet_context_field("group_name", col("group_name"));
    let free_selected = free_pick.predicate();
    let shared_pick = Selection::new("shared_pick").empty_selects_nothing();
    let shared_selected = shared_pick.predicate();

    let free_leaf = Plot::<Cartesian>::new()
        .mark(selection_points(free_selected, "#2563eb"))
        .mark(lasso_overlay(FREE_LASSO_STORE, "#2563eb"));
    let free_facets = Plot::<FacetColumn>::new()
        .data(df.clone())
        .mark(
            Subplot::new(free_leaf)
                .column(col("group_name"))
                .label("Free sharing"),
        )
        .event_binding(lasso_drag_binding(
            FREE_LASSO_STORE,
            "free_pick",
            CoordinationScope::Free,
        ))
        .event_binding(lasso_clear_binding(FREE_LASSO_STORE, "free_pick"));

    let shared_leaf = Plot::<Cartesian>::new()
        .mark(selection_points(shared_selected, "#d97706"))
        .mark(lasso_overlay(SHARED_LASSO_STORE, "#d97706"));
    let shared_facets = Plot::<FacetColumn>::new()
        .data(df)
        .mark(
            Subplot::new(shared_leaf)
                .column(col("group_name"))
                .label("Shared sharing"),
        )
        .event_binding(lasso_drag_binding(
            SHARED_LASSO_STORE,
            "shared_pick",
            CoordinationScope::Shared,
        ))
        .event_binding(lasso_clear_binding(SHARED_LASSO_STORE, "shared_pick"));

    let plot = Chart::<HConcat>::new()
        .canvas_size(1440.0, 520.0)
        .selection(free_pick)
        .selection(shared_pick)
        .store(lasso_overlay_store(
            FREE_LASSO_STORE,
            CoordinationScope::Free,
        ))
        .store(lasso_overlay_store(
            SHARED_LASSO_STORE,
            CoordinationScope::Shared,
        ))
        .mark(Subplot::new(free_facets).name("free"))
        .mark(Subplot::new(shared_facets).name("shared"))
        .event_binding(cursor_binding());

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

fn selection_points(selected: Expr, selected_fill: &str) -> Symbol<Cartesian> {
    Symbol::new()
        .id("points")
        .x(col("x"))
        .y(col("y"))
        .fill_with(lit("#b8beca"), |c| {
            c.no_scale()
                .when_value(selected, lit(selected_fill))
                .no_legend()
        })
        .stroke("#ffffff")
        .stroke_width(0.75)
        .size(110.0)
}

fn selectable_scope() -> Expr {
    ev::event_facet_value(0).is_not_null()
}

fn selectable_start_scope() -> Expr {
    ev::start_facet_value(0).is_not_null()
}

fn cursor_binding() -> ChartEventBinding {
    let over_selectable = ev::event_coord("x")
        .is_not_null()
        .and(ev::event_coord("y").is_not_null())
        .and(selectable_scope());
    let cursor_expr = when(over_selectable, ev::cursor(CursorStyle::Crosshair))
        .otherwise(ev::cursor(CursorStyle::Default))
        .expect("valid cursor expression");
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .set_cursor(cursor_expr)
        .preview()
}

fn lasso_drag_binding(
    store_name: &str,
    selection_id: &str,
    sharing: CoordinationScope,
) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .between(
            ChartEventStream::on(ChartEventType::MouseDown).filter(ev::button().eq(lit("left"))),
            ChartEventStream::on(ChartEventType::MouseUp),
        )
        .filter(selectable_start_scope())
        .filter(ev::start_coord("x").is_not_null())
        .filter(ev::start_coord("y").is_not_null())
        .filter(ev::event_path_svg().is_not_null())
        .set_cursor(ev::cursor(CursorStyle::Grabbing))
        .set_store_at_start_scope_replacing_scopes(store_name, lasso_overlay_update("active"))
        .set_selection_at_start_scope(
            selection_id,
            SelectionUpdate::replace_all_from_scene_query(lasso_query_update(sharing)),
        )
        .event_path_min_distance_px(6.0)
        .preview()
        .settle_exact()
}

fn lasso_clear_binding(store_name: &str, selection_id: &str) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .filter(selectable_scope())
        .set_store_replacing_scopes(store_name, StoreUpdate::clear())
        .clear_selection(selection_id)
        .exact()
}

fn lasso_query_update(sharing: CoordinationScope) -> SelectionSceneQuery {
    SelectionSceneQuery::new(
        SceneGeometryQuery::polygon(ev::event_path())
            .hit_policy(SceneGeometryHitPolicy::AnchorInside)
            .mark("points")
            .datum_field(
                SceneQueryDatumField::new("row_id")
                    .datum("row_id")
                    .field_expr(col("row_id")),
            )
            .unique_by(["row_id"]),
    )
    .sharing(sharing)
}

fn lasso_overlay(store: &str, stroke: &str) -> PathMark<Cartesian> {
    PathMark::<Cartesian>::new()
        .data_store(StoreData::new(store))
        .x_with(col("anchor_x"), |c| c.exclude_from_scale_domain())
        .y_with(col("anchor_y"), |c| c.exclude_from_scale_domain())
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
    let groups = ["Alpha", "Beta", "Gamma"];
    let items = ["A", "B", "C", "D", "E", "F"];
    let mut row_ids = Vec::new();
    let mut group_values = Vec::new();
    let mut item_values = Vec::new();
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for (group_index, group) in groups.iter().enumerate() {
        for (item_index, item) in items.iter().enumerate() {
            row_ids.push(format!("{group}-{item}"));
            group_values.push((*group).to_string());
            item_values.push((*item).to_string());
            xs.push(item_index as f64 + 0.18 * group_index as f64);
            let base = item_index as f64 * 0.55 + group_index as f64 * 1.15;
            ys.push(base + ((item_index + group_index) as f64 * 0.7).sin() * 0.2);
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("row_id", DataType::Utf8, false),
        Field::new("group_name", DataType::Utf8, false),
        Field::new("item", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(row_ids)),
            Arc::new(StringArray::from(group_values)),
            Arc::new(StringArray::from(item_values)),
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
