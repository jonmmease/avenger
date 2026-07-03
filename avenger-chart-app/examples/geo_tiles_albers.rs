//! Warped satellite tiles on the CONUS Albers projection with pan/zoom
//! and the adaptive Mercator blend (scratch/geo phase 6).
//!
//! At the fitted view the imagery tiles are warped through the Albers
//! projection (curved tile edges follow the graticule); zooming in past
//! the blend threshold morphs the projection into Web Mercator, where
//! the tiles become ordinary slippy-map squares. Markers call out a few
//! landmarks to fly to.
//!
//! Pan with the left mouse button, scroll to zoom around the pointer,
//! Shift-drag to box zoom, and double-click to reset to the fitted view.
//! Requires network access for the Esri World Imagery tile service.
//!
//! Run with:
//! ```bash
//! cargo run --release -p avenger-chart-app --example geo_tiles_albers --features winit-wgpu
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    CanvasConfig, ChartAppOptions, ChartResizeBinding, ChartRuntimeResources, WgpuImagePlaceholder,
    WgpuImageResourceConfig, WgpuMissingImagePolicy, WinitWgpuAvengerApp,
    WinitWgpuAvengerAppOptions, chart_avenger_app_with_runtime_resources,
};
use avenger_chart_geo::{
    BlendConfig, Geo, GeoPanZoom, GeoPositionChannels, GraticuleStyle, RasterTileLayer, Symbol,
};
use avenger_image::ImageResourceCache;
use avenger_resource::RenderInvalidationHub;
use datafusion::prelude::SessionContext;
use winit::window::WindowAttributes;

/// Esri World Imagery (satellite): note the `{z}/{y}/{x}` path order.
const IMAGERY_TILE_TEMPLATE: &str =
    "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}";

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    // Tiles load through the image resource cache; the invalidation hub
    // redraws the window as tiles arrive, and the canvas draws
    // checkerboard placeholders for tiles that are still pending.
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
                .with_title("avenger-chart Geo: warped satellite tiles on Albers")
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
    let landmarks = ctx
        .sql(
            "SELECT * FROM (VALUES
                ('Grand Canyon',         -112.11, 36.11, '#f59e0b'),
                ('Old Faithful',         -110.83, 44.46, '#22c55e'),
                ('Golden Gate Bridge',   -122.48, 37.82, '#ef4444'),
                ('Niagara Falls',         -79.07, 43.08, '#3b82f6'),
                ('French Quarter',        -90.07, 29.95, '#a855f7'),
                ('Kennedy Space Center',  -80.65, 28.57, '#ec4899'),
                ('Mount Rushmore',       -103.46, 43.88, '#14b8a6')
            ) AS t(name, lon, lat, color)",
        )
        .await
        .expect("landmark data");

    let tiles = RasterTileLayer::xyz(IMAGERY_TILE_TEMPLATE)
        .id("imagery")
        .max_zoom(19)
        .attribution("© Esri, Maxar, Earthstar Geographics")
        .smooth_zoom();

    let geo = Geo::albers_usa_conus()
        .viewport_id("us")
        .center_lon_lat(-96.0, 38.5)
        .zoom(4.4)
        .graticule(GraticuleStyle::default())
        .tiles(tiles)
        .adaptive_blend(BlendConfig {
            z0: 4.0,
            z1: 12.0,
            ..Default::default()
        });
    let plot = Plot::with_coord(geo.clone())
        .canvas_size(860.0, 600.0)
        .title("Warped satellite tiles — zoom in to morph into Mercator")
        .data(landmarks)
        .mark(
            Symbol::new()
                .lon_lat(&geo, col("lon"), col("lat"))
                .fill_with(col("color"), |fill| {
                    fill.no_scale().legend(|legend| legend.visible(false))
                })
                .stroke("#ffffff")
                .stroke_width(1.6)
                .size(110.0),
        )
        .tool(GeoPanZoom::new().viewport_id("us").settle_exact(true));

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
