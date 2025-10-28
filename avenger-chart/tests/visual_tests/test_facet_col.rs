use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

#[tokio::test]
async fn facet_col_iris_scatter() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Build outer facet column plot
    let outer = Plot::<FacetCol>::new().data(df).mark(
        Facet::new().col(col("species")).subplot(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x(col("sepal_length"))
                    .y(col("sepal_width"))
                    .size(36.0)
                    .fill("#4682b4"),
            ),
        ),
    );

    let compiled = outer.compile(&ctx).await.expect("compile outer");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_col_iris_scatter").await;
}

#[tokio::test]
async fn facet_col_shared_y_scale() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Test shared y-scale across columns (y should have unified axis)
    let outer = Plot::<FacetCol>::new().data(df).mark(
        Facet::new().col(col("species")).subplot(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x(col("sepal_length"))
                    .y_with(col("sepal_width"), |c| {
                        c.scale_with::<Linear>(|s| s)
                            .share_scale(ScaleSharing::Shared)
                            .axis(|a| a.title("Sepal Width"))
                    })
                    .size(36.0)
                    .fill("#4682b4"),
            ),
        ),
    );

    let compiled = outer.compile(&ctx).await.expect("compile outer");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_col_shared_y_scale").await;
}

#[tokio::test]
async fn facet_col_free_scales() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Test free scales (each column gets independent x and y scales)
    let outer = Plot::<FacetCol>::new().data(df).mark(
        Facet::new().col(col("species")).subplot(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x_with(col("sepal_length"), |c| c.share_scale(ScaleSharing::Free))
                    .y_with(col("sepal_width"), |c| c.share_scale(ScaleSharing::Free))
                    .size(36.0)
                    .fill("#4682b4"),
            ),
        ),
    );

    let compiled = outer.compile(&ctx).await.expect("compile outer");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_col_free_scales").await;
}

#[tokio::test]
async fn facet_col_with_title() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Test facet title configuration
    let outer = Plot::<FacetCol>::new().data(df).mark(
        Facet::new()
            .col_with(col("species"), |c| c.facet(|f| f.title("Iris Species")))
            .subplot(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("sepal_length"))
                        .y(col("sepal_width"))
                        .size(36.0)
                        .fill("#4682b4"),
                ),
            ),
    );

    let compiled = outer.compile(&ctx).await.expect("compile outer");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_col_with_title").await;
}

#[tokio::test]
async fn facet_col_custom_spacing() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Test custom spacing between facets
    let outer = Plot::<FacetCol>::new().data(df).mark(
        Facet::new()
            .col_with(col("species"), |c| c.facet(|f| f.spacing(20.0)))
            .subplot(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(col("sepal_length"))
                        .y(col("sepal_width"))
                        .size(36.0)
                        .fill("#4682b4"),
                ),
            ),
    );

    let compiled = outer.compile(&ctx).await.expect("compile outer");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_col_custom_spacing").await;
}

#[tokio::test]
async fn facet_col_with_line_mark() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Test with line marks instead of symbols
    let outer = Plot::<FacetCol>::new().data(df).mark(
        Facet::new().col(col("species")).subplot(
            Plot::<Cartesian>::new().mark(
                Line::new()
                    .x(col("sepal_length"))
                    .y(col("sepal_width"))
                    .stroke("#4682b4")
                    .stroke_width(2.0),
            ),
        ),
    );

    let compiled = outer.compile(&ctx).await.expect("compile outer");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_col_with_line_mark").await;
}

#[tokio::test]
async fn facet_col_hybrid_sharing() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Test hybrid scale sharing: y shared, x free
    let outer = Plot::<FacetCol>::new().data(df).mark(
        Facet::new().col(col("species")).subplot(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x_with(col("sepal_length"), |c| c.share_scale(ScaleSharing::Free))
                    .y_with(col("sepal_width"), |c| c.share_scale(ScaleSharing::Shared))
                    .size(36.0)
                    .fill("#4682b4"),
            ),
        ),
    );

    let compiled = outer.compile(&ctx).await.expect("compile outer");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_col_hybrid_sharing").await;
}

#[tokio::test]
async fn facet_col_x_axis_top() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Test with x-axis on top - facet labels should be below plot
    let outer = Plot::<FacetCol>::new().data(df).mark(
        Facet::new().col(col("species")).subplot(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x_with(col("sepal_length"), |c| c.axis(|a| a.position("top")))
                    .y(col("sepal_width"))
                    .size(36.0)
                    .fill("#4682b4"),
            ),
        ),
    );

    let compiled = outer.compile(&ctx).await.expect("compile outer");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_col_x_axis_top").await;
}
