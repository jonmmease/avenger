use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

#[tokio::test]
async fn facet_row_iris_free_scatter() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Outer facet row with free scales (per-channel)
    let outer = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .with_scale_sharing(ScaleSharing::Free)
                                .axis(|a| a.title("Sepal Length"))
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .with_scale_sharing(ScaleSharing::Free)
                                .axis(|a| a.title("Sepal Width"))
                        })
                        .size(36.0)
                        .fill("#8a2be2"),
                ),
            )
            .row(col("species")),
        )
        .canvas_size(600.0, 500.0);

    let compiled = outer.compile(&ctx).await.expect("compile outer");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "facet",
        "facet_row_iris_free_scatter",
    )
    .await;
}
