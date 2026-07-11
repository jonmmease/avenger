use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

#[tokio::test]
async fn facet_row_iris_scatter() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Build outer facet row plot
    let outer = Chart::<FacetRow>::new().data(df).mark(
        Subplot::new(
            Plot::<Cartesian>::new().mark(
                Symbol::new()
                    .x(col("sepal_length"))
                    .y(col("sepal_width"))
                    .size(36.0)
                    .fill("#4682b4"),
            ),
        )
        .row(col("species")),
    );

    let compiled = outer.compile(&ctx).await.expect("compile outer");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_row_iris_scatter").await;
}
