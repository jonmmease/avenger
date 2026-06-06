//! Faceted Cartesian pan/zoom example (shared domains).
//!
//! A column-faceted scatter where the x and y scales are shared across all
//! cells and read shared raw-domain params. Dragging with the left mouse button
//! or scrolling inside ANY cell pans/zooms EVERY cell together: the bindings
//! route the pointer to the cell under it, invert through that cell's scale, and
//! write the shared (root) domain params, so all cells re-render with the same
//! new domain.
//!
//! This demonstrates the faceted scope-export + routing path with `CoordinationScope`
//! at the global (Shared) level. See `cartesian_facet_pan_free` for per-cell
//! independent pan/zoom behavior.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example cartesian_facet_pan --features winit-wgpu --release
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
            .with_title("avenger-chart faceted pan/zoom — drag or scroll any cell")
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

    // Leaf scatter: x and y scales are shared across cells; the tool installs
    // matching shared raw-domain params.
    let leaf = Plot::<Cartesian>::new()
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.share_scale())
                .y_with(col("y"), |c| c.share_scale())
                .fill(col("group_name"))
                .size(80.0),
        )
        .tool(PanScrollZoom::cartesian().settle_exact(true));

    let plot = Plot::<FacetColumn>::new()
        .canvas_size(820.0, 360.0)
        .data(df)
        .mark(Subplot::new(leaf).column(col("group_name")));

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
