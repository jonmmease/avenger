//! Manual perf probe: per-frame preview cost of the Geo adaptive
//! raster/scatter taxi plot during a simulated wheel zoom.
//!
//! Mirrors `avenger-chart-app/examples/taxi_geo_mercator.rs` at the session
//! level (no tiles — isolates the mark/measurement cost from tile
//! resources). Ignored by default; expects the HoloViz NYC taxi parquet at
//! `scratch/data/nyc_taxi_wide.parquet` relative to the workspace root.
//!
//! ```bash
//! cargo test --release -p avenger-chart-geo --test perf_probe_geo_raster -- --ignored --nocapture
//! ```

use std::{path::PathBuf, sync::Arc, time::Duration, time::Instant};

use avenger_chart::prelude::*;
use avenger_chart::render::EvaluationMetrics;
use avenger_chart_geo::{
    Geo, GeoPanZoom, GeoPositionChannels, GeoUniformRaster2DChannels, Symbol as GeoSymbol,
    UniformRaster2D as GeoUniformRaster2D, crs,
};
use datafusion::{
    arrow::datatypes::DataType,
    datasource::MemTable,
    functions::expr_fn::{abs, floor, log2, power, round},
    logical_expr::{Expr, expr_fn::cast, when},
    prelude::{ParquetReadOptions, SessionContext, col, lit},
    scalar::ScalarValue,
};

fn taxi_max_rows() -> usize {
    std::env::var("AVENGER_PROBE_ROWS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_000_000)
}
const POINT_BUDGET: i64 = 10_000;
const MIN_RASTER_CELL_METERS: f64 = 10.0;
const TAXI_X_MIN: f64 = -8_242_500.0;
const TAXI_X_MAX: f64 = -8_226_500.0;
const TAXI_Y_MIN: f64 = 4_968_000.0;
const TAXI_Y_MAX: f64 = 4_983_000.0;

async fn taxi_dataframe(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scratch/data/nyc_taxi_wide.parquet");
    let df = ctx
        .read_parquet(path.to_str().unwrap(), ParquetReadOptions::default())
        .await
        .unwrap()
        .limit(0, Some(taxi_max_rows()))
        .unwrap()
        .filter(
            col("pickup_x")
                .gt_eq(lit(TAXI_X_MIN))
                .and(col("pickup_x").lt_eq(lit(TAXI_X_MAX)))
                .and(col("pickup_y").gt_eq(lit(TAXI_Y_MIN)))
                .and(col("pickup_y").lt_eq(lit(TAXI_Y_MAX))),
        )
        .unwrap()
        .select_columns(&["pickup_x", "pickup_y"])
        .unwrap();
    let batches = df.collect().await.unwrap();
    let schema = batches.first().unwrap().schema();
    let target_partitions = ctx.state().config().target_partitions().max(1);
    let mut partitions = vec![Vec::new(); target_partitions];
    for (index, batch) in batches.into_iter().enumerate() {
        partitions[index % target_partitions].push(batch);
    }
    partitions.retain(|partition| !partition.is_empty());
    let table = Arc::new(MemTable::try_new(schema, partitions).unwrap());
    ctx.register_table("taxi_pickups", table).unwrap();
    ctx.table("taxi_pickups").await.unwrap()
}

fn meters_x(raw: Expr) -> Expr {
    crs::from_mercator_units_x(crs::EPSG_3857, raw).unwrap()
}

fn meters_y(raw: Expr) -> Expr {
    crs::from_mercator_units_y(crs::EPSG_3857, raw).unwrap()
}

fn raster_bins(start: Expr, stop: Expr, view_pixels: Expr) -> Expr {
    let view_pixels = floor(cast(view_pixels, DataType::Float64) / lit(2.0));
    let span_limited_bins = floor(abs(stop - start) / lit(MIN_RASTER_CELL_METERS));
    let domain_limited_bins = when(span_limited_bins.clone().gt(lit(1.0)), span_limited_bins)
        .otherwise(lit(1.0))
        .unwrap();
    when(
        domain_limited_bins.clone().lt(view_pixels.clone()),
        domain_limited_bins,
    )
    .otherwise(view_pixels)
    .unwrap()
}

fn raster_child(v: &ViewRef, stats: &ScalarAggregateOutput) -> GeoUniformRaster2D<Geo> {
    let gate = stats.scalar("n").gt_eq(lit(POINT_BUDGET));
    let density = cast(stats.scalar("n"), DataType::Float64) / lit(9_000.0);
    let density_floor = when(density.clone().gt(lit(1.0)), density)
        .otherwise(lit(1.0))
        .unwrap();
    let normalizer = power(lit(2.0), round(vec![log2(density_floor)]));
    let x_start = meters_x(v.x().domain_start());
    let x_end = meters_x(v.x().domain_end());
    let y_start = meters_y(v.y().domain_start());
    let y_end = meters_y(v.y().domain_end());
    let x_bins = raster_bins(x_start.clone(), x_end.clone(), v.x().pixels());
    let y_bins = raster_bins(y_start.clone(), y_end.clone(), v.y().pixels());
    GeoUniformRaster2D::new()
        .transform(
            Rasterize2D::new(col("pickup_x"), col("pickup_y"))
                .frame(crs::EPSG_3857)
                .x(|x| x.extent(x_start, x_end).bins(x_bins))
                .y(|y| y.extent(y_start, y_end).bins(y_bins))
                .value(lit(1.0) / normalizer)
                .agg("sum"),
            move |mark, hist| {
                let mark = mark.transform(Filter::new(gate), |mark, _| mark);
                GeoUniformRaster2DChannels::raster_with(mark, hist.raster(), |r| {
                    r.x(hist.x_dim()).y(hist.y_dim()).fill(|fill| {
                        fill.scale_with::<Sqrt>(|scale| {
                            scale.clamp(true).domain((0.0, 1.0)).nice(false).zero(false)
                        })
                    })
                })
            },
        )
        .smooth(false)
}

fn scatter_child(stats: &ScalarAggregateOutput) -> GeoSymbol<Geo> {
    let gate = stats.scalar("n").lt(lit(POINT_BUDGET));
    let raw_x = crs::to_mercator_units_x(crs::EPSG_3857, col("pickup_x")).unwrap();
    let raw_y = crs::to_mercator_units_y(crs::EPSG_3857, col("pickup_y")).unwrap();
    GeoSymbol::new()
        .transform(Filter::new(gate), |mark, _| mark)
        .projected_x(raw_x)
        .projected_y(raw_y)
        .size(12.0)
        .fill("#08519c")
}

fn geo_plot(df: datafusion::dataframe::DataFrame, coord: Geo) -> Plot<Geo> {
    Plot::with_coord(coord)
        .title("probe")
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
        .tool(GeoPanZoom::new().viewport_id("nyc"))
}

async fn wait_for_materializations(session: &PlotSession) {
    for _ in 0..600 {
        if !session.has_pending_materializations() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("materializations did not settle");
}

#[ignore = "manual perf probe; needs the taxi parquet fixture"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn geo_preview_zoom_profile() {
    let ctx = Arc::new(SessionContext::new());
    let df = taxi_dataframe(&ctx).await;
    let coord = Geo::mercator()
        .viewport_id("nyc")
        .center_lon_lat(-73.977, 40.75)
        .zoom(11.0);
    let center_x_param = coord.center_x_param();
    let center_y_param = coord.center_y_param();
    let upp_param = coord.units_per_pixel_param();
    let compiled = Arc::new(geo_plot(df, coord).compile(&ctx).await.unwrap());
    let mut session = compiled.instantiate(ctx);

    let warmup_start = Instant::now();
    let (_plot, warmup) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact())
        .await
        .unwrap();
    println!(
        "warmup exact: {:?} (queued={})",
        warmup_start.elapsed(),
        warmup.pipeline.materialization_queued
    );
    println!("warmup timings: {:#?}", warmup.timings);
    wait_for_materializations(&session).await;

    let _ = session
        .evaluate_with_metrics(EvaluationRequest::new().preview())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(320)).await;
    let (_plot, consume) = session
        .evaluate_with_metrics(EvaluationRequest::new().preview())
        .await
        .unwrap();
    println!(
        "consume preview: data_mark_reuses={} ready_used={}",
        consume.pipeline.preview_data_mark_reuses, consume.pipeline.materialization_ready_used
    );

    // Simulated wheel zoom: shrink units-per-pixel 3% per frame around the
    // authored NYC center (raw units).
    let frames: usize = std::env::var("AVENGER_PROBE_FRAMES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(40);
    let (center_x, center_y) = {
        let projection =
            avenger_geo::projector::Projection::new(avenger_geo::raw::ProjectionKind::Mercator);
        projection.project_raw_units(-73.977, 40.75)
    };
    // zoom 11 on a 960px canvas: world span 2π/2^11 per 512 logical px.
    let base_upp = 2.0 * std::f64::consts::PI / (512.0 * 2.0_f64.powf(11.0));
    let mut frame_times = Vec::new();
    let mut slowest: Option<(usize, Duration, EvaluationMetrics)> = None;
    for frame in 0..frames {
        let upp = base_upp * 0.97_f64.powi(frame as i32 + 1);
        let mut patch = indexmap::IndexMap::new();
        patch.insert(center_x_param.clone(), ScalarValue::Float64(Some(center_x)));
        patch.insert(center_y_param.clone(), ScalarValue::Float64(Some(center_y)));
        patch.insert(upp_param.clone(), ScalarValue::Float64(Some(upp)));
        let start = Instant::now();
        let (_plot, metrics) = session
            .evaluate_with_metrics(
                EvaluationRequest::new()
                    .preview()
                    .param_patch(patch)
                    .options(avenger_chart::render::EvaluationOptions {
                        build_scene_rtree: false,
                        ..Default::default()
                    }),
            )
            .await
            .unwrap();
        let elapsed = start.elapsed();
        frame_times.push((elapsed, metrics.pipeline.preview_data_mark_reuses));
        if slowest.as_ref().is_none_or(|(_, max, _)| elapsed > *max) {
            slowest = Some((frame, elapsed, metrics));
        }
        tokio::time::sleep(Duration::from_millis(8)).await;
    }

    let mut sorted: Vec<Duration> = frame_times.iter().map(|(d, _)| *d).collect();
    sorted.sort();
    let retargets = frame_times.iter().filter(|(_, r)| *r > 0).count();
    println!(
        "zoom frames: n={} retargets={} min={:?} p50={:?} p90={:?} max={:?}",
        sorted.len(),
        sorted.len() - (sorted.len() - retargets),
        sorted[0],
        sorted[sorted.len() / 2],
        sorted[sorted.len() * 9 / 10],
        sorted[sorted.len() - 1],
    );
    for (index, (elapsed, reuses)) in frame_times.iter().enumerate().take(40) {
        println!("frame {index:02}: {elapsed:?} data_mark_reuses={reuses}");
    }
    if let Some((frame, elapsed, metrics)) = slowest {
        println!("slowest frame {frame}: {elapsed:?}");
        println!("timings: {:#?}", metrics.timings);
    }
}
