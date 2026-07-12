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

async fn concat_facet_alignment_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "SELECT
            column1 AS x,
            column2 AS y,
            column3 AS x2,
            column4 AS y2,
            column5 AS group_name
         FROM (VALUES
            (1.0, 1.1, 11.0, 101.0, 'Alpha'),
            (1.4, 1.8, 14.0, 118.0, 'Alpha'),
            (1.9, 2.4, 19.0, 124.0, 'Alpha'),
            (2.5, 3.1, 25.0, 131.0, 'Alpha'),
            (90.0, 920.0, 910.0, 2100.0, 'Beta'),
            (140.0, 980.0, 1140.0, 2380.0, 'Beta'),
            (190.0, 1040.0, 1390.0, 2640.0, 'Beta'),
            (250.0, 1110.0, 1650.0, 2910.0, 'Beta'),
            (-12.0, -30.0, 0.12, 0.03, 'Gamma'),
            (-8.0, -22.0, 0.19, 0.08, 'Gamma'),
            (-4.0, -14.0, 0.27, 0.13, 'Gamma'),
            (2.0, -6.0, 0.34, 0.19, 'Gamma')
         )",
    )
    .await
    .expect("create concat/facet alignment data")
}

fn sepal_child() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::<Cartesian>::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .fill_with(col("species"), |c| c.legend(|l| l.title("Species")))
            .stroke("#ffffff")
            .stroke_width(1.0),
    )
}

fn petal_child() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
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

fn numeric_cartesian_child_alt() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
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

fn shared_x_child(data: DataFrame) -> Plot<Cartesian> {
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

fn shared_y_child(data: DataFrame) -> Plot<Cartesian> {
    Plot::<Cartesian>::new().data(data).mark(
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

fn grid_shared_axes_child(
    x: &'static str,
    y: &'static str,
    _title: &str,
    fill: &str,
) -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::<Cartesian>::new()
            .x_with(col(x), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .with_domain_scope(CoordinationScope::Shared)
                    .axis(|a| a.title(x))
            })
            .y_with(col(y), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .with_domain_scope(CoordinationScope::Shared)
                    .axis(|a| a.title(y))
            })
            .fill(fill)
            .opacity(0.72)
            .stroke("#ffffff")
            .stroke_width(0.75)
            .size(54.0),
    )
}

fn grid_splom_child(x: &'static str, y: &'static str) -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::<Cartesian>::new()
            .x_with(col(x), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .with_domain_group(x)
                    .share_domain()
                    .axis(|a| a.title(x))
            })
            .y_with(col(y), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .with_domain_group(y)
                    .share_domain()
                    .axis(|a| a.title(y))
            })
            .fill_with(col("species"), |c| {
                c.with_domain_scope(CoordinationScope::Shared)
                    .legend(|l| l.title("Species"))
            })
            .opacity(0.78)
            .stroke("#ffffff")
            .stroke_width(0.5)
            .size(34.0),
    )
}

fn grid_span_child(x: &'static str, y: &'static str, fill: &str) -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::<Cartesian>::new()
            .x_with(col(x), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .axis(|a| a.title(x))
            })
            .y_with(col(y), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .axis(|a| a.title(y))
            })
            .fill(fill)
            .opacity(0.78)
            .stroke("#ffffff")
            .stroke_width(0.75)
            .size(72.0),
    )
}

fn alignment_grid_cell(x: &'static str, y: &'static str, fill: &str) -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::<Cartesian>::new()
            .x_with(col(x), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .axis(|a| a.title(x))
            })
            .y_with(col(y), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .axis(|a| a.title(y))
            })
            .fill(fill)
            .opacity(0.78)
            .stroke("#ffffff")
            .stroke_width(0.75)
            .size(64.0),
    )
}

fn alignment_spanned_grid_concat() -> Plot<GridConcat> {
    Plot::<GridConcat>::new()
        .configure_coord(|c| {
            c.rows(2)
                .columns(3)
                .axis_guide_visibility(AxisGuideVisibilityPolicy::OuterEdges)
        })
        .mark(
            Subplot::new(alignment_grid_cell("x", "y", "#2f7ed8"))
                .at(0, 0)
                .span(2, 2)
                .name("spanned_xy")
                .label("Spanned xy"),
        )
        .mark(
            Subplot::new(alignment_grid_cell("x2", "y", "#8bbc21"))
                .at(0, 2)
                .name("top_right")
                .label("Top right"),
        )
        .mark(
            Subplot::new(alignment_grid_cell("x", "y2", "#f28f43"))
                .at(1, 2)
                .name("bottom_right")
                .label("Bottom right"),
        )
}

fn alignment_grid_concat() -> Plot<GridConcat> {
    Plot::<GridConcat>::new()
        .configure_coord(|c| {
            c.rows(2)
                .columns(2)
                .axis_guide_visibility(AxisGuideVisibilityPolicy::OuterEdges)
        })
        .mark(
            Subplot::new(alignment_grid_cell("x", "y", "#2f7ed8"))
                .at(0, 0)
                .name("xy"),
        )
        .mark(
            Subplot::new(alignment_grid_cell("x2", "y", "#8bbc21"))
                .at(0, 1)
                .name("x2y"),
        )
        .mark(
            Subplot::new(alignment_grid_cell("x", "y2", "#f28f43"))
                .at(1, 0)
                .name("xy2"),
        )
        .mark(
            Subplot::new(alignment_grid_cell("x2", "y2", "#910000"))
                .at(1, 1)
                .name("x2y2"),
        )
}

fn alignment_wrap_concat() -> Plot<WrapConcat> {
    Plot::<WrapConcat>::new()
        .configure_coord(|c| {
            c.columns(2)
                .axis_guide_visibility(AxisGuideVisibilityPolicy::OuterEdges)
        })
        .mark(Subplot::new(alignment_grid_cell("x", "y", "#2f7ed8")).name("xy"))
        .mark(Subplot::new(alignment_grid_cell("x2", "y", "#8bbc21")).name("x2y"))
        .mark(Subplot::new(alignment_grid_cell("x", "y2", "#f28f43")).name("xy2"))
}

fn facet_column_alignment_cell(x: &'static str, y: &'static str, fill: &str) -> Plot<FacetColumn> {
    let child = Plot::<Cartesian>::new().mark(
        Symbol::<Cartesian>::new()
            .x_with(col(x), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .axis(|a| a.title(x))
            })
            .y_with(col(y), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .axis(|a| a.title(y))
            })
            .fill(fill)
            .opacity(0.78)
            .stroke("#ffffff")
            .stroke_width(0.75)
            .size(58.0),
    );
    Plot::<FacetColumn>::new().mark(Subplot::new(child).column(col("group_name")))
}

fn shared_color_child(data: DataFrame, position: LegendPosition) -> Plot<Cartesian> {
    Plot::<Cartesian>::new().data(data).mark(
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

fn local_right_legend_child(
    x: &'static str,
    y: &'static str,
    y_title: &'static str,
) -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::<Cartesian>::new()
            .x_with(col(x), |c| c.axis(|a| a.title(x)))
            .y_with(col(y), |c| c.axis(|a| a.title(y_title)))
            .fill_with(col("group_name"), |c| {
                c.legend(|l| l.title("group").position(LegendPosition::Right))
            })
            .size(88.0)
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
    Plot::<Polar>::new().mark(
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
    let plot = Chart::<HConcat>::new()
        .canvas_size(780.0, 320.0)
        .title("Shared concat x domain")
        .mark(
            Subplot::new(shared_x_child(left))
                .caption("Local low x")
                .name("low"),
        )
        .mark(
            Subplot::new(shared_x_child(right))
                .caption("Local high x")
                .name("high"),
        );

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
    let plot = Chart::<HConcat>::new()
        .canvas_size(780.0, 320.0)
        .title("Shared concat y axis")
        .mark(
            Subplot::new(shared_y_child(left))
                .caption("Local low y")
                .name("low"),
        )
        .mark(
            Subplot::new(shared_y_child(right))
                .caption("Local high y")
                .name("high"),
        );

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
    let plot = Chart::<VConcat>::new()
        .canvas_size(560.0, 620.0)
        .title("Shared concat x axis")
        .mark(Subplot::new(shared_x_child_no_title(top)).name("low"))
        .mark(Subplot::new(shared_x_child_no_title(bottom)).name("high"));

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
    let plot = Chart::<HConcat>::new()
        .canvas_size(780.0, 320.0)
        .title("Shared concat legend")
        .mark(
            Subplot::new(shared_color_child(left, LegendPosition::Right))
                .caption("Left")
                .name("left"),
        )
        .mark(
            Subplot::new(shared_color_child(right, LegendPosition::Right))
                .caption("Right")
                .name("right"),
        );

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
async fn grid_concat_local_right_legends_coordinated_chrome() {
    let ctx = SessionContext::new();
    let plot = Chart::<GridConcat>::new()
        .canvas_size(940.0, 300.0)
        .data(concat_facet_alignment_data(&ctx).await)
        .title("Local right legends")
        .mark(
            Subplot::new(local_right_legend_child("x", "y", "y"))
                .caption("Small y labels")
                .at(0, 0)
                .name("small-y"),
        )
        .mark(
            Subplot::new(local_right_legend_child("x2", "y2", "wide y"))
                .caption("Large y labels")
                .at(0, 1)
                .name("large-y"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile local right legend GridConcat chart");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "grid_concat_local_right_legends_coordinated_chrome",
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
    let plot = Chart::<VConcat>::new()
        .canvas_size(560.0, 620.0)
        .title("Shared concat legend")
        .mark(
            Subplot::new(shared_color_child(top, LegendPosition::Bottom))
                .caption("Top")
                .name("top"),
        )
        .mark(
            Subplot::new(shared_color_child(bottom, LegendPosition::Bottom))
                .caption("Bottom")
                .name("bottom"),
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
        .mark(Subplot::new(level1_shared_x_child(datasets[0].clone())).name("lt"))
        .mark(Subplot::new(level1_shared_x_child(datasets[1].clone())).name("lb"));
    let right_column = Plot::<VConcat>::new()
        .mark(Subplot::new(level1_shared_x_child(datasets[2].clone())).name("rt"))
        .mark(Subplot::new(level1_shared_x_child(datasets[3].clone())).name("rb"));
    let plot = Chart::<HConcat>::new()
        .canvas_size(820.0, 560.0)
        .title("Nested Level(1) concat sharing")
        .mark(Subplot::new(left_column).name("left").label("Left"))
        .mark(Subplot::new(right_column).name("right").label("Right"));

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
async fn grid_concat_shared_axes_complete() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = Chart::<GridConcat>::new()
        .canvas_size(820.0, 620.0)
        .data(df)
        .title("Grid concat shared axes")
        .configure_coord(|c| c.rows(2).columns(2))
        .mark(
            Subplot::new(grid_shared_axes_child(
                "sepal_length",
                "sepal_width",
                "Sepal width",
                "#2f7ed8",
            ))
            .at(0, 0)
            .name("sepal_width"),
        )
        .mark(
            Subplot::new(grid_shared_axes_child(
                "petal_length",
                "sepal_width",
                "Sepal by petal",
                "#8bbc21",
            ))
            .at(0, 1)
            .name("petal_sepal"),
        )
        .mark(
            Subplot::new(grid_shared_axes_child(
                "sepal_length",
                "petal_width",
                "Petal by sepal",
                "#f28f43",
            ))
            .at(1, 0)
            .name("sepal_petal"),
        )
        .mark(
            Subplot::new(grid_shared_axes_child(
                "petal_length",
                "petal_width",
                "Petal width",
                "#910000",
            ))
            .at(1, 1)
            .name("petal_width"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile complete GridConcat chart");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "grid_concat_shared_axes_complete",
    )
    .await;
}

#[tokio::test]
async fn grid_concat_holey_shared_axes() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = Chart::<GridConcat>::new()
        .canvas_size(900.0, 560.0)
        .data(df)
        .title("Holey grid concat")
        .configure_coord(|c| c.rows(2).columns(3))
        .mark(
            Subplot::new(grid_shared_axes_child(
                "sepal_length",
                "sepal_width",
                "Top left",
                "#2f7ed8",
            ))
            .at(0, 0)
            .name("top_left"),
        )
        .mark(
            Subplot::new(grid_shared_axes_child(
                "petal_length",
                "sepal_width",
                "Top right",
                "#8bbc21",
            ))
            .at(0, 2)
            .name("top_right"),
        )
        .mark(
            Subplot::new(grid_shared_axes_child(
                "sepal_length",
                "petal_width",
                "Bottom middle",
                "#f28f43",
            ))
            .at(1, 1)
            .name("bottom_middle"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile holey GridConcat chart");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "grid_concat_holey_shared_axes",
    )
    .await;
}

#[tokio::test]
async fn grid_concat_splom_named_domains() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let variables = ["sepal_length", "sepal_width", "petal_length"];
    let mut plot = Chart::<GridConcat>::new()
        .canvas_size(820.0, 760.0)
        .data(df)
        .title("Manual GridConcat SPLOM")
        .configure_coord(|c| c.rows(3).columns(3));

    for (row, y) in variables.iter().enumerate() {
        for (column, x) in variables.iter().enumerate() {
            plot = plot.mark(
                Subplot::new(grid_splom_child(x, y))
                    .at(row, column)
                    .name(format!("{y}__{x}")),
            );
        }
    }

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile manual GridConcat SPLOM chart");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "grid_concat_splom_named_domains",
    )
    .await;
}

#[tokio::test]
async fn grid_concat_span_basic() {
    let ctx = SessionContext::new();
    let plot = Chart::<GridConcat>::new()
        .canvas_size(900.0, 680.0)
        .data(concat_numeric_data(&ctx).await)
        .title("Grid concat span basic")
        .configure_coord(|c| {
            c.rows(3)
                .columns(3)
                .axis_guide_visibility(AxisGuideVisibilityPolicy::OuterEdges)
        })
        .mark(
            Subplot::new(grid_span_child("x", "y", "#2f7ed8"))
                .caption("Spans 2 x 2")
                .at(0, 0)
                .span(2, 2)
                .name("span"),
        )
        .mark(
            Subplot::new(grid_span_child("x2", "y", "#8bbc21"))
                .caption("Top right")
                .at(0, 2)
                .name("top_right"),
        )
        .mark(
            Subplot::new(grid_span_child("x", "y2", "#f28f43"))
                .caption("Bottom left")
                .at(2, 0)
                .name("bottom_left"),
        )
        .mark(
            Subplot::new(grid_span_child("x2", "y2", "#910000"))
                .caption("Bottom right")
                .at(2, 2)
                .name("bottom_right"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile basic GridConcat span chart");
    assert_visual_match_default(&compiled, &ctx, None, "concat", "grid_concat_span_basic").await;
}

#[tokio::test]
async fn grid_concat_span_chrome() {
    let ctx = SessionContext::new();
    let plot = Chart::<GridConcat>::new()
        .canvas_size(940.0, 680.0)
        .data(concat_numeric_data(&ctx).await)
        .title("Grid concat span chrome")
        .configure_coord(|c| {
            c.rows(2)
                .columns(3)
                .axis_guide_visibility(AxisGuideVisibilityPolicy::OuterEdges)
        })
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::<Cartesian>::new()
                        .x_with(col("x"), |c| {
                            c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                                .axis(|a| a.title("Long x axis title on spanning child"))
                        })
                        .y_with(col("y2"), |c| {
                            c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                                .axis(|a| a.title("Long y axis title on spanning child"))
                        })
                        .fill("#2f7ed8")
                        .opacity(0.78)
                        .stroke("#ffffff")
                        .stroke_width(0.75)
                        .size(78.0),
                ),
            )
            .caption("Spanning plot with longer chrome")
            .at(0, 0)
            .span(2, 2)
            .name("span_chrome"),
        )
        .mark(
            Subplot::new(grid_span_child("x2", "y", "#8bbc21"))
                .caption("Top neighbor")
                .at(0, 2)
                .name("top_neighbor"),
        )
        .mark(
            Subplot::new(grid_span_child("x2", "y2", "#f28f43"))
                .at(1, 2)
                .name("bottom_neighbor"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile chrome GridConcat span chart");
    assert_visual_match_default(&compiled, &ctx, None, "concat", "grid_concat_span_chrome").await;
}

#[tokio::test]
async fn grid_concat_span_holes() {
    let ctx = SessionContext::new();
    let plot = Chart::<GridConcat>::new()
        .canvas_size(900.0, 680.0)
        .data(concat_numeric_data(&ctx).await)
        .title("Grid concat spans with holes")
        .configure_coord(|c| {
            c.rows(3)
                .columns(3)
                .axis_guide_visibility(AxisGuideVisibilityPolicy::OuterEdges)
        })
        .mark(
            Subplot::new(grid_span_child("x", "y", "#2f7ed8"))
                .caption("Top-left edge")
                .at(0, 0)
                .name("top_left"),
        )
        .mark(
            Subplot::new(grid_span_child("x2", "y2", "#8bbc21"))
                .caption("Spans holes")
                .at(0, 1)
                .span(2, 2)
                .name("span_holes"),
        )
        .mark(
            Subplot::new(grid_span_child("x", "y2", "#f28f43"))
                .caption("Bottom middle")
                .at(2, 1)
                .name("bottom_middle"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile holey GridConcat span chart");
    assert_visual_match_default(&compiled, &ctx, None, "concat", "grid_concat_span_holes").await;
}

#[tokio::test]
async fn grid_concat_span_domain_groups() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = Chart::<GridConcat>::new()
        .canvas_size(940.0, 680.0)
        .data(df)
        .title("Grid concat span domain groups")
        .configure_coord(|c| {
            c.rows(3)
                .columns(3)
                .axis_guide_visibility(AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups)
        })
        .mark(
            Subplot::new(grid_splom_child("sepal_length", "sepal_width"))
                .at(0, 0)
                .span(2, 2)
                .name("span_sepal"),
        )
        .mark(
            Subplot::new(grid_splom_child("petal_length", "sepal_width"))
                .at(0, 2)
                .name("top_right"),
        )
        .mark(
            Subplot::new(grid_splom_child("sepal_length", "petal_width"))
                .at(2, 0)
                .name("bottom_left"),
        )
        .mark(
            Subplot::new(grid_splom_child("petal_length", "petal_width"))
                .at(2, 2)
                .name("bottom_right"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile named-domain GridConcat span chart");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "grid_concat_span_domain_groups",
    )
    .await;
}

#[tokio::test]
async fn grid_concat_span_inside_facet_aligned() {
    let ctx = SessionContext::new();
    let plot = Chart::<FacetColumn>::new()
        .data(concat_facet_alignment_data(&ctx).await)
        .canvas_size(1560.0, 580.0)
        .mark(Subplot::new(alignment_spanned_grid_concat()).column(col("group_name")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile spanned GridConcat inside FacetColumn");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "grid_concat_span_inside_facet_aligned",
    )
    .await;
}

#[tokio::test]
async fn grid_concat_inside_facet_column_aligned() {
    let ctx = SessionContext::new();
    let plot = Chart::<FacetColumn>::new()
        .data(concat_facet_alignment_data(&ctx).await)
        .canvas_size(1320.0, 420.0)
        .mark(Subplot::new(alignment_grid_concat()).column(col("group_name")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile GridConcat inside FacetColumn");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "grid_concat_inside_facet_column_aligned",
    )
    .await;
}

#[tokio::test]
async fn grid_concat_inside_facet_wrap_aligned() {
    let ctx = SessionContext::new();
    let plot = Chart::<FacetWrap>::new()
        .data(concat_facet_alignment_data(&ctx).await)
        .canvas_size(1320.0, 760.0)
        .mark(
            Subplot::new(alignment_grid_concat())
                .wrap_with(col("group_name"), |c| c.columns(2).empty_cells_as_holes()),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile GridConcat inside FacetWrap");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "grid_concat_inside_facet_wrap_aligned",
    )
    .await;
}

#[tokio::test]
async fn wrap_concat_inside_facet_column_aligned() {
    let ctx = SessionContext::new();
    let plot = Chart::<FacetColumn>::new()
        .data(concat_facet_alignment_data(&ctx).await)
        .canvas_size(1320.0, 380.0)
        .mark(Subplot::new(alignment_wrap_concat()).column(col("group_name")));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile WrapConcat inside FacetColumn");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "wrap_concat_inside_facet_column_aligned",
    )
    .await;
}

#[tokio::test]
async fn facet_column_inside_grid_concat_smoke() {
    let ctx = SessionContext::new();
    let plot = Chart::<GridConcat>::new()
        .data(concat_facet_alignment_data(&ctx).await)
        .canvas_size(1320.0, 520.0)
        .configure_coord(|c| {
            c.rows(2)
                .columns(2)
                .axis_guide_visibility(AxisGuideVisibilityPolicy::OuterEdges)
        })
        .mark(
            Subplot::new(facet_column_alignment_cell("x", "y", "#2f7ed8"))
                .at(0, 0)
                .name("xy"),
        )
        .mark(
            Subplot::new(facet_column_alignment_cell("x2", "y", "#8bbc21"))
                .at(0, 1)
                .name("x2y"),
        )
        .mark(
            Subplot::new(facet_column_alignment_cell("x", "y2", "#f28f43"))
                .at(1, 0)
                .name("xy2"),
        )
        .mark(
            Subplot::new(facet_column_alignment_cell("x2", "y2", "#910000"))
                .at(1, 1)
                .name("x2y2"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile FacetColumn inside GridConcat");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "facet_column_inside_grid_concat_smoke",
    )
    .await;
}

#[tokio::test]
async fn facet_column_inside_holey_grid_concat_aligned() {
    let ctx = SessionContext::new();
    let plot = Chart::<GridConcat>::new()
        .data(concat_facet_alignment_data(&ctx).await)
        .canvas_size(1320.0, 820.0)
        .configure_coord(|c| {
            c.rows(2)
                .columns(3)
                .axis_guide_visibility(AxisGuideVisibilityPolicy::OuterEdges)
        })
        .mark(
            Subplot::new(facet_column_alignment_cell("x", "y", "#2f7ed8"))
                .at(0, 0)
                .name("left_facets"),
        )
        .mark(
            Subplot::new(facet_column_alignment_cell("x2", "y2", "#8bbc21"))
                .at(0, 2)
                .name("right_facets"),
        )
        .mark(
            Subplot::new(alignment_grid_concat())
                .at(1, 1)
                .name("unrelated_grid"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile holey FacetColumn inside GridConcat");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "facet_column_inside_holey_grid_concat_aligned",
    )
    .await;
}

#[tokio::test]
async fn wrap_concat_fixed_columns_shared_axes() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = Chart::<WrapConcat>::new()
        .plot_size(720.0, 420.0)
        .data(df)
        .title("Wrapped concat fixed columns")
        .configure_coord(|c| c.columns(3))
        .mark(
            Subplot::new(grid_shared_axes_child(
                "sepal_length",
                "sepal_width",
                "A",
                "#2f7ed8",
            ))
            .name("a"),
        )
        .mark(
            Subplot::new(grid_shared_axes_child(
                "petal_length",
                "sepal_width",
                "B",
                "#8bbc21",
            ))
            .name("b"),
        )
        .mark(
            Subplot::new(grid_shared_axes_child(
                "sepal_length",
                "petal_width",
                "C",
                "#f28f43",
            ))
            .name("c"),
        )
        .mark(
            Subplot::new(grid_shared_axes_child(
                "petal_length",
                "petal_width",
                "D",
                "#910000",
            ))
            .name("d"),
        )
        .mark(
            Subplot::new(grid_shared_axes_child(
                "sepal_width",
                "petal_width",
                "E",
                "#492970",
            ))
            .name("e"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile fixed WrapConcat chart");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "wrap_concat_fixed_columns_shared_axes",
    )
    .await;
}

#[tokio::test]
async fn wrap_concat_responsive_columns_narrow() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = responsive_wrap_concat_plot(df, 430.0, 520.0);
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile narrow responsive WrapConcat chart");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "wrap_concat_responsive_columns_narrow",
    )
    .await;
}

#[tokio::test]
async fn wrap_concat_responsive_columns_wide() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = responsive_wrap_concat_plot(df, 760.0, 420.0);
    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile wide responsive WrapConcat chart");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "wrap_concat_responsive_columns_wide",
    )
    .await;
}

fn responsive_wrap_concat_plot(df: DataFrame, width: f64, height: f64) -> Chart<WrapConcat> {
    Chart::<WrapConcat>::new()
        .plot_size(width, height)
        .data(df)
        .title("Wrapped concat responsive columns")
        .configure_coord(|c| c.responsive_columns(220.0))
        .mark(
            Subplot::new(grid_shared_axes_child(
                "sepal_length",
                "sepal_width",
                "Sepal",
                "#2f7ed8",
            ))
            .name("sepal"),
        )
        .mark(
            Subplot::new(grid_shared_axes_child(
                "petal_length",
                "sepal_width",
                "Petal x",
                "#8bbc21",
            ))
            .name("petal_x"),
        )
        .mark(
            Subplot::new(grid_shared_axes_child(
                "sepal_length",
                "petal_width",
                "Petal y",
                "#f28f43",
            ))
            .name("petal_y"),
        )
        .mark(
            Subplot::new(grid_shared_axes_child(
                "petal_length",
                "petal_width",
                "Petal",
                "#910000",
            ))
            .name("petal"),
        )
        .mark(
            Subplot::new(grid_shared_axes_child(
                "sepal_width",
                "petal_width",
                "Widths",
                "#492970",
            ))
            .name("widths"),
        )
}

#[tokio::test]
async fn hconcat_two_cartesian() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = Chart::<HConcat>::new()
        .canvas_size(820.0, 360.0)
        .data(df)
        .title("Horizontal concat")
        .mark(
            Subplot::new(sepal_child())
                .caption("Sepal")
                .name("sepal")
                .label("Sepal"),
        )
        .mark(
            Subplot::new(petal_child())
                .caption("Petal")
                .name("petal")
                .label("Petal"),
        );

    let compiled = plot.compile(&ctx).await.expect("compile hconcat chart");
    assert_visual_match_default(&compiled, &ctx, None, "concat", "hconcat_two_cartesian").await;
}

#[tokio::test]
async fn vconcat_two_cartesian() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = Chart::<VConcat>::new()
        .canvas_size(560.0, 720.0)
        .data(df)
        .title("Vertical concat")
        .mark(
            Subplot::new(sepal_child())
                .caption("Sepal")
                .name("sepal")
                .label("Sepal"),
        )
        .mark(
            Subplot::new(petal_child())
                .caption("Petal")
                .name("petal")
                .label("Petal"),
        );

    let compiled = plot.compile(&ctx).await.expect("compile vconcat chart");
    assert_visual_match_default(&compiled, &ctx, None, "concat", "vconcat_two_cartesian").await;
}

#[tokio::test]
async fn hconcat_plot_size_two_cartesian() {
    let ctx = SessionContext::new();
    let df = iris_with_petal_width_bin(&ctx).await;
    let plot = Chart::<HConcat>::new()
        .plot_size(620.0, 240.0)
        .data(df)
        .title("Plot-size concat")
        .mark(
            Subplot::new(sepal_child())
                .caption("Sepal")
                .name("sepal")
                .label("Sepal"),
        )
        .mark(
            Subplot::new(petal_child())
                .caption("Petal")
                .name("petal")
                .label("Petal"),
        );

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
    let plot = Chart::<HConcat>::new()
        .canvas_size(820.0, 360.0)
        .data(df)
        .title("Bare subplot concat")
        .mark(Subplot::new(sepal_child()).caption("Sepal"))
        .mark(Subplot::new(petal_child()).caption("Petal"));

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
    let plot = Chart::<HConcat>::new()
        .canvas_size(820.0, 380.0)
        .data(df)
        .title("Mixed coordinate concat")
        .mark(
            Subplot::new(numeric_cartesian_child())
                .caption("Cartesian")
                .name("cartesian")
                .label("Cartesian"),
        )
        .mark(
            Subplot::new(numeric_polar_child())
                .caption("Polar")
                .name("polar")
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
        .mark(Subplot::new(sepal_child()).caption("Sepal").name("sepal"))
        .mark(Subplot::new(petal_child()).caption("Petal").name("petal"));
    let plot = Chart::<VConcat>::new()
        .canvas_size(820.0, 640.0)
        .data(df)
        .title("Nested concat")
        .mark(Subplot::new(top_row).name("top-row").label("Top Row"))
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::<Cartesian>::new()
                        .x(col("sepal_length"))
                        .y(col("petal_length"))
                        .fill_with(col("species"), |c| c.legend(|l| l.title("Species")))
                        .size(80.0),
                ),
            )
            .caption("Combined")
            .name("combined")
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
        .mark(
            Subplot::new(sepal_child())
                .caption("Sepal")
                .name("sepal")
                .label("Sepal"),
        )
        .mark(
            Subplot::new(petal_child())
                .caption("Petal")
                .name("petal")
                .label("Petal"),
        );
    let plot = Chart::<FacetRow>::new()
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
    let plot = Chart::<HConcat>::new()
        .canvas_size(820.0, 380.0)
        .data(df)
        .title("Debug concat")
        .mark(
            Subplot::new(numeric_cartesian_child())
                .caption("Cartesian")
                .name("cartesian")
                .label("Cartesian"),
        )
        .mark(
            Subplot::new(numeric_cartesian_child_alt())
                .caption("Alternate Axes")
                .name("alternate")
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

#[tokio::test]
async fn hconcat_widths_flex_split() {
    // Three children: a fixed 140px plot-area column and a 2:1 flex split
    // of the remainder.
    let ctx = SessionContext::new();
    let plot = Chart::<HConcat>::new()
        .data(concat_numeric_data(&ctx).await)
        .canvas_size(900.0, 300.0)
        .configure_coord(|c| {
            c.widths([
                TrackSizing::Px(140.0),
                TrackSizing::Flex(2.0),
                TrackSizing::Flex(1.0),
            ])
        })
        .title("hconcat widths: Px(140) | Flex(2) | Flex(1)")
        .mark(
            Subplot::new(numeric_cartesian_child())
                .caption("Cartesian")
                .name("fixed"),
        )
        .mark(
            Subplot::new(numeric_cartesian_child_alt())
                .caption("Alternate Axes")
                .name("wide"),
        )
        .mark(
            Subplot::new(numeric_cartesian_child())
                .caption("Cartesian")
                .name("narrow"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile hconcat widths chart");
    assert_visual_match_default(&compiled, &ctx, None, "concat", "hconcat_widths_flex_split").await;
}

#[tokio::test]
async fn vconcat_heights_px_rows() {
    // Pixel-pinned top row, flexible bottom row.
    let ctx = SessionContext::new();
    let plot = Chart::<VConcat>::new()
        .data(concat_numeric_data(&ctx).await)
        .canvas_size(420.0, 560.0)
        .configure_coord(|c| c.heights([TrackSizing::Px(120.0), TrackSizing::Flex(1.0)]))
        .title("vconcat heights: Px(120) | Flex(1)")
        .mark(
            Subplot::new(numeric_cartesian_child())
                .caption("Cartesian")
                .name("pinned"),
        )
        .mark(
            Subplot::new(numeric_cartesian_child_alt())
                .caption("Alternate Axes")
                .name("flex"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile vconcat heights chart");
    assert_visual_match_default(&compiled, &ctx, None, "concat", "vconcat_heights_px_rows").await;
}

#[tokio::test]
async fn grid_concat_fixed_sidebar_column() {
    // A rigid 150px sidebar column beside flexible content columns; row
    // heights split 1:2.
    let ctx = SessionContext::new();
    let plot = Chart::<GridConcat>::new()
        .data(concat_facet_alignment_data(&ctx).await)
        .canvas_size(900.0, 520.0)
        .configure_coord(|c| {
            c.rows(2)
                .columns(2)
                .column_widths([TrackSizing::Px(150.0), TrackSizing::Flex(1.0)])
                .row_heights([TrackSizing::Flex(1.0), TrackSizing::Flex(2.0)])
                .axis_guide_visibility(AxisGuideVisibilityPolicy::OuterEdges)
        })
        .title("grid: Px(150) sidebar, rows Flex 1:2")
        .mark(
            Subplot::new(alignment_grid_cell("x", "y", "#2f7ed8"))
                .at(0, 0)
                .name("side_top"),
        )
        .mark(
            Subplot::new(alignment_grid_cell("x2", "y", "#8bbc21"))
                .at(0, 1)
                .name("main_top"),
        )
        .mark(
            Subplot::new(alignment_grid_cell("x", "y2", "#f28f43"))
                .at(1, 0)
                .name("side_bottom"),
        )
        .mark(
            Subplot::new(alignment_grid_cell("x2", "y2", "#910000"))
                .at(1, 1)
                .name("main_bottom"),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile grid sidebar chart");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "concat",
        "grid_concat_fixed_sidebar_column",
    )
    .await;
}
