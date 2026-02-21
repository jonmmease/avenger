use crate::visual_tests::helpers::assert_visual_match_default_with_options;
use avenger_chart::plot::CompiledPlot;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

async fn compile_facet_debug_snapshot_plot(
    ctx: &SessionContext,
) -> Result<CompiledPlot, AvengerChartError> {
    // Use varied axis label lengths to create meaningful overflow coordination behavior.
    let sql = r#"
        SELECT * FROM (VALUES
            ('short_labels', 'A', 10.0),
            ('short_labels', 'B', 20.0),
            ('short_labels', 'C', 30.0),
            ('long_labels', 'Very Long Category Name 1', 15.0),
            ('long_labels', 'Very Long Category Name 2', 25.0),
            ('long_labels', 'Very Long Category Name 3', 35.0)
        ) AS t(row_category, y_label, bar_val)
    "#;

    let df = ctx.sql(sql).await.expect("create data");

    let plot = Plot::<FacetRow>::new().data(df).canvas_size(600, 400).mark(
        Facet::new()
            .row_with(col("row_category"), |c| c.facet(|f| f.title("Category")))
            .subplot(
                Plot::<Cartesian>::new().mark(
                    Rect::new()
                        .y_with(col("y_label"), |c| {
                            c.scale_with::<Band>(|s| s)
                                .axis(|a| a.title("Y Axis").grid(false))
                        })
                        .y2_with(col(":y"), |c| c.band(1.0))
                        .x_with(lit(0.0), |c| {
                            c.scale(|s| s.domain((0.0, 40.0)))
                                .axis(|a| a.title("Value").grid(true))
                        })
                        .x2(col("bar_val"))
                        .fill("#4682b4")
                        .stroke("#2c5282")
                        .stroke_width(1.0),
                ),
            ),
    );

    plot.compile(ctx).await
}

#[tokio::test]
async fn facet_layout_snapshot_initial_with_debug_overlay() {
    let ctx = SessionContext::new();
    let compiled = compile_facet_debug_snapshot_plot(&ctx)
        .await
        .expect("compile facet debug snapshot plot");

    assert_visual_match_default_with_options(
        &compiled,
        &ctx,
        None,
        EvaluationOptions {
            layout_snapshot: LayoutSnapshot::Initial,
            debug_layout_lines: true,
        },
        "facet_debug",
        "facet_row_varied_overflow_debug_initial",
    )
    .await;
}

#[tokio::test]
async fn facet_layout_snapshot_coordinated_with_debug_overlay() {
    let ctx = SessionContext::new();
    let compiled = compile_facet_debug_snapshot_plot(&ctx)
        .await
        .expect("compile facet debug snapshot plot");

    assert_visual_match_default_with_options(
        &compiled,
        &ctx,
        None,
        EvaluationOptions {
            layout_snapshot: LayoutSnapshot::Coordinated,
            debug_layout_lines: true,
        },
        "facet_debug",
        "facet_row_varied_overflow_debug_coordinated",
    )
    .await;
}

#[tokio::test]
async fn facet_layout_snapshot_final_with_debug_overlay() {
    let ctx = SessionContext::new();
    let compiled = compile_facet_debug_snapshot_plot(&ctx)
        .await
        .expect("compile facet debug snapshot plot");

    assert_visual_match_default_with_options(
        &compiled,
        &ctx,
        None,
        EvaluationOptions {
            layout_snapshot: LayoutSnapshot::Final,
            debug_layout_lines: true,
        },
        "facet_debug",
        "facet_row_varied_overflow_debug_final",
    )
    .await;
}
