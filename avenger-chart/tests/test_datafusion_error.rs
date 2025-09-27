use avenger_chart::cartesian::Cartesian;
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, Int32Array};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_datafusion_context_mismatch() {
    // Create data with one SessionContext
    let ctx1 = SessionContext::new();

    let x_values = Int32Array::from(vec![0, 1, 2, 3, 4]);
    let y_values = Float64Array::from(vec![10.0, 25.0, 35.0, 30.0, 45.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Int32, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)])
        .expect("Failed to create RecordBatch");

    let df = ctx1.read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    // Create a plot with the DataFrame
    let plot = Plot::<Cartesian>::new()
        .data(df.clone())
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke("#4682b4")
                .stroke_width(2.0),
        );

    // Now try to compile with a DIFFERENT SessionContext
    let ctx2 = SessionContext::new();

    // Clone plot for first test
    let plot_clone = Plot::<Cartesian>::new()
        .data(df)
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke("#4682b4")
                .stroke_width(2.0),
        );

    // This should show us the actual DataFusion error
    match plot_clone.compile(&ctx2).await {
        Ok(_) => panic!("Compilation should have failed when using a different SessionContext"),
        Err(e) => {
            println!("Compilation error (as expected): {:?}", e);
            // Verify the error message mentions SessionContext
            let error_str = format!("{:?}", e);
            assert!(error_str.contains("SessionContext"),
                "Error message should mention SessionContext mismatch");
            assert!(error_str.contains("Failed to serialize DataFrame"),
                "Error message should mention DataFrame serialization failure");
        }
    }

    // Even with the same context, DataFrame serialization fails because
    // DataFusion's MemTable doesn't have a LogicalExtensionCodec registered
    match plot.compile(&ctx1).await {
        Ok(_) => panic!("Compilation should have failed due to missing LogicalExtensionCodec"),
        Err(e) => {
            println!("Compilation error with same context (expected): {:?}", e);
            let error_str = format!("{:?}", e);
            assert!(error_str.contains("LogicalExtensionCodec"),
                "Error should mention missing LogicalExtensionCodec");
        }
    }
}