//! Interactive Equal Earth world map: land polygons, great-circle flight
//! routes, airports, sphere + graticule — pan/zoom on a non-WebMercator
//! coordinate space (scratch/geo phase 5a).
//!
//! Pan with the left mouse button, scroll to zoom around the pointer,
//! Shift-drag to box zoom, and double-click to reset to the world view.
//! Great-circle routes re-resample adaptively at every view change, so
//! zooming in keeps the arcs smooth.
//!
//! Run with:
//! ```bash
//! cargo run --release -p avenger-chart-app --example geo_equal_earth_world --features winit-wgpu
//! ```

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    ChartAppOptions, ChartResizeBinding, WinitWgpuAvengerApp, WinitWgpuAvengerAppOptions,
    chart_avenger_app,
};
use avenger_chart_geo::{
    Geo, GeoPanZoom, GeoPositionChannels, GeoShape, GraticuleStyle, Line, SphereStyle, Symbol,
    register_geojson,
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
            .with_title("avenger-chart Geo: Equal Earth world")
            .with_resizable(false),
    );
    let (mut app, event_loop) =
        WinitWgpuAvengerApp::new_and_event_loop_with_options(avenger_app, options, tokio_runtime);
    event_loop.run_app(&mut app).expect("run app");
}

async fn build_app() -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let ctx = Arc::new(SessionContext::new());
    let land_path = format!(
        "{}/../avenger-chart/tests/data/geo/ne_110m_land.geojson",
        env!("CARGO_MANIFEST_DIR")
    );
    let land = register_geojson(&ctx, "land", land_path)
        .await
        .expect("register land");
    let routes = ctx
        .sql(
            "SELECT * FROM (VALUES
                ('JFK-LHR', 0, -73.78, 40.64), ('JFK-LHR', 1, -0.45, 51.47),
                ('JFK-NRT', 0, -73.78, 40.64), ('JFK-NRT', 1, 140.39, 35.76),
                ('JFK-SYD', 0, -73.78, 40.64), ('JFK-SYD', 1, 151.18, -33.95),
                ('JFK-GRU', 0, -73.78, 40.64), ('JFK-GRU', 1, -46.47, -23.43),
                ('JFK-SIN', 0, -73.78, 40.64), ('JFK-SIN', 1, 103.99, 1.36)
            ) AS t(route, seq, lon, lat)",
        )
        .await
        .expect("route data");

    let geo = Geo::equal_earth()
        .viewport_id("world")
        .sphere(SphereStyle::default())
        .graticule(GraticuleStyle::default());
    let plot = Chart::with_coord(geo.clone())
        .canvas_size(900.0, 560.0)
        .title("Great-circle routes — drag to pan, scroll to zoom")
        .mark(
            GeoShape::new()
                .data(land)
                .geometry(&geo, col("geometry"))
                .fill("#d1d5db")
                .stroke("#9ca3af")
                .stroke_width(0.3),
        )
        .mark(
            Line::new()
                .data(routes.clone())
                .lon_lat(&geo, "lon", "lat")
                .details(["route"])
                .order(col("seq"))
                .stroke_with(col("route"), |c| c.legend(|l| l.visible(false)))
                .stroke_width(1.6),
        )
        .mark(
            Symbol::new()
                .data(routes)
                .lon_lat(&geo, "lon", "lat")
                .size(26.0)
                .fill("#111827"),
        )
        .tool(GeoPanZoom::new().viewport_id("world").settle_exact(true));

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
