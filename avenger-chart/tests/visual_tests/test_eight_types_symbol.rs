use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::cartesian::Cartesian;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::{ArrayRef, Float32Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

#[tokio::test]
async fn test_eight_types_fill_shape() {
    // Create test data with 8 categories - same structure as line test
    // Each type has 5 points in a time series
    let x_values = Float32Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, // Type A
        1.0, 2.0, 3.0, 4.0, 5.0, // Type B
        1.0, 2.0, 3.0, 4.0, 5.0, // Type C
        1.0, 2.0, 3.0, 4.0, 5.0, // Type D
        1.0, 2.0, 3.0, 4.0, 5.0, // Type E
        1.0, 2.0, 3.0, 4.0, 5.0, // Type F
        1.0, 2.0, 3.0, 4.0, 5.0, // Type G
        1.0, 2.0, 3.0, 4.0, 5.0, // Type H
    ]);

    let y_values = Float32Array::from(vec![
        22.0, 23.0, 21.0, 24.0, 25.0, // Type A
        19.0, 20.0, 18.0, 21.0, 22.0, // Type B
        16.0, 17.0, 15.0, 18.0, 19.0, // Type C
        13.0, 14.0, 12.0, 15.0, 16.0, // Type D
        10.0, 11.0, 9.0, 12.0, 13.0, // Type E
        7.0, 8.0, 6.0, 9.0, 10.0, // Type F
        4.0, 5.0, 3.0, 6.0, 7.0, // Type G
        1.0, 2.0, 0.0, 3.0, 4.0, // Type H
    ]);

    let category = StringArray::from(vec![
        "Type A", "Type A", "Type A", "Type A", "Type A", "Type B", "Type B", "Type B", "Type B",
        "Type B", "Type C", "Type C", "Type C", "Type C", "Type C", "Type D", "Type D", "Type D",
        "Type D", "Type D", "Type E", "Type E", "Type E", "Type E", "Type E", "Type F", "Type F",
        "Type F", "Type F", "Type F", "Type G", "Type G", "Type G", "Type G", "Type G", "Type H",
        "Type H", "Type H", "Type H", "Type H",
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float32, false),
        Field::new("y", DataType::Float32, false),
        Field::new("category", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values) as ArrayRef,
            Arc::new(y_values) as ArrayRef,
            Arc::new(category) as ArrayRef,
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    // Create scatter plot with both fill and shape encoded by the same column
    // This will use default Okabe-Ito colors and default shapes
    let plot = Plot::new(Cartesian)
        .data(df)
        .title("Eight Category Scatter Plot")
        .subtitle("Okabe-Ito colors with distinct shapes")
        .axis_x(|axis| axis.title("Sample Index").grid(true))
        .axis_y(|axis| axis.title("Performance Metric (%)").grid(true))
        .legend_fill(|legend| legend.title("Category")) // Only one legend since both use same column
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("category")) // Uses default Okabe-Ito colors
                .shape(col("category")) // Uses default 8 shapes
                .size(100.0), // Fixed size as number, not literal
        );

    assert_visual_match_default(plot, "symbol", "eight_types_fill_shape").await;
}
