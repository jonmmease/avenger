use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

/// Test nested faceting: FacetRow inside FacetColumn
///
/// This test demonstrates nested faceting where data flows from the parent facet
/// to child facets through the data_override mechanism. The outer FacetColumn
/// facets by species, and within each species column, FacetRow facets by
/// petal_width_bin.
///
/// Inner facet plots must NOT have their own data attached - data flows from
/// the parent facet. This prevents infinite recursion and ensures correct data
/// filtering at each nesting level.
#[test]
fn test_col_with_nested_row() {
    // Build a runtime with a larger worker stack to avoid stack overflow in nested renders.
    // The current_thread runtime runs everything on its own thread, which we create
    // with a large stack using std::thread::Builder.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024) // 64 MB
        .spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build runtime");

            rt.block_on(async {
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
                        Subplot::new(
                            // Middle layer: FacetRow (data flows from parent via data_override)
                            Plot::<FacetRow>::new().mark(
                                Subplot::new(
                                    // Innermost: Cartesian plot (receives doubly-filtered data)
                                    Plot::<Cartesian>::new().mark(
                                        Symbol::new()
                                            .x(col("sepal_length"))
                                            .y(col("sepal_width"))
                                            .size(25.0)
                                            .fill("#4682b4"),
                                    ),
                                )
                                .row_with(col("petal_width_bin"), |c| {
                                    c.facet(|f| f.title("Petal Width"))
                                }),
                            ),
                        )
                        .col_with(col("species"), |c| c.facet(|f| f.title("Species"))),
                    );

                let compiled = outer.compile(&ctx).await.expect("compile nested facets");
                assert_visual_match_default(&compiled, &ctx, None, "facet", "nested_col_row").await;
            });
        })
        .expect("failed to spawn thread")
        .join()
        .expect("thread panicked");
}
