//! Structural parity: a chart at top level renders pixel-identically to the
//! same chart as the sole child of a concat, when the child declares no
//! layout spec of its own.
//!
//! The two paths use different sizing causality (the standalone chart solves
//! its frame canvas-first; the concat child solves plot-area-first inside
//! the parent's coordination loop), but both run on the same frame model and
//! must converge to the same geometry. This pins that equivalence through
//! layout refactors.

use avenger_chart::prelude::*;
use avenger_common::canvas::CanvasDimensions;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::prelude::*;
use image::RgbaImage;

fn parity_child() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::<Cartesian>::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("category"), |c| c.legend(|l| l.title("Category")))
            .size(96.0)
            .stroke("#ffffff")
            .stroke_width(1.0),
    )
}

async fn parity_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT column1 AS x, column2 AS y, column3 AS category
         FROM (VALUES
            (1.0, 2.0, 'Low'), (1.7, 2.6, 'High'), (2.4, 3.1, 'Low'),
            (3.1, 2.8, 'High'), (3.8, 3.6, 'Low'), (4.5, 4.1, 'High')
         )",
    )
    .await
    .expect("create parity data")
}

async fn render_image(
    compiled: &avenger_chart::plot::CompiledPlot,
    ctx: &SessionContext,
) -> RgbaImage {
    let result = compiled
        .evaluate(ctx, None)
        .await
        .expect("evaluate compiled plot");
    let dims = CanvasDimensions {
        size: [result.scene_graph.width, result.scene_graph.height],
        scale: 2.0,
    };
    let mut canvas = PngCanvas::new(dims, CanvasConfig::default())
        .await
        .expect("create canvas");
    canvas.set_scene(&result.scene_graph).expect("set scene");
    canvas.render().await.expect("render")
}

async fn assert_parity(standalone: Chart<Cartesian>, wrapped: Chart<HConcat>, label: &str) {
    let ctx = SessionContext::new();
    let standalone = standalone
        .compile(&ctx)
        .await
        .expect("compile standalone chart");
    let wrapped = wrapped
        .compile(&ctx)
        .await
        .expect("compile single-child concat");

    let a = render_image(&standalone, &ctx).await;
    let b = render_image(&wrapped, &ctx).await;

    assert_eq!(
        a.dimensions(),
        b.dimensions(),
        "{label}: canvas dimensions diverged between standalone and single-child concat"
    );
    assert_eq!(
        a.as_raw(),
        b.as_raw(),
        "{label}: pixels diverged between standalone and single-child concat"
    );
}

#[tokio::test]
async fn standalone_chart_matches_single_child_hconcat_fixed_canvas() {
    let ctx = SessionContext::new();
    let data = parity_data(&ctx).await;

    assert_parity(
        Chart::from_plot(parity_child().data(data.clone()))
            .canvas_size(400.0, 300.0)
            .title("Sole Child"),
        Chart::<HConcat>::new()
            .canvas_size(400.0, 300.0)
            .mark(Subplot::new(parity_child().data(data.clone())).caption("Sole Child")),
        "fixed canvas",
    )
    .await;
}

#[tokio::test]
async fn standalone_chart_matches_single_child_hconcat_auto_sizing() {
    let ctx = SessionContext::new();
    let data = parity_data(&ctx).await;

    assert_parity(
        Chart::from_plot(parity_child().data(data.clone())).title("Sole Child"),
        Chart::<HConcat>::new()
            .mark(Subplot::new(parity_child().data(data.clone())).caption("Sole Child")),
        "auto sizing",
    )
    .await;
}
