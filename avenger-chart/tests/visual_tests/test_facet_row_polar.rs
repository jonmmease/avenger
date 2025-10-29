use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

#[tokio::test]
async fn facet_row_iris_polar_scatter() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Outer facet row with shared scales to align polar axes across facets
    let outer = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Facet::new().row(col("species")).subplot(
                Plot::<Polar>::new().mark(
                    Symbol::new()
                        .r_with(col("sepal_length"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .with_scale_sharing(ScaleSharing::Shared)
                        })
                        .theta_with(col("sepal_width"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .with_scale_sharing(ScaleSharing::Shared)
                        })
                        .size(36.0)
                        .fill("#cd5c5c"),
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
        "facet_row_iris_polar_scatter",
    )
    .await;
}
