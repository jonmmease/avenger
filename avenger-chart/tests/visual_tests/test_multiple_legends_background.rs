use super::helpers::assert_visual_match_default;
use avenger_chart::cartesian::Cartesian;
use avenger_chart::legend::LegendPosition;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use avenger_chart::scales::{Linear, Ordinal};
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::{col, lit};
use datafusion::prelude::*;
use std::sync::Arc;

/// Test multiple legends with backgrounds to visualize spacing
#[tokio::test]
async fn test_multiple_legends_with_backgrounds() {
    // Create sample data
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("size_value", DataType::Float64, false),
        Field::new("shape_type", DataType::Utf8, false),
    ]));

    let x_data = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    let y_data = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 9.0, 7.5]);
    let category_data = StringArray::from(vec!["A", "B", "A", "B", "C", "A", "C", "B", "C"]);
    let size_data = Float64Array::from(vec![10.0, 20.0, 15.0, 25.0, 30.0, 12.0, 35.0, 22.0, 28.0]);
    let shape_data = StringArray::from(vec![
        "Type 1", "Type 2", "Type 1", "Type 3", "Type 2", "Type 3", "Type 1", "Type 2", "Type 3",
    ]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(x_data),
            Arc::new(y_data),
            Arc::new(category_data),
            Arc::new(size_data),
            Arc::new(shape_data),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .scale_x(|s| s.domain((0.0, 10.0)))
        .scale_y(|s| s.domain((0.0, 10.0)))
        .scale_fill_with::<Ordinal>(|s| s)
        .scale_size(|s| s.domain((5.0, 40.0)).range_interval(lit(25.0), lit(200.0)))
        .scale_shape_with::<Ordinal>(|s| s)
        .axis_x(|axis| axis.title("X Axis"))
        .axis_y(|axis| axis.title("Y Axis"))
        .legend_fill(|legend| {
            legend
                .title("Category")
                .position(LegendPosition::Right)
                .order(1)
                .background_fill("rgba(255, 200, 200, 0.3)")
                .background_stroke("red")
                .background_corner_radius(4.0)
                .background_padding(8.0)
        })
        .legend_size(|legend| {
            legend
                .title("Size")
                .position(LegendPosition::Right)
                .order(2)
                .background_fill("rgba(200, 255, 200, 0.3)")
                .background_stroke("green")
                .background_corner_radius(4.0)
                .background_padding(8.0)
        })
        .legend_shape(|legend| {
            legend
                .title("Shape")
                .position(LegendPosition::Right)
                .order(3)
                .background_fill("rgba(200, 200, 255, 0.3)")
                .background_stroke("blue")
                .background_corner_radius(4.0)
                .background_padding(8.0)
        })
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("category"))
                .size(col("size_value"))
                .shape(col("shape_type")),
        );

    assert_visual_match_default(plot, "layout", "multiple_legends_with_backgrounds").await;
}

/// Test mixed legend types with backgrounds (colorbar + symbols)
#[tokio::test]
async fn test_colorbar_with_symbols_backgrounds() {
    // Create sample data
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("temperature", DataType::Float64, false),
        Field::new("shape_type", DataType::Utf8, false),
        Field::new("series", DataType::Utf8, false),
    ]));

    let x_data = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
    let y_data = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 7.5]);
    let temp_data = Float64Array::from(vec![10.0, 20.0, 30.0, 25.0, 15.0, 35.0, 28.0, 22.0]);
    let shape_data = StringArray::from(vec![
        "Sensor", "Device", "Sensor", "Monitor", "Device", "Monitor", "Sensor", "Device",
    ]);
    let series_data = StringArray::from(vec!["A", "B", "A", "B", "A", "B", "A", "B"]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(x_data),
            Arc::new(y_data),
            Arc::new(temp_data),
            Arc::new(shape_data),
            Arc::new(series_data),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .scale_x(|s| s.domain((0.0, 9.0)))
        .scale_y(|s| s.domain((0.0, 9.0)))
        .scale_fill_with::<Linear>(|s| s.domain((5.0, 40.0)))
        .scale_shape_with::<Ordinal>(|s| s)
        .scale_stroke_with::<Ordinal>(|s| s)
        .axis_x(|axis| axis.title("X Axis"))
        .axis_y(|axis| axis.title("Y Axis"))
        .legend_fill(|legend| {
            legend
                .title("Temperature °C")
                .position(LegendPosition::Right)
                .order(1)
                .background_fill("rgba(255, 255, 200, 0.3)")
                .background_stroke("orange")
                .background_corner_radius(4.0)
                .background_padding(10.0)
        })
        .legend_shape(|legend| {
            legend
                .title("Type")
                .position(LegendPosition::Right)
                .order(2)
                .background_fill("rgba(200, 200, 255, 0.3)")
                .background_stroke("blue")
                .background_corner_radius(4.0)
                .background_padding(10.0)
        })
        .legend_stroke(|legend| {
            legend
                .title("Series")
                .position(LegendPosition::Right)
                .order(3)
                .background_fill("rgba(200, 255, 200, 0.3)")
                .background_stroke("green")
                .background_corner_radius(4.0)
                .background_padding(10.0)
        })
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("temperature"))
                .shape(col("shape_type"))
                .stroke(col("series"))
                .size(100.0),
        );

    assert_visual_match_default(plot, "layout", "colorbar_with_symbols_backgrounds").await;
}

/// Test legends at different positions with backgrounds
#[tokio::test]
async fn test_legends_different_positions_backgrounds() {
    // Create sample data
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("size_value", DataType::Float64, false),
        Field::new("series", DataType::Utf8, false),
    ]));

    let x_data = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let y_data = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 6.0, 4.5]);
    let category_data = StringArray::from(vec!["A", "B", "A", "B", "C", "C"]);
    let size_data = Float64Array::from(vec![10.0, 20.0, 15.0, 25.0, 30.0, 18.0]);
    let series_data = StringArray::from(vec!["X", "Y", "X", "Y", "X", "Y"]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(x_data),
            Arc::new(y_data),
            Arc::new(category_data),
            Arc::new(size_data),
            Arc::new(series_data),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .scale_x(|s| s.domain((0.0, 7.0)))
        .scale_y(|s| s.domain((0.0, 7.0)))
        .scale_fill_with::<Ordinal>(|s| s)
        .scale_size(|s| s.domain((5.0, 35.0)).range_interval(lit(25.0), lit(150.0)))
        .scale_stroke_with::<Ordinal>(|s| s)
        .axis_x(|axis| axis.title("X Axis"))
        .axis_y(|axis| axis.title("Y Axis"))
        .legend_fill(|legend| {
            legend
                .title("Category")
                .position(LegendPosition::Right)
                .background_fill("rgba(255, 200, 200, 0.3)")
                .background_stroke("red")
                .background_corner_radius(6.0)
                .background_padding(10.0)
        })
        .legend_stroke(|legend| {
            legend
                .title("Series")
                .position(LegendPosition::Right)
                .background_fill("rgba(200, 255, 200, 0.3)")
                .background_stroke("green")
                .background_corner_radius(6.0)
                .background_padding(10.0)
        })
        .legend_size(|legend| {
            legend
                .title("Size")
                .position(LegendPosition::Bottom)
                .background_fill("rgba(200, 200, 255, 0.3)")
                .background_stroke("blue")
                .background_corner_radius(6.0)
                .background_padding(10.0)
        })
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("category"))
                .size(col("size_value"))
                .stroke(col("series")),
        );

    assert_visual_match_default(plot, "layout", "legends_different_positions_backgrounds").await;
}
