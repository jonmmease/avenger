//! Replay simple single-panel instanced pan through `PngCanvas`.
//!
//! This isolates which phases scale with point count for raw-domain Preview pan.
//! By default it runs 10k and 100k symbols. Override with
//! `AVENGER_POINT_COUNTS=10000,100000,200000`.
//!
//! Run with:
//! ```bash
//! cargo run -p avenger-chart --example simple_instanced_pan_png_replay --release
//! ```

use std::{
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
use datafusion::{
    arrow::{
        array::Float64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::SessionContext,
    scalar::ScalarValue,
};
use indexmap::IndexMap;

const PNG_CANVAS_SIZE: [f32; 2] = [960.0, 640.0];
const PNG_CANVAS_SCALE: f32 = 2.0;
const PAN_FRAMES: usize = 12;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_diagnostics();

    for point_count in point_counts() {
        run_count(point_count).await?;
    }

    Ok(())
}

async fn run_count(point_count: usize) -> Result<(), Box<dyn std::error::Error>> {
    let ctx = Arc::new(SessionContext::new());
    let compiled = Arc::new(build_plot(&ctx, point_count).await?);
    let mut session = compiled.instantiate(ctx);
    let mut canvas = PngCanvas::new(
        CanvasDimensions {
            size: PNG_CANVAS_SIZE,
            scale: PNG_CANVAS_SCALE,
        },
        CanvasConfig::default(),
    )
    .await?;

    let (evaluated, metrics) = session
        .evaluate_with_metrics(no_scene_rtree_request(EvaluationRequest::new().exact()))
        .await?;
    render_png_frame(
        &mut canvas,
        &evaluated.scene_graph,
        0,
        point_count,
        &metrics,
    )
    .await?;

    let mut eval_total = Duration::ZERO;
    let mut set_scene_total = Duration::ZERO;
    let mut render_total = Duration::ZERO;
    let mut layout_setup_total_us = 0u64;
    let mut measurement_clone_total_us = 0u64;
    let mut scale_refresh_total_us = 0u64;
    let mut scale_context_setup_total_us = 0u64;
    let mut scale_cache_key_total_us = 0u64;
    let mut scale_cache_lookup_total_us = 0u64;
    let mut scale_build_total_us = 0u64;
    let mut scale_coord_adjust_total_us = 0u64;
    let mut measurement_retarget_total_us = 0u64;
    let mut facet_domain_override_total_us = 0u64;
    let mut build_total_us = 0u64;
    let mut components_total_us = 0u64;

    for idx in 0..PAN_FRAMES {
        let shift = (idx as f64 + 1.0) * 3.0;
        session.apply_param_patch(IndexMap::from([
            ("x_domain".to_string(), list_domain(-shift, 400.0 - shift)),
            (
                "y_domain".to_string(),
                list_domain(-shift * 0.8, 250.0 - shift * 0.8),
            ),
        ]));

        let eval_start = Instant::now();
        let (evaluated, metrics) = session
            .evaluate_with_metrics(no_scene_rtree_request(EvaluationRequest::new().preview()))
            .await?;
        let eval_elapsed = eval_start.elapsed();

        let (set_scene_elapsed, render_elapsed) = render_png_frame(
            &mut canvas,
            &evaluated.scene_graph,
            idx + 1,
            point_count,
            &metrics,
        )
        .await?;

        eval_total += eval_elapsed;
        set_scene_total += set_scene_elapsed;
        render_total += render_elapsed;
        layout_setup_total_us += metrics.timings.preview_layout_setup_us;
        measurement_clone_total_us += metrics.timings.preview_measurement_clone_us;
        scale_refresh_total_us += metrics.timings.preview_scale_refresh_us;
        scale_context_setup_total_us += metrics.timings.preview_scale_context_setup_us;
        scale_cache_key_total_us += metrics.timings.preview_scale_cache_key_us;
        scale_cache_lookup_total_us += metrics.timings.preview_scale_cache_lookup_us;
        scale_build_total_us += metrics.timings.preview_scale_build_us;
        scale_coord_adjust_total_us += metrics.timings.preview_scale_coord_adjust_us;
        measurement_retarget_total_us += metrics.timings.preview_measurement_retarget_us;
        facet_domain_override_total_us += metrics.timings.preview_facet_domain_override_us;
        build_total_us += metrics.timings.build_plot_components_us;
        components_total_us += metrics.timings.components_to_evaluated_plot_us;

        println!(
            "simple_instanced_pan count={} seq={} eval={:.2}ms setup={:.2}ms clone={:.2}ms scales={:.2}ms scale_ctx={:.2}ms scale_key={:.2}ms scale_lookup={:.2}ms scale_build={:.2}ms scale_adjust={:.2}ms retarget={:.2}ms facet_override={:.2}ms build={:.2}ms components={:.2}ms set_scene={:.2}ms png_render={:.2}ms frame_total={:.2}ms data_reuse={} data_miss={} mark_collects={} guide_ms={:.2}",
            point_count,
            idx + 1,
            ms(eval_elapsed),
            us_to_ms(metrics.timings.preview_layout_setup_us),
            us_to_ms(metrics.timings.preview_measurement_clone_us),
            us_to_ms(metrics.timings.preview_scale_refresh_us),
            us_to_ms(metrics.timings.preview_scale_context_setup_us),
            us_to_ms(metrics.timings.preview_scale_cache_key_us),
            us_to_ms(metrics.timings.preview_scale_cache_lookup_us),
            us_to_ms(metrics.timings.preview_scale_build_us),
            us_to_ms(metrics.timings.preview_scale_coord_adjust_us),
            us_to_ms(metrics.timings.preview_measurement_retarget_us),
            us_to_ms(metrics.timings.preview_facet_domain_override_us),
            us_to_ms(metrics.timings.build_plot_components_us),
            us_to_ms(metrics.timings.components_to_evaluated_plot_us),
            ms(set_scene_elapsed),
            ms(render_elapsed),
            ms(eval_elapsed + set_scene_elapsed + render_elapsed),
            metrics.pipeline.preview_data_mark_reuses,
            metrics.pipeline.preview_data_mark_reuse_misses,
            metrics.pipeline.mark_data_collects,
            us_to_ms(metrics.timings.guide_overflow_measure_us),
        );
    }

    let n = PAN_FRAMES as f64;
    let frame_total = eval_total + set_scene_total + render_total;
    println!(
        "simple_instanced_pan SUMMARY count={} frames={} avg_eval={:.2}ms avg_setup={:.2}ms avg_clone={:.2}ms avg_scales={:.2}ms avg_scale_ctx={:.2}ms avg_scale_key={:.2}ms avg_scale_lookup={:.2}ms avg_scale_build={:.2}ms avg_scale_adjust={:.2}ms avg_retarget={:.2}ms avg_facet_override={:.2}ms avg_build={:.2}ms avg_components={:.2}ms avg_set_scene={:.2}ms avg_png_render={:.2}ms avg_frame={:.2}ms est_fps={:.1}",
        point_count,
        PAN_FRAMES,
        ms(eval_total) / n,
        us_to_ms(layout_setup_total_us) / n,
        us_to_ms(measurement_clone_total_us) / n,
        us_to_ms(scale_refresh_total_us) / n,
        us_to_ms(scale_context_setup_total_us) / n,
        us_to_ms(scale_cache_key_total_us) / n,
        us_to_ms(scale_cache_lookup_total_us) / n,
        us_to_ms(scale_build_total_us) / n,
        us_to_ms(scale_coord_adjust_total_us) / n,
        us_to_ms(measurement_retarget_total_us) / n,
        us_to_ms(facet_domain_override_total_us) / n,
        us_to_ms(build_total_us) / n,
        us_to_ms(components_total_us) / n,
        ms(set_scene_total) / n,
        ms(render_total) / n,
        ms(frame_total) / n,
        1000.0 / (ms(frame_total) / n),
    );

    Ok(())
}

async fn build_plot(
    ctx: &SessionContext,
    point_count: usize,
) -> Result<avenger_chart::plot::CompiledPlot, Box<dyn std::error::Error>> {
    let df = ctx.read_batch(make_points_batch(point_count))?;
    let x_domain = Param::raw_domain("x_domain");
    let y_domain = Param::raw_domain("y_domain");
    let x_raw = x_domain.expr();
    let y_raw = y_domain.expr();

    Ok(Plot::<Cartesian>::new()
        .add_param(x_domain.clone())
        .add_param(y_domain.clone())
        .canvas_size(PNG_CANVAS_SIZE[0], PNG_CANVAS_SIZE[1])
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), move |c| {
                    c.scale_with::<Linear>(move |s| {
                        s.raw_domain(x_raw.clone()).nice(false).zero(false)
                    })
                })
                .y_with(col("y"), move |c| {
                    c.scale_with::<Linear>(move |s| {
                        s.raw_domain(y_raw.clone()).nice(false).zero(false)
                    })
                })
                .fill("#1f77b4")
                .size(if point_count >= 100_000 { 7.0 } else { 14.0 }),
        )
        .compile(ctx)
        .await?)
}

fn make_points_batch(point_count: usize) -> RecordBatch {
    let columns = (point_count as f64).sqrt().ceil() as usize;
    let rows = point_count.div_ceil(columns);
    let mut xs = Vec::with_capacity(point_count);
    let mut ys = Vec::with_capacity(point_count);
    for row in 0..rows {
        for col in 0..columns {
            if xs.len() == point_count {
                break;
            }
            let idx = row * columns + col;
            let jitter_x = (((idx * 37 + 11) % 100) as f64 - 50.0) / 120.0;
            let jitter_y = (((idx * 53 + 7) % 100) as f64 - 50.0) / 120.0;
            let x = col as f64 + jitter_x;
            let wave = (col as f64 / 16.0).sin() * 12.0 + (col as f64 / 37.0).cos() * 6.0;
            let y = row as f64 + wave + jitter_y;
            xs.push(x);
            ys.push(y);
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Float64Array::from(xs)),
            Arc::new(Float64Array::from(ys)),
        ],
    )
    .expect("build generated point batch")
}

async fn render_png_frame(
    canvas: &mut PngCanvas,
    scene_graph: &avenger_scenegraph::scene_graph::SceneGraph,
    seq: usize,
    point_count: usize,
    metrics: &EvaluationMetrics,
) -> Result<(Duration, Duration), Box<dyn std::error::Error>> {
    let set_scene_start = Instant::now();
    canvas.set_scene(scene_graph)?;
    let set_scene_elapsed = set_scene_start.elapsed();

    let render_start = Instant::now();
    let _image = canvas.render().await?;
    let render_elapsed = render_start.elapsed();

    if seq == 0 {
        println!(
            "simple_instanced_pan count={} seq=0 mode=Exact set_scene={:.2}ms png_render={:.2}ms guides={} build_ms={:.2} components_ms={:.2}",
            point_count,
            ms(set_scene_elapsed),
            ms(render_elapsed),
            metrics.pipeline.guide_overflow_measure_calls,
            us_to_ms(metrics.timings.build_plot_components_us),
            us_to_ms(metrics.timings.components_to_evaluated_plot_us),
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

fn point_counts() -> Vec<usize> {
    std::env::var("AVENGER_POINT_COUNTS")
        .ok()
        .map(|counts| {
            counts
                .split(',')
                .filter_map(|count| count.trim().parse().ok())
                .collect::<Vec<_>>()
        })
        .filter(|counts| !counts.is_empty())
        .unwrap_or_else(|| vec![10_000, 100_000])
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
