use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use avenger_chart::theme::Theme;

use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::col;
use datafusion::logical_expr::lit;
use datafusion::prelude::*;
use std::sync::Arc;
// Visual tests for symbol charts

/// Create a simple scatter plot dataset
fn create_scatter_data() -> DataFrame {
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
    let y_values = Float64Array::from(vec![2.5, 3.2, 4.8, 3.1, 5.9, 7.2, 6.5, 8.1, 7.8, 9.5]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)])
        .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    ctx.read_batch(batch)
        .expect("Failed to read batch into DataFrame")
}

#[tokio::test]
async fn test_simple_scatter_plot() {
    let ctx = SessionContext::new();
    let df = create_scatter_data();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.scale_with::<Linear>(|s| s).axis(|a| a.title("X Value"))
            })
            .y_with(col("y"), |c| {
                c.scale_with::<Linear>(|s| s).axis(|a| a.title("Y Value"))
            })
            .size(100.0)
            .fill("#4682b4")
            .stroke("#000000")
            .stroke_width(1.0),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "symbol", "simple_scatter_plot").await;
}

#[tokio::test]
async fn test_simple_scatter_plot_dark() {
    let ctx = SessionContext::new();
    let df = create_scatter_data();

    let plot = Plot::<Cartesian>::new().data(df).theme(Theme::dark()).mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.scale_with::<Linear>(|s| s).axis(|a| a.title("X Value"))
            })
            .y_with(col("y"), |c| {
                c.scale_with::<Linear>(|s| s).axis(|a| a.title("Y Value"))
            })
            .size(100.0)
            .fill("#4C9ED9"), // Use a color from the dark theme palette
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "symbol", "simple_scatter_plot_dark").await;
}

#[tokio::test]
async fn test_scatter_with_shapes() {
    // Create data with different categories
    let x_values = Float64Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 1.5, 2.5, 3.5, 4.5, 5.5, 1.2, 2.2, 3.2, 4.2, 5.2,
    ]);
    let y_values = Float64Array::from(vec![
        2.0, 3.5, 2.8, 4.2, 5.1, 3.1, 4.2, 3.5, 5.0, 6.2, 1.8, 2.9, 2.5, 3.8, 4.5,
    ]);
    let category = StringArray::from(vec![
        "A", "A", "A", "A", "A", "B", "B", "B", "B", "B", "C", "C", "C", "C", "C",
    ]);
    let shape_values = StringArray::from(vec![
        "circle",
        "circle",
        "circle",
        "circle",
        "circle",
        "square",
        "square",
        "square",
        "square",
        "square",
        "triangle-up",
        "triangle-up",
        "triangle-up",
        "triangle-up",
        "triangle-up",
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("shape", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(category),
            Arc::new(shape_values),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.scale_with::<Linear>(|s| s).axis(|a| a.title("X Value"))
            })
            .y_with(col("y"), |c| {
                c.scale_with::<Linear>(|s| s).axis(|a| a.title("Y Value"))
            })
            .shape(col("shape")) // Now uses automatic ordinal scale
            .fill("#ff6347")
            .size(120.0)
            .stroke("#333333")
            .stroke_width(1.5),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "symbol", "scatter_with_shapes").await;
}

#[tokio::test]
async fn test_scatter_with_size_encoding() {
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 5.0, 8.0]);
    let y_values = Float64Array::from(vec![2.0, 3.5, 2.8, 4.2, 5.1, 4.8, 6.2, 5.5]);
    let size_values = Float64Array::from(vec![
        50.0, 50000.0, 75.0, 150.0, 200.0, 125.0, 175.0, 30625.0,
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("size", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(size_values),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::<Cartesian>::new()
        .canvas_size(600.0, 450.0)
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale_with::<Linear>(|s| s.nice(false))
                        .axis(|a| a.title("X Value").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                        .axis(|a| a.title("Y Value").grid(true))
                })
                .size_with(col("size"), |c| c.no_scale())
                .fill("rgba(255, 99, 71, 0.5)")
                .stroke("#8b0000")
                .stroke_width(2.0),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "symbol", "scatter_with_size").await;
}

#[tokio::test]
async fn test_scatter_with_size_encoding_legend() {
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 5.0, 8.0]);
    let y_values = Float64Array::from(vec![2.0, 3.5, 2.8, 4.2, 5.1, 4.8, 6.2, 5.5]);
    let size_values = Float64Array::from(vec![
        50.0, 50000.0, 75.0, 150.0, 200.0, 125.0, 175.0, 30625.0,
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("size", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(size_values),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::<Cartesian>::new()
        .canvas_size(600.0, 450.0)
        .data(df)
        .legend("fill", |legend| legend)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale_with::<Linear>(|s| s.nice(false))
                        .axis(|a| a.title("X Value").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                        .axis(|a| a.title("Y Value").grid(true))
                })
                .size_with(col("size"), |c| c.no_scale())
                .fill(col("size"))
                .stroke("#8b0000")
                .stroke_width(2.0),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "symbol", "scatter_with_size_legend").await;
}

#[tokio::test]
async fn test_scatter_with_angle() {
    // Create data for triangles with different rotations
    let x_values = Float64Array::from(vec![2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 18.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 18.0]);
    let angle_values = Float64Array::from(vec![
        0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0, 360.0,
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("angle", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(angle_values),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.axis(|a| a.title("X Position")))
            .y_with(col("y"), |c| c.axis(|a| a.title("Y Position")))
            .shape("arrow")
            .angle_with(col("angle"), |c| c.no_scale())
            .size(800.0)
            .fill("#ff8c00")
            .stroke("#000000")
            .stroke_width(2.0),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "symbol", "scatter_with_angle").await;
}

#[tokio::test]
async fn test_scatter_with_angle_scale() {
    // Create data with angle values from 0 to 100 that should scale to 0-360 degrees
    let x_values = Float64Array::from(vec![2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0]);
    // Angle values from 0 to 100 (should scale to 0-360 degrees)
    let angle_values = Float64Array::from(vec![0.0, 12.5, 25.0, 37.5, 50.0, 62.5, 75.0, 100.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("angle", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(angle_values),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|s| s))
            .y_with(col("y"), |c| c.scale(|s| s))
            .shape("arrow")
            .angle(col("angle"))
            .size(400.0)
            .fill("#2ecc71")
            .stroke("#27ae60")
            .stroke_width(2.0),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "symbol", "scatter_with_angle_scale").await;
}

#[tokio::test]
async fn test_scatter_with_default_shape_scale() {
    // Create data with categories that will map to default shapes
    let x_values = Float64Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0,
        1.5, 2.5, 3.5, 4.5, 5.5, 6.5,
    ]);
    let y_values = Float64Array::from(vec![
        2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0, 4.0, 4.0, 4.0, 4.0, 4.0, 4.0,
        5.0, 5.0, 5.0, 5.0, 5.0, 5.0,
    ]);
    let category = StringArray::from(vec![
        "A", "B", "C", "D", "E", "F", "A", "B", "C", "D", "E", "F", "A", "B", "C", "D", "E", "F",
        "A", "B", "C", "D", "E", "F",
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(x_values), Arc::new(y_values), Arc::new(category)],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.axis(|a| a.title("X Value")))
            .y_with(col("y"), |c| c.axis(|a| a.title("Y Value")))
            .shape(col("category")) // This will trigger default ordinal scale
            .size(200.0)
            .fill("#3498db")
            .stroke("#2c3e50")
            .stroke_width(2.0),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "symbol",
        "scatter_with_default_shapes",
    )
    .await;
}

#[tokio::test]
async fn test_scatter_with_threshold_shape() {
    // Create continuous data that will be mapped to shapes using quantize scale
    let x_values = Float64Array::from(vec![
        1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5,
        9.5, 10.5,
    ]);
    let y_values = Float64Array::from(vec![
        2.0, 3.5, 2.8, 4.2, 5.1, 4.8, 6.2, 5.5, 7.0, 6.5, 3.0, 2.5, 3.8, 4.5, 5.5, 5.2, 6.8, 6.0,
        7.5, 7.2,
    ]);
    // Values from 0 to 100 that will be mapped to shapes using quantize scale
    let magnitude = Float64Array::from(vec![
        5.0, 12.0, 18.0, 25.0, 32.0, 38.0, 45.0, 52.0, 58.0, 65.0, 72.0, 78.0, 85.0, 92.0, 15.0,
        28.0, 42.0, 55.0, 68.0, 82.0,
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("magnitude", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(x_values), Arc::new(y_values), Arc::new(magnitude)],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::<Cartesian>::new()
        .data(df)
        .legend("shape", |legend| legend.title("Magnitude"))
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale_with::<Linear>(|s| s)
                        .axis(|a| a.title("X Position"))
                })
                .y_with(col("y"), |c| {
                    c.scale_with::<Linear>(|s| s)
                        .axis(|a| a.title("Y Position"))
                })
                .shape_with(col("magnitude"), |c| {
                    c.scale_with::<Threshold>(|s| {
                        s.domain_discrete(vec![lit(0.0), lit(20.0), lit(40.0), lit(100.0)])
                            .range_discrete(vec![
                                "circle",
                                "square",
                                "triangle-up",
                                "diamond",
                                "cross",
                            ])
                    })
                })
                .size(150.0)
                .fill("#e74c3c")
                .stroke("#c0392b")
                .stroke_width(2.0),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "symbol",
        "scatter_with_threshold_shape",
    )
    .await;
}
