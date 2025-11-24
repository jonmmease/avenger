use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

/// Test nested faceting: FacetRow inside FacetColumn
///
/// CURRENT STATUS: This test is disabled because nested faceting causes stack overflow.
/// The facet system was designed with the assumption that facets only contain Cartesian
/// plots, not other facets. Supporting nested facets would require significant
/// architectural changes to how data filtering and facet evaluation works.
///
/// The key issues are:
/// 1. Both facet levels try to operate on the full dataset independently
/// 2. The inner facet doesn't receive the filtered data from the outer facet
/// 3. This creates infinite recursion during compilation
///
/// For now, use GridFacet for multi-dimensional faceting instead of nesting.
#[tokio::test]
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
                    // Middle layer: FacetRow (needs same data reference)
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
