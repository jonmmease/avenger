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
    functions::expr_fn::{floor, log2, power, round},
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

fn half_pixel_bins(view_pixels: Expr) -> Expr {
    floor(cast(view_pixels, DataType::Float64) / lit(2.0))
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
    let x_bins = half_pixel_bins(v.x().pixels());
    let y_bins = half_pixel_bins(v.y().pixels());
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
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
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
        let (plot, metrics) = session
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
        frame_times.push((
            elapsed,
            (
                metrics.pipeline.preview_data_mark_reuses,
                metrics.pipeline.materialization_ready_used,
                metrics.pipeline.materialization_stale_fallback_used,
            ),
            displayed_raster_rect(&plot.scene_graph),
        ));
        if slowest.as_ref().is_none_or(|(_, max, _)| elapsed > *max) {
            slowest = Some((frame, elapsed, metrics));
        }
        tokio::time::sleep(Duration::from_millis(8)).await;
    }

    let mut sorted: Vec<Duration> = frame_times.iter().map(|(d, _, _)| *d).collect();
    sorted.sort();
    let retargets = frame_times.iter().filter(|(_, r, _)| r.0 > 0).count();
    println!(
        "zoom frames: n={} retargets={} min={:?} p50={:?} p90={:?} max={:?}",
        sorted.len(),
        sorted.len() - (sorted.len() - retargets),
        sorted[0],
        sorted[sorted.len() / 2],
        sorted[sorted.len() * 9 / 10],
        sorted[sorted.len() - 1],
    );
    for (index, (elapsed, (reuses, ready, fallback), rect)) in
        frame_times.iter().enumerate().take(40)
    {
        println!(
            "frame {index:02}: {elapsed:?} data_mark_reuses={reuses} ready={ready} fallback={fallback} raster_rect={rect:?}"
        );
    }
    if let Some((frame, elapsed, metrics)) = slowest {
        println!("slowest frame {frame}: {elapsed:?}");
        println!("timings: {:#?}", metrics.timings);
    }

    // The displayed stale raster must progress monotonically toward the
    // current view: its retargeted width grows a few percent per frame and
    // resets to roughly plot size when a fresher raster swaps in. A jump
    // BACK to a much larger rect means the fallback regressed to an older
    // raster (the flash-between-rasters bug).
    let widths: Vec<f32> = frame_times
        .iter()
        .filter_map(|(_, _, rect)| rect.map(|[_, _, width, _]| width))
        .collect();
    for pair in widths.windows(2) {
        assert!(
            pair[1] <= pair[0] * 1.05 || pair[1] <= 1_200.0,
            "displayed raster rect widened abruptly ({} -> {}): stale fallback regressed to an older raster",
            pair[0],
            pair[1]
        );
    }
}

/// End-of-scroll convergence probe: after the last simulated wheel event,
/// the app only re-evaluates when the session's evaluation-invalidation hub
/// asks it to (winit's contract; the taxi examples don't opt into
/// settle-exact, so there is no interaction-settle safety net). A correct
/// session therefore maintains the invariant: while the displayed raster is
/// stale, an immediate or delayed invalidation is always pending. This test
/// replays wheel gestures at varying phases against the 100ms schedule
/// throttle and fails on either a stall (stale + nothing pending = lost
/// wakeup) or a convergence timeout.
#[ignore = "manual probe; needs the taxi parquet fixture"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn geo_end_of_scroll_convergence() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
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

    let (_plot, _warmup) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact())
        .await
        .unwrap();
    wait_for_materializations(&session).await;

    // Winit-like inbox: invalidations arrive (from executor tasks or the
    // eval itself) and become due immediately or after their delay. Entries
    // record when they were requested so pop_due can mirror winit's
    // delayed-event redundancy rule (8cc61ed58): a delayed wake-up requested
    // before the last evaluation STARTED is redundant — that evaluation
    // re-parked whatever wake-up is still needed. Without this rule stale
    // schedule-throttle wake-ups multiply into a self-sustaining eval storm
    // that real winit never runs.
    struct InboxEntry {
        due: Instant,
        pushed_at: Instant,
        delayed: bool,
        reason: String,
    }
    type Inbox = Arc<std::sync::Mutex<Vec<InboxEntry>>>;
    let inbox: Inbox = Arc::new(std::sync::Mutex::new(Vec::new()));
    let inbox_writer = inbox.clone();
    let _subscription =
        session.subscribe_to_evaluation_invalidations(Arc::new(move |invalidation| {
            use avenger_chart_core::EvaluationInvalidationSchedule as Schedule;
            let now = Instant::now();
            let (due, delayed) = match invalidation.schedule {
                Schedule::Now => (now, false),
                Schedule::After(delay) => (now + delay, true),
            };
            inbox_writer.lock().unwrap().push(InboxEntry {
                due,
                pushed_at: now,
                delayed,
                reason: format!("{:?}", invalidation.reason),
            });
        }));
    let pop_due = |inbox: &Inbox, last_eval_start: Instant| -> Option<String> {
        let mut inbox = inbox.lock().unwrap();
        let now = Instant::now();
        // Winit redundancy rule: drop due delayed entries requested before
        // the last evaluation started.
        inbox.retain(|entry| {
            !(entry.delayed && entry.due <= now && entry.pushed_at <= last_eval_start)
        });
        let index = inbox
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.due <= now)
            .min_by_key(|(_, entry)| entry.due)
            .map(|(index, _)| index)?;
        Some(inbox.remove(index).reason)
    };
    let earliest_pending = |inbox: &Inbox| -> Option<Instant> {
        inbox.lock().unwrap().iter().map(|entry| entry.due).min()
    };

    let (center_x, center_y) = {
        let projection =
            avenger_geo::projector::Projection::new(avenger_geo::raw::ProjectionKind::Mercator);
        projection.project_raw_units(-73.977, 40.75)
    };
    let base_upp = 2.0 * std::f64::consts::PI / (512.0 * 2.0_f64.powf(11.0));

    let trials: usize = std::env::var("AVENGER_PROBE_TRIALS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20);
    const GESTURE_FRAMES: i32 = 12;
    let mut failures = Vec::new();
    let mut zoomed_in = false;

    // The final trial zooms deep enough to cross the 10k point budget so
    // the raster->scatter gate flip after gesture end is covered: with
    // async scalars the count is one cycle stale mid-gesture, and the flip
    // must still land once the settled count materializes.
    let total_trials = trials + 1;
    for trial in 0..total_trials {
        let gate_flip_trial = trial == trials;
        let mut log: Vec<String> = Vec::new();
        let trial_start = Instant::now();
        let mut last_eval_start = trial_start;
        let log_eval = |log: &mut Vec<String>,
                            trigger: &str,
                            metrics: &EvaluationMetrics,
                            rect: Option<[f32; 4]>| {
            log.push(format!(
                "t=+{:>4}ms {trigger}: reuses={} ready={} fallback={} queued={} rect={rect:?}",
                trial_start.elapsed().as_millis(),
                metrics.pipeline.preview_data_mark_reuses,
                metrics.pipeline.materialization_ready_used,
                metrics.pipeline.materialization_stale_fallback_used,
                metrics.pipeline.materialization_queued,
            ));
        };

        // Shift the gesture's phase against the 100ms schedule throttle.
        tokio::time::sleep(Duration::from_millis((trial as u64 * 13) % 100)).await;

        // Alternate zoom-in / zoom-out gestures so keys revisit cached results.
        let zoom_out = if gate_flip_trial { false } else { zoomed_in };
        if !gate_flip_trial {
            zoomed_in = !zoom_out;
        }
        // The flip gesture must end deep enough that the in-view count
        // genuinely drops below the 10k budget: 0.6^12 ~ 0.002x span (a
        // ~75m window in midtown at the probe's default 1M rows).
        let zoom_factor: f64 = if gate_flip_trial { 0.6 } else { 0.97 };
        for frame in 1..=GESTURE_FRAMES {
            // Winit interleaves completion-invalidation rebuilds with wheel
            // events; mirror that before each wheel frame.
            for _ in 0..3 {
                let Some(reason) = pop_due(&inbox, last_eval_start) else {
                    break;
                };
                last_eval_start = Instant::now();
                let (plot, metrics) = session
                    .evaluate_with_metrics(EvaluationRequest::new().preview())
                    .await
                    .unwrap();
                log_eval(
                    &mut log,
                    &format!("mid-gesture invalidation [{reason}]"),
                    &metrics,
                    displayed_raster_rect(&plot.scene_graph),
                );
            }
            let step = if zoom_out {
                GESTURE_FRAMES - frame
            } else {
                frame
            };
            let upp = base_upp * zoom_factor.powi(step);
            let mut patch = indexmap::IndexMap::new();
            patch.insert(center_x_param.clone(), ScalarValue::Float64(Some(center_x)));
            patch.insert(center_y_param.clone(), ScalarValue::Float64(Some(center_y)));
            patch.insert(upp_param.clone(), ScalarValue::Float64(Some(upp)));
            last_eval_start = Instant::now();
            let (plot, metrics) = session
                .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
                .await
                .unwrap();
            log_eval(
                &mut log,
                &format!("wheel frame {frame:02}"),
                &metrics,
                displayed_raster_rect(&plot.scene_graph),
            );
            tokio::time::sleep(Duration::from_millis(16)).await;
        }

        // Gesture over: evaluate ONLY on invalidations, winit-style.
        let gesture_end = Instant::now();
        let deadline = gesture_end + Duration::from_secs(6);
        let mut outcome: Option<(Duration, Option<[f32; 4]>)> = None;
        while Instant::now() < deadline {
            if let Some(reason) = pop_due(&inbox, last_eval_start) {
                last_eval_start = Instant::now();
                let (plot, metrics) = session
                    .evaluate_with_metrics(EvaluationRequest::new().preview())
                    .await
                    .unwrap();
                let rect = displayed_raster_rect(&plot.scene_graph);
                log_eval(
                    &mut log,
                    &format!("post-gesture invalidation [{reason}]"),
                    &metrics,
                    rect,
                );
                // Consume signature: the preview declined mark reuse and
                // rebuilt from the ready (desired) materialization.
                if metrics.pipeline.preview_data_mark_reuses == 0
                    && metrics.pipeline.materialization_ready_used >= 1
                {
                    // The gate-flip trial's FIRST consume legitimately shows
                    // a raster built with the stale mid-gesture count; the
                    // settled count then flips the gate through a second
                    // schedule->rasterize->consume cycle. Wait for the
                    // scatter takeover (NaN rect marker) there.
                    let flipped = rect.is_some_and(|r| r[0].is_nan());
                    if !gate_flip_trial || flipped {
                        outcome = Some((gesture_end.elapsed(), rect));
                        break;
                    }
                }
                continue;
            }
            match earliest_pending(&inbox) {
                Some(due) => {
                    tokio::time::sleep(
                        due.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
                    )
                    .await;
                }
                None if session.has_pending_materializations() => {
                    // Executor still running; its completion will invalidate.
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                None => {
                    log.push(format!(
                        "t=+{:>4}ms STALL: stale raster, no pending invalidation, no running materialization",
                        trial_start.elapsed().as_millis()
                    ));
                    break;
                }
            }
        }
        match outcome {
            Some((latency, rect)) => {
                println!(
                    "trial {trial:02} ({}): consumed {:>4}ms after gesture end rect={rect:?}",
                    if gate_flip_trial {
                        "flip"
                    } else if zoom_out {
                        "out"
                    } else {
                        "in "
                    },
                    latency.as_millis()
                );

            }
            None => {
                println!(
                    "trial {trial:02} ({}): FAILED to converge",
                    if gate_flip_trial {
                        "flip"
                    } else if zoom_out {
                        "out"
                    } else {
                        "in "
                    }
                );
                for line in &log {
                    println!("    {line}");
                }
                failures.push(trial);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "end-of-scroll convergence failed in trials {failures:?}"
    );
}

/// Plot-frame rect `[x, y, width, height]` of the first uniform-raster
/// image mark in the scene, if any.
fn displayed_raster_rect(
    scene_graph: &avenger_scenegraph::scene_graph::SceneGraph,
) -> Option<[f32; 4]> {
    use avenger_scenegraph::marks::mark::SceneMark;
    fn walk(marks: &[SceneMark]) -> Option<[f32; 4]> {
        for mark in marks {
            match mark {
                SceneMark::Group(group) => {
                    if let Some(rect) = walk(&group.marks) {
                        return Some(rect);
                    }
                }
                SceneMark::Image(image) if image.name == "uniform_raster_2d" => {
                    return Some([
                        image.x.as_vec(1, None)[0],
                        image.y.as_vec(1, None)[0],
                        image.width.as_vec(1, None)[0],
                        image.height.as_vec(1, None)[0],
                    ]);
                }
                SceneMark::Symbol(symbol) if symbol.len > 10 => {
                    // Adaptive switch: the scatter child took over.
                    return Some([f32::NAN, f32::NAN, symbol.len as f32, f32::NAN]);
                }
                _ => {}
            }
        }
        None
    }
    walk(&scene_graph.marks)
}
