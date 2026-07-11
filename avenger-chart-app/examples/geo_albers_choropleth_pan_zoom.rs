//! Interactive US-states choropleth on the CONUS Albers projection —
//! pan/zoom on a non-WebMercator coordinate space (scratch/geo phase 5a).
//!
//! Pan with the left mouse button, scroll to zoom around the pointer,
//! Shift-drag to box zoom, and double-click to reset to the fitted view.
//! The graticule and state polygons re-project through the adaptive
//! resampler at every view change.
//!
//! Run with:
//! ```bash
//! cargo run --release -p avenger-chart-app --example geo_albers_choropleth_pan_zoom --features winit-wgpu
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use avenger_chart_geo::{Geo, GeoPanZoom, GeoShape, GraticuleStyle, register_geojson};
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
            .with_title("avenger-chart Geo: Albers choropleth pan/zoom")
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

    let geo = Geo::albers_usa_conus()
        .viewport_id("us")
        .graticule(GraticuleStyle::default());
    let plot = Chart::with_coord(geo.clone())
        .canvas_size(860.0, 600.0)
        .title("Population density — drag to pan, scroll to zoom")
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
                .stroke("#ffffff")
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
