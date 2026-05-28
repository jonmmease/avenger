use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

async fn load_iris(ctx: &SessionContext) -> DataFrame {
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    ctx.read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset")
}

async fn mixed_nested_data(ctx: &SessionContext) -> DataFrame {
    ctx.sql(
        "CREATE TABLE mixed_nested AS VALUES
        ('DivA', 'Team1', 1.0, 1.0, 'Low'),
        ('DivA', 'Team1', 1.4, 1.3, 'High'),
        ('DivA', 'Team2', 2.0, 1.1, 'Low'),
        ('DivA', 'Team2', 2.4, 1.5, 'High'),
        ('DivB', 'Team1', 3.0, 1.2, 'Low'),
        ('DivB', 'Team1', 3.4, 1.6, 'High'),
        ('DivB', 'Team2', 4.0, 1.3, 'Low'),
        ('DivB', 'Team2', 4.4, 1.7, 'High')",
    )
    .await
    .expect("create mixed nested data");

    ctx.sql(
        "SELECT
            column1 AS division,
            column2 AS team,
            column3 AS x_val,
            column4 AS y_val,
            column5 AS category
         FROM mixed_nested",
    )
    .await
    .expect("read mixed nested data")
}

#[tokio::test]
async fn mixed_width_canvas_height_plot_row_iris_scatter() {
    let ctx = SessionContext::new();
    let df = load_iris(&ctx).await;

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .canvas_constraint(CanvasConstraint::width(520.0))
        .plot_constraint(PlotConstraint::height(82.0))
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("sepal_length"))
                        .y(col("sepal_width"))
                        .size(30.0)
                        .fill("#4682b4"),
                ),
            )
            .row(col("species")),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_dimension_mixed",
        "mixed_width_canvas_height_plot_row_iris_scatter",
    )
    .await;
}

#[tokio::test]
async fn mixed_width_canvas_height_plot_nested_col_row_shared() {
    let ctx = SessionContext::new();
    let df = mixed_nested_data(&ctx).await;

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .canvas_constraint(CanvasConstraint::width(760.0))
        .plot_constraint(PlotConstraint::height(74.0))
        .mark(
            Subplot::new(
                Plot::<FacetRow>::new().mark(
                    Subplot::new(
                        Plot::<Cartesian>::new().mark(
                            Symbol::new()
                                .x_with(col("x_val"), |c| c.with_scale_sharing(Sharing::Shared))
                                .y_with(col("y_val"), |c| c.with_scale_sharing(Sharing::Shared))
                                .fill_with(col("category"), |c| c.legend(|l| l.title("Category")))
                                .size(48.0),
                        ),
                    )
                    .row_with(col("team"), |c| c.guide(|g| g.title("team"))),
                ),
            )
            .col_with(col("division"), |c| c.guide(|g| g.title("division"))),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_dimension_mixed",
        "mixed_width_canvas_height_plot_nested_col_row_shared",
    )
    .await;
}

#[tokio::test]
async fn mixed_height_canvas_width_plot_col_iris_scatter() {
    let ctx = SessionContext::new();
    let df = load_iris(&ctx).await;

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .canvas_constraint(CanvasConstraint::height(360.0))
        .plot_constraint(PlotConstraint::width(96.0))
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("sepal_length"))
                        .y(col("sepal_width"))
                        .size(30.0)
                        .fill("#4682b4"),
                ),
            )
            .column(col("species")),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_dimension_mixed",
        "mixed_height_canvas_width_plot_col_iris_scatter",
    )
    .await;
}
