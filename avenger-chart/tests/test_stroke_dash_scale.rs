use avenger_chart::cartesian::Cartesian;
use avenger_chart::plot::Plot;
use avenger_chart::prelude::Line;
use datafusion::arrow::array::{Float32Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_stroke_dash_scale_inference() {
    // Create simple test data
    let x_values = Float32Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let y_values = Float32Array::from(vec![10.0, 20.0, 15.0, 25.0, 18.0, 22.0]);
    let line_type = StringArray::from(vec!["A", "A", "A", "B", "B", "B"]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
        Field::new("line_type", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(x_values), Arc::new(y_values), Arc::new(line_type)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Create a line chart with stroke_dash channel
    let plot = Plot::<Cartesian>::new().data(df).mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke_dash(col("line_type")),
    );

    // Try to compile the plot
    let compiled = plot.compile(&ctx).await;

    // Check if we get the scale inference error
    match compiled {
        Ok(_) => println!("Plot compiled successfully!"),
        Err(e) => {
            println!("Error compiling plot: {:?}", e);
            panic!("Failed to compile plot with stroke_dash: {:?}", e);
        }
    }
}
