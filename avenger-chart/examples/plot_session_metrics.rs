//! Print cache metrics for one-shot and reusable session evaluation.

use std::sync::Arc;

use avenger_chart::prelude::*;
use avenger_chart::render::EvaluationMetrics;
use datafusion::{prelude::SessionContext, scalar::ScalarValue};
use indexmap::IndexMap;

fn print_metrics(label: &str, metrics: &EvaluationMetrics) {
    println!(
        "{label}: scale hits/misses={}/{}, guide hits/misses={}/{}, preview reuses/misses/fallbacks={}/{}/{}, component measures={}, skipped={}",
        metrics.pipeline.scale_domain_cache_hits,
        metrics.pipeline.scale_domain_cache_misses,
        metrics.pipeline.guide_overflow_cache_hits,
        metrics.pipeline.guide_overflow_cache_misses,
        metrics.pipeline.preview_profile_reuses,
        metrics.pipeline.preview_profile_misses,
        metrics.pipeline.preview_fallbacks,
        metrics.facet_layout.plot_component_measure_calls,
        metrics.pipeline.skipped_component_measure_calls,
    );
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = Arc::new(SessionContext::new());

    run_resize_scenario(ctx.clone()).await?;
    run_responsive_wrap_scenario(ctx.clone()).await?;
    run_pan_zoom_scenario(ctx.clone()).await?;

    Ok(())
}

async fn run_resize_scenario(ctx: Arc<SessionContext>) -> Result<(), Box<dyn std::error::Error>> {
    let width = Param::new("width", ScalarValue::Float64(Some(420.0)));
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
                (1.0, 2.0), (2.0, 3.5), (3.0, 5.0), (4.0, 4.0)
            ) AS t(x, y)",
        )
        .await?;

    let plot = Plot::<Cartesian>::new()
        .add_param(width.clone())
        .canvas_size(width.expr(), 320.0)
        .data(df)
        .mark(Symbol::new().x(col("x")).y(col("y")).size(24.0));

    let compiled = Arc::new(plot.compile(ctx.as_ref()).await?);

    println!("\nregular resize");
    let (_evaluated, one_shot) = compiled
        .evaluate_with_options_and_metrics(ctx.as_ref(), None, EvaluationOptions::default())
        .await?;
    print_metrics("one-shot exact", &one_shot);

    let mut session = compiled.instantiate(ctx.clone());
    let (_evaluated, warm_exact) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact())
        .await?;
    print_metrics("session warm exact", &warm_exact);

    let mut resize = IndexMap::new();
    resize.insert("width".to_string(), ScalarValue::Float64(Some(640.0)));
    let (_evaluated, warm_resize) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(resize))
        .await?;
    print_metrics("session exact resize", &warm_resize);

    let mut preview_resize = IndexMap::new();
    preview_resize.insert("width".to_string(), ScalarValue::Float64(Some(720.0)));
    let (_evaluated, preview) = session
        .evaluate_with_metrics(
            EvaluationRequest::new()
                .preview()
                .param_patch(preview_resize),
        )
        .await?;
    print_metrics("session preview resize", &preview);

    let (_evaluated, settle) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact())
        .await?;
    print_metrics("session exact settle", &settle);

    Ok(())
}

async fn run_responsive_wrap_scenario(
    ctx: Arc<SessionContext>,
) -> Result<(), Box<dyn std::error::Error>> {
    let width = Param::new("width", ScalarValue::Float64(Some(420.0)));
    let df = ctx
        .sql(
            "SELECT * FROM (VALUES
                ('A', 1.0, 2.0), ('B', 2.0, 3.0), ('C', 3.0, 4.0),
                ('D', 4.0, 5.0), ('E', 5.0, 6.0), ('F', 6.0, 7.0)
            ) AS t(facet, x, y)",
        )
        .await?;

    let plot = Plot::<FacetWrap>::new()
        .add_param(width.clone())
        .canvas_constraint(CanvasConstraint::width(width.expr()))
        .plot_constraint(PlotConstraint::height(120.0))
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y")).size(18.0)),
            )
            .wrap_with(col("facet"), |c| c.responsive_columns(180.0)),
        );

    let compiled = Arc::new(plot.compile(ctx.as_ref()).await?);
    let mut session = compiled.instantiate(ctx.clone());

    println!("\nresponsive wrap width sweep");
    let (_evaluated, warm) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact())
        .await?;
    print_metrics("wrap warm exact", &warm);

    let mut wider = IndexMap::new();
    wider.insert("width".to_string(), ScalarValue::Float64(Some(900.0)));
    let (_evaluated, exact_wide) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact().param_patch(wider))
        .await?;
    print_metrics("wrap exact wide", &exact_wide);

    Ok(())
}

async fn run_pan_zoom_scenario(ctx: Arc<SessionContext>) -> Result<(), Box<dyn std::error::Error>> {
    let x_min = Param::new("x_min", ScalarValue::Float64(Some(0.0)));
    let x_max = Param::new("x_max", ScalarValue::Float64(Some(10.0)));
    let domain_min = x_min.clone();
    let domain_max = x_max.clone();
    let df = ctx
        .sql("SELECT * FROM (VALUES (1.0, 2.0), (3.0, 3.0), (8.0, 5.0)) AS t(x, y)")
        .await?;

    let plot = Plot::<Cartesian>::new()
        .add_param(x_min.clone())
        .add_param(x_max.clone())
        .canvas_size(420.0, 320.0)
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), move |c| {
                    c.scale_with::<Linear>(move |s| {
                        s.domain((domain_min.expr(), domain_max.expr()))
                            .nice(false)
                            .zero(false)
                    })
                })
                .y(col("y"))
                .size(22.0),
        );

    let compiled = Arc::new(plot.compile(ctx.as_ref()).await?);
    let mut session = compiled.instantiate(ctx.clone());

    println!("\npan/zoom domain params");
    let (_evaluated, warm) = session
        .evaluate_with_metrics(EvaluationRequest::new().exact())
        .await?;
    print_metrics("pan warm exact", &warm);

    let mut zoom = IndexMap::new();
    zoom.insert("x_min".to_string(), ScalarValue::Float64(Some(2.0)));
    zoom.insert("x_max".to_string(), ScalarValue::Float64(Some(6.0)));
    let (_evaluated, preview) = session
        .evaluate_with_metrics(EvaluationRequest::new().preview().param_patch(zoom))
        .await?;
    print_metrics("pan preview", &preview);

    Ok(())
}
