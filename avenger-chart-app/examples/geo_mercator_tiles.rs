//! Geo (mercator) app example with URL-loaded raster map tiles.
//!
//! Pan with the left mouse button, scroll to zoom around the pointer, Shift-drag
//! to box zoom, and double-click to reset the inferred/authored view. Tiles are
//! loaded from URLs through the image resource cache; WGPU draws placeholders
//! while requests are pending and redraws when resources become ready.
//!
//! Run with:
//! ```bash
//! cargo run --release -p avenger-chart-app --example geo_mercator_tiles --features winit-wgpu
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    CanvasConfig, ChartAppOptions, ChartResizeBinding, ChartRuntimeResources, WgpuImagePlaceholder,
    WgpuImageResourceConfig, WgpuMissingImagePolicy, WinitWgpuAvengerApp,
    WinitWgpuAvengerAppOptions, chart_avenger_app_with_runtime_resources,
};
use avenger_chart_geo::{Geo, GeoPanZoom, GeoPositionChannels, RasterTileLayer, Symbol};
use avenger_image::ImageResourceCache;
use avenger_resource::RenderInvalidationHub;
use datafusion::prelude::SessionContext;
use winit::window::WindowAttributes;

const OSM_TILE_TEMPLATE: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";

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
                .with_title("avenger-chart Geo mercator tiles")
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
                (-73.9857, 40.7484, '#ef4444'),
                (-73.9772, 40.7527, '#2563eb'),
                (-73.9680, 40.7851, '#16a34a'),
                (-74.0445, 40.6892, '#f59e0b')
            ) AS t(lon, lat, color)",
        )
        .await
        .expect("build data");

    let tiles = RasterTileLayer::xyz(OSM_TILE_TEMPLATE)
        .id("osm")
        .max_zoom(19)
        .attribution("OpenStreetMap contributors")
        .zindex(-10)
        .smooth_zoom();
    let coord = Geo::mercator()
        .viewport_id("nyc")
        .center_lon_lat(-73.9857, 40.7484)
        .zoom(12.0)
        .tiles(tiles);
    let plot = Chart::with_coord(coord.clone())
        .canvas_size(800.0, 560.0)
        .data(df)
        .mark(
            Symbol::new()
                .lon_lat(&coord, col("lon"), col("lat"))
                .fill_with(col("color"), |fill| {
                    fill.no_scale().legend(|legend| legend.visible(false))
                })
                .stroke("#111827")
                .stroke_width(1.5)
                .size(160.0),
        )
        .tool(GeoPanZoom::new().viewport_id("nyc").settle_exact(true));

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
