use super::helpers::assert_visual_match_default;
use avenger_chart::cartesian::Cartesian;
use avenger_chart::legend::LegendPosition;
use avenger_chart::marks::line::Line;
use avenger_chart::marks::rect::Rect;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use avenger_chart::scales::{Linear, Ordinal};
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::{col, lit};
use datafusion::prelude::*;
use std::sync::Arc;

/// Test multiple legends for a scatter plot with size, shape, and color encodings
#[tokio::test]
async fn test_scatter_multiple_legends() {
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
        "circle", "square", "circle", "triangle", "square", "triangle", "circle", "square",
        "triangle",
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

    let plot = Plot::new(Cartesian)
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
        })
        .legend_size(|legend| {
            legend
                .title("Size")
                .position(LegendPosition::Right)
                .order(2)
        })
        .legend_shape(|legend| {
            legend
                .title("Shape")
                .position(LegendPosition::Right)
                .order(3)
        })
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("category"))
                .size(col("size_value"))
                .shape(col("shape_type")),
        );

    assert_visual_match_default(plot, "layout", "scatter_multiple_legends").await;
}

/// Test mixed legend types (symbol, line, colorbar) in same position
#[tokio::test]
async fn test_mixed_legend_types() {
    // Create sample data with multiple marks
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y1", DataType::Float64, false),
        Field::new("y2", DataType::Float64, false),
        Field::new("series", DataType::Utf8, false),
        Field::new("temperature", DataType::Float64, false),
    ]));

    let x_data = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let y1_data = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.5]);
    let y2_data = Float64Array::from(vec![1.5, 3.0, 4.0, 3.5, 5.0]);
    let series_data = StringArray::from(vec!["A", "A", "B", "B", "A"]);
    let temp_data = Float64Array::from(vec![10.0, 20.0, 30.0, 25.0, 15.0]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(x_data),
            Arc::new(y1_data),
            Arc::new(y2_data),
            Arc::new(series_data),
            Arc::new(temp_data),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|s| s.domain((0.0, 6.0)))
        .scale_y(|s| s.domain((0.0, 8.0)))
        .scale_stroke_with::<Ordinal>(|s| s)
        .scale_shape_with::<Ordinal>(|s| s)
        .scale_fill_with::<Linear>(|s| s.domain((0.0, 35.0)))
        .axis_x(|axis| axis.title("X Axis"))
        .axis_y(|axis| axis.title("Y Axis"))
        .legend_stroke(|legend| {
            legend
                .title("Line Series")
                .position(LegendPosition::Right)
                .order(1)
        })
        .legend_shape(|legend| {
            legend
                .title("Symbol Type")
                .position(LegendPosition::Right)
                .order(2)
        })
        .legend_fill(|legend| {
            legend
                .title("Temperature")
                .position(LegendPosition::Right)
                .order(3)
        })
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y1"))
                .stroke(col("series"))
                .stroke_width(2.0),
        )
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y2"))
                .shape(col("series"))
                .size(50.0),
        )
        .mark(
            Rect::new()
                .x(col("x") - lit(0.3))
                .x2(col("x") + lit(0.3))
                .y(lit(0.0))
                .y2(col("temperature") / lit(5.0))
                .fill(col("temperature")),
        );

    assert_visual_match_default(plot, "layout", "mixed_legend_types").await;
}

/// Test legends at different positions
#[tokio::test]
async fn test_legends_different_positions() {
    // Create sample data
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("size_value", DataType::Float64, false),
    ]));

    let x_data = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    let y_data = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 6.0, 4.5]);
    let category_data = StringArray::from(vec!["A", "B", "A", "B", "C", "C"]);
    let size_data = Float64Array::from(vec![10.0, 20.0, 15.0, 25.0, 30.0, 18.0]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(x_data),
            Arc::new(y_data),
            Arc::new(category_data),
            Arc::new(size_data),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|s| s.domain((0.0, 7.0)))
        .scale_y(|s| s.domain((0.0, 7.0)))
        .scale_fill_with::<Ordinal>(|s| s)
        .scale_size(|s| s.domain((5.0, 35.0)).range_interval(lit(25.0), lit(150.0)))
        .axis_x(|axis| axis.title("X Axis"))
        .axis_y(|axis| axis.title("Y Axis"))
        .legend_fill(|legend| legend.title("Category").position(LegendPosition::Right))
        .legend_size(|legend| legend.title("Size").position(LegendPosition::Bottom))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("category"))
                .size(col("size_value")),
        );

    assert_visual_match_default(plot, "layout", "legends_different_positions").await;
}

/// Test colorbar legend with symbol legends
#[tokio::test]
async fn test_colorbar_with_symbols() {
    // Create sample data
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("temperature", DataType::Float64, false),
        Field::new("shape_type", DataType::Utf8, false),
    ]));

    let x_data = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
    let y_data = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 7.5]);
    let temp_data = Float64Array::from(vec![10.0, 20.0, 30.0, 25.0, 15.0, 35.0, 28.0, 22.0]);
    let shape_data = StringArray::from(vec![
        "circle", "square", "circle", "triangle", "square", "triangle", "circle", "square",
    ]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(x_data),
            Arc::new(y_data),
            Arc::new(temp_data),
            Arc::new(shape_data),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|s| s.domain((0.0, 9.0)))
        .scale_y(|s| s.domain((0.0, 9.0)))
        .scale_fill_with::<Linear>(|s| s.domain((5.0, 40.0)))
        .scale_shape_with::<Ordinal>(|s| s)
        .axis_x(|axis| axis.title("X Axis"))
        .axis_y(|axis| axis.title("Y Axis"))
        .legend_fill(|legend| {
            legend
                .title("Temperature °C")
                .position(LegendPosition::Right)
                .order(1)
        })
        .legend_shape(|legend| {
            legend
                .title("Type")
                .position(LegendPosition::Right)
                .order(2)
        })
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("temperature"))
                .shape(col("shape_type"))
                .size(100.0),
        );

    assert_visual_match_default(plot, "layout", "colorbar_with_symbols").await;
}

/// Test legend ordering with explicit order values
#[tokio::test]
async fn test_legend_ordering() {
    // Create sample data
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("a", DataType::Utf8, false),
        Field::new("b", DataType::Utf8, false),
        Field::new("c", DataType::Utf8, false),
    ]));

    let x_data = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    let y_data = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.5]);
    let a_data = StringArray::from(vec!["A1", "A2", "A1", "A2", "A1"]);
    let b_data = StringArray::from(vec!["B1", "B1", "B2", "B2", "B1"]);
    let c_data = StringArray::from(vec!["C1", "C2", "C3", "C1", "C2"]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(x_data),
            Arc::new(y_data),
            Arc::new(a_data),
            Arc::new(b_data),
            Arc::new(c_data),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    let df = ctx.read_batch(batch).unwrap();

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|s| s.domain((0.0, 6.0)))
        .scale_y(|s| s.domain((0.0, 6.0)))
        .scale_fill_with::<Ordinal>(|s| s)
        .scale_shape_with::<Ordinal>(|s| s)
        .scale_stroke_with::<Ordinal>(|s| s)
        .axis_x(|axis| axis.title("X Axis"))
        .axis_y(|axis| axis.title("Y Axis"))
        // Test explicit ordering - should appear in order 3, 1, 2
        .legend_shape(|legend| {
            legend
                .title("Legend A")
                .position(LegendPosition::Right)
                .order(3)
        })
        .legend_fill(|legend| {
            legend
                .title("Legend C")
                .position(LegendPosition::Right)
                .order(1)
        })
        .legend_stroke(|legend| {
            legend
                .title("Legend B")
                .position(LegendPosition::Right)
                .order(2)
        })
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill(col("c"))
                .shape(col("a"))
                .stroke(col("b")),
        );

    assert_visual_match_default(plot, "layout", "legend_ordering").await;
}
