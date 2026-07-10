//! The "client" side: loads the baked artifact and renders it interactively
//! in a FRESH session with zero access to the server's data sources — no
//! parquet file, no registered tables. Move the cursor across the chart to
//! sweep the `$min` threshold; the embedded rows re-aggregate live.

use std::{error::Error, sync::Arc};

use avenger_chart::plot::CompiledPlot;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WindowSceneSizing, WinitWgpuAvengerApp,
    WinitWgpuAvengerAppOptions, chart_avenger_app_with_default_runtime_resources,
};
use chart_bake_server_client::{artifact_path, init_logging};
use datafusion::prelude::SessionContext;
use winit::window::WindowAttributes;

fn main() -> Result<(), Box<dyn Error>> {
    init_logging();
    let artifact = artifact_path();
    let bytes = std::fs::read(&artifact).map_err(|err| {
        format!(
            "failed to read {} ({err}); run the server first: \
             cargo run --release -p chart-bake-server-client --bin server",
            artifact.display()
        )
    })?;
    let baked: CompiledPlot = bincode::deserialize(&bytes)?;
    println!(
        "loaded baked chart from {} ({} bytes); rendering with NO data sources",
        artifact.display(),
        bytes.len()
    );

    // A fresh session: nothing registered, nothing read from disk. Every
    // table the chart needs travels inside the artifact.
    let ctx = Arc::new(SessionContext::new());

    let tokio_runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let bundle = tokio_runtime.block_on(chart_avenger_app_with_default_runtime_resources(
        baked,
        ctx,
        ChartAppOptions {
            resize_binding: ChartResizeBinding::none(),
            resize_throttle_ms: None,
            exact_on_resize_settle: true,
            log_metrics: true,
        },
    ))?;
    let options = WinitWgpuAvengerAppOptions::new(2.0)
        .window_attributes(
            WindowAttributes::default()
                .with_title("baked chart — no data sources")
                .with_resizable(false),
        )
        .window_scene_sizing(WindowSceneSizing::MatchSceneGraph)
        .render_invalidation_hub(bundle.runtime_resources.render_invalidation_hub);
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(bundle.app, options, tokio_runtime);
    event_loop.run_app(&mut app)?;
    Ok(())
}
