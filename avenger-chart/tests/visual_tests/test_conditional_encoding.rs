use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use avenger_chart::scales::Linear;
use datafusion::arrow::array::{BooleanArray, Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::SessionContext;
use palette::rgb::Srgba;
use std::sync::Arc;

#[tokio::test]
async fn test_conditional_color_when_value() {
    // Test when_value: highlight specific points with a fixed color
    let ctx = SessionContext::new();

    // Create sample data with a selection condition
    let x = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
    let y = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.5, 6.0, 5.5, 7.0]);
    let value = Float64Array::from(vec![10.0, 20.0, 15.0, 30.0, 25.0, 35.0, 32.0, 40.0]);
    let highlight = BooleanArray::from(vec![false, false, true, false, true, false, false, true]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("value", DataType::Float64, false),
        Field::new("highlight", DataType::Boolean, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x),
            Arc::new(y),
            Arc::new(value),
            Arc::new(highlight),
        ],
    )
    .unwrap();

    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .size(100.0)
            // Use conditional color: red for highlighted points, blue gradient for others
            .fill_with(col("value"), |c| {
                c.when_value(col("highlight"), lit("#ff0000")) // Red for highlighted
                    .scale_with::<Linear>(|s| {
                        s.range_colors(vec![
                            Srgba::new(0.0, 0.0, 0.5, 1.0), // Dark blue
                            Srgba::new(0.0, 0.5, 1.0, 1.0), // Light blue
                        ])
                    })
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "conditional",
        "conditional_color_when_value",
    )
    .await;
}

#[tokio::test]
async fn test_conditional_size_when_scaled() {
    // Test when_scaled: use different scaled values based on condition
    let ctx = SessionContext::new();

    // Create data with categories and importance scores
    let x = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
    let y = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.5, 6.0, 5.5, 7.0]);
    let base_size = Float64Array::from(vec![10.0, 15.0, 12.0, 20.0, 18.0, 25.0, 22.0, 30.0]);
    let importance = Float64Array::from(vec![1.0, 3.0, 2.0, 5.0, 4.0, 4.0, 3.0, 5.0]);
    let is_important = BooleanArray::from(vec![false, true, false, true, true, true, false, true]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("base_size", DataType::Float64, false),
        Field::new("importance", DataType::Float64, false),
        Field::new("is_important", DataType::Boolean, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x),
            Arc::new(y),
            Arc::new(base_size),
            Arc::new(importance),
            Arc::new(is_important),
        ],
    )
    .unwrap();

    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            // Use conditional size: scale by importance if important, otherwise use base size
            .size_with(col("base_size"), |c| {
                c.when_scaled(col("is_important"), col("importance") * lit(20.0))
                    .scale(|s| s.range_interval(lit(50.0), lit(200.0)))
            })
            .fill("#4682b4")
            .stroke("#000000")
            .stroke_width(1.0),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "conditional",
        "conditional_size_when_scaled",
    )
    .await;
}

#[tokio::test]
async fn test_conditional_multiple_conditions() {
    // Test multiple conditions with both when_value and when_scaled
    let ctx = SessionContext::new();

    // Create data with multiple categories
    let x = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    let y = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.5, 6.0, 5.5, 7.0, 6.5]);
    let category = StringArray::from(vec!["A", "B", "C", "A", "B", "C", "A", "B", "C"]);
    let value = Float64Array::from(vec![10.0, 20.0, 15.0, 25.0, 30.0, 18.0, 35.0, 40.0, 22.0]);
    let error_flag = BooleanArray::from(vec![
        false, false, true, false, false, false, false, true, false,
    ]);
    let warning_flag = BooleanArray::from(vec![
        false, true, false, false, true, false, false, false, true,
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
        Field::new("error", DataType::Boolean, false),
        Field::new("warning", DataType::Boolean, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x),
            Arc::new(y),
            Arc::new(category),
            Arc::new(value),
            Arc::new(error_flag),
            Arc::new(warning_flag),
        ],
    )
    .unwrap();

    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .size(150.0)
            // Multiple conditions: errors are red, warnings are orange, otherwise use value scale
            .fill_with(col("value"), |c| {
                c.when_value(col("error"), lit("#ff0000")) // Red for errors
                    .when_value(col("warning"), lit("#ff8800")) // Orange for warnings
                    .scale_with::<Linear>(|s| {
                        s.range_colors(vec![
                            Srgba::new(0.2, 0.4, 0.8, 1.0), // Blue
                            Srgba::new(0.2, 0.8, 0.4, 1.0), // Green
                        ])
                    })
                    .legend(|l| l.title("Status"))
            })
            // Shape based on category
            .shape_with(col("category"), |c| c.legend(|l| l.title("Category"))),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "conditional",
        "conditional_multiple_conditions",
    )
    .await;
}

#[tokio::test]
async fn test_conditional_with_legend() {
    // Test that conditional encoding works properly with legends
    let ctx = SessionContext::new();

    // Create temperature data with threshold-based coloring
    let x = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
    let y = Float64Array::from(vec![15.0, 18.0, 22.0, 28.0, 32.0, 35.0, 38.0, 20.0]);
    let temp = Float64Array::from(vec![15.0, 18.0, 22.0, 28.0, 32.0, 35.0, 38.0, 20.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("temperature", DataType::Float64, false),
    ]));

    let batch =
        RecordBatch::try_new(schema, vec![Arc::new(x), Arc::new(y), Arc::new(temp)]).unwrap();

    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.scale(|s| s.domain((0.0, 10.0))).axis(|a| a.title("Time"))
            })
            .y_with(col("y"), |c| {
                c.scale(|s| s.domain((10.0, 40.0)))
                    .axis(|a| a.title("Temperature (°C)"))
            })
            .size(200.0)
            // Conditional color with thresholds
            .fill_with(col("temperature"), |c| {
                c.when_value(col("temperature").gt(lit(35)), lit("#ff0000")) // Red for > 35°C
                    .when_value(col("temperature").gt(lit(30)), lit("#ff8800")) // Orange for > 30°C
                    .when_value(col("temperature").lt(lit(20)), lit("#0088ff")) // Blue for < 20°C
                    .scale_with::<Linear>(|s| {
                        s.range_colors(vec![
                            Srgba::new(0.2, 0.8, 0.2, 1.0), // Green for normal range
                            Srgba::new(0.8, 0.8, 0.2, 1.0), // Yellow-green
                        ])
                    })
                    .legend(|l| l.title("Temperature"))
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "conditional",
        "conditional_with_legend",
    )
    .await;
}
