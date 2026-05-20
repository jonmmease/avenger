use super::{
    datasets::iris_with_petal_width_bin,
    helpers::{assert_visual_match_default, assert_visual_match_default_with_options},
};
use avenger_chart::prelude::*;
use datafusion::prelude::*;

async fn concat_numeric_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT
            column1 AS x,
            column2 AS y,
            column3 AS x2,
            column4 AS y2,
            column5 AS radius,
            column6 AS theta,
            column7 AS category
         FROM (VALUES
            (1.0, 2.0, 0.5, 1.2, 25.0, 0.0, 'Low'),
            (1.7, 2.6, 1.0, 1.8, 40.0, 0.7, 'High'),
            (2.4, 3.1, 1.8, 2.4, 55.0, 1.4, 'Low'),
            (3.1, 2.8, 2.5, 2.9, 70.0, 2.1, 'High'),
            (3.8, 3.6, 3.4, 3.5, 85.0, 2.8, 'Low'),
            (4.5, 4.1, 4.2, 4.2, 95.0, 3.5, 'High'),
            (5.2, 3.8, 5.0, 4.8, 65.0, 4.2, 'Low'),
            (5.9, 4.6, 5.8, 5.3, 45.0, 4.9, 'High')
         )",
    )
    .await
    .expect("create concat visual test data")
}

fn sepal_child() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().title("Sepal").mark(
        Symbol::<Cartesian>::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .fill_with(col("species"), |c| c.legend(|l| l.title("Species")))
            .stroke("#ffffff")
            .stroke_width(1.0),
    )
}

fn petal_child() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().title("Petal").mark(
        Symbol::<Cartesian>::new()
            .x(col("petal_length"))
            .y(col("petal_width"))
            .fill_with(col("petal_width_bin"), |c| {
                c.legend(|l| l.title("Width Bin"))
            })
            .stroke("#ffffff")
            .stroke_width(1.0),
    )
}

fn numeric_cartesian_child() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().title("Cartesian").mark(
        Symbol::<Cartesian>::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("category"), |c| c.legend(|l| l.title("Category")))
            .size(96.0)
            .stroke("#ffffff")
            .stroke_width(1.0),
    )
}

fn numeric_cartesian_child_alt() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().title("Alternate Axes").mark(
        Symbol::<Cartesian>::new()
            .x(col("x2"))
            .y(col("y2"))
            .fill_with(col("category"), |c| c.legend(|l| l.title("Category")))
            .shape_with(col("category"), |c| c.legend(|l| l.title("Category")))
            .size(112.0)
            .stroke("#333333")
            .stroke_width(1.0),
    )
}

fn numeric_polar_child() -> Plot<Polar> {
    Plot::<Polar>::new().title("Polar").mark(
        Symbol::<Polar>::new()
            .r_with(col("radius"), |c| {
                c.scale_with::<Linear>(|s| s.domain((0.0, 110.0)))
            })
            .theta(col("theta"))
            .fill_with(col("category"), |c| c.legend(|l| l.title("Category")))
            .size(128.0)
            .stroke("#333333")
            .stroke_width(1.0),
    )
}

#[tokio::test]
async fn hconcat_two_cartesian() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = Plot::<HConcat>::new()
        .canvas_size(820.0, 360.0)
        .data(df)
        .title("Horizontal concat")
        .mark(Subplot::new(sepal_child()).key("sepal").label("Sepal"))
        .mark(Subplot::new(petal_child()).key("petal").label("Petal"));

    let compiled = plot.compile(&ctx).await.expect("compile hconcat chart");
    assert_visual_match_default(&compiled, &ctx, None, "concat", "hconcat_two_cartesian").await;
}

#[tokio::test]
async fn vconcat_two_cartesian() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = Plot::<VConcat>::new()
        .canvas_size(560.0, 720.0)
        .data(df)
        .title("Vertical concat")
        .mark(Subplot::new(sepal_child()).key("sepal").label("Sepal"))
        .mark(Subplot::new(petal_child()).key("petal").label("Petal"));

    let compiled = plot.compile(&ctx).await.expect("compile vconcat chart");
    assert_visual_match_default(&compiled, &ctx, None, "concat", "vconcat_two_cartesian").await;
}

#[tokio::test]
async fn hconcat_plot_size_two_cartesian() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = Plot::<HConcat>::new()
        .plot_size(620.0, 240.0)
        .data(df)
        .title("Plot-size concat")
        .mark(Subplot::new(sepal_child()).key("sepal").label("Sepal"))
        .mark(Subplot::new(petal_child()).key("petal").label("Petal"));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile plot-size hconcat chart");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "hconcat_plot_size_two_cartesian",
    )
    .await;
}

#[tokio::test]
async fn hconcat_cartesian_polar() {
    let ctx = SessionContext::new();
    let df = concat_numeric_data(&ctx).await;
    let plot = Plot::<HConcat>::new()
        .canvas_size(820.0, 380.0)
        .data(df)
        .title("Mixed coordinate concat")
        .mark(
            Subplot::new(numeric_cartesian_child())
                .key("cartesian")
                .label("Cartesian"),
        )
        .mark(
            Subplot::new(numeric_polar_child())
                .key("polar")
                .label("Polar"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile mixed concat chart");
    assert_visual_match_default(&compiled, &ctx, None, "concat", "hconcat_cartesian_polar").await;
}

#[tokio::test]
async fn nested_concat_grid() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let top_row = Plot::<HConcat>::new()
        .mark(Subplot::new(sepal_child()).key("sepal"))
        .mark(Subplot::new(petal_child()).key("petal"));
    let plot = Plot::<VConcat>::new()
        .canvas_size(820.0, 640.0)
        .data(df)
        .title("Nested concat")
        .mark(Subplot::new(top_row).key("top-row").label("Top Row"))
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().title("Combined").mark(
                    Symbol::<Cartesian>::new()
                        .x(col("sepal_length"))
                        .y(col("petal_length"))
                        .fill_with(col("species"), |c| c.legend(|l| l.title("Species")))
                        .size(80.0),
                ),
            )
            .key("combined")
            .label("Combined"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile nested concat chart");
    assert_visual_match_default(&compiled, &ctx, None, "concat", "nested_concat_grid").await;
}

#[tokio::test]
async fn facet_row_hconcat_shared_data() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let subplot = Plot::<HConcat>::new()
        .mark(Subplot::new(sepal_child()).key("sepal").label("Sepal"))
        .mark(Subplot::new(petal_child()).key("petal").label("Petal"));
    let plot = Plot::<FacetRow>::new()
        .canvas_size(860.0, 760.0)
        .data(df)
        .mark(Facet::new().row(col("species")).subplot(subplot));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile facet of hconcat chart");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "facet_row_hconcat_shared_data",
    )
    .await;
}

#[tokio::test]
async fn hconcat_components_debug() {
    let ctx = SessionContext::new();
    let df = concat_numeric_data(&ctx).await;
    let plot = Plot::<HConcat>::new()
        .canvas_size(820.0, 380.0)
        .data(df)
        .title("Debug concat")
        .mark(
            Subplot::new(numeric_cartesian_child())
                .key("cartesian")
                .label("Cartesian"),
        )
        .mark(
            Subplot::new(numeric_cartesian_child_alt())
                .key("alternate")
                .label("Alternate"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile debug concat chart");
    assert_visual_match_default_with_options(
        &compiled,
        &ctx,
        None,
        EvaluationOptions {
            debug_layout_overlay: LayoutDebugOverlayMode::Components,
            layout_snapshot: LayoutSnapshot::Final,
            ..EvaluationOptions::default()
        },
        "concat",
        "hconcat_components_debug",
    )
    .await;
}
