//! Treemap leaf click selection.
//!
//! Click a leaf rectangle to select its path fields and emphasize the cell.
//! Double-click to clear.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example treemap_leaf_click_selection --features winit-wgpu --release
//! ```

use std::sync::Arc;

use avenger_chart::{event as ev, prelude::*};
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use avenger_chart_treemap::{
    TreeRect, Treemap, TreemapGuide,
    event::{self as treemap_event, HIERARCHY_SURFACE_KIND_LEAF_RECT},
};
use datafusion::{
    arrow::{
        array::{Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    functions_aggregate::expr_fn::sum,
    logical_expr::{col, lit, when},
    prelude::SessionContext,
};
use winit::window::WindowAttributes;

const SIZE: [f32; 2] = [860.0, 540.0];

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart treemap leaf click selection")
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let picked = Selection::new("picked").empty_selects_nothing();
    let selected = picked.predicate();

    let plot = Chart::with_coord(
        Treemap::new()
            .path_columns(["region", "category", "product"])
            .value(sum(col("sales"))),
    )
    .title("Click a treemap leaf")
    .canvas_size(SIZE[0], SIZE[1])
    .data(ctx.read_batch(treemap_batch()).expect("read data"))
    .selection(picked)
    .configure_guide(
        TreemapGuide::new()
            .headers(true)
            .separators(true)
            .breadcrumbs(false),
    )
    .mark(
        TreeRect::new()
            .id("cells")
            .fill_with(lit("#d8dde3"), |fill| {
                fill.no_scale()
                    .when_value(selected.clone(), col("color"))
                    .no_legend()
            })
            .stroke_with(lit("#ffffff"), |stroke| {
                stroke
                    .no_scale()
                    .when_value(selected.clone(), lit("#111827"))
                    .no_legend()
            })
            .stroke_width_with(lit(1.0), |stroke_width| {
                stroke_width
                    .no_scale()
                    .when_value(selected.clone(), lit(3.0))
                    .no_legend()
            }),
    )
    .event_binding(cursor_binding())
    .event_binding(select_leaf_binding())
    .event_binding(clear_selection_binding());

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
    let over_leaf =
        treemap_event::hierarchy_surface_kind().eq(lit(HIERARCHY_SURFACE_KIND_LEAF_RECT));
    let cursor_expr = when(over_leaf, ev::cursor(CursorStyle::Grab))
        .otherwise(ev::cursor(CursorStyle::Default))
        .expect("valid cursor expression");
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .set_cursor(cursor_expr)
        .preview()
}

fn select_leaf_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::Click)
        .filter(ev::button().eq(lit("left")))
        .filter(treemap_event::hierarchy_surface_kind().eq(lit(HIERARCHY_SURFACE_KIND_LEAF_RECT)))
        .set_selection(
            "picked",
            SelectionUpdate::replace_clause(
                SelectionClauseUpdate::equality(lit("active"))
                    .facet_scope(CoordinationScope::Shared)
                    .dimension_named("region", col("region"), ev::datum("region"))
                    .dimension_named("category", col("category"), ev::datum("category"))
                    .dimension_named("product", col("product"), ev::datum("product"))
                    .build(),
            ),
        )
        .exact()
}

fn clear_selection_binding() -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .clear_selection("picked")
        .exact()
}

fn treemap_batch() -> RecordBatch {
    let region = [
        "Americas", "Americas", "Americas", "Americas", "Americas", "EMEA", "EMEA", "EMEA", "EMEA",
        "APAC", "APAC", "APAC",
    ];
    let category = [
        "Platform", "Platform", "Services", "Services", "Services", "Platform", "Platform",
        "Services", "Services", "Platform", "Services", "Services",
    ];
    let product = [
        "Core",
        "Data",
        "Support",
        "Training",
        "Consulting",
        "Core",
        "Data",
        "Support",
        "Consulting",
        "Core",
        "Support",
        "Training",
    ];
    let sales = [
        54.0, 36.0, 42.0, 18.0, 28.0, 44.0, 31.0, 33.0, 21.0, 39.0, 29.0, 24.0,
    ];
    let color = [
        "#2563eb", "#2563eb", "#16a34a", "#16a34a", "#16a34a", "#7c3aed", "#7c3aed", "#ea580c",
        "#ea580c", "#0891b2", "#db2777", "#db2777",
    ];
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("region", DataType::Utf8, false),
            Field::new("category", DataType::Utf8, false),
            Field::new("product", DataType::Utf8, false),
            Field::new("sales", DataType::Float64, false),
            Field::new("color", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(StringArray::from(region.to_vec())),
            Arc::new(StringArray::from(category.to_vec())),
            Arc::new(StringArray::from(product.to_vec())),
            Arc::new(Float64Array::from(sales.to_vec())),
            Arc::new(StringArray::from(color.to_vec())),
        ],
    )
    .expect("treemap example data")
}

fn init_diagnostics() {
    if std::env::var_os("RUST_LOG").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
            .try_init();
    } else {
        let _ = tracing_subscriber::fmt().try_init();
    }
}
