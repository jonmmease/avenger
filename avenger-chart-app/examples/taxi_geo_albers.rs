//! NYC taxi density raster on Albers USA under the adaptive blend: the
//! density image warps in lockstep with tiles and graticule through every
//! blend step.
//!
//! A static 3857-framed density raster (fixed extents over the taxi bbox)
//! renders through the warped mesh path on `Geo::albers_usa_conus()`. Zoom
//! toward NYC and the adaptive blend morphs the projection into Web
//! Mercator — tiles, graticule, and the density raster all follow the same
//! blended view projector. Drag to pan, scroll to zoom, double-click to
//! reset.
//!
//! Unlike `taxi_geo_mercator`, the raster here is NOT view-adaptive: view
//! domain params on Albers are Albers-plane units, and converting them to
//! the taxi columns' EPSG:3857 meters needs inverse-projection expressions
//! that don't exist yet. Static extents keep the demo honest.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example taxi_geo_albers --features winit-wgpu --release
//! ```
//!
//! This example expects the HoloViz NYC taxi parquet at
//! `scratch/data/nyc_taxi_wide.parquet` relative to the workspace root.

use std::{path::PathBuf, sync::Arc};

use avenger_chart::prelude::*;
use avenger_chart_app::{
    CanvasConfig, ChartAppOptions, ChartResizeBinding, ChartRuntimeResources, WgpuImagePlaceholder,
    WgpuImageResourceConfig, WgpuMissingImagePolicy, WinitWgpuAvengerApp,
    WinitWgpuAvengerAppOptions, chart_avenger_app_with_runtime_resources,
};
use avenger_chart_geo::{
    BlendConfig, Geo, GeoPanZoom, GeoUniformRaster2DChannels, GraticuleStyle, RasterTileLayer,
    UniformRaster2D, crs,
};
use avenger_image::ImageResourceCache;
use avenger_resource::RenderInvalidationHub;
use datafusion::{
    arrow::{compute::concat_batches, record_batch::RecordBatch},
    dataframe::DataFrame,
    datasource::MemTable,
    error::{DataFusionError, Result as DataFusionResult},
    prelude::{ParquetReadOptions, SessionContext},
};
use palette::rgb::Srgba;
use winit::window::WindowAttributes;

const CARTO_TILE_TEMPLATE: &str =
    "https://basemaps.cartocdn.com/rastertiles/voyager_nolabels/{z}/{x}/{y}.png";
const TAXI_TABLE: &str = "taxi_pickups";
const TAXI_MAX_ROWS: usize = 1_000_000;
const TAXI_BATCH_ROWS: usize = 8192;
const TAXI_X_MIN: f64 = -8_242_500.0;
const TAXI_X_MAX: f64 = -8_226_500.0;
const TAXI_Y_MIN: f64 = 4_968_000.0;
const TAXI_Y_MAX: f64 = 4_983_000.0;
/// Static raster resolution over the taxi bbox (~31 m cells).
const RASTER_X_BINS: i64 = 512;
const RASTER_Y_BINS: i64 = 480;

fn main() {
    init_diagnostics();
    let taxi_path = taxi_fixture_path();
    if !taxi_path.exists() {
        eprintln!(
            "taxi fixture not found at {}\nDownload the HoloViz `nyc_taxi_wide.parquet` dataset \
             and place it there before running this example.",
            taxi_path.display()
        );
        std::process::exit(1);
    }
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
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
                .with_title("avenger-chart taxi density on Albers under the blend")
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
    let df = cached_taxi_dataframe(&ctx)
        .await
        .expect("load cached NYC taxi fixture");

    let tiles = RasterTileLayer::xyz(CARTO_TILE_TEMPLATE)
        .id("carto")
        .max_zoom(19)
        .attribution("© OpenStreetMap contributors © CARTO")
        .zindex(-10)
        .smooth_zoom();
    let coord = Geo::albers_usa_conus()
        .viewport_id("nyc")
        .center_lon_lat(-73.977, 40.75)
        .zoom(8.0)
        .graticule(GraticuleStyle::default())
        .tiles(tiles)
        .adaptive_blend(BlendConfig {
            z0: 8.5,
            z1: 11.5,
            ..Default::default()
        });

    let plot = Chart::with_coord(coord.clone())
        .title("Taxi density warps with the blend")
        .canvas_size(960.0, 720.0)
        .data(df)
        .mark(
            UniformRaster2D::<Geo>::new()
                .transform(
                    Rasterize2D::new(col("pickup_x"), col("pickup_y"))
                        .frame(crs::EPSG_3857)
                        .x(|x| {
                            x.extent(lit(TAXI_X_MIN), lit(TAXI_X_MAX))
                                .bins(lit(RASTER_X_BINS))
                        })
                        .y(|y| {
                            y.extent(lit(TAXI_Y_MIN), lit(TAXI_Y_MAX))
                                .bins(lit(RASTER_Y_BINS))
                        })
                        .value(lit(1.0))
                        .agg("sum"),
                    |mark, hist| {
                        // UFCS: the chart prelude also globs the Cartesian
                        // raster_with trait.
                        GeoUniformRaster2DChannels::raster_with(mark, hist.raster(), |r| {
                            r.x(hist.x_dim()).y(hist.y_dim()).fill(|fill| {
                                fill.scale_with::<Sqrt>(|scale| {
                                    scale
                                        .clamp(true)
                                        .domain((0.0, 150.0))
                                        .range_colors(vec![
                                            Srgba::new(0.87, 0.92, 0.97, 1.0),
                                            Srgba::new(0.031, 0.318, 0.612, 1.0),
                                        ])
                                        .nice(false)
                                        .zero(false)
                                })
                                .legend(|legend| legend.title("Trips"))
                            })
                        })
                    },
                )
                .smooth(false),
        )
        .tool(GeoPanZoom::new().viewport_id("nyc"));

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    chart_avenger_app_with_runtime_resources(
        compiled,
        ctx,
        ChartAppOptions {
            resize_binding: ChartResizeBinding::none(),
            resize_throttle_ms: None,
            exact_on_resize_settle: false,
            log_metrics: true,
        },
        runtime_resources,
    )
    .await
    .expect("build chart app")
}

async fn cached_taxi_dataframe(ctx: &SessionContext) -> DataFusionResult<DataFrame> {
    let taxi_path = taxi_fixture_path();
    let df = ctx
        .read_parquet(
            taxi_path
                .to_str()
                .expect("taxi fixture path should be valid UTF-8"),
            ParquetReadOptions::default(),
        )
        .await?
        .limit(0, Some(TAXI_MAX_ROWS))?
        .filter(
            col("pickup_x")
                .gt_eq(lit(TAXI_X_MIN))
                .and(col("pickup_x").lt_eq(lit(TAXI_X_MAX)))
                .and(col("pickup_y").gt_eq(lit(TAXI_Y_MIN)))
                .and(col("pickup_y").lt_eq(lit(TAXI_Y_MAX))),
        )?
        .select_columns(&["pickup_x", "pickup_y"])?;
    let batches = rechunk_record_batches(df.collect().await?, TAXI_BATCH_ROWS)?;
    let schema = batches
        .first()
        .map(RecordBatch::schema)
        .ok_or_else(|| DataFusionError::Execution("taxi fixture produced no rows".to_string()))?;
    let partitions = partition_record_batches(batches, ctx.state().config().target_partitions());
    let table = Arc::new(MemTable::try_new(schema, partitions)?);
    ctx.register_table(TAXI_TABLE, table)?;
    ctx.table(TAXI_TABLE).await
}

fn rechunk_record_batches(
    batches: Vec<RecordBatch>,
    target_rows: usize,
) -> DataFusionResult<Vec<RecordBatch>> {
    if target_rows == 0 {
        return Err(DataFusionError::Execution(
            "target_rows must be greater than zero".to_string(),
        ));
    }
    let Some(schema) = batches.first().map(RecordBatch::schema) else {
        return Ok(Vec::new());
    };
    let mut output = Vec::new();
    let mut pending = Vec::new();
    let mut pending_rows = 0usize;

    for batch in batches {
        let mut offset = 0usize;
        while offset < batch.num_rows() {
            let available = batch.num_rows() - offset;
            let needed = target_rows - pending_rows;
            let take = available.min(needed);
            pending.push(batch.slice(offset, take));
            pending_rows += take;
            offset += take;

            if pending_rows == target_rows {
                output.push(concat_batches(&schema, pending.iter())?);
                pending.clear();
                pending_rows = 0;
            }
        }
    }
    if !pending.is_empty() {
        output.push(concat_batches(&schema, pending.iter())?);
    }
    Ok(output)
}

fn partition_record_batches(
    batches: Vec<RecordBatch>,
    target_partitions: usize,
) -> Vec<Vec<RecordBatch>> {
    let partition_count = target_partitions.max(1).min(batches.len().max(1));
    let mut partitions = vec![Vec::new(); partition_count];
    for (index, batch) in batches.into_iter().enumerate() {
        partitions[index % partition_count].push(batch);
    }
    partitions
        .into_iter()
        .filter(|partition| !partition.is_empty())
        .collect()
}

fn taxi_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scratch/data/nyc_taxi_wide.parquet")
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
