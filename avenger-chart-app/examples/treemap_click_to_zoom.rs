//! Treemap click-to-zoom using hierarchy event datum fields.
//!
//! Click a group header, collapsed cell, or breadcrumb to focus the treemap on
//! that hierarchy path. Double-click anywhere to reset to the full tree.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example treemap_click_to_zoom --features winit-wgpu --release
//! ```

use std::sync::Arc;

use avenger_chart::{event as ev, prelude::*};
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use avenger_chart_treemap::{
    ROOT_PATH_ID, TreeRect, Treemap, TreemapGuide, event as treemap_event,
};
use datafusion::{
    arrow::{
        array::{Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::ScalarValue,
    functions_aggregate::expr_fn::sum,
    logical_expr::{col, lit, when},
    prelude::SessionContext,
};
use winit::window::WindowAttributes;

const SIZE: [f32; 2] = [900.0, 560.0];

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart treemap click to zoom")
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let cursor = Param::cursor("treemap_zoom_cursor", CursorStyle::Default);
    let root = Param::new("treemap_root", ScalarValue::Utf8(None));

    let plot = Chart::with_coord(
        Treemap::new()
            .path_columns(["division", "region", "team", "product"])
            .value(sum(col("sales")))
            .root_path_param(root.name.clone())
            .display_levels(2),
    )
    .title("Click a treemap group to zoom")
    .canvas_size(SIZE[0], SIZE[1])
    .data(ctx.read_batch(treemap_batch()).expect("read data"))
    .param(cursor.clone())
    .param(root.clone())
    .cursor_param(cursor.name.clone())
    .configure_guide(
        TreemapGuide::new()
            .headers(true)
            .separators(true)
            .breadcrumbs(true),
    )
    .mark(
        TreeRect::new()
            .id("cells")
            .fill_with(col("color"), |fill| fill.no_scale().no_legend())
            .stroke("#ffffff")
            .stroke_width(1.0),
    )
    .event_binding(cursor_binding(&cursor))
    .event_binding(zoom_binding(&root))
    .event_binding(reset_zoom_binding(&root));

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
    let cursor_expr = when(
        treemap_event::hierarchy_can_zoom().eq(lit(true)),
        ev::cursor(CursorStyle::Grab),
    )
    .otherwise(ev::cursor(CursorStyle::Default))
    .expect("valid cursor expression");
    ChartEventBinding::on(ChartEventType::CursorMoved)
        .set_param(cursor, cursor_expr)
        .preview()
}

fn zoom_binding(root: &Param) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::Click)
        .filter(ev::button().eq(lit("left")))
        .filter(treemap_event::hierarchy_can_zoom().eq(lit(true)))
        .set_param(root, treemap_event::hierarchy_path_id())
        .exact()
}

fn reset_zoom_binding(root: &Param) -> ChartEventBinding {
    ChartEventBinding::on(ChartEventType::DoubleClick)
        .set_param(root, lit(ROOT_PATH_ID))
        .exact()
}

fn treemap_batch() -> RecordBatch {
    let division = [
        "Enterprise",
        "Enterprise",
        "Enterprise",
        "Enterprise",
        "Enterprise",
        "Enterprise",
        "Consumer",
        "Consumer",
        "Consumer",
        "Consumer",
        "International",
        "International",
        "International",
        "International",
        "International",
    ];
    let region = [
        "North America",
        "North America",
        "North America",
        "Europe",
        "Europe",
        "APAC",
        "North America",
        "North America",
        "Europe",
        "APAC",
        "Latin America",
        "Latin America",
        "MEA",
        "APAC",
        "APAC",
    ];
    let team = [
        "Platform",
        "Platform",
        "Services",
        "Platform",
        "Services",
        "Services",
        "Retail",
        "Retail",
        "Retail",
        "Support",
        "Growth",
        "Growth",
        "Growth",
        "Expansion",
        "Expansion",
    ];
    let product = [
        "Core",
        "Data",
        "Support",
        "Core",
        "Consulting",
        "Support",
        "Storefront",
        "Loyalty",
        "Storefront",
        "Support",
        "Acquisition",
        "Retention",
        "Acquisition",
        "Localization",
        "Payments",
    ];
    let sales = [
        52.0, 38.0, 34.0, 33.0, 28.0, 26.0, 44.0, 31.0, 37.0, 24.0, 41.0, 29.0, 22.0, 35.0, 27.0,
    ];
    let color = [
        "#2563eb", "#2563eb", "#16a34a", "#7c3aed", "#16a34a", "#16a34a", "#ea580c", "#ea580c",
        "#ea580c", "#0891b2", "#db2777", "#db2777", "#db2777", "#0f766e", "#0f766e",
    ];
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("division", DataType::Utf8, false),
            Field::new("region", DataType::Utf8, false),
            Field::new("team", DataType::Utf8, false),
            Field::new("product", DataType::Utf8, false),
            Field::new("sales", DataType::Float64, false),
            Field::new("color", DataType::Utf8, false),
        ])),
        vec![
            Arc::new(StringArray::from(division.to_vec())),
            Arc::new(StringArray::from(region.to_vec())),
            Arc::new(StringArray::from(team.to_vec())),
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
