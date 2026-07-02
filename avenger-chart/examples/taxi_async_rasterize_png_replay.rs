//! Headless replay for async taxi rasterization through `PngCanvas`.
//!
//! This is the deterministic companion to
//! `avenger-chart-app/examples/taxi_async_rasterize.rs`: it warms one exact
//! raster, renders several Preview pan frames that should use cached-raster
//! fallback, then settles on a final exact raster.
//!
//! Set `AVENGER_TAXI_RASTER_REPLAY_DIR=/some/dir` to dump `frame_NNN.png`.
//! Set `AVENGER_TAXI_RASTER_DELAY_MS=250` to make pending/fallback behavior
//! easier to see in the printed metrics.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart --example taxi_async_rasterize_png_replay --features wgpu --release
//! ```

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use avenger_chart::{
    plot::EvaluationRequest,
    prelude::*,
    render::{EvaluationMetrics, EvaluationOptions},
};
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::{prelude::CsvReadOptions, prelude::SessionContext, scalar::ScalarValue};

const PNG_CANVAS_SIZE: [f32; 2] = [960.0, 720.0];
const PNG_CANVAS_SCALE: f32 = 2.0;
const PAN_FRAMES: usize = 5;

const TAXI_X_MIN: f64 = -8_242_500.0;
const TAXI_X_MAX: f64 = -8_226_500.0;
const TAXI_Y_MIN: f64 = 4_968_000.0;
const TAXI_Y_MAX: f64 = 4_983_000.0;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_diagnostics();

    let ctx = Arc::new(SessionContext::new());
    let compiled = Arc::new(build_plot(&ctx).await?);
    let mut session = compiled.instantiate(ctx);
    let mut canvas = PngCanvas::new(
        CanvasDimensions {
            size: PNG_CANVAS_SIZE,
            scale: PNG_CANVAS_SCALE,
        },
        CanvasConfig::default(),
    )
    .await?;
    let dump_dir = std::env::var("AVENGER_TAXI_RASTER_REPLAY_DIR").ok();
    if let Some(dir) = &dump_dir {
        std::fs::create_dir_all(dir)?;
    }

    let (evaluated, metrics) = evaluate_settled_exact(&mut session, None).await?;
    render_png_frame(
        &mut canvas,
        &evaluated.scene_graph,
        0,
        "ExactReady",
        &metrics,
        &dump_dir,
    )
    .await?;

    let mut eval_total = Duration::ZERO;
    let mut set_scene_total = Duration::ZERO;
    let mut render_total = Duration::ZERO;
    let mut last_patch = None;

    for idx in 0..PAN_FRAMES {
        let frac = (idx as f64 + 1.0) / (PAN_FRAMES as f64 + 1.0);
        let x_shift = frac * 1_800.0;
        let y_shift = frac * 1_100.0;
        let patch = pan_patch(
            TAXI_X_MIN + x_shift,
            TAXI_X_MAX + x_shift,
            TAXI_Y_MIN + y_shift,
            TAXI_Y_MAX + y_shift,
        );
        last_patch = Some(patch.clone());

        let eval_start = Instant::now();
        let (evaluated, metrics) = session
            .evaluate_with_metrics(no_scene_rtree_request(
                EvaluationRequest::new().preview().param_patch(patch),
            ))
            .await?;
        let eval_elapsed = eval_start.elapsed();
        let (set_scene_elapsed, render_elapsed) = render_png_frame(
            &mut canvas,
            &evaluated.scene_graph,
            idx + 1,
            "Preview",
            &metrics,
            &dump_dir,
        )
        .await?;

        eval_total += eval_elapsed;
        set_scene_total += set_scene_elapsed;
        render_total += render_elapsed;
        print_metrics(
            "Preview",
            idx + 1,
            eval_elapsed,
            set_scene_elapsed,
            render_elapsed,
            &metrics,
        );
    }

    if let Some(patch) = last_patch {
        let (evaluated, metrics) = evaluate_settled_exact(&mut session, Some(patch)).await?;
        let (set_scene_elapsed, render_elapsed) = render_png_frame(
            &mut canvas,
            &evaluated.scene_graph,
            PAN_FRAMES + 1,
            "ExactSettled",
            &metrics,
            &dump_dir,
        )
        .await?;
        print_metrics(
            "ExactSettled",
            PAN_FRAMES + 1,
            Duration::ZERO,
            set_scene_elapsed,
            render_elapsed,
            &metrics,
        );
    }

    let n = PAN_FRAMES as f64;
    let frame_total = eval_total + set_scene_total + render_total;
    println!(
        "taxi_async_rasterize_png_replay SUMMARY preview_frames={} avg_eval={:.2}ms avg_set_scene={:.2}ms avg_png_render={:.2}ms avg_frame={:.2}ms est_fps={:.1}",
        PAN_FRAMES,
        ms(eval_total) / n,
        ms(set_scene_total) / n,
        ms(render_total) / n,
        ms(frame_total) / n,
        1000.0 / (ms(frame_total) / n),
    );

    Ok(())
}

async fn build_plot(
    ctx: &SessionContext,
) -> Result<avenger_chart::plot::CompiledPlot, Box<dyn std::error::Error>> {
    let df = ctx
        .read_csv(
            taxi_fixture_path()
                .to_str()
                .expect("taxi fixture path should be valid UTF-8"),
            CsvReadOptions::new(),
        )
        .await?
        .filter(
            col("pickup_x")
                .gt_eq(lit(TAXI_X_MIN))
                .and(col("pickup_x").lt_eq(lit(TAXI_X_MAX)))
                .and(col("pickup_y").gt_eq(lit(TAXI_Y_MIN)))
                .and(col("pickup_y").lt_eq(lit(TAXI_Y_MAX))),
        )?;

    Ok(Plot::with_coord(Cartesian::new().unit_aspect(1.0))
        .title("NYC taxi pickup density")
        .canvas_size(PNG_CANVAS_SIZE[0], PNG_CANVAS_SIZE[1])
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
        .tool(PanScrollZoom::cartesian().settle_exact(true))
        .compile(ctx)
        .await?)
}

async fn evaluate_settled_exact(
    session: &mut avenger_chart::plot::PlotSession,
    patch: Option<indexmap::IndexMap<String, ScalarValue>>,
) -> Result<(avenger_chart::render::EvaluatedPlot, EvaluationMetrics), Box<dyn std::error::Error>> {
    let mut latest = None;
    for _ in 0..16 {
        let mut request = EvaluationRequest::new().exact();
        if let Some(patch) = patch.clone() {
            request = request.param_patch(patch);
        }
        let (evaluated, metrics) = session
            .evaluate_with_metrics(no_scene_rtree_request(request))
            .await?;
        let ready = metrics.pipeline.materialization_ready_used > 0;
        latest = Some((evaluated, metrics));
        if ready && !session.has_pending_materializations() {
            break;
        }
        wait_for_pending_materializations(session).await;
    }
    latest.ok_or_else(|| "settled exact evaluation did not run".into())
}

async fn wait_for_pending_materializations(session: &avenger_chart::plot::PlotSession) {
    for _ in 0..200 {
        if !session.has_pending_materializations() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
        tokio::task::yield_now().await;
    }
}

async fn render_png_frame(
    canvas: &mut PngCanvas,
    scene_graph: &avenger_scenegraph::scene_graph::SceneGraph,
    seq: usize,
    mode: &str,
    metrics: &EvaluationMetrics,
    dump_dir: &Option<String>,
) -> Result<(Duration, Duration), Box<dyn std::error::Error>> {
    let set_scene_start = Instant::now();
    canvas.set_scene(scene_graph)?;
    let set_scene_elapsed = set_scene_start.elapsed();

    let render_start = Instant::now();
    let image = canvas.render().await?;
    let render_elapsed = render_start.elapsed();

    if let Some(dir) = dump_dir {
        let path = format!("{dir}/frame_{seq:03}.png");
        image.save(&path)?;
    }

    if seq == 0 {
        println!(
            "taxi_async_rasterize_png_replay seq=0 mode={mode} scene={:.0}x{:.0} set_scene={:.2}ms png_render={:.2}ms materialization_ready={} stale_fallback={} queued={} running={} build_ms={:.2} components_ms={:.2}",
            scene_graph.width,
            scene_graph.height,
            ms(set_scene_elapsed),
            ms(render_elapsed),
            metrics.pipeline.materialization_ready_used,
            metrics.pipeline.materialization_stale_fallback_used,
            metrics.pipeline.materialization_queued,
            metrics.pipeline.materialization_running,
            us_to_ms(metrics.timings.build_plot_components_us),
            us_to_ms(metrics.timings.components_to_evaluated_plot_us),
        );
    }

    Ok((set_scene_elapsed, render_elapsed))
}

fn print_metrics(
    mode: &str,
    seq: usize,
    eval_elapsed: Duration,
    set_scene_elapsed: Duration,
    render_elapsed: Duration,
    metrics: &EvaluationMetrics,
) {
    println!(
        "taxi_async_rasterize_png_replay seq={} mode={} eval={:.2}ms set_scene={:.2}ms png_render={:.2}ms frame_total={:.2}ms requests={} ready={} stale_fallback={} queued={} running={} errors={} preview_reuse={} data_reuse={} mark_collects={} build_ms={:.2} components_ms={:.2}",
        seq,
        mode,
        ms(eval_elapsed),
        ms(set_scene_elapsed),
        ms(render_elapsed),
        ms(eval_elapsed + set_scene_elapsed + render_elapsed),
        metrics.pipeline.materialization_requests_emitted,
        metrics.pipeline.materialization_ready_used,
        metrics.pipeline.materialization_stale_fallback_used,
        metrics.pipeline.materialization_queued,
        metrics.pipeline.materialization_running,
        metrics.pipeline.materialization_errors,
        metrics.pipeline.preview_profile_reuses,
        metrics.pipeline.preview_data_mark_reuses,
        metrics.pipeline.mark_data_collects,
        us_to_ms(metrics.timings.build_plot_components_us),
        us_to_ms(metrics.timings.components_to_evaluated_plot_us),
    );
}

fn pan_patch(
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
) -> indexmap::IndexMap<String, ScalarValue> {
    indexmap::IndexMap::from([
        (
            "__tool_pan_scroll_zoom__x_domain".to_string(),
            list_domain(x_min, x_max),
        ),
        (
            "__tool_pan_scroll_zoom__y_domain".to_string(),
            list_domain(y_min, y_max),
        ),
    ])
}

fn list_domain(min: f64, max: f64) -> ScalarValue {
    ScalarValue::List(ScalarValue::new_list(
        &[
            ScalarValue::Float64(Some(min)),
            ScalarValue::Float64(Some(max)),
        ],
        &datafusion::arrow::datatypes::DataType::Float64,
        true,
    ))
}

fn no_scene_rtree_request(request: EvaluationRequest) -> EvaluationRequest {
    request.options(EvaluationOptions {
        build_scene_rtree: false,
        ..EvaluationOptions::default()
    })
}

fn taxi_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/nyc_taxi_2015/nyc_taxi.csv")
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn us_to_ms(us: u64) -> f64 {
    us as f64 / 1000.0
}

fn init_diagnostics() {
    if std::env::var_os("RUST_LOG").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();
    }
}
