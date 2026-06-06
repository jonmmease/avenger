use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

async fn load_iris(ctx: &SessionContext) -> DataFrame {
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    ctx.read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset")
}

#[tokio::test]
async fn facet_plot_size_row_iris_scatter() {
    let ctx = SessionContext::new();
    let df = load_iris(&ctx).await;

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .plot_size(140.0, 100.0)
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
        "facet_plot_size",
        "facet_plot_size_row_iris_scatter",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_col_iris_scatter() {
    let ctx = SessionContext::new();
    let df = load_iris(&ctx).await;

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .plot_size(140.0, 100.0)
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
        "facet_plot_size",
        "facet_plot_size_col_iris_scatter",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_row_shared_scales() {
    let ctx = SessionContext::new();
    let df = load_iris(&ctx).await;

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .plot_size(140.0, 100.0)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.with_scale_sharing(CoordinationScope::Shared)
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.with_scale_sharing(CoordinationScope::Shared)
                        })
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
        "facet_plot_size",
        "facet_plot_size_row_shared_scales",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_col_free_scales() {
    let ctx = SessionContext::new();
    let df = load_iris(&ctx).await;

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .plot_size(140.0, 100.0)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.with_scale_sharing(CoordinationScope::Free)
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.with_scale_sharing(CoordinationScope::Free)
                        })
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
        "facet_plot_size",
        "facet_plot_size_col_free_scales",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_row_hybrid_shared_x_free_y() {
    let ctx = SessionContext::new();
    let df = load_iris(&ctx).await;

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .plot_size(140.0, 100.0)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.with_scale_sharing(CoordinationScope::Shared)
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.with_scale_sharing(CoordinationScope::Free)
                        })
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
        "facet_plot_size",
        "facet_plot_size_row_hybrid_shared_x_free_y",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_col_with_title() {
    let ctx = SessionContext::new();
    let df = load_iris(&ctx).await;

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .plot_size(140.0, 100.0)
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
            .col_with(col("species"), |c| c.guide(|g| g.title("Iris Species"))),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_plot_size",
        "facet_plot_size_col_with_title",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_col_line_mark() {
    let ctx = SessionContext::new();
    let df = load_iris(&ctx).await;

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .plot_size(140.0, 100.0)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Line::new()
                        .x(col("sepal_length"))
                        .y(col("sepal_width"))
                        .stroke("#4682b4")
                        .stroke_width(2.0),
                ),
            )
            .column(col("species")),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_plot_size",
        "facet_plot_size_col_line_mark",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_col_x_axis_top() {
    let ctx = SessionContext::new();
    let df = load_iris(&ctx).await;

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .plot_size(140.0, 100.0)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| c.axis(|a| a.position("top")))
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
        "facet_plot_size",
        "facet_plot_size_col_x_axis_top",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_row_y_axis_right() {
    let ctx = SessionContext::new();
    let df = load_iris(&ctx).await;

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .plot_size(140.0, 100.0)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("sepal_length"))
                        .y_with(col("sepal_width"), |c| c.axis(|a| a.position("right")))
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
        "facet_plot_size",
        "facet_plot_size_row_y_axis_right",
    )
    .await;
}

#[tokio::test]
async fn facet_plot_size_row_polar() {
    let ctx = SessionContext::new();
    let df = load_iris(&ctx).await;

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .plot_size(140.0, 100.0)
        .mark(
            Subplot::new(
                Plot::<Polar>::new().mark(
                    Symbol::new()
                        .r_with(col("sepal_length"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .with_scale_sharing(CoordinationScope::Shared)
                        })
                        .theta_with(col("sepal_width"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .with_scale_sharing(CoordinationScope::Shared)
                        })
                        .size(30.0)
                        .fill("#cd5c5c"),
                ),
            )
            .row(col("species")),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet_plot_size",
        "facet_plot_size_row_polar",
    )
    .await;
}
