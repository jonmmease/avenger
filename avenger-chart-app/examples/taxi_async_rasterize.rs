//! Native async taxi rasterization demo.
//!
//! Drag with the left mouse button inside the plot area to pan, scroll to zoom,
//! and watch the previous ready raster retarget while the next view-dependent
//! raster is computed off the interaction path.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example taxi_async_rasterize --features winit-wgpu --release
//! ```
//!
//! To make the cached-raster handoff easier to see:
//! ```bash
//! AVENGER_TAXI_RASTER_DELAY_MS=250 cargo run -p avenger-chart-app --example taxi_async_rasterize --features winit-wgpu --release
//! ```
//!
//! Set `RUST_LOG=avenger_chart::transforms::rasterize_2d=debug,avenger_chart::marks::uniform_raster_2d=debug`
//! for query/raster construction diagnostics. The app also prints per-frame
//! evaluation metrics, including materialization request and fallback counters.

use std::{path::PathBuf, sync::Arc};

use avenger_chart::prelude::*;
use avenger_chart_app::{
    CanvasConfig, ChartAppOptions, ChartResizeBinding, ChartRuntimeResources, WgpuImagePlaceholder,
    WgpuImageResourceConfig, WgpuMissingImagePolicy, WinitWgpuAvengerApp,
    WinitWgpuAvengerAppOptions, chart_avenger_app_with_runtime_resources,
};
use avenger_image::ImageResourceCache;
use avenger_resource::RenderInvalidationHub;
use datafusion::prelude::{CsvReadOptions, SessionContext};
use winit::window::WindowAttributes;

const TAXI_X_MIN: f64 = -8_242_500.0;
const TAXI_X_MAX: f64 = -8_226_500.0;
const TAXI_Y_MIN: f64 = 4_968_000.0;
const TAXI_Y_MAX: f64 = 4_983_000.0;

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
                .with_title("avenger-chart async taxi rasterization")
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
    let taxi_path = taxi_fixture_path();
    let df = ctx
        .read_csv(
            taxi_path
                .to_str()
                .expect("taxi fixture path should be valid UTF-8"),
            CsvReadOptions::new(),
        )
        .await
        .expect("load NYC taxi fixture")
        .filter(
            col("pickup_x")
                .gt_eq(lit(TAXI_X_MIN))
                .and(col("pickup_x").lt_eq(lit(TAXI_X_MAX)))
                .and(col("pickup_y").gt_eq(lit(TAXI_Y_MIN)))
                .and(col("pickup_y").lt_eq(lit(TAXI_Y_MAX))),
        )
        .expect("filter taxi fixture to valid projected pickup coordinates");

    let plot = Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .title("NYC taxi pickup density")
        .canvas_size(960.0, 720.0)
        .data(df)
        .mark(
            UniformRaster2D::new()
                .view(
                    View::cartesian()
                        .id("pickup_density")
                        .x_domain(col("pickup_x"))
                        .y_domain(col("pickup_y"))
                        .preview_cached(true),
                    |mark, view| {
                        mark.transform(
                            Rasterize2D::new(col("pickup_x"), col("pickup_y"))
                                .x(|x| {
                                    x.extent(view.x().domain_start(), view.x().domain_end())
                                        .bins(view.x().pixels())
                                })
                                .y(|y| {
                                    y.extent(view.y().domain_start(), view.y().domain_end())
                                        .bins(view.y().pixels())
                                })
                                .agg("count"),
                            |mark, hist| {
                                mark.raster_with(hist.raster(), |r| {
                                    r.x_with(hist.x_dim(), |x| {
                                        x.scale_with::<Linear>(|scale| {
                                            scale.nice(false).zero(false)
                                        })
                                        .axis(|axis| {
                                            axis.title("Pickup x").tick_count(4).format(".4~s")
                                        })
                                    })
                                    .y_with(hist.y_dim(), |y| {
                                        y.scale_with::<Linear>(|scale| {
                                            scale.nice(false).zero(false)
                                        })
                                        .axis(|axis| {
                                            axis.title("Pickup y").tick_count(4).format(".4~s")
                                        })
                                    })
                                    .fill(|fill| {
                                        fill.scale_with::<Sqrt>(|scale| {
                                            scale.domain((0.0, 120.0)).nice(false).zero(false)
                                        })
                                        .legend(|legend| legend.title("Trips"))
                                    })
                                })
                            },
                        )
                    },
                )
                .smooth(false),
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

fn taxi_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../avenger-chart/tests/data/nyc_taxi_2015/nyc_taxi.csv")
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
