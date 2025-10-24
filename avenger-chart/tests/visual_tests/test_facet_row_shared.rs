use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

#[tokio::test]
async fn facet_row_iris_shared_scatter() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Outer facet row with shared scales (per-channel)
    let outer = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Facet::new().row(col("species")).subplot(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .share_scale(ScaleSharing::Shared)
                                .axis(|a| a.title("Sepal Length"))
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .share_scale(ScaleSharing::Shared)
                                .axis(|a| a.title("Sepal Width"))
                        })
                        .size(36.0)
                        .fill("#4682b4"),
                ),
            ),
        )
        .canvas_size(600.0, 500.0);

    let compiled = outer.compile(&ctx).await.expect("compile outer");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet",
        "facet_row_iris_shared_scatter",
    )
    .await;
}
