//! Manual perf probe: per-frame preview cost of the adaptive raster/scatter
//! taxi plot during a simulated drag.
//!
//! Mirrors `avenger-chart-app/examples/taxi_adaptive_points.rs` at the
//! session level so preview frame costs can be measured headlessly. Ignored
//! by default; expects the HoloViz NYC taxi parquet at
//! `scratch/data/nyc_taxi_wide.parquet` relative to the workspace root.
//!
//! ```bash
//! cargo test --release -p avenger-chart --test perf_probe_adaptive_preview -- --ignored --nocapture
//! ```

use std::{path::PathBuf, sync::Arc, time::Duration, time::Instant};

use avenger_chart::prelude::*;
use avenger_chart::render::EvaluationMetrics;
use datafusion::{
    arrow::datatypes::DataType,
    datasource::MemTable,
    functions::expr_fn::{log2, power, round},
    logical_expr::{expr_fn::cast, when},
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
const TAXI_X_MIN: f64 = -8_242_500.0;
const TAXI_X_MAX: f64 = -8_226_500.0;
const TAXI_Y_MIN: f64 = 4_968_000.0;
const TAXI_Y_MAX: f64 = 4_983_000.0;

fn list_domain(min: f64, max: f64) -> ScalarValue {
    ScalarValue::List(ScalarValue::new_list(
        &[
            ScalarValue::Float64(Some(min)),
            ScalarValue::Float64(Some(max)),
        ],
        &DataType::Float64,
        true,
    ))
}

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

fn adaptive_plot(df: datafusion::dataframe::DataFrame) -> Chart<Cartesian> {
    Chart::with_coord(Cartesian::new().unit_aspect(1.0))
        .title("probe")
        .canvas_size(960.0, 720.0)
        .mark(
            MarkGroup::<Cartesian>::new().data(df).view(
                View::cartesian()
                    .id("pickups")
                    .x_domain(col("pickup_x"))
                    .y_domain(col("pickup_y"))
                    .preview_cached(true)
                    .throttle(Duration::from_millis(100)),
                |group, v| {
                    let in_view = col("pickup_x")
                        .gt_eq(v.x().domain_start())
                        .and(col("pickup_x").lt_eq(v.x().domain_end()))
                        .and(col("pickup_y").gt_eq(v.y().domain_start()))
                        .and(col("pickup_y").lt_eq(v.y().domain_end()));
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
        .tool(PanScrollZoom::cartesian())
}

fn raster_child(v: &ViewRef, stats: &ScalarAggregateOutput) -> UniformRaster2D<Cartesian> {
    let gate = stats.scalar("n").gt_eq(lit(POINT_BUDGET));
    let density = cast(stats.scalar("n"), DataType::Float64) / lit(9_000.0);
    let density_floor = when(density.clone().gt(lit(1.0)), density)
        .otherwise(lit(1.0))
        .unwrap();
    let normalizer = power(lit(2.0), round(vec![log2(density_floor)]));
    UniformRaster2D::new()
        .transform(
            Rasterize2D::new(col("pickup_x"), col("pickup_y"))
                .x(|x| {
                    x.extent(v.x().domain_start(), v.x().domain_end())
                        .bins(v.x().pixels())
                })
                .y(|y| {
                    y.extent(v.y().domain_start(), v.y().domain_end())
                        .bins(v.y().pixels())
                })
                .value(lit(1.0) / normalizer)
                .agg("sum"),
            move |mark, hist| {
                mark.transform(Filter::new(gate), |mark, _| mark)
                    .raster_with(hist.raster(), |r| {
                        r.x_with(hist.x_dim(), |x| {
                            x.scale_with::<Linear>(|scale| scale.nice(false).zero(false))
                        })
                        .y_with(hist.y_dim(), |y| {
                            y.scale_with::<Linear>(|scale| scale.nice(false).zero(false))
                        })
                        .fill(|fill| {
                            fill.scale_with::<Sqrt>(|scale| {
                                scale.clamp(true).domain((0.0, 1.0)).nice(false).zero(false)
                            })
                        })
                    })
            },
        )
        .smooth(false)
}

fn scatter_child(stats: &ScalarAggregateOutput) -> Symbol<Cartesian> {
    let gate = stats.scalar("n").lt(lit(POINT_BUDGET));
    Symbol::new()
        .transform(Filter::new(gate), |mark, _| mark)
        .x(col("pickup_x"))
        .y(col("pickup_y"))
        .size(12.0)
        .fill("#08519c")
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
async fn adaptive_preview_drag_profile() {
    let ctx = Arc::new(SessionContext::new());
    let df = taxi_dataframe(&ctx).await;
    let compiled = Arc::new(adaptive_plot(df).compile(&ctx).await.unwrap());
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
    wait_for_materializations(&session).await;

    // Consume the warm raster (defer frame + stability window + consume).
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

    // Simulated drag: preview frames panning the window right by 40m each.
    // AVENGER_PROBE_FRAMES overrides the frame count (for profiling runs).
    let frames: usize = std::env::var("AVENGER_PROBE_FRAMES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(40);
    let span = 4_000.0_f64;
    let x0 = -8_238_000.0_f64;
    let y0 = 4_973_000.0_f64;
    let y_span = 3_000.0_f64;
    let mut frame_times = Vec::new();
    let mut slowest: Option<(usize, Duration, EvaluationMetrics)> = None;
    for frame in 0..frames {
        let shift = (frame % 200) as f64 * 40.0;
        let mut patch = indexmap::IndexMap::new();
        patch.insert(
            "__tool_pan_scroll_zoom__x_domain".to_string(),
            list_domain(x0 + shift, x0 + span + shift),
        );
        patch.insert(
            "__tool_pan_scroll_zoom__y_domain".to_string(),
            list_domain(y0, y0 + y_span),
        );
        let start = Instant::now();
        // Match the app: it skips the scene rtree on every evaluation.
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
        "drag frames: n={} retargets={} min={:?} p50={:?} p90={:?} max={:?}",
        sorted.len(),
        retargets,
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
