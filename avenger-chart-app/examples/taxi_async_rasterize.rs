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
use avenger_image::ImageResourceCache;
use avenger_resource::RenderInvalidationHub;
use datafusion::{
    arrow::{compute::concat_batches, datatypes::DataType, record_batch::RecordBatch},
    dataframe::DataFrame,
    datasource::MemTable,
    error::{DataFusionError, Result as DataFusionResult},
    functions::expr_fn::{abs, floor},
    logical_expr::{expr_fn::cast, when},
    prelude::{ParquetReadOptions, SessionContext},
};
use winit::window::WindowAttributes;

const TAXI_TABLE: &str = "taxi_pickups";
const TAXI_MAX_ROWS: usize = 11_000_000;
const TAXI_BATCH_ROWS: usize = 8192;
// Taxi coordinates are projected meters; keep deep-zoom raster cells aggregating nearby trips.
const MIN_RASTER_PIXEL_DOMAIN_SIZE: f64 = 10.0;
const TAXI_X_MIN: f64 = -8_242_500.0;
const TAXI_X_MAX: f64 = -8_226_500.0;
const TAXI_Y_MIN: f64 = 4_968_000.0;
const TAXI_Y_MAX: f64 = 4_983_000.0;

fn main() {
    init_diagnostics();
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
    let df = cached_taxi_dataframe(&ctx)
        .await
        .expect("load cached NYC taxi fixture");

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
                                        .bins(raster_bins_with_min_domain_size(
                                            view.x().domain_start(),
                                            view.x().domain_end(),
                                            view.x().pixels(),
                                        ))
                                })
                                .y(|y| {
                                    y.extent(view.y().domain_start(), view.y().domain_end())
                                        .bins(raster_bins_with_min_domain_size(
                                            view.y().domain_start(),
                                            view.y().domain_end(),
                                            view.y().pixels(),
                                        ))
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
                                            scale.nice(false).zero(false)
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

fn raster_bins_with_min_domain_size(start: Expr, stop: Expr, view_pixels: Expr) -> Expr {
    let view_pixels = cast(view_pixels, DataType::Float64);
    let span_limited_bins = floor(abs(stop - start) / lit(MIN_RASTER_PIXEL_DOMAIN_SIZE));
    let domain_limited_bins = when(span_limited_bins.clone().gt(lit(1.0)), span_limited_bins)
        .otherwise(lit(1.0))
        .expect("valid minimum raster bin count expression");
    when(
        domain_limited_bins.clone().lt(view_pixels.clone()),
        domain_limited_bins,
    )
    .otherwise(view_pixels)
    .expect("valid raster bin count expression")
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
