use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
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

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .legend("fill", |legend| {
            legend
                .title("Category")
                .position(LegendPosition::Right)
                .order(1)
        })
        .legend("size", |legend| {
            legend
                .title("Size")
                .position(LegendPosition::Right)
                .order(2)
        })
        .legend("shape", |legend| {
            legend
                .title("Shape")
                .position(LegendPosition::Right)
                .order(3)
        })
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 10.0)))
                        .axis(|axis| axis.title("X Axis"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 10.0)))
                        .axis(|axis| axis.title("Y Axis"))
                })
                .fill_with(col("category"), |c| c.scale_with::<Ordinal>(|s| s))
                .size_with(col("size_value"), |c| {
                    c.scale(|s| s.domain((5.0, 40.0)).range_interval(lit(25.0), lit(200.0)))
                })
                .shape_with(col("shape_type"), |c| c.scale_with::<Ordinal>(|s| s)),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "scatter_multiple_legends").await;
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

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .legend("stroke", |legend| {
            legend
                .title("Line Series")
                .position(LegendPosition::Right)
                .order(1)
        })
        .legend("shape", |legend| {
            legend
                .title("Symbol Type")
                .position(LegendPosition::Right)
                .order(2)
        })
        .legend("fill", |legend| {
            legend
                .title("Temperature")
                .position(LegendPosition::Right)
                .order(3)
        })
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 6.0)))
                        .axis(|axis| axis.title("X Axis"))
                })
                .y_with(col("y1"), |c| {
                    c.scale(|s| s.domain((0.0, 8.0)))
                        .axis(|axis| axis.title("Y Axis"))
                })
                .stroke_with(col("series"), |c| c.scale_with::<Ordinal>(|s| s))
                .stroke_width(2.0),
        )
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 6.0))))
                .y_with(col("y2"), |c| c.scale(|s| s.domain((0.0, 8.0))))
                .shape_with(col("series"), |c| c.scale_with::<Ordinal>(|s| s))
                .size(50.0),
        )
        .mark(
            Rect::new()
                .x_with(col("x") - lit(0.3), |c| c.scale(|s| s.domain((0.0, 6.0))))
                .x2_with(col("x") + lit(0.3), |c| c.scale(|s| s.domain((0.0, 6.0))))
                .y_with(lit(0.0), |c| c.scale(|s| s.domain((0.0, 8.0))))
                .y2_with(col("temperature") / lit(5.0), |c| {
                    c.scale(|s| s.domain((0.0, 8.0)))
                })
                .fill_with(col("temperature"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((0.0, 35.0)))
                }),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "mixed_legend_types").await;
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

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .legend("fill", |legend| {
            legend.title("Category").position(LegendPosition::Right)
        })
        .legend("size", |legend| {
            legend.title("Size").position(LegendPosition::Bottom)
        })
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 7.0)))
                        .axis(|axis| axis.title("X Axis"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 7.0)))
                        .axis(|axis| axis.title("Y Axis"))
                })
                .fill_with(col("category"), |c| c.scale_with::<Ordinal>(|s| s))
                .size_with(col("size_value"), |c| {
                    c.scale(|s| s.domain((5.0, 35.0)).range_interval(lit(25.0), lit(150.0)))
                }),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "layout",
        "legends_different_positions",
    )
    .await;
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

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .legend("fill", |legend| {
            legend
                .title("Temperature °C")
                .position(LegendPosition::Right)
                .order(1)
        })
        .legend("shape", |legend| {
            legend
                .title("Type")
                .position(LegendPosition::Right)
                .order(2)
        })
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 9.0)))
                        .axis(|axis| axis.title("X Axis"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 9.0)))
                        .axis(|axis| axis.title("Y Axis"))
                })
                .fill_with(col("temperature"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((5.0, 40.0)))
                })
                .shape_with(col("shape_type"), |c| c.scale_with::<Ordinal>(|s| s))
                .size(100.0),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "colorbar_with_symbols").await;
}

/// Test colorbar and symbol legends both at bottom position
#[tokio::test]
async fn test_colorbar_with_symbols_bottom() {
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

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .legend("fill", |legend| {
            legend
                .title("Temperature °C")
                .position(LegendPosition::Bottom)
                .order(1)
        })
        .legend("shape", |legend| {
            legend
                .title("Type")
                .position(LegendPosition::Bottom)
                .order(2)
        })
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 9.0)))
                        .axis(|axis| axis.title("X Axis"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 9.0)))
                        .axis(|axis| axis.title("Y Axis"))
                })
                .fill_with(col("temperature"), |c| {
                    c.scale_with::<Linear>(|s| s.domain((5.0, 40.0)))
                })
                .shape_with(col("shape_type"), |c| c.scale_with::<Ordinal>(|s| s))
                .size(100.0),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "colorbar_with_symbols_bottom").await;
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

    let plot = Plot::<Cartesian>::new()
        .data(df)
        // Test explicit ordering - should appear in order 3, 1, 2
        .legend("shape", |legend| {
            legend
                .title("Legend A")
                .position(LegendPosition::Right)
                .order(3)
        })
        .legend("fill", |legend| {
            legend
                .title("Legend C")
                .position(LegendPosition::Right)
                .order(1)
        })
        .legend("stroke", |legend| {
            legend
                .title("Legend B")
                .position(LegendPosition::Right)
                .order(2)
        })
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 6.0)))
                        .axis(|axis| axis.title("X Axis"))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 6.0)))
                        .axis(|axis| axis.title("Y Axis"))
                })
                .fill_with(col("c"), |c| c.scale_with::<Ordinal>(|s| s))
                .shape_with(col("a"), |c| c.scale_with::<Ordinal>(|s| s))
                .stroke_with(col("b"), |c| c.scale_with::<Ordinal>(|s| s)),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "legend_ordering").await;
}
