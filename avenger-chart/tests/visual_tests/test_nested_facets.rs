use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

/// Test that nested facets with invalid data attachment are rejected
///
/// This test verifies that the system correctly prevents inner facet plots from
/// having their own data attached. The validation error should occur during
/// compilation with a clear message explaining that data must flow from the
/// parent facet through the data_override mechanism.
///
/// Expected behavior: Test fails with InvalidArgument error during compilation.
///
/// For multi-dimensional faceting, use GridFacet instead of nested facets.
#[tokio::test]
#[should_panic(expected = "Nested facet plots should not have their own data attached")]
async fn test_col_with_nested_row() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let iris = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Add binned petal_width column
    let df = iris
        .with_column(
            "petal_width_bin",
            when(col("petal_width").lt_eq(lit(0.8)), lit("narrow"))
                .when(col("petal_width").lt_eq(lit(1.7)), lit("medium"))
                .otherwise(lit("wide"))
                .unwrap(),
        )
        .unwrap();

    // Outer facet: Column by species (3 columns)
    // Inner facet: Row by petal_width_bin (variable rows per column)
    // Innermost: Scatter plot of sepal dimensions
    let outer = Plot::<FacetColumn>::new()
        .data(df.clone())
        .canvas_size(800, 600)
        .mark(
            Facet::new()
                .col_with(col("species"), |c| c.facet(|f| f.title("Species")))
                .subplot(
                    // Middle layer: FacetRow (INVALID: should not have own data)
                    Plot::<FacetRow>::new().data(df).mark(
                        Facet::new()
                            .row_with(col("petal_width_bin"), |c| {
                                c.facet(|f| f.title("Petal Width"))
                            })
                            .subplot(
                                // Innermost: Cartesian plot (receives doubly-filtered data)
                                Plot::<Cartesian>::new().mark(
                                    Symbol::new()
                                        .x(col("sepal_length"))
                                        .y(col("sepal_width"))
                                        .size(25.0)
                                        .fill("#4682b4"),
                                ),
                            ),
                    ),
                ),
        );

    let compiled = outer
        .compile(&ctx)
        .await
        .expect("compile nested facets");
    assert_visual_match_default(&compiled, &ctx, None, "facet", "nested_col_row").await;
}
