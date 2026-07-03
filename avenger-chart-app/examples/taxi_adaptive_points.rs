//! Adaptive raster-to-scatter taxi demo.
//!
//! A group view scope shares one in-view pickup count between two child
//! marks: an async rasterized density view (shown while the viewport holds
//! at least `POINT_BUDGET` pickups) and a synchronous scatter plot of the
//! exact points (shown below the budget). Drag to pan and scroll to zoom;
//! zoom into a block-sized region to watch the representation switch to
//! individual points, and zoom back out to return to the density raster.
//!
//! The switch decision is exact on every evaluation: the shared in-view
//! count is an eager `ScalarAggregate` executed once per evaluation, and
//! both children's gates read the same scalar. The raster keeps
//! materializing in the background while in scatter mode, so switching back
//! retargets a warm raster.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart-app --example taxi_adaptive_points --features winit-wgpu --release
//! ```
//!
//! To make the cached-raster handoff easier to see:
//! ```bash
//! AVENGER_TAXI_RASTER_DELAY_MS=250 cargo run -p avenger-chart-app --example taxi_adaptive_points --features winit-wgpu --release
//! ```
//!
//! Set `RUST_LOG=avenger_chart_transforms=debug` to trace the eager
//! count-scan latency per evaluation (`scalar_aggregate_eager` spans), and
//! `RUST_LOG=avenger_chart::transforms::rasterize_2d=debug` for raster
//! query diagnostics. Per-frame evaluation metrics (including
//! materialization request and fallback counters) print via `log_metrics`.
//!
//! This example expects the HoloViz NYC taxi parquet at
//! `scratch/data/nyc_taxi_wide.parquet` relative to the workspace root.

use std::{path::PathBuf, sync::Arc, time::Duration};

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
    logical_expr::{Expr, expr_fn::cast, when},
    prelude::{ParquetReadOptions, SessionContext},
};
use winit::window::WindowAttributes;

const TAXI_TABLE: &str = "taxi_pickups";
const TAXI_MAX_ROWS: usize = 1_000_000;
const TAXI_BATCH_ROWS: usize = 8192;
/// Switch to the scatter representation below this in-view pickup count.
const POINT_BUDGET: i64 = 10_000;
// Taxi coordinates are projected meters; keep deep-zoom raster cells aggregating nearby trips.
const MIN_RASTER_PIXEL_DOMAIN_SIZE: f64 = 10.0;
const TAXI_X_MIN: f64 = -8_242_500.0;
const TAXI_X_MAX: f64 = -8_226_500.0;
const TAXI_Y_MIN: f64 = 4_968_000.0;
const TAXI_Y_MAX: f64 = 4_983_000.0;

fn main() {
    init_diagnostics();
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
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
                .with_title("avenger-chart adaptive raster/scatter taxi demo")
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
        .title("NYC taxi pickups: raster above 10k in view, points below")
        .canvas_size(960.0, 720.0)
        .mark(
            MarkGroup::<Cartesian>::new().data(df).view(
                View::cartesian()
                    .id("pickups")
                    .x_domain(col("pickup_x"))
                    .y_domain(col("pickup_y"))
                    .preview_cached(true)
                    // Rate-limit preview rasterizations during drag/zoom:
                    // without a throttle every pointer-move frame starts a
                    // new raster, and each mid-gesture completion forces a
                    // data-mark rebuild instead of a cheap retarget. Settled
                    // (gesture-release) requests bypass the throttle.
                    .throttle(Duration::from_millis(100)),
                |group, v| {
                    let in_view = col("pickup_x")
                        .gt_eq(v.x().domain_start())
                        .and(col("pickup_x").lt_eq(v.x().domain_end()))
                        .and(col("pickup_y").gt_eq(v.y().domain_start()))
                        .and(col("pickup_y").lt_eq(v.y().domain_end()));
                    group
                        // Group view-local: the shared in-view filter and the
                        // single eager count both children's gates read.
                        .transform(Filter::new(in_view), |group, _| group)
                        .transform(ScalarAggregate::new().count("n"), |group, stats| {
                            group
                                .mark(raster_child(&v, &stats))
                                .mark(scatter_child(&stats))
                        })
                },
            ),
        )
        // Stay in Preview across the whole interaction (no settle-exact):
        // exact evaluations re-measure axis tick labels, which can change the
        // plot-area size on gesture release and make the view jump. Preview
        // reuses the measured layout profile, and freshly completed rasters
        // still swap in: once the desired materialization is ready and the
        // view has stopped moving for the consume stability window, the
        // preview rebuilds data marks and renders it.
        .tool(PanScrollZoom::cartesian());

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

/// Async rasterized density, gated to viewports holding at least the budget.
/// The threshold gate sits AFTER `Rasterize2D`, dropping the raster row in
/// scatter mode: the gate value never enters the materialization key, and
/// background rasters keep the stale-fallback cache warm for the switch
/// back.
fn raster_child(v: &ViewRef, stats: &ScalarAggregateOutput) -> UniformRaster2D<Cartesian> {
    let gate = stats.scalar("n").gt_eq(lit(POINT_BUDGET));
    UniformRaster2D::new()
        .transform(
            Rasterize2D::new(col("pickup_x"), col("pickup_y"))
                .x(|x| {
                    x.extent(v.x().domain_start(), v.x().domain_end()).bins(
                        raster_bins_with_min_domain_size(
                            v.x().domain_start(),
                            v.x().domain_end(),
                            v.x().pixels(),
                        ),
                    )
                })
                .y(|y| {
                    y.extent(v.y().domain_start(), v.y().domain_end()).bins(
                        raster_bins_with_min_domain_size(
                            v.y().domain_start(),
                            v.y().domain_end(),
                            v.y().pixels(),
                        ),
                    )
                })
                .agg("count"),
            move |mark, hist| {
                mark.transform(Filter::new(gate), |mark, _| mark)
                    .raster_with(hist.raster(), |r| {
                        r.x_with(hist.x_dim(), |x| {
                            x.scale_with::<Linear>(|scale| scale.nice(false).zero(false))
                                .axis(|axis| axis.title("Pickup x").tick_count(4).format(".4~s"))
                        })
                        .y_with(hist.y_dim(), |y| {
                            y.scale_with::<Linear>(|scale| scale.nice(false).zero(false))
                                .axis(|axis| axis.title("Pickup y").tick_count(4).format(".4~s"))
                        })
                        .fill(|fill| {
                            // Explicit fill domain: inferred (visible-domain)
                            // fill scales currently break the preview path
                            // when an async raster first becomes ready (see
                            // the ignored *_inferred_fill_preview_after_ready
                            // session tests), and an explicit domain also
                            // keeps the colorbar stable during pan/zoom.
                            fill.scale_with::<Sqrt>(|scale| {
                                scale.domain((0.0, 120.0)).nice(false).zero(false)
                            })
                            .legend(|legend| legend.title("Trips"))
                        })
                    })
            },
        )
        .smooth(false)
}

/// Synchronous scatter of the exact in-view points, gated below the budget.
/// Its input is the group's shared filtered dataframe, so it never rescans
/// the source table.
fn scatter_child(stats: &ScalarAggregateOutput) -> Symbol<Cartesian> {
    let gate = stats.scalar("n").lt(lit(POINT_BUDGET));
    Symbol::new()
        .transform(Filter::new(gate), |mark, _| mark)
        .x(col("pickup_x"))
        .y(col("pickup_y"))
        .size(12.0)
        .fill("rgba(31, 119, 180, 0.6)")
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
