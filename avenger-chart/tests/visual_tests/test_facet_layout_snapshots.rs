use crate::visual_tests::{
    datasets::{legend_sharing_hierarchy_df, numeric_hierarchy_df},
    helpers::assert_visual_match_default_with_options,
};
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
        Subplot::new(
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
        )
        .row_with(col("row_category"), |c| c.guide(|g| g.title("Category"))),
    );

    plot.compile(ctx).await
}

async fn compile_nested_mixed_facet_debug_snapshot_plot(
    ctx: &SessionContext,
) -> Result<CompiledPlot, AvengerChartError> {
    let df = legend_sharing_hierarchy_df(ctx).await;

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .canvas_size(960.0, 760.0)
        .mark(
            Subplot::new(
                Plot::<FacetColumn>::new().mark(
                    Subplot::new(
                        Plot::<FacetRow>::new().mark(
                            Subplot::new(
                                Plot::<Cartesian>::new().mark(
                                    Symbol::new()
                                        .x_with(col("x_val"), |c| {
                                            c.with_scale_sharing(ScaleSharing::Shared)
                                                .axis(|a| a.title("x_val"))
                                        })
                                        .y_with(col("y_val"), |c| {
                                            c.with_scale_sharing(ScaleSharing::Free)
                                                .axis(|a| a.title("y_val"))
                                        })
                                        .fill_with(col("category"), |c| {
                                            c.with_scale_sharing(ScaleSharing::Shared).legend(|l| {
                                                l.title("Category").position(LegendPosition::Right)
                                            })
                                        })
                                        .size(60.0),
                                ),
                            )
                            .row_with(col("team"), |c| c.guide(|g| g.title("Team"))),
                        ),
                    )
                    .col_with(col("department"), |c| c.guide(|g| g.title("Department"))),
                ),
            )
            .row_with(col("division"), |c| c.guide(|g| g.title("Division"))),
        );

    plot.compile(ctx).await
}

async fn compile_nested_column_facet_debug_snapshot_plot(
    ctx: &SessionContext,
) -> Result<CompiledPlot, AvengerChartError> {
    let df = legend_sharing_hierarchy_df(ctx).await;

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .canvas_size(1120.0, 380.0)
        .mark(
            Subplot::new(
                Plot::<FacetColumn>::new().mark(
                    Subplot::new(
                        Plot::<FacetColumn>::new().mark(
                            Subplot::new(
                                Plot::<Cartesian>::new().mark(
                                    Symbol::new()
                                        .x_with(col("x_val"), |c| {
                                            c.with_scale_sharing(ScaleSharing::Shared)
                                                .axis(|a| a.title("x_val"))
                                        })
                                        .y_with(col("y_val"), |c| {
                                            c.with_scale_sharing(ScaleSharing::Shared)
                                                .axis(|a| a.title("y_val"))
                                        })
                                        .fill_with(col("category"), |c| {
                                            c.with_scale_sharing(ScaleSharing::Level(2)).legend(
                                                |l| {
                                                    l.title("Category")
                                                        .position(LegendPosition::Right)
                                                },
                                            )
                                        })
                                        .stroke_with(col("category"), |c| {
                                            c.with_scale_sharing(ScaleSharing::Shared)
                                        })
                                        .stroke_width(2.0)
                                        .size(58.0),
                                ),
                            )
                            .col_with(col("team"), |c| c.guide(|g| g.title("Team"))),
                        ),
                    )
                    .col_with(col("department"), |c| c.guide(|g| g.title("Department"))),
                ),
            )
            .col_with(col("division"), |c| c.guide(|g| g.title("Division"))),
        );

    plot.compile(ctx).await
}

async fn compile_plot_size_numeric_facet_debug_snapshot_plot(
    ctx: &SessionContext,
) -> Result<CompiledPlot, AvengerChartError> {
    let df = numeric_hierarchy_df(ctx).await;

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .plot_size(110.0, 80.0)
        .mark(
            Subplot::new(
                Plot::<FacetColumn>::new().mark(
                    Subplot::new(
                        Plot::<FacetRow>::new().mark(
                            Subplot::new(
                                Plot::<Cartesian>::new().mark(
                                    Symbol::new()
                                        .x_with(col("x_val"), |c| {
                                            c.with_scale_sharing(ScaleSharing::Shared)
                                        })
                                        .y_with(col("y_val"), |c| {
                                            c.with_scale_sharing(ScaleSharing::Shared)
                                        })
                                        .size(58.0)
                                        .fill("#4682b4"),
                                ),
                            )
                            .row(col("team_id")),
                        ),
                    )
                    .column(col("dept_id")),
                ),
            )
            .column(col("division_id")),
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
            layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::LocalMeasured),
            debug_layout_overlay: LayoutDebugOverlayMode::Components,
            ..EvaluationOptions::default()
        },
        "facet_debug",
        "facet_row_varied_overflow_debug_initial",
    )
    .await;
}

#[tokio::test]
async fn nested_mixed_facet_layout_snapshot_final_with_debug_overlay() {
    let ctx = SessionContext::new();
    let compiled = compile_nested_mixed_facet_debug_snapshot_plot(&ctx)
        .await
        .expect("compile nested mixed facet debug snapshot plot");

    assert_visual_match_default_with_options(
        &compiled,
        &ctx,
        None,
        EvaluationOptions {
            layout_snapshot: LayoutSnapshot::Final,
            debug_layout_overlay: LayoutDebugOverlayMode::Components,
            ..EvaluationOptions::default()
        },
        "facet_debug",
        "facet_nested_mixed_debug_final",
    )
    .await;
}

#[tokio::test]
async fn nested_column_facet_layout_snapshot_final_with_debug_overlay() {
    let ctx = SessionContext::new();
    let compiled = compile_nested_column_facet_debug_snapshot_plot(&ctx)
        .await
        .expect("compile nested column facet debug snapshot plot");

    assert_visual_match_default_with_options(
        &compiled,
        &ctx,
        None,
        EvaluationOptions {
            layout_snapshot: LayoutSnapshot::Final,
            debug_layout_overlay: LayoutDebugOverlayMode::Components,
            ..EvaluationOptions::default()
        },
        "facet_debug",
        "facet_nested_column_debug_final",
    )
    .await;
}

#[tokio::test]
async fn plot_size_numeric_facet_layout_snapshot_final_with_debug_overlay() {
    let ctx = SessionContext::new();
    let compiled = compile_plot_size_numeric_facet_debug_snapshot_plot(&ctx)
        .await
        .expect("compile plot-size numeric facet debug snapshot plot");

    assert_visual_match_default_with_options(
        &compiled,
        &ctx,
        None,
        EvaluationOptions {
            layout_snapshot: LayoutSnapshot::Final,
            debug_layout_overlay: LayoutDebugOverlayMode::Components,
            ..EvaluationOptions::default()
        },
        "facet_debug",
        "facet_plot_size_nested_col_col_row_numeric_domain_order_debug_final",
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
            layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::Coordination(
                CoordinationCheckpoint::FinalPropagationComplete,
            )),
            debug_layout_overlay: LayoutDebugOverlayMode::Components,
            ..EvaluationOptions::default()
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
            debug_layout_overlay: LayoutDebugOverlayMode::Components,
            ..EvaluationOptions::default()
        },
        "facet_debug",
        "facet_row_varied_overflow_debug_final",
    )
    .await;
}

#[tokio::test]
async fn facet_layout_snapshot_initial_with_allocation_debug_overlay() {
    let ctx = SessionContext::new();
    let compiled = compile_facet_debug_snapshot_plot(&ctx)
        .await
        .expect("compile facet debug snapshot plot");

    assert_visual_match_default_with_options(
        &compiled,
        &ctx,
        None,
        EvaluationOptions {
            layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::LocalMeasured),
            debug_layout_overlay: LayoutDebugOverlayMode::AllocationDemand,
            ..EvaluationOptions::default()
        },
        "facet_debug_allocation",
        "facet_row_varied_overflow_debug_initial",
    )
    .await;
}

#[tokio::test]
async fn facet_layout_snapshot_coordinated_with_allocation_debug_overlay() {
    let ctx = SessionContext::new();
    let compiled = compile_facet_debug_snapshot_plot(&ctx)
        .await
        .expect("compile facet debug snapshot plot");

    assert_visual_match_default_with_options(
        &compiled,
        &ctx,
        None,
        EvaluationOptions {
            layout_snapshot: LayoutSnapshot::Whole(WholeChartSnapshot::Coordination(
                CoordinationCheckpoint::FinalPropagationComplete,
            )),
            debug_layout_overlay: LayoutDebugOverlayMode::AllocationDemand,
            ..EvaluationOptions::default()
        },
        "facet_debug_allocation",
        "facet_row_varied_overflow_debug_coordinated",
    )
    .await;
}

#[tokio::test]
async fn facet_layout_snapshot_final_with_allocation_debug_overlay() {
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
            debug_layout_overlay: LayoutDebugOverlayMode::AllocationDemand,
            ..EvaluationOptions::default()
        },
        "facet_debug_allocation",
        "facet_row_varied_overflow_debug_final",
    )
    .await;
}

#[tokio::test]
async fn nested_mixed_facet_layout_snapshot_final_with_allocation_debug_overlay() {
    let ctx = SessionContext::new();
    let compiled = compile_nested_mixed_facet_debug_snapshot_plot(&ctx)
        .await
        .expect("compile nested mixed facet debug snapshot plot");

    assert_visual_match_default_with_options(
        &compiled,
        &ctx,
        None,
        EvaluationOptions {
            layout_snapshot: LayoutSnapshot::Final,
            debug_layout_overlay: LayoutDebugOverlayMode::AllocationDemand,
            ..EvaluationOptions::default()
        },
        "facet_debug_allocation",
        "facet_nested_mixed_debug_final",
    )
    .await;
}

#[tokio::test]
async fn nested_column_facet_layout_snapshot_final_with_allocation_debug_overlay() {
    let ctx = SessionContext::new();
    let compiled = compile_nested_column_facet_debug_snapshot_plot(&ctx)
        .await
        .expect("compile nested column facet debug snapshot plot");

    assert_visual_match_default_with_options(
        &compiled,
        &ctx,
        None,
        EvaluationOptions {
            layout_snapshot: LayoutSnapshot::Final,
            debug_layout_overlay: LayoutDebugOverlayMode::AllocationDemand,
            ..EvaluationOptions::default()
        },
        "facet_debug_allocation",
        "facet_nested_column_debug_final",
    )
    .await;
}

#[tokio::test]
async fn plot_size_numeric_facet_layout_snapshot_final_with_allocation_debug_overlay() {
    let ctx = SessionContext::new();
    let compiled = compile_plot_size_numeric_facet_debug_snapshot_plot(&ctx)
        .await
        .expect("compile plot-size numeric facet debug snapshot plot");

    assert_visual_match_default_with_options(
        &compiled,
        &ctx,
        None,
        EvaluationOptions {
            layout_snapshot: LayoutSnapshot::Final,
            debug_layout_overlay: LayoutDebugOverlayMode::AllocationDemand,
            ..EvaluationOptions::default()
        },
        "facet_debug_allocation",
        "facet_plot_size_nested_col_col_row_numeric_domain_order_debug_final",
    )
    .await;
}
