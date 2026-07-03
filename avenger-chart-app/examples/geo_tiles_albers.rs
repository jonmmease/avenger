//! Warped raster tiles on the CONUS Albers projection with pan/zoom and
//! the adaptive Mercator blend (scratch/geo phase 6).
//!
//! At the fitted view the basemap tiles are warped through the Albers
//! projection (curved tile edges follow the graticule); zooming in past
//! the blend threshold morphs the projection into Web Mercator, where the
//! tiles become ordinary slippy-map squares.
//!
//! Pan with the left mouse button, scroll to zoom around the pointer,
//! Shift-drag to box zoom, and double-click to reset to the fitted view.
//! Requires network access for the CARTO basemap CDN.
//!
//! Run with:
//! ```bash
//! cargo run --release -p avenger-chart-app --example geo_tiles_albers --features winit-wgpu
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use avenger_chart_geo::{
    BlendConfig, Geo, GeoPanZoom, GeoShape, GraticuleStyle, RasterTileLayer, register_geojson,
};
use datafusion::prelude::SessionContext;
use palette::rgb::Srgba;
use winit::window::WindowAttributes;

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build tokio runtime");
    let avenger_app = tokio_runtime.block_on(build_app());
    let options = WinitWgpuAvengerAppOptions::new(2.0).window_attributes(
        WindowAttributes::default()
            .with_title("avenger-chart Geo: warped tiles on Albers")
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let path = format!(
        "{}/../avenger-chart/tests/data/geo/us-states.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let df = register_geojson(&ctx, "us_states", path)
        .await
        .expect("register us states");

    let tiles = RasterTileLayer::xyz(
        "https://basemaps.cartocdn.com/rastertiles/voyager_nolabels/{z}/{x}/{y}.png",
    )
    .id("carto")
    .max_zoom(19)
    .attribution("© OpenStreetMap contributors © CARTO")
    .smooth_zoom();

    let geo = Geo::albers_usa_conus()
        .viewport_id("us")
        .graticule(GraticuleStyle::default())
        .tiles(tiles)
        .adaptive_blend(BlendConfig::default());
    let plot = Plot::with_coord(geo.clone())
        .canvas_size(860.0, 600.0)
        .title("Warped tiles — zoom in to morph into Mercator")
        .data(df)
        .mark(
            GeoShape::new()
                .geometry(&geo, col("geometry"))
                .fill_with(col("density"), |c| {
                    c.scale_with::<Log>(|s| {
                        s.range_colors(vec![
                            Srgba::new(1.0, 0.96, 0.92, 1.0),
                            Srgba::new(0.99, 0.68, 0.42, 1.0),
                            Srgba::new(0.85, 0.28, 0.10, 1.0),
                            Srgba::new(0.50, 0.14, 0.05, 1.0),
                        ])
                    })
                })
                .opacity(0.35)
                .stroke("#475569")
                .stroke_width(0.6),
        )
        .tool(GeoPanZoom::new().viewport_id("us").settle_exact(true));

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
