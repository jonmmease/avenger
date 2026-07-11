use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::functions_aggregate::average::avg;
use datafusion::prelude::*;

const BASELINE_CATEGORY: &str = "facet_data_scope";

async fn facet_scope_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT
            column1 AS facet_row,
            column2 AS facet_col,
            column3 AS x,
            column4 AS y
         FROM (VALUES
            ('North', 'West',   0.0, 0.20),
            ('North', 'West',   1.0, 0.35),
            ('North', 'West',   2.0, 0.50),
            ('North', 'East',  20.0, 0.50),
            ('North', 'East',  21.0, 0.65),
            ('North', 'East',  22.0, 0.80),
            ('South', 'West', 100.0, 0.20),
            ('South', 'West', 101.0, 0.35),
            ('South', 'West', 102.0, 0.50),
            ('South', 'East', 120.0, 0.50),
            ('South', 'East', 121.0, 0.65),
            ('South', 'East', 122.0, 0.80)
         )",
    )
    .await
    .expect("create facet data scope test data")
}

fn scoped_leaf(
    background_scope: FacetDataScope,
    aggregate_scope: Option<FacetDataScope>,
) -> Plot<Cartesian> {
    let mut plot = Plot::<Cartesian>::new()
        .mark(
            Symbol::<Cartesian>::new()
                .x(col("x"))
                .y(col("y"))
                .size(62.0)
                .fill("#cfd3da")
                .facet_data_scope(background_scope),
        )
        .mark(
            Symbol::<Cartesian>::new()
                .x(col("x"))
                .y(col("y"))
                .size(92.0)
                .fill("#2b6cb0")
                .stroke("#ffffff")
                .stroke_width(1.0),
        );

    if let Some(scope) = aggregate_scope {
        plot = plot.mark(
            Symbol::<Cartesian>::new()
                .x(avg(col("x")))
                .y(avg(col("y")))
                .size(720.0)
                .shape("diamond")
                .fill("#d94841")
                .stroke("#1f2933")
                .stroke_width(1.8)
                .facet_data_scope(scope),
        );
    }

    plot
}

fn scoped_facet_plot(
    df: DataFrame,
    background_scope: FacetDataScope,
    aggregate_scope: Option<FacetDataScope>,
    title: &str,
) -> Chart<FacetRow> {
    let leaf = scoped_leaf(background_scope, aggregate_scope);
    let col_plot = Plot::<FacetColumn>::new()
        .mark(Subplot::new(leaf).col_with(col("facet_col"), |c| c.guide(|g| g.title("Column"))));

    Chart::<FacetRow>::new()
        .data(df)
        .canvas_size(980, 680)
        .title(title)
        .mark(Subplot::new(col_plot).row_with(col("facet_row"), |c| c.guide(|g| g.title("Row"))))
}

async fn assert_facet_data_scope_baseline(
    name: &'static str,
    make_plot: impl FnOnce(DataFrame) -> Chart<FacetRow> + Send + 'static,
) {
    let ctx = SessionContext::new();
    let plot = make_plot(facet_scope_data(&ctx).await);
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile facet data scope plot");
    assert_visual_match_default(&compiled, &ctx, None, BASELINE_CATEGORY, name).await;
}

#[tokio::test]
async fn facet_data_scope_broadcast_background() {
    assert_facet_data_scope_baseline("facet_data_scope_broadcast_background", |df| {
        scoped_facet_plot(
            df,
            FacetDataScope::BROADCAST,
            None,
            "Broadcast background, filtered foreground",
        )
    })
    .await;
}

#[tokio::test]
async fn facet_data_scope_row_level_background() {
    assert_facet_data_scope_baseline("facet_data_scope_row_level_background", |df| {
        scoped_facet_plot(
            df,
            FacetDataScope::level(1),
            None,
            "Row-level background, filtered foreground",
        )
    })
    .await;
}

#[tokio::test]
async fn facet_data_scope_row_level_average_marker() {
    assert_facet_data_scope_baseline("facet_data_scope_row_level_average_marker", |df| {
        scoped_facet_plot(
            df,
            FacetDataScope::level(1),
            Some(FacetDataScope::level(1)),
            "Row-level average marker",
        )
    })
    .await;
}

#[tokio::test]
async fn facet_data_scope_filtered_average_marker() {
    assert_facet_data_scope_baseline("facet_data_scope_filtered_average_marker", |df| {
        scoped_facet_plot(
            df,
            FacetDataScope::level(1),
            Some(FacetDataScope::FILTERED),
            "Filtered average marker",
        )
    })
    .await;
}
