//! Row-faceted Cartesian pan/zoom (shared domains).
//!
//! A row-faceted scatter: each unique `group_name` becomes a horizontal band
//! stacked vertically. The x and y scales are shared across all rows and read
//! shared raw-domain params, so dragging with the left mouse button or scrolling
//! inside ANY row pans/zooms EVERY row together.
//!
//! This is the row-axis counterpart to `cartesian_facet_pan` (which facets by
//! column). It exercises the same shared scope-export + routing path with the
//! facet bands laid out along the vertical axis.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example cartesian_row_facet_pan --features winit-wgpu --release
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::prelude::SessionContext;
use winit::window::WindowAttributes;

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart row-faceted pan/zoom — drag or scroll any row")
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
                ('A', 1.0, 2.0), ('A', 3.0, 4.5), ('A', 5.0, 3.2), ('A', 7.0, 6.1),
                ('B', 2.0, 5.0), ('B', 4.0, 3.0), ('B', 6.0, 7.0), ('B', 8.0, 4.4),
                ('C', 1.5, 3.5), ('C', 3.5, 6.0), ('C', 5.5, 2.5), ('C', 7.5, 5.5)
            ) AS t(group_name, x, y)",
        )
        .await
        .expect("build data");

    let leaf = Plot::<Cartesian>::new()
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.share_domain())
                .y_with(col("y"), |c| c.share_domain())
                .fill(col("group_name"))
                .size(80.0),
        )
        .tool(PanScrollZoom::cartesian().settle_exact(true));

    let plot = Plot::<FacetRow>::new()
        .canvas_size(480.0, 720.0)
        .data(df)
        .mark(Subplot::new(leaf).row(col("group_name")));

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
