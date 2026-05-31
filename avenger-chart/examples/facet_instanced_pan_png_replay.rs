//! Replay a simulated 2x3 faceted pan through `PngCanvas`.
//!
//! This is the headless companion to the interactive
//! `avenger-chart-app/examples/cartesian_facet_instanced_pan.rs` example. It uses
//! the same 6 facet groups with 200 points per facet cell, so every leaf symbol
//! mark takes the `avenger-wgpu` instanced renderer path. The chart is warmed with
//! one Exact frame, then a sequence of global raw-domain pan updates is evaluated
//! in Preview mode and rendered through `PngCanvas`.
//!
//! Read the breakdown as:
//! - `eval` = chart evaluation / scene graph construction,
//! - `set_scene` = uploading the evaluated scene into `PngCanvas`,
//! - `png_render` = GPU draw plus PNG readback,
//! - `data_reuse` / `data_miss` = Preview data-mark retarget reuse signals.
//!
//! Set `AVENGER_PAN_REPLAY_DIR=/some/dir` to dump `frame_NNN.png` per step.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart --example facet_instanced_pan_png_replay --release
//! ```

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use avenger_chart::{
    plot::{EvaluationRequest, ScopedParamAssignment},
    prelude::*,
    render::EvaluationOptions,
};
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::{arrow::datatypes::DataType, prelude::SessionContext, scalar::ScalarValue};

const PNG_CANVAS_SIZE: [f32; 2] = [960.0, 640.0];
const PNG_CANVAS_SCALE: f32 = 2.0;
const PAN_FRAMES: usize = 22;

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

    let (evaluated, metrics) = session
        .evaluate_with_metrics(no_scene_rtree_request(EvaluationRequest::new().exact()))
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

    let mut eval_total = Duration::ZERO;
    let mut set_scene_total = Duration::ZERO;
    let mut render_total = Duration::ZERO;
    let mut steady_eval_total = Duration::ZERO;
    let mut steady_set_scene_total = Duration::ZERO;
    let mut steady_render_total = Duration::ZERO;

    for idx in 0..PAN_FRAMES {
        let shift = (idx as f64 + 1.0) * 0.12;
        session.apply_scoped_param_patch(vec![
            ScopedParamAssignment {
                name: "x_domain".to_string(),
                owner_path: Vec::new(),
                value: list_domain(0.0 - shift, 10.0 - shift),
            },
            ScopedParamAssignment {
                name: "y_domain".to_string(),
                owner_path: Vec::new(),
                value: list_domain(0.0 - shift * 0.8, 10.0 - shift * 0.8),
            },
        ]);

        let eval_start = Instant::now();
        let (evaluated, metrics) = session
            .evaluate_with_metrics(no_scene_rtree_request(EvaluationRequest::new().preview()))
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
        if idx > 0 {
            steady_eval_total += eval_elapsed;
            steady_set_scene_total += set_scene_elapsed;
            steady_render_total += render_elapsed;
        }

        println!(
            "instanced_pan_replay seq={} mode=Preview scene={:.0}x{:.0} eval={:.2}ms set_scene={:.2}ms png_render={:.2}ms frame_total={:.2}ms preview_reuse={} reflow_reuse={} cell_reuse={} data_reuse={} data_miss={} cells_built={} chrome={} guides={} scale_hits={} scale_misses={} scale_builds={} facet_tree_builds={} mark_collects={} guide_ms={:.2} probe_ms={:.2} build_ms={:.2} components_ms={:.2}",
            idx + 1,
            evaluated.scene_graph.width,
            evaluated.scene_graph.height,
            ms(eval_elapsed),
            ms(set_scene_elapsed),
            ms(render_elapsed),
            ms(eval_elapsed + set_scene_elapsed + render_elapsed),
            metrics.pipeline.preview_profile_reuses,
            metrics.pipeline.preview_structure_reflow_reuses,
            metrics.pipeline.facet_cell_measurement_profile_reuses,
            metrics.pipeline.preview_data_mark_reuses,
            metrics.pipeline.preview_data_mark_reuse_misses,
            metrics.pipeline.facet_cells_built,
            metrics
                .pipeline
                .facet_cell_measurement_profile_chrome_refreshes,
            metrics.pipeline.guide_overflow_measure_calls,
            metrics.pipeline.scale_domain_cache_hits,
            metrics.pipeline.scale_domain_cache_misses,
            metrics.pipeline.scale_builder_builds,
            metrics.pipeline.facet_tree_builds,
            metrics.pipeline.mark_data_collects,
            us_to_ms(metrics.timings.guide_overflow_measure_us),
            us_to_ms(metrics.timings.measure_cells_overflow_probe_us),
            us_to_ms(metrics.timings.build_plot_components_us),
            us_to_ms(metrics.timings.components_to_evaluated_plot_us),
        );
    }

    print_summary("all", PAN_FRAMES, eval_total, set_scene_total, render_total);
    if PAN_FRAMES > 1 {
        print_summary(
            "steady_skip_first",
            PAN_FRAMES - 1,
            steady_eval_total,
            steady_set_scene_total,
            steady_render_total,
        );
    }

    Ok(())
}

async fn build_plot(
    ctx: &SessionContext,
) -> Result<avenger_chart::plot::CompiledPlot, Box<dyn std::error::Error>> {
    let df = ctx.sql(&scatter_values_sql()).await?;
    let x_domain = Param::raw_domain("x_domain");
    let y_domain = Param::raw_domain("y_domain");
    let x_raw = x_domain.expr();
    let y_raw = y_domain.expr();

    let leaf = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("x"), move |c| {
                c.scale_with::<Linear>(move |s| s.raw_domain(x_raw.clone()).nice(false).zero(false))
                    .share_scale()
            })
            .y_with(col("y"), move |c| {
                c.scale_with::<Linear>(move |s| s.raw_domain(y_raw.clone()).nice(false).zero(false))
                    .share_scale()
            })
            .fill(col("group_name"))
            .size(40.0),
    );

    Ok(Plot::<FacetWrap>::new()
        .add_param(x_domain.clone())
        .add_param(y_domain.clone())
        .canvas_size(PNG_CANVAS_SIZE[0], PNG_CANVAS_SIZE[1])
        .data(df)
        .mark(Subplot::new(leaf).wrap_with(col("group_name"), |c| c.columns(3)))
        .compile(ctx)
        .await?)
}

fn scatter_values_sql() -> String {
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next_unit = || -> f64 {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        let v = seed.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (v >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut rows = String::new();
    for group_index in 0..6 {
        for point_index in 0..200 {
            if !(group_index == 0 && point_index == 0) {
                rows.push(',');
            }
            let x = next_unit() * 10.0;
            let y = next_unit() * 10.0;
            rows.push_str(&format!("('Group {group_index}', {x:.4}, {y:.4})"));
        }
    }
    format!("SELECT * FROM (VALUES {rows}) AS t(group_name, x, y)")
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
            "instanced_pan_replay seq=0 mode={mode} scene={:.0}x{:.0} set_scene={:.2}ms png_render={:.2}ms guides={} guide_ms={:.2} probe_ms={:.2} build_ms={:.2}",
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

fn no_scene_rtree_request(request: EvaluationRequest) -> EvaluationRequest {
    request.options(EvaluationOptions {
        build_scene_rtree: false,
        ..EvaluationOptions::default()
    })
}

fn print_summary(
    label: &str,
    frames: usize,
    eval_total: Duration,
    set_scene_total: Duration,
    render_total: Duration,
) {
    let n = frames as f64;
    let frame_total = eval_total + set_scene_total + render_total;
    println!(
        "instanced_pan_replay SUMMARY label={} frames={} avg_eval={:.2}ms avg_set_scene={:.2}ms avg_png_render={:.2}ms avg_frame={:.2}ms est_fps={:.1}",
        label,
        frames,
        ms(eval_total) / n,
        ms(set_scene_total) / n,
        ms(render_total) / n,
        ms(frame_total) / n,
        1000.0 / (ms(frame_total) / n),
    );
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
