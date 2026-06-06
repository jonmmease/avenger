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

fn shared_x_child(data: DataFrame, title: &str) -> Plot<Cartesian> {
    Plot::<Cartesian>::new().data(data).title(title).mark(
        Symbol::<Cartesian>::new()
            .x_with(col("x"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .with_domain_scope(CoordinationScope::Shared)
                    .axis(|a| a.title("Shared x"))
            })
            .y_with(col("y"), |c| c.axis(|a| a.title("y")))
            .fill("#4682b4")
            .stroke("#ffffff")
            .stroke_width(1.0)
            .size(96.0),
    )
}

fn shared_x_child_no_title(data: DataFrame) -> Plot<Cartesian> {
    Plot::<Cartesian>::new().data(data).mark(
        Symbol::<Cartesian>::new()
            .x_with(col("x"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .with_domain_scope(CoordinationScope::Shared)
                    .axis(|a| a.title("Shared x"))
            })
            .y_with(col("y"), |c| c.axis(|a| a.title("y")))
            .fill("#4682b4")
            .stroke("#ffffff")
            .stroke_width(1.0)
            .size(96.0),
    )
}

fn shared_y_child(data: DataFrame, title: &str) -> Plot<Cartesian> {
    Plot::<Cartesian>::new().data(data).title(title).mark(
        Symbol::<Cartesian>::new()
            .x_with(col("x"), |c| c.axis(|a| a.title("x")))
            .y_with(col("y"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .with_domain_scope(CoordinationScope::Shared)
                    .axis(|a| a.title("Shared y"))
            })
            .fill("#4682b4")
            .stroke("#ffffff")
            .stroke_width(1.0)
            .size(96.0),
    )
}

fn shared_color_child(data: DataFrame, title: &str, position: LegendPosition) -> Plot<Cartesian> {
    Plot::<Cartesian>::new().data(data).title(title).mark(
        Symbol::<Cartesian>::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("category"), |c| {
                c.with_domain_scope(CoordinationScope::Shared)
                    .legend(|l| l.title("Category").position(position))
            })
            .size(96.0)
            .stroke("#ffffff")
            .stroke_width(1.0),
    )
}

fn level1_shared_x_child(data: DataFrame) -> Plot<Cartesian> {
    Plot::<Cartesian>::new().data(data).mark(
        Symbol::<Cartesian>::new()
            .x_with(col("x"), |c| {
                c.with_domain_scope(CoordinationScope::Level(1))
                    .axis(|a| a.title("Level 1 x"))
            })
            .y(col("y"))
            .fill("#4682b4")
            .stroke("#ffffff")
            .stroke_width(1.0)
            .size(80.0),
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
async fn hconcat_shared_x_domains() {
    let ctx = SessionContext::new();
    let left = ctx
        .sql(
            "SELECT column1 AS x, column2 AS y
             FROM (VALUES (1.0, 1.0), (1.5, 1.8), (2.0, 2.4))",
        )
        .await
        .expect("create left shared-domain concat data");
    let right = ctx
        .sql(
            "SELECT column1 AS x, column2 AS y
             FROM (VALUES (100.0, 1.2), (100.5, 2.1), (101.0, 2.8))",
        )
        .await
        .expect("create right shared-domain concat data");
    let plot = Plot::<HConcat>::new()
        .canvas_size(780.0, 320.0)
        .title("Shared concat x domain")
        .mark(Subplot::new(shared_x_child(left, "Local low x")).key("low"))
        .mark(Subplot::new(shared_x_child(right, "Local high x")).key("high"));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile shared-domain hconcat chart");
    assert_visual_match_default(&compiled, &ctx, None, "concat", "hconcat_shared_x_domains").await;
}

#[tokio::test]
async fn hconcat_shared_y_axis() {
    let ctx = SessionContext::new();
    let left = ctx
        .sql(
            "SELECT column1 AS x, column2 AS y
             FROM (VALUES (1.0, 1.0), (1.5, 1.8), (2.0, 2.4))",
        )
        .await
        .expect("create left shared-y concat data");
    let right = ctx
        .sql(
            "SELECT column1 AS x, column2 AS y
             FROM (VALUES (1.0, 100.0), (1.5, 100.5), (2.0, 101.0))",
        )
        .await
        .expect("create right shared-y concat data");
    let plot = Plot::<HConcat>::new()
        .canvas_size(780.0, 320.0)
        .title("Shared concat y axis")
        .mark(Subplot::new(shared_y_child(left, "Local low y")).key("low"))
        .mark(Subplot::new(shared_y_child(right, "Local high y")).key("high"));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile shared-y hconcat chart");
    assert_visual_match_default(&compiled, &ctx, None, "concat", "hconcat_shared_y_axis").await;
}

#[tokio::test]
async fn vconcat_shared_x_axis() {
    let ctx = SessionContext::new();
    let top = ctx
        .sql(
            "SELECT column1 AS x, column2 AS y
             FROM (VALUES (1.0, 1.0), (1.5, 1.8), (2.0, 2.4))",
        )
        .await
        .expect("create top shared-x concat data");
    let bottom = ctx
        .sql(
            "SELECT column1 AS x, column2 AS y
             FROM (VALUES (100.0, 1.2), (100.5, 2.1), (101.0, 2.8))",
        )
        .await
        .expect("create bottom shared-x concat data");
    let plot = Plot::<VConcat>::new()
        .canvas_size(560.0, 620.0)
        .title("Shared concat x axis")
        .mark(Subplot::new(shared_x_child_no_title(top)).key("low"))
        .mark(Subplot::new(shared_x_child_no_title(bottom)).key("high"));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile shared-x vconcat chart");
    assert_visual_match_default(&compiled, &ctx, None, "concat", "vconcat_shared_x_axis").await;
}

#[tokio::test]
async fn hconcat_shared_color_legend_right() {
    let ctx = SessionContext::new();
    let left = ctx
        .sql(
            "SELECT column1 AS x, column2 AS y, column3 AS category
             FROM (VALUES (1.0, 1.0, 'Low'), (1.5, 1.8, 'High'), (2.0, 2.4, 'Low'))",
        )
        .await
        .expect("create left shared legend concat data");
    let right = ctx
        .sql(
            "SELECT column1 AS x, column2 AS y, column3 AS category
             FROM (VALUES (1.0, 1.2, 'High'), (1.5, 2.1, 'Low'), (2.0, 2.8, 'High'))",
        )
        .await
        .expect("create right shared legend concat data");
    let plot = Plot::<HConcat>::new()
        .canvas_size(780.0, 320.0)
        .title("Shared concat legend")
        .mark(Subplot::new(shared_color_child(left, "Left", LegendPosition::Right)).key("left"))
        .mark(Subplot::new(shared_color_child(right, "Right", LegendPosition::Right)).key("right"));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile shared-legend hconcat chart");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "hconcat_shared_color_legend_right",
    )
    .await;
}

#[tokio::test]
async fn vconcat_shared_color_legend_bottom() {
    let ctx = SessionContext::new();
    let top = ctx
        .sql(
            "SELECT column1 AS x, column2 AS y, column3 AS category
             FROM (VALUES (1.0, 1.0, 'Low'), (1.5, 1.8, 'High'), (2.0, 2.4, 'Low'))",
        )
        .await
        .expect("create top shared legend concat data");
    let bottom = ctx
        .sql(
            "SELECT column1 AS x, column2 AS y, column3 AS category
             FROM (VALUES (1.0, 1.2, 'High'), (1.5, 2.1, 'Low'), (2.0, 2.8, 'High'))",
        )
        .await
        .expect("create bottom shared legend concat data");
    let plot = Plot::<VConcat>::new()
        .canvas_size(560.0, 620.0)
        .title("Shared concat legend")
        .mark(Subplot::new(shared_color_child(top, "Top", LegendPosition::Bottom)).key("top"))
        .mark(
            Subplot::new(shared_color_child(bottom, "Bottom", LegendPosition::Bottom))
                .key("bottom"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile shared-legend vconcat chart");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "vconcat_shared_color_legend_bottom",
    )
    .await;
}

#[tokio::test]
async fn nested_concat_level1_shared_axis() {
    let ctx = SessionContext::new();
    let datasets = [
        ctx.sql(
            "SELECT column1 AS x, column2 AS y
             FROM (VALUES (1.0, 1.0), (1.5, 1.8), (2.0, 2.4))",
        )
        .await
        .expect("create nested concat data a"),
        ctx.sql(
            "SELECT column1 AS x, column2 AS y
             FROM (VALUES (10.0, 1.2), (10.5, 2.1), (11.0, 2.8))",
        )
        .await
        .expect("create nested concat data b"),
        ctx.sql(
            "SELECT column1 AS x, column2 AS y
             FROM (VALUES (100.0, 1.5), (100.5, 2.0), (101.0, 2.7))",
        )
        .await
        .expect("create nested concat data c"),
        ctx.sql(
            "SELECT column1 AS x, column2 AS y
             FROM (VALUES (110.0, 1.3), (110.5, 2.2), (111.0, 2.9))",
        )
        .await
        .expect("create nested concat data d"),
    ];

    let left_column = Plot::<VConcat>::new()
        .mark(Subplot::new(level1_shared_x_child(datasets[0].clone())).key("lt"))
        .mark(Subplot::new(level1_shared_x_child(datasets[1].clone())).key("lb"));
    let right_column = Plot::<VConcat>::new()
        .mark(Subplot::new(level1_shared_x_child(datasets[2].clone())).key("rt"))
        .mark(Subplot::new(level1_shared_x_child(datasets[3].clone())).key("rb"));
    let plot = Plot::<HConcat>::new()
        .canvas_size(820.0, 560.0)
        .title("Nested Level(1) concat sharing")
        .mark(Subplot::new(left_column).key("left").label("Left"))
        .mark(Subplot::new(right_column).key("right").label("Right"));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile nested level1 concat chart");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "nested_concat_level1_shared_axis",
    )
    .await;
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
async fn hconcat_no_key_no_label() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = Plot::<HConcat>::new()
        .canvas_size(820.0, 360.0)
        .data(df)
        .title("Bare subplot concat")
        .mark(Subplot::new(sepal_child()))
        .mark(Subplot::new(petal_child()));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile bare hconcat chart");
    assert_visual_match_default(&compiled, &ctx, None, "concat", "hconcat_no_key_no_label").await;
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
        .mark(Subplot::new(subplot).row(col("species")));

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
