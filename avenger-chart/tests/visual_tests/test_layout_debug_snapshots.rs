use crate::visual_tests::helpers::assert_visual_match_default_with_options;
use avenger_chart::plot::CompiledPlot;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

async fn compile_regular_debug_snapshot_plot(
    ctx: &SessionContext,
) -> Result<CompiledPlot, AvengerChartError> {
    let df = ctx
        .sql(
            r#"
            SELECT * FROM (VALUES
                (1.0, 2.0, 'A'),
                (2.0, 4.0, 'A'),
                (3.0, 3.0, 'B'),
                (4.0, 6.0, 'B'),
                (5.0, 5.0, 'C'),
                (6.0, 8.0, 'C'),
                (7.0, 7.0, 'A'),
                (8.0, 9.0, 'B')
            ) AS t(x_val, y_val, series)
            "#,
        )
        .await?;

    let plot = Plot::<Cartesian>::new()
        .canvas_size(540.0, 360.0)
        .title("Regular Chart")
        .subtitle("Allocation/demand debug overlay")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x_val"), |c| {
                    c.scale(|s| s.domain((0.0, 9.0)))
                        .axis(|a| a.title("x_val").grid(true))
                })
                .y_with(col("y_val"), |c| {
                    c.scale(|s| s.domain((0.0, 10.0)))
                        .axis(|a| a.title("y_val").grid(true))
                })
                .fill_with(col("series"), |c| {
                    c.legend(|l| l.title("Series").position(LegendPosition::Right))
                })
                .stroke("#2d3748")
                .stroke_width(1.0)
                .size(85.0),
        );

    plot.compile(ctx).await
}

#[tokio::test]
async fn regular_chart_components_debug_overlay() {
    let ctx = SessionContext::new();
    let compiled = compile_regular_debug_snapshot_plot(&ctx)
        .await
        .expect("compile regular debug snapshot plot");

    assert_visual_match_default_with_options(
        &compiled,
        &ctx,
        None,
        EvaluationOptions {
            layout_snapshot: LayoutSnapshot::Final,
            debug_layout_overlay: LayoutDebugOverlayMode::Components,
            ..EvaluationOptions::default()
        },
        "layout_debug",
        "regular_chart_components",
    )
    .await;
}

#[tokio::test]
async fn regular_chart_allocation_debug_overlay() {
    let ctx = SessionContext::new();
    let compiled = compile_regular_debug_snapshot_plot(&ctx)
        .await
        .expect("compile regular debug snapshot plot");

    assert_visual_match_default_with_options(
        &compiled,
        &ctx,
        None,
        EvaluationOptions {
            layout_snapshot: LayoutSnapshot::Final,
            debug_layout_overlay: LayoutDebugOverlayMode::AllocationDemand,
            ..EvaluationOptions::default()
        },
        "layout_debug_allocation",
        "regular_chart_allocation",
    )
    .await;
}
