//! Cartesian box-zoom example.
//!
//! Drag a rectangle inside the plot to zoom both axes to the selected extent.
//! Double-click reset is intentionally not included here; this example isolates
//! the `BoxZoom` tool and its ordinary `Rect<Cartesian>` overlay mark.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example cartesian_box_zoom --features winit-wgpu --release
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
            .with_title("avenger-chart box zoom")
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
                ('A', 1.0, 2.0), ('A', 2.0, 2.7), ('A', 3.0, 4.2), ('A', 4.0, 4.8),
                ('B', 1.4, 5.5), ('B', 2.6, 4.9), ('B', 3.8, 6.8), ('B', 5.0, 7.2),
                ('C', 2.0, 1.2), ('C', 3.2, 1.8), ('C', 4.4, 2.4), ('C', 5.6, 3.1)
            ) AS t(group_name, x, y)",
        )
        .await
        .expect("build data");

    let plot = Plot::<Cartesian>::new()
        .canvas_size(640.0, 420.0)
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.share_domain())
                .y_with(col("y"), |c| c.share_domain())
                .fill(col("group_name"))
                .size(90.0),
        )
        .tool(BoxZoom::cartesian());

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
