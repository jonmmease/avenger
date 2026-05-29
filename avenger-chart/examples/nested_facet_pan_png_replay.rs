//! Replay a simulated nested-facet **pan** through `PngCanvas`.
//!
//! Headless profiling companion to the interactive `cartesian_nested_facet_pan`
//! example. It builds the same nested row × column facet with `Level(1)`
//! raw-domain params/scales, warms one Exact frame, then replays a sequence of
//! per-row pan steps in Preview mode — exactly the path the app drives while you
//! drag — and prints a per-frame timing breakdown so we can see where the cost
//! goes (the measurement *cache* path vs the component *retarget/rebuild* path).
//!
//! Read the breakdown as:
//! - `eval` = total chart evaluation, split into:
//!   - `probe_ms`  → `measure_cells_overflow_probe` (per-cell measurement / cache)
//!   - `build_ms`  → `build_plot_components` (per-cell retarget / rebuild)
//!   - `guide_ms`  → guide overflow measurement
//! - `set_scene` + `png_render` = scene upload + GPU draw.
//! - `*_reuse` counters show how much of the layout profile / data marks were
//!   reused vs re-measured.
//!
//! Set `AVENGER_PAN_REPLAY_DIR=/some/dir` to also dump `frame_NNN.png` per step.
//!
//! Run with:
//! ```bash
//! RUST_LOG=avenger_chart=info \
//!   cargo run -p avenger-chart --example nested_facet_pan_png_replay --release
//! ```

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use avenger_chart::{
    plot::{EvaluationRequest, ScopedParamAssignment},
    prelude::*,
};
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::{arrow::datatypes::DataType, prelude::SessionContext, scalar::ScalarValue};

const PNG_CANVAS_SIZE: [f32; 2] = [820.0, 520.0];
const PNG_CANVAS_SCALE: f32 = 2.0;
const PAN_FRAMES: usize = 30;

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

    let dump_dir = std::env::var("AVENGER_PAN_REPLAY_DIR").ok();
    if let Some(dir) = &dump_dir {
        std::fs::create_dir_all(dir)?;
    }

    // Warm Exact frame (no pan yet) to establish the layout profile.
    let (evaluated, metrics) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact())
        .await?;
    render_png_frame(
        &mut canvas,
        &evaluated.scene_graph,
        0,
        "Exact",
        &metrics,
        &dump_dir,
    )
    .await?;

    // Pan the "Top" row: Level(1) owner path is the row value. Each frame shifts
    // the domain so the preview path does real work (not a no-op).
    let row_owner = vec![ScalarValue::Utf8(Some("Top".to_string()))];

    let mut eval_total = Duration::ZERO;
    let mut set_scene_total = Duration::ZERO;
    let mut render_total = Duration::ZERO;

    for idx in 0..PAN_FRAMES {
        let shift = (idx as f64 + 1.0) * 0.25;
        session.apply_scoped_param_patch(vec![
            ScopedParamAssignment {
                name: "x_domain".to_string(),
                owner_path: row_owner.clone(),
                value: list_domain(0.0 - shift, 10.0 - shift),
            },
            ScopedParamAssignment {
                name: "y_domain".to_string(),
                owner_path: row_owner.clone(),
                value: list_domain(0.0 - shift * 0.8, 10.0 - shift * 0.8),
            },
        ]);

        let eval_start = Instant::now();
        let (evaluated, metrics) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview())
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

        println!(
            "pan_replay seq={} mode=Preview scene={:.0}x{:.0} eval={:.2}ms set_scene={:.2}ms png_render={:.2}ms frame_total={:.2}ms reflow_reuse={} preview_reuse={} cell_reuse={} data_reuse={} data_miss={} chrome={} guides={} guide_ms={:.2} probe_ms={:.2} build_ms={:.2}",
            idx + 1,
            evaluated.scene_graph.width,
            evaluated.scene_graph.height,
            ms(eval_elapsed),
            ms(set_scene_elapsed),
            ms(render_elapsed),
            ms(eval_elapsed + set_scene_elapsed + render_elapsed),
            metrics.pipeline.preview_structure_reflow_reuses,
            metrics.pipeline.preview_profile_reuses,
            metrics.pipeline.facet_cell_measurement_profile_reuses,
            metrics.pipeline.preview_data_mark_reuses,
            metrics.pipeline.preview_data_mark_reuse_misses,
            metrics
                .pipeline
                .facet_cell_measurement_profile_chrome_refreshes,
            metrics.pipeline.guide_overflow_measure_calls,
            us_to_ms(metrics.timings.guide_overflow_measure_us),
            us_to_ms(metrics.timings.measure_cells_overflow_probe_us),
            us_to_ms(metrics.timings.build_plot_components_us),
        );
    }

    let n = PAN_FRAMES as f64;
    let frame_total = eval_total + set_scene_total + render_total;
    println!(
        "pan_replay SUMMARY frames={} avg_eval={:.2}ms avg_set_scene={:.2}ms avg_png_render={:.2}ms avg_frame={:.2}ms est_fps={:.1}",
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
    let x_domain = Param::raw_domain("x_domain");
    let y_domain = Param::raw_domain("y_domain");
    let x_raw = x_domain.expr();
    let y_raw = y_domain.expr();

    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
                ('Top','Left', 1.0, 2.0),  ('Top','Left', 3.0, 4.5),  ('Top','Left', 5.0, 3.0),
                ('Top','Mid', 2.0, 5.0),   ('Top','Mid', 4.0, 3.0),   ('Top','Mid', 6.0, 7.0),
                ('Top','Right', 1.5, 3.5), ('Top','Right', 3.5, 6.0),  ('Top','Right', 5.5, 2.5),
                ('Bottom','Left', 2.0, 6.0),  ('Bottom','Left', 4.0, 4.0),  ('Bottom','Left', 6.0, 8.0),
                ('Bottom','Mid', 1.0, 4.5),   ('Bottom','Mid', 3.0, 2.5),   ('Bottom','Mid', 5.0, 6.5),
                ('Bottom','Right', 2.5, 3.0), ('Bottom','Right', 4.5, 5.5),  ('Bottom','Right', 6.5, 4.0)
            ) AS t(row_name, col_name, x, y)",
        )
        .await?;

    let leaf = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("x"), move |c| {
                c.scale_with::<Linear>(move |s| s.raw_domain(x_raw.clone()).nice(false).zero(false))
                    .with_scale_sharing(Sharing::Level(1))
            })
            .y_with(col("y"), move |c| {
                c.scale_with::<Linear>(move |s| s.raw_domain(y_raw.clone()).nice(false).zero(false))
                    .with_scale_sharing(Sharing::Level(1))
            })
            .fill(col("col_name"))
            .size(80.0),
    );

    let columns = Plot::<FacetColumn>::new().mark(Subplot::new(leaf).column(col("col_name")));

    Ok(Plot::<FacetRow>::new()
        .add_param_with_sharing(x_domain.clone(), Sharing::Level(1))
        .add_param_with_sharing(y_domain.clone(), Sharing::Level(1))
        .canvas_size(PNG_CANVAS_SIZE[0], PNG_CANVAS_SIZE[1])
        .data(df)
        .mark(Subplot::new(columns).row(col("row_name")))
        .compile(ctx)
        .await?)
}

async fn render_png_frame(
    canvas: &mut PngCanvas,
    scene_graph: &avenger_scenegraph::scene_graph::SceneGraph,
    seq: usize,
    mode: &str,
    metrics: &avenger_chart::render::EvaluationMetrics,
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
            "pan_replay seq=0 mode={mode} scene={:.0}x{:.0} set_scene={:.2}ms png_render={:.2}ms guides={} guide_ms={:.2} probe_ms={:.2} build_ms={:.2}",
            scene_graph.width,
            scene_graph.height,
            ms(set_scene_elapsed),
            ms(render_elapsed),
            metrics.pipeline.guide_overflow_measure_calls,
            us_to_ms(metrics.timings.guide_overflow_measure_us),
            us_to_ms(metrics.timings.measure_cells_overflow_probe_us),
            us_to_ms(metrics.timings.build_plot_components_us),
        );
    }

    Ok((set_scene_elapsed, render_elapsed))
}

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
