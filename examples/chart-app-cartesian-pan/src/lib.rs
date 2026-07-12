use std::sync::{Arc, OnceLock};

use avenger_chart::physical_cache::{
    EvaluationCache, EvaluationCacheConfig, cached_session_context,
};
use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WindowSceneSizing, WinitWgpuAvengerApp,
    WinitWgpuAvengerAppOptions, chart_avenger_app_with_default_runtime_resources,
};
use winit::window::WindowAttributes;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// The physical result cache backing the app's session context; exposed so
/// the browser console (or native code) can inspect reuse while panning.
static PHYSICAL_CACHE: OnceLock<Arc<EvaluationCache>> = OnceLock::new();

/// Snapshot the physical cache metrics as a debug string. In the browser:
/// call `wasm.cache_metrics()` from the console before and after panning —
/// preview evaluations during the gesture should show `hits` growing with
/// `admitted_writes` unchanged (observe-only), and post-gesture settling
/// should admit and then hit.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub fn cache_metrics() -> String {
    match PHYSICAL_CACHE.get() {
        Some(cache) => format!("{:?}", cache.metrics()),
        None => "physical cache not initialized".to_string(),
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub async fn run() {
    init_diagnostics();

    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            let bundle = build_app().await;
            let options = app_options(bundle.runtime_resources.render_invalidation_hub);
            let (mut app, event_loop) =
                WinitWgpuAvengerApp::new_and_event_loop_with_options(bundle.app, options);
            event_loop.run_app(&mut app).expect("run app");
        } else {
            let tokio_runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("build tokio runtime");
            let bundle = tokio_runtime.block_on(build_app());
            let options = app_options(bundle.runtime_resources.render_invalidation_hub);
            let (mut app, event_loop) = WinitWgpuAvengerApp::new_and_event_loop_with_options(
                bundle.app,
                options,
                tokio_runtime,
            );
            event_loop.run_app(&mut app).expect("run app");
        }
    }
}

fn app_options(
    render_invalidation_hub: avenger_resource::RenderInvalidationHub,
) -> WinitWgpuAvengerAppOptions {
    WinitWgpuAvengerAppOptions::new(2.0)
        .window_attributes(
            WindowAttributes::default()
                .with_title("avenger-chart Cartesian pan/zoom")
                .with_resizable(false),
        )
        .window_scene_sizing(WindowSceneSizing::MatchSceneGraph)
        .render_invalidation_hub(render_invalidation_hub)
}

async fn build_app() -> avenger_chart_app::ChartAppBundle {
    // Physical result cache installed at context build time; repeated and
    // preview evaluations reuse executed subtrees (AVENGER_PHYSICAL_CACHE=0
    // disables on native; wasm has no env).
    let (ctx, cache) = cached_session_context(EvaluationCacheConfig::default());
    let _ = PHYSICAL_CACHE.set(cache);
    let ctx = Arc::new(ctx);
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
                (1.0, 2.0, 'A'),
                (2.0, 3.6, 'A'),
                (3.0, 4.2, 'B'),
                (4.0, 3.1, 'B'),
                (5.0, 5.2, 'C'),
                (6.0, 4.8, 'C'),
                (7.0, 6.5, 'D'),
                (8.0, 5.8, 'D')
            ) AS t(x, y, group_name)",
        )
        .await
        .expect("build data");

    let plot = Chart::<Cartesian>::new()
        .canvas_size(760.0, 520.0)
        .configure_title("Grouped scatter: $y = alpha x^2 + beta$", |title| {
            title.typst()
        })
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("group_name"))
                .size(120.0),
        )
        .tool(PanScrollZoom::cartesian().settle_exact(true));

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app_with_default_runtime_resources(
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
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            std::panic::set_hook(Box::new(console_error_panic_hook::hook));
            console_log::init_with_level(log::Level::Warn).expect("initialize logger");
        } else if #[cfg(not(target_arch = "wasm32"))] {
            if std::env::var_os("RUST_LOG").is_some() {
                let _ = tracing_subscriber::fmt()
                    .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
                    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                    .try_init();
            } else {
                let _ = env_logger::try_init();
            }
        }
    }
}
