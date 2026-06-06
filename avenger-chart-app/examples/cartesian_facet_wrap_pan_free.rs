//! Wrapped-facet Cartesian pan/zoom with **per-cell (`Free`) sharing**.
//!
//! A wrap facet lays out one subplot per category, flowing into a fixed grid of
//! columns (here 3 across, so six categories wrap into 2 rows). The x and y
//! raw-domain params are declared `CoordinationScope::Free` and each cell's scales are
//! `free_domain()` (independent per cell). Dragging with the left mouse button
//! or scrolling inside ONE cell pans/zooms ONLY that cell — the bindings route
//! the pointer to the cell under it and write the param at that cell's own owner
//! path.
//!
//! This is the per-cell counterpart to `cartesian_facet_wrap_pan` (Shared). It
//! also demonstrates that `Free` resolves to a single wrapped cell even though a
//! wrap facet is physically laid out as hidden row bands: the per-cell owner
//! path is the cell's full facet path, so neighbors are untouched.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example cartesian_facet_wrap_pan_free --features winit-wgpu --release
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use datafusion::prelude::{SessionContext, lit};
use winit::window::WindowAttributes;

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart wrap pan/zoom (Free) — drag or scroll one cell")
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
                ('C', 1.5, 3.5), ('C', 3.5, 6.0), ('C', 5.5, 2.5), ('C', 7.5, 5.5),
                ('D', 2.5, 1.5), ('D', 4.5, 4.0), ('D', 6.5, 6.5), ('D', 8.5, 3.5),
                ('E', 1.0, 4.0), ('E', 3.0, 2.0), ('E', 5.0, 5.5), ('E', 7.0, 3.0),
                ('F', 2.0, 6.0), ('F', 4.0, 4.5), ('F', 6.0, 2.0), ('F', 8.0, 5.0)
            ) AS t(group_name, x, y)",
        )
        .await
        .expect("build data");

    // Leaf scatter: x and y scales are independent per cell (`free_domain`).
    // The tool mirrors that sharing for the generated raw-domain params.
    let leaf = Plot::<Cartesian>::new()
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.free_domain())
                .y_with(col("y"), |c| c.free_domain())
                .fill(col("group_name"))
                .size(70.0),
        )
        .tool(PanScrollZoom::cartesian().settle_exact(true));

    let plot = Plot::<FacetWrap>::new()
        .canvas_size(760.0, 480.0)
        .data(df)
        .mark(Subplot::new(leaf).wrap_with(col("group_name"), |c| c.columns(lit(3))));

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
