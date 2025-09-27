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

    // With our MemTable serialization support, this should now work!
    match plot_clone.compile(&ctx2).await {
        Ok(_compiled) => {
            println!("Compilation succeeded with different context!");
        }
        Err(e) => {
            println!("Compilation error: {:?}", e);
            panic!("Compilation should succeed now that we have MemTable serialization support");
        }
    }

    // Compiling with the same context should also work
    match plot.compile(&ctx1).await {
        Ok(compiled) => {
            println!("Compilation succeeded with same context!");

            // Now test rendering with different context
            match compiled.render(&ctx2).await {
                Ok(_) => println!("Render succeeded with different context!"),
                Err(e) => println!("Render error with different context: {:?}", e),
            }
        }
        Err(e) => {
            panic!("Compilation should succeed: {:?}", e);
        }
    }
}