use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart_app::{
    CanvasConfig, ChartAppOptions, ChartResizeBinding, ChartRuntimeResources, WgpuImagePlaceholder,
    WgpuImageResourceConfig, WgpuMissingImagePolicy, WindowSceneSizing, WinitWgpuAvengerApp,
    WinitWgpuAvengerAppOptions, chart_avenger_app_with_default_runtime_resources,
};
use avenger_chart_geo::{Geo, GeoPanZoom, GeoPositionChannels, RasterTileLayer, Symbol};
use datafusion::prelude::SessionContext;
use winit::window::WindowAttributes;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const CARTO_TILE_TEMPLATE: &str =
    "https://basemaps.cartocdn.com/rastertiles/voyager_nolabels/{z}/{x}/{y}.png";

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub async fn run() {
    init_diagnostics();

    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            let bundle = build_app().await;
            let options = app_options(&bundle.runtime_resources);
            let (mut app, event_loop) =
                WinitWgpuAvengerApp::new_and_event_loop_with_options(bundle.app, options);
            event_loop.run_app(&mut app).expect("run app");
        } else {
            let tokio_runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("build tokio runtime");
            let bundle = tokio_runtime.block_on(build_app());
            let options = app_options(&bundle.runtime_resources);
            let (mut app, event_loop) = WinitWgpuAvengerApp::new_and_event_loop_with_options(
                bundle.app,
                options,
                tokio_runtime,
            );
            event_loop.run_app(&mut app).expect("run app");
        }
    }
}

fn app_options(runtime_resources: &ChartRuntimeResources) -> WinitWgpuAvengerAppOptions {
    let canvas_config = CanvasConfig {
        image_resource_config: WgpuImageResourceConfig {
            resolver: Some(runtime_resources.image_resource_resolver.clone()),
            missing_policy: WgpuMissingImagePolicy::DrawPlaceholder,
            placeholder: WgpuImagePlaceholder::Checkerboard,
        },
        ..CanvasConfig::default()
    };
    WinitWgpuAvengerAppOptions::new(2.0)
        .window_attributes(
            WindowAttributes::default()
                .with_title("avenger-chart Mercator map tiles")
                .with_resizable(false),
        )
        .window_scene_sizing(WindowSceneSizing::MatchSceneGraph)
        .canvas_config(canvas_config)
        .render_invalidation_hub(runtime_resources.render_invalidation_hub.clone())
}

async fn build_app() -> avenger_chart_app::ChartAppBundle {
    let ctx = Arc::new(SessionContext::new());
    let landmarks = ctx
        .sql(
            "SELECT * FROM (VALUES
                ('Times Square',         -73.9855, 40.7580, '#e11d48'),
                ('Grand Central',        -73.9772, 40.7527, '#2563eb'),
                ('Empire State',         -73.9857, 40.7484, '#16a34a'),
                ('Bryant Park',          -73.9832, 40.7536, '#ca8a04'),
                ('Madison Square Park',  -73.9880, 40.7420, '#7c3aed')
            ) AS t(name, lon, lat, color)",
        )
        .await
        .expect("build landmark data");

    let tiles = RasterTileLayer::xyz(CARTO_TILE_TEMPLATE)
        .id("carto")
        .max_zoom(19)
        .attribution("© CARTO, © OpenStreetMap contributors")
        .smooth_zoom();
    let geo = Geo::mercator()
        .viewport_id("nyc")
        .center_lon_lat(-73.9855, 40.7505)
        .zoom(13.0)
        .tiles(tiles);

    let plot = Plot::with_coord(geo.clone())
        .canvas_size(920.0, 620.0)
        .configure_title("Mercator tiles: $z = 13$", |title| title.typst())
        .data(landmarks)
        .mark(
            Symbol::new()
                .lon_lat(&geo, col("lon"), col("lat"))
                .fill_with(col("color"), |fill| {
                    fill.no_scale().legend(|legend| legend.visible(false))
                })
                .stroke("#ffffff")
                .stroke_width(1.6)
                .size(140.0),
        )
        .tool(GeoPanZoom::new().viewport_id("nyc").settle_exact(true));

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
