use crate::visual_tests::datasets::{iris_with_petal_width_bin, legend_sharing_hierarchy_df};
use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::legend::LegendPosition;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

fn nested_col_row_plot(
    df: DataFrame,
    x_sharing: CoordinationScope,
    y_sharing: CoordinationScope,
) -> Plot<FacetColumn> {
    Plot::<FacetColumn>::new()
        .data(df)
        .plot_size(120.0, 90.0)
        .mark(
            Subplot::new(
                Plot::<FacetRow>::new().mark(
                    Subplot::new(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("sepal_length"), move |c| {
                                    c.with_scale_sharing(x_sharing.clone())
                                })
                                .y_with(col("sepal_width"), move |c| {
                                    c.with_scale_sharing(y_sharing.clone())
                                })
                                .size(24.0)
                                .fill("#4682b4"),
                        ),
                    )
                    .row(col("species")),
                ),
            )
            .column(col("petal_width_bin")),
        )
}

fn nested_row_col_plot(
    df: DataFrame,
    x_sharing: CoordinationScope,
    y_sharing: CoordinationScope,
) -> Plot<FacetRow> {
    Plot::<FacetRow>::new()
        .data(df)
        .plot_size(120.0, 90.0)
        .mark(
            Subplot::new(
                Plot::<FacetColumn>::new().mark(
                    Subplot::new(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("sepal_length"), move |c| {
                                    c.with_scale_sharing(x_sharing.clone())
                                })
                                .y_with(col("sepal_width"), move |c| {
                                    c.with_scale_sharing(y_sharing.clone())
                                })
                                .size(24.0)
                                .fill("#4682b4"),
                        ),
                    )
                    .column(col("petal_width_bin")),
                ),
            )
            .row(col("species")),
        )
}

fn two_level_col_legend_plot(
    df: DataFrame,
    sharing: CoordinationScope,
    position: LegendPosition,
) -> Plot<FacetColumn> {
    Plot::<FacetColumn>::new()
        .data(df)
        .plot_size(120.0, 90.0)
        .mark(
            Subplot::new(
                Plot::<FacetColumn>::new().mark(
                    Subplot::new(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("x_val"), |c| {
                                    c.with_scale_sharing(CoordinationScope::Shared)
                                })
                                .y_with(col("y_val"), |c| {
                                    c.with_scale_sharing(CoordinationScope::Shared)
                                })
                                .fill_with(col("category"), move |c| {
                                    c.with_scale_sharing(sharing.clone())
                                        .legend(|l| l.title("Category").position(position))
                                })
                                .size(64.0),
                        ),
                    )
                    .column(col("department")),
                ),
            )
            .column(col("division")),
        )
}

#[tokio::test]
async fn facet_plot_size_nested_col_row_free_scales() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = nested_col_row_plot(df, CoordinationScope::Free, CoordinationScope::Free);
    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_plot_size",
        "facet_plot_size_nested_col_row_free_scales",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_nested_col_row_shared_both() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = nested_col_row_plot(df, CoordinationScope::Shared, CoordinationScope::Shared);
    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_plot_size",
        "facet_plot_size_nested_col_row_shared_both",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_nested_col_row_shared_x() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = nested_col_row_plot(df, CoordinationScope::Shared, CoordinationScope::Free);
    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_plot_size",
        "facet_plot_size_nested_col_row_shared_x",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_nested_col_row_shared_y() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = nested_col_row_plot(df, CoordinationScope::Free, CoordinationScope::Shared);
    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_plot_size",
        "facet_plot_size_nested_col_row_shared_y",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_nested_row_col_free_scales() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = nested_row_col_plot(df, CoordinationScope::Free, CoordinationScope::Free);
    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_plot_size",
        "facet_plot_size_nested_row_col_free_scales",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_nested_row_col_shared_both() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = nested_row_col_plot(df, CoordinationScope::Shared, CoordinationScope::Shared);
    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_plot_size",
        "facet_plot_size_nested_row_col_shared_both",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_nested_col_col_legend_level1_right() {
    let ctx = SessionContext::new();
    let df = legend_sharing_hierarchy_df(&ctx).await;
    let plot = two_level_col_legend_plot(df, CoordinationScope::Level(1), LegendPosition::Right);
    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_plot_size",
        "facet_plot_size_nested_col_col_legend_level1_right",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_nested_col_col_legend_level1_left() {
    let ctx = SessionContext::new();
    let df = legend_sharing_hierarchy_df(&ctx).await;
    let plot = two_level_col_legend_plot(df, CoordinationScope::Level(1), LegendPosition::Left);
    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_plot_size",
        "facet_plot_size_nested_col_col_legend_level1_left",
    )
    .await;
}
