//! Cartesian app example with URL-loaded image marks.
//!
//! The image is loaded from a URL so the example exercises image URL loading
//! through the chart image channel.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example cartesian_image_mark --features winit-wgpu --release
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    CanvasConfig, ChartAppOptions, ChartResizeBinding, ChartRuntimeResources, WgpuImagePlaceholder,
    WgpuImageResourceConfig, WgpuMissingImagePolicy, WinitWgpuAvengerApp,
    WinitWgpuAvengerAppOptions, chart_avenger_app_with_runtime_resources,
};
use avenger_image::ImageResourceCache;
use avenger_resource::RenderInvalidationHub;
use datafusion::prelude::SessionContext;
use winit::window::WindowAttributes;

const PICSUM_IMAGE_URL: &str = "https://picsum.photos/200/300";

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let invalidations = RenderInvalidationHub::default();
    let image_cache = Arc::new(
        ImageResourceCache::new().with_render_invalidation_sink(Arc::new(invalidations.clone())),
    );
    let runtime_resources = ChartRuntimeResources::new(image_cache.clone(), invalidations.clone());
    let canvas_config = CanvasConfig {
        image_resource_config: WgpuImageResourceConfig {
            resolver: Some(image_cache),
            missing_policy: WgpuMissingImagePolicy::DrawPlaceholder,
            placeholder: WgpuImagePlaceholder::Checkerboard,
        },
        ..CanvasConfig::default()
    };
    let avenger_app = tokio_runtime.block_on(build_app(runtime_resources));
    let options = WinitWgpuAvengerAppOptions::new(2.0)
        .window_attributes(
            WindowAttributes::default()
                .with_title("avenger-chart image mark app (drag to pan, scroll to zoom)")
                .with_resizable(false),
        )
        .canvas_config(canvas_config)
        .render_invalidation_hub(invalidations);
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app(
    runtime_resources: ChartRuntimeResources,
) -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
                (1.0, 3.2, 'North', '#2563eb'),
                (2.4, 4.8, 'East', '#16a34a'),
                (4.1, 2.7, 'South', '#ea580c'),
                (5.7, 5.6, 'West', '#7c3aed')
            ) AS t(x, y, label, color)",
        )
        .await
        .expect("build data");

    let plot = Chart::<Cartesian>::new()
        .title("Image marks in an app")
        .canvas_size(760.0, 520.0)
        .data(df)
        .mark(
            Image::new()
                .x(col("x"))
                .y(col("y"))
                .image(PICSUM_IMAGE_URL)
                .width(ChannelValue::from(64.0).no_scale())
                .height(ChannelValue::from(96.0).no_scale())
                .align("center")
                .baseline("middle")
                .smooth(true),
        )
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size_with(lit(42.0), |size| size.no_scale())
                .fill_with(col("color"), |fill| {
                    fill.no_scale().legend(|legend| legend.visible(false))
                })
                .stroke("#111827")
                .stroke_width(1.0),
        )
        .mark(
            Text::new()
                .x(col("x"))
                .y(col("y") + lit(0.62))
                .text(col("label"))
                .align("center")
                .baseline("bottom")
                .font_size(12.0)
                .color("#111827"),
        )
        .tool(PanScrollZoom::cartesian().settle_exact(true));

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app_with_runtime_resources(
        compiled,
        ctx,
        ChartAppOptions {
            resize_binding: ChartResizeBinding::none(),
            resize_throttle_ms: None,
            exact_on_resize_settle: true,
            log_metrics: true,
        },
        runtime_resources,
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
