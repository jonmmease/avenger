//! Adaptive raster-to-scatter taxi demo on the Geo (Mercator) coordinate
//! system, over OSM map tiles.
//!
//! The same group-view shape as `taxi_adaptive_points`, relocated onto a
//! map: a shared in-view pickup count gates an async rasterized density
//! child (>= `POINT_BUDGET` pickups in view) against a synchronous scatter
//! child (below it). The taxi columns are EPSG:3857 meters, so the raster is
//! binned in native meters with `Rasterize2D::frame(crs::EPSG_3857)` and the
//! Geo mark converts its extents to the authored Mercator plane at display
//! time; the scatter positions convert per point via
//! `crs::to_mercator_units_*`. Drag to pan, scroll to zoom around the
//! pointer, double-click to reset.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example taxi_geo_mercator --features winit-wgpu --release
//! ```
//!
//! This example expects the HoloViz NYC taxi parquet at
//! `scratch/data/nyc_taxi_wide.parquet` relative to the workspace root
//! (see `scratch/data/README.md` if present, or download `nyc_taxi_wide`
//! from the HoloViz datasets bucket).

use std::{path::PathBuf, sync::Arc, time::Duration};

use avenger_chart::prelude::*;
use avenger_chart_app::{
    CanvasConfig, ChartAppOptions, ChartResizeBinding, ChartRuntimeResources, WgpuImagePlaceholder,
    WgpuImageResourceConfig, WgpuMissingImagePolicy, WinitWgpuAvengerApp,
    WinitWgpuAvengerAppOptions, chart_avenger_app_with_runtime_resources,
};
use avenger_chart_geo::{
    Geo, GeoPanZoom, GeoPositionChannels, GeoUniformRaster2DChannels, RasterTileLayer, Symbol,
    UniformRaster2D, crs,
};
use avenger_image::ImageResourceCache;
use avenger_resource::RenderInvalidationHub;
use datafusion::{
    arrow::{compute::concat_batches, datatypes::DataType, record_batch::RecordBatch},
    dataframe::DataFrame,
    datasource::MemTable,
    error::{DataFusionError, Result as DataFusionResult},
    functions::expr_fn::{floor, log2, power, round},
    logical_expr::{Expr, expr_fn::cast, when},
    prelude::{ParquetReadOptions, SessionContext},
};
use palette::rgb::Srgba;
use winit::window::WindowAttributes;

const OSM_TILE_TEMPLATE: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";
const TAXI_TABLE: &str = "taxi_pickups";
const TAXI_MAX_ROWS: usize = 11_000_000;
const TAXI_BATCH_ROWS: usize = 8192;
/// Switch to the scatter representation below this in-view pickup count.
const POINT_BUDGET: i64 = 10_000;
/// Dark-blue endpoint of the raster ramp (#08519c), shared with the scatter
/// points so the raster-to-scatter transition is less jarring.
// The green channel happens to round near 1/π; this is a literal sRGB color.
#[allow(clippy::approx_constant)]
const SCATTER_BLUE: Srgba = Srgba::new(0.031, 0.318, 0.612, 1.0);
/// Density normalization for the fill domain (see `taxi_adaptive_points`).
const FILL_DOMAIN_POINTS_DIVISOR: f64 = 9_000.0;
const TAXI_X_MIN: f64 = -8_242_500.0;
const TAXI_X_MAX: f64 = -8_226_500.0;
const TAXI_Y_MIN: f64 = 4_968_000.0;
const TAXI_Y_MAX: f64 = 4_983_000.0;

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
                .with_title("avenger-chart taxi density on Geo mercator tiles")
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

    let tiles = RasterTileLayer::xyz(OSM_TILE_TEMPLATE)
        .id("osm")
        .max_zoom(19)
        .attribution("OpenStreetMap contributors")
        .zindex(-10)
        .smooth_zoom();
    let coord = Geo::mercator()
        .viewport_id("nyc")
        .center_lon_lat(-73.977, 40.75)
        .zoom(11.0)
        .tiles(tiles);

    let plot = Chart::with_coord(coord.clone())
        .title("NYC taxi pickups: raster above 10k in view, points below")
        .canvas_size(960.0, 720.0)
        .mark(
            MarkGroup::<Geo>::new().data(df).view(
                View::cartesian()
                    .id("pickups")
                    .x_domain(col("pickup_x"))
                    .y_domain(col("pickup_y"))
                    .preview_cached(true)
                    .throttle(Duration::from_millis(100)),
                |group, v| {
                    // The view-domain params are authored-plane raw units;
                    // the taxi columns are 3857 meters — compare in meters.
                    let x_start = meters_x(v.x().domain_start());
                    let x_end = meters_x(v.x().domain_end());
                    let y_start = meters_y(v.y().domain_start());
                    let y_end = meters_y(v.y().domain_end());
                    let in_view = col("pickup_x")
                        .gt_eq(x_start)
                        .and(col("pickup_x").lt_eq(x_end))
                        .and(col("pickup_y").gt_eq(y_start))
                        .and(col("pickup_y").lt_eq(y_end));
                    group
                        .transform(Filter::new(in_view), |group, _| group)
                        .transform(ScalarAggregate::new().count("n"), |group, stats| {
                            group
                                .mark(raster_child(&v, &stats))
                                .mark(scatter_child(&stats))
                        })
                },
            ),
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

/// Authored-plane raw-unit expr → EPSG:3857 meters.
fn meters_x(raw: Expr) -> Expr {
    crs::from_mercator_units_x(crs::EPSG_3857, raw).expect("epsg:3857 is a supported frame")
}

fn meters_y(raw: Expr) -> Expr {
    crs::from_mercator_units_y(crs::EPSG_3857, raw).expect("epsg:3857 is a supported frame")
}

/// Async rasterized density, binned in native 3857 meters and tagged with
/// `frame(crs::EPSG_3857)`; the Geo mark converts extents back to the
/// authored plane at display time. Gate and density normalization follow
/// `taxi_adaptive_points`.
fn raster_child(v: &ViewRef, stats: &ScalarAggregateOutput) -> UniformRaster2D<Geo> {
    let gate = stats.scalar("n").gt_eq(lit(POINT_BUDGET));
    let density = cast(stats.scalar("n"), DataType::Float64) / lit(FILL_DOMAIN_POINTS_DIVISOR);
    let density_floor = when(density.clone().gt(lit(1.0)), density)
        .otherwise(lit(1.0))
        .expect("valid density normalizer floor expression");
    let normalizer = power(lit(2.0), round(vec![log2(density_floor)]));
    let x_start = meters_x(v.x().domain_start());
    let x_end = meters_x(v.x().domain_end());
    let y_start = meters_y(v.y().domain_start());
    let y_end = meters_y(v.y().domain_end());
    let x_bins = half_pixel_bins(v.x().pixels());
    let y_bins = half_pixel_bins(v.y().pixels());
    UniformRaster2D::new()
        .transform(
            Rasterize2D::new(col("pickup_x"), col("pickup_y"))
                .frame(crs::EPSG_3857)
                .x(|x| x.extent(x_start, x_end).bins(x_bins))
                .y(|y| y.extent(y_start, y_end).bins(y_bins))
                .value(lit(1.0) / normalizer)
                .agg("sum"),
            move |mark, hist| {
                let mark = mark.transform(Filter::new(gate), |mark, _| mark);
                // UFCS: the chart prelude also globs the Cartesian
                // raster_with trait, which confuses method resolution.
                GeoUniformRaster2DChannels::raster_with(mark, hist.raster(), |r| {
                    r.x(hist.x_dim()).y(hist.y_dim()).fill(|fill| {
                        fill.scale_with::<Sqrt>(|scale| {
                            scale
                                .clamp(true)
                                .domain((0.0, 1.0))
                                .range_colors(vec![
                                    Srgba::new(0.87, 0.92, 0.97, 1.0), // Light blue (#deebf7)
                                    SCATTER_BLUE,
                                ])
                                .nice(false)
                                .zero(false)
                        })
                        .legend(|legend| legend.title("Density"))
                    })
                })
            },
        )
        .smooth(false)
}

/// Synchronous scatter of the exact in-view points, positioned by
/// converting the 3857-meter columns to authored raw units per point.
fn scatter_child(stats: &ScalarAggregateOutput) -> Symbol<Geo> {
    let gate = stats.scalar("n").lt(lit(POINT_BUDGET));
    let raw_x = crs::to_mercator_units_x(crs::EPSG_3857, col("pickup_x"))
        .expect("epsg:3857 is a supported frame");
    let raw_y = crs::to_mercator_units_y(crs::EPSG_3857, col("pickup_y"))
        .expect("epsg:3857 is a supported frame");
    Symbol::new()
        .transform(Filter::new(gate), |mark, _| mark)
        .projected_x(raw_x)
        .projected_y(raw_y)
        .size(20.0)
        .fill("#08519c")
}

/// Bin count = half the view pixels. Must stay whole-numbered: Rasterize2D
/// rejects fractional bin counts, and a failed materialization means the
/// raster child silently never renders.
fn half_pixel_bins(view_pixels: Expr) -> Expr {
    floor(cast(view_pixels, DataType::Float64) / lit(2.0))
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
