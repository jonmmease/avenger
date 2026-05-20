use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

// Demonstrates inner x-axis positioned on the top; verifies layout under facet row
#[tokio::test]
async fn facet_row_iris_x_axis_top() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    let outer = Plot::<FacetRow>::new()
        .data(df)
        .mark(
            Subplot::new(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x_with(col("sepal_length"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Length").position("top"))
                        })
                        .y_with(col("sepal_width"), |c| {
                            c.scale_with::<Linear>(|s| s)
                                .axis(|a| a.title("Sepal Width"))
                        })
                        .size(28.0)
                        .fill("#1f78b4"),
                ),
            )
            .row_with(col("species"), |c| c.facet(|f| f.title("Species"))),
        )
        .canvas_size(600.0, 500.0);

    let compiled = outer.compile(&ctx).await.expect("compile outer");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "facet_row_iris_x_axis_top").await;
}
