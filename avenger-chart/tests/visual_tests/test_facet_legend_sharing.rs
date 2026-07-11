use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::legend::LegendPosition;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

async fn make_legend_sharing_df(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "CREATE TABLE legend_sharing AS VALUES
        ('DivA', 'Dept1', 'Team1', 1.0, 1.0, 'Low'),
        ('DivA', 'Dept1', 'Team1', 1.4, 1.3, 'High'),
        ('DivA', 'Dept1', 'Team2', 2.0, 1.1, 'Low'),
        ('DivA', 'Dept1', 'Team2', 2.3, 1.4, 'High'),
        ('DivA', 'Dept2', 'Team1', 1.1, 2.0, 'Low'),
        ('DivA', 'Dept2', 'Team1', 1.5, 2.3, 'High'),
        ('DivA', 'Dept2', 'Team2', 2.1, 2.1, 'Low'),
        ('DivA', 'Dept2', 'Team2', 2.4, 2.4, 'High'),
        ('DivB', 'Dept1', 'Team1', 3.0, 1.0, 'Low'),
        ('DivB', 'Dept1', 'Team1', 3.4, 1.3, 'High'),
        ('DivB', 'Dept1', 'Team2', 4.0, 1.1, 'Low'),
        ('DivB', 'Dept1', 'Team2', 4.3, 1.4, 'High'),
        ('DivB', 'Dept2', 'Team1', 3.1, 2.0, 'Low'),
        ('DivB', 'Dept2', 'Team1', 3.5, 2.3, 'High'),
        ('DivB', 'Dept2', 'Team2', 4.1, 2.1, 'Low'),
        ('DivB', 'Dept2', 'Team2', 4.4, 2.4, 'High')",
    )
    .await
    .expect("create legend sharing test data");

    ctx.sql(
        "SELECT
            column1 AS division,
            column2 AS department,
            column3 AS team,
            column4 AS x_val,
            column5 AS y_val,
            column6 AS category
         FROM legend_sharing",
    )
    .await
    .expect("load legend sharing test data")
}

fn make_fill_legend_symbol(
    sharing: CoordinationScope,
    position: LegendPosition,
) -> Symbol<Cartesian> {
    Symbol::new()
        .x_with(col("x_val"), |c| {
            c.with_domain_scope(CoordinationScope::Shared)
        })
        .y_with(col("y_val"), |c| {
            c.with_domain_scope(CoordinationScope::Shared)
        })
        .fill_with(col("category"), move |c| {
            c.with_domain_scope(sharing.clone())
                .legend(|l| l.title("Category").position(position))
        })
        .size(70.0)
}

fn make_two_level_col_plot(
    df: DataFrame,
    sharing: CoordinationScope,
    position: LegendPosition,
) -> Chart<FacetColumn> {
    Chart::<FacetColumn>::new()
        .data(df)
        .canvas_size(960.0, 420.0)
        .mark(
            Subplot::new(
                Plot::<FacetColumn>::new().mark(
                    Subplot::new(
                        Plot::<Cartesian>::new().mark(make_fill_legend_symbol(sharing, position)),
                    )
                    .column(col("department")),
                ),
            )
            .column(col("division")),
        )
}

fn make_three_level_col_plot(
    df: DataFrame,
    sharing: CoordinationScope,
    position: LegendPosition,
) -> Chart<FacetColumn> {
    Chart::<FacetColumn>::new()
        .data(df)
        .canvas_size(1500.0, 380.0)
        .mark(
            Subplot::new(
                Plot::<FacetColumn>::new().mark(
                    Subplot::new(
                        Plot::<FacetColumn>::new().mark(
                            Subplot::new(
                                Plot::<Cartesian>::new()
                                    .mark(make_fill_legend_symbol(sharing, position)),
                            )
                            .column(col("team")),
                        ),
                    )
                    .column(col("department")),
                ),
            )
            .column(col("division")),
        )
}

fn make_two_level_col_plot_merged_group(df: DataFrame) -> Chart<FacetColumn> {
    Chart::<FacetColumn>::new()
        .data(df)
        .canvas_size(960.0, 420.0)
        .mark(
            Subplot::new(
                Plot::<FacetColumn>::new().mark(
                    Subplot::new(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("x_val"), |c| {
                                    c.with_domain_scope(CoordinationScope::Shared)
                                })
                                .y_with(col("y_val"), |c| {
                                    c.with_domain_scope(CoordinationScope::Shared)
                                })
                                .fill_with(col("category"), |c| {
                                    c.with_domain_scope(CoordinationScope::Level(1))
                                        .legend(|l| {
                                            l.title("Category").position(LegendPosition::Right)
                                        })
                                })
                                .stroke_with(col("category"), |c| {
                                    c.with_domain_scope(CoordinationScope::Shared)
                                })
                                .stroke_width(2.0)
                                .size(70.0),
                        ),
                    )
                    .column(col("department")),
                ),
            )
            .column(col("division")),
        )
}

#[tokio::test]
async fn test_facet_col_legend_sharing_level0_right() {
    let ctx = SessionContext::new();
    let df = make_legend_sharing_df(&ctx).await;

    let plot = make_two_level_col_plot(df, CoordinationScope::Free, LegendPosition::Right);
    let compiled = plot.compile(&ctx).await.expect("compile level0-right plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legend_sharing",
        "facet_col_legend_sharing_level0_right",
    )
    .await;
}

#[tokio::test]
async fn test_facet_col_legend_sharing_level1_right() {
    let ctx = SessionContext::new();
    let df = make_legend_sharing_df(&ctx).await;

    let plot = make_two_level_col_plot(df, CoordinationScope::Level(1), LegendPosition::Right);
    let compiled = plot.compile(&ctx).await.expect("compile level1-right plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legend_sharing",
        "facet_col_legend_sharing_level1_right",
    )
    .await;
}

#[tokio::test]
async fn test_facet_col_legend_sharing_level1_left() {
    let ctx = SessionContext::new();
    let df = make_legend_sharing_df(&ctx).await;

    let plot = make_two_level_col_plot(df, CoordinationScope::Level(1), LegendPosition::Left);
    let compiled = plot.compile(&ctx).await.expect("compile level1-left plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legend_sharing",
        "facet_col_legend_sharing_level1_left",
    )
    .await;
}

#[tokio::test]
async fn test_facet_col_legend_sharing_level1_top() {
    let ctx = SessionContext::new();
    let df = make_legend_sharing_df(&ctx).await;

    let plot = make_two_level_col_plot(df, CoordinationScope::Level(1), LegendPosition::Top);
    let compiled = plot.compile(&ctx).await.expect("compile level1-top plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legend_sharing",
        "facet_col_legend_sharing_level1_top",
    )
    .await;
}

#[tokio::test]
async fn test_facet_col_legend_sharing_level1_bottom() {
    let ctx = SessionContext::new();
    let df = make_legend_sharing_df(&ctx).await;

    let plot = make_two_level_col_plot(df, CoordinationScope::Level(1), LegendPosition::Bottom);
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile level1-bottom plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legend_sharing",
        "facet_col_legend_sharing_level1_bottom",
    )
    .await;
}

#[tokio::test]
async fn test_facet_col_legend_sharing_level2_right_three_levels() {
    let ctx = SessionContext::new();
    let df = make_legend_sharing_df(&ctx).await;

    let plot = make_three_level_col_plot(df, CoordinationScope::Level(2), LegendPosition::Right);
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile level2-right-three-level plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legend_sharing",
        "facet_col_legend_sharing_level2_right_three_levels",
    )
    .await;
}

#[tokio::test]
async fn test_facet_col_legend_sharing_shared255_right_phase1() {
    let ctx = SessionContext::new();
    let df = make_legend_sharing_df(&ctx).await;

    let plot = make_two_level_col_plot(df, CoordinationScope::Shared, LegendPosition::Right);
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile shared255-right-phase1 plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legend_sharing",
        "facet_col_legend_sharing_shared255_right_phase1",
    )
    .await;
}

#[tokio::test]
async fn test_facet_col_legend_sharing_merged_group_min_level() {
    let ctx = SessionContext::new();
    let df = make_legend_sharing_df(&ctx).await;

    let plot = make_two_level_col_plot_merged_group(df);
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile merged-group-min-level plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_legend_sharing",
        "facet_col_legend_sharing_merged_group_min_level",
    )
    .await;
}
