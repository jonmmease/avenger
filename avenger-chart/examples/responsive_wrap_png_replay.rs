//! Replay responsive `FacetWrap` width changes through `PngCanvas`.
//!
//! This is a headless profiling companion to the interactive
//! `avenger-chart-app` responsive wrap example. It keeps Winit out of the
//! loop so chart evaluation and PNG/WGPU rendering costs can be compared.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use avenger_chart::{plot::EvaluationRequest, prelude::*};
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::{functions_aggregate::min_max::max, prelude::SessionContext, scalar::ScalarValue};
use indexmap::IndexMap;

const PNG_CANVAS_SIZE: [f32; 2] = [1180.0, 700.0];
const PNG_CANVAS_SCALE: f32 = 2.0;

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

    let widths = [
        700.0, 700.0, 700.0, 874.0, 915.0, 945.0, 949.0, 938.0, 912.0, 880.0, 700.0, 655.0, 591.0,
        562.0, 550.0, 536.0, 621.0, 700.0,
    ];

    let mut warm = IndexMap::new();
    warm.insert("width".to_string(), ScalarValue::Float64(Some(700.0)));
    let (evaluated, metrics) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(warm))
        .await?;
    render_png_frame(
        &mut canvas,
        &evaluated.scene_graph,
        0,
        700.0,
        "Exact",
        &metrics,
    )
    .await?;

    for (idx, width) in widths.iter().copied().enumerate() {
        let mut patch = IndexMap::new();
        patch.insert("width".to_string(), ScalarValue::Float64(Some(width)));

        let eval_start = Instant::now();
        let (evaluated, metrics) = session
            .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(patch))
            .await?;
        let eval_elapsed = eval_start.elapsed();

        let (set_scene_elapsed, render_elapsed) = render_png_frame(
            &mut canvas,
            &evaluated.scene_graph,
            idx + 1,
            width,
            "Preview",
            &metrics,
        )
        .await?;

        println!(
            "png_replay seq={} mode=Preview width_param={:.1} scene={:.1}x{:.1} eval={:.2}ms set_scene={:.2}ms png_render={:.2}ms cells_built={} preview_reuse={} data_reuse={} data_miss={} sb_builds={} sd_hit={} sd_miss={} sd_collects={} mark_collects={} guide_ms={:.2} probe_ms={:.2} build_ms={:.2}",
            idx + 1,
            width,
            evaluated.scene_graph.width,
            evaluated.scene_graph.height,
            ms(eval_elapsed),
            ms(set_scene_elapsed),
            ms(render_elapsed),
            metrics.pipeline.facet_cells_built,
            metrics.pipeline.preview_profile_reuses,
            metrics.pipeline.preview_data_mark_reuses,
            metrics.pipeline.preview_data_mark_reuse_misses,
            metrics.pipeline.scale_builder_builds,
            metrics.pipeline.scale_domain_cache_hits,
            metrics.pipeline.scale_domain_cache_misses,
            metrics.pipeline.scale_domain_collects,
            metrics.pipeline.mark_data_collects,
            us_to_ms(metrics.timings.guide_overflow_measure_us),
            us_to_ms(metrics.timings.measure_cells_overflow_probe_us),
            us_to_ms(metrics.timings.build_plot_components_us),
        );
    }

    Ok(())
}

async fn build_plot(
    ctx: &SessionContext,
) -> Result<avenger_chart::plot::CompiledPlot, Box<dyn std::error::Error>> {
    let width = Param::new("width", ScalarValue::Float64(Some(700.0)));
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
                ('Alpha', 0.0, 0.4), ('Alpha', 1.0, 1.1), ('Alpha', 2.0, 1.8),
                ('Beta', 0.0, 1.4), ('Beta', 1.0, 1.8), ('Beta', 2.0, 2.7),
                ('Gamma', 0.0, 0.8), ('Gamma', 1.0, 2.2), ('Gamma', 2.0, 3.0),
                ('Delta', 0.0, 1.7), ('Delta', 1.0, 2.5), ('Delta', 2.0, 3.2),
                ('Epsilon', 0.0, 2.0), ('Epsilon', 1.0, 2.7), ('Epsilon', 2.0, 4.1),
                ('Zeta', 0.0, 2.4), ('Zeta', 1.0, 3.5), ('Zeta', 2.0, 4.6),
                ('Eta', 0.0, 1.1), ('Eta', 1.0, 3.1), ('Eta', 2.0, 5.2),
                ('Theta', 0.0, 2.8), ('Theta', 1.0, 4.2), ('Theta', 2.0, 5.6),
                ('Iota', 0.0, 3.0), ('Iota', 1.0, 4.4), ('Iota', 2.0, 6.0)
            ) AS t(facet, x, y)",
        )
        .await?;

    let child = Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill(col("facet"))
            .size(90.0),
    );

    Ok(Plot::<FacetWrap>::new()
        .add_param(width.clone())
        .canvas_constraint(CanvasConstraint::width(width.expr()))
        .plot_constraint(PlotConstraint::height(150.0))
        .data(df)
        .mark(Subplot::new(child).wrap_with(col("facet"), |c| {
            c.responsive_columns(190.0)
                .order_by(max(col("y")))
                .order_desc()
                .guide(|g| g.title("Facet"))
        }))
        .compile(ctx)
        .await?)
}

async fn render_png_frame(
    canvas: &mut PngCanvas,
    scene_graph: &avenger_scenegraph::scene_graph::SceneGraph,
    seq: usize,
    width: f64,
    mode: &str,
    metrics: &avenger_chart::render::EvaluationMetrics,
) -> Result<(Duration, Duration), Box<dyn std::error::Error>> {
    let set_scene_start = Instant::now();
    canvas.set_scene(scene_graph)?;
    let set_scene_elapsed = set_scene_start.elapsed();

    let render_start = Instant::now();
    let _image = canvas.render().await?;
    let render_elapsed = render_start.elapsed();

    if seq == 0 {
        println!(
            "png_replay seq=0 mode={mode} width_param={width:.1} scene={:.1}x{:.1} set_scene={:.2}ms png_render={:.2}ms guides={} guide_ms={:.2} probe_ms={:.2} build_ms={:.2}",
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
