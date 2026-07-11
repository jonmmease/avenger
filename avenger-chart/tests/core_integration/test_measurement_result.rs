//! Tests for PlotMeasurementResult dimension consistency.
//!
//! Note: Most of the MeasurementResult validation tests are internal to the
//! avenger-chart crate (in src/plot/compiled/mod.rs) because they require
//! access to internal types like ScaleBuilder and ScaleProvider.
//!
//! This file contains integration tests that validate the public API behavior.

use avenger_chart::prelude::*;
use datafusion::prelude::*;

/// Test that evaluate() still works (regression test during refactoring).
#[tokio::test]
async fn test_evaluate_still_works_after_refactor() {
    let ctx = SessionContext::new();
    let iris_path = format!("{}/tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(iris_path, ParquetReadOptions::default())
        .await
        .expect("load iris dataset");

    // Create a simple scatter plot
    let plot = Chart::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(36.0)
            .fill("#4682b4"),
    );

    // Compile and evaluate
    let compiled = plot.compile(&ctx).await.expect("compile plot");
    let result = compiled.evaluate(&ctx, None).await;

    assert!(result.is_ok(), "evaluate() should succeed");
    let evaluated = result.unwrap();
    assert!(
        evaluated.scene_graph.width > 0.0,
        "Scene graph should have positive width"
    );
    assert!(
        evaluated.scene_graph.height > 0.0,
        "Scene graph should have positive height"
    );
}
