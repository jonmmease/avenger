//! Visual tests for symbol charts

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::marks::ChannelExpr;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::logical_expr::{col, lit};
use datafusion::prelude::*;
use std::sync::Arc;

use super::helpers::assert_visual_match_default;

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
    let df = create_scatter_data();

    let plot = Plot::new(Cartesian)
        .preferred_size(600.0, 400.0)
        .data(df)
        .scale_x(|scale| scale.scale_type("linear"))
        .scale_y(|scale| scale.scale_type("linear"))
        .axis_x(|axis| axis.title("X Value"))
        .axis_y(|axis| axis.title("Y Value"))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(lit(100.0))
                .fill("#4682b4")
                .stroke("#000000")
                .stroke_width(lit(1.0)),
        );

    // Verify symbol mark is created correctly
    let renderer = avenger_chart::render::PlotRenderer::new(&plot);
    let result = renderer.render().await.expect("Render should succeed");
    let marks = &result.scene_graph.marks;
    assert_eq!(marks.len(), 1);
    
    // Find symbol mark within groups
    fn find_symbol_mark(mark: &avenger_scenegraph::marks::mark::SceneMark) -> Option<&avenger_scenegraph::marks::symbol::SceneSymbolMark> {
        match mark {
            avenger_scenegraph::marks::mark::SceneMark::Symbol(s) => Some(s),
            avenger_scenegraph::marks::mark::SceneMark::Group(g) => {
                for m in &g.marks {
                    if let Some(s) = find_symbol_mark(m) {
                        return Some(s);
                    }
                }
                None
            }
            _ => None,
        }
    }
    
    let symbol = find_symbol_mark(&marks[0]).expect("Should find symbol mark");
    assert_eq!(symbol.len, 10); // 10 data points

    assert_visual_match_default(plot, "symbol", "simple_scatter_plot").await;
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
        "circle", "circle", "circle", "circle", "circle",
        "square", "square", "square", "square", "square",
        "triangle-up", "triangle-up", "triangle-up", "triangle-up", "triangle-up",
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

    let plot = Plot::new(Cartesian)
        .preferred_size(600.0, 400.0)
        .data(df)
        .scale_x(|scale| scale.scale_type("linear"))
        .scale_y(|scale| scale.scale_type("linear"))
        .axis_x(|axis| axis.title("X Value"))
        .axis_y(|axis| axis.title("Y Value"))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .shape(col("shape"))  // Now uses automatic ordinal scale
                .fill("#ff6347")
                .size(lit(120.0))
                .stroke("#333333")
                .stroke_width(lit(1.5)),
        );

    // Verify symbol mark has correct shapes
    let renderer = avenger_chart::render::PlotRenderer::new(&plot);
    let result = renderer.render().await.expect("Render should succeed");
    let marks = &result.scene_graph.marks;
    assert_eq!(marks.len(), 1);
    
    // Find symbol mark within groups
    fn find_symbol_mark(mark: &avenger_scenegraph::marks::mark::SceneMark) -> Option<&avenger_scenegraph::marks::symbol::SceneSymbolMark> {
        match mark {
            avenger_scenegraph::marks::mark::SceneMark::Symbol(s) => Some(s),
            avenger_scenegraph::marks::mark::SceneMark::Group(g) => {
                for m in &g.marks {
                    if let Some(s) = find_symbol_mark(m) {
                        return Some(s);
                    }
                }
                None
            }
            _ => None,
        }
    }
    
    let symbol = find_symbol_mark(&marks[0]).expect("Should find symbol mark");
    assert_eq!(symbol.len, 15); // 15 data points
    assert_eq!(symbol.shapes.len(), 3); // 3 unique shapes

    assert_visual_match_default(plot, "symbol", "scatter_with_shapes").await;
}

#[tokio::test]
async fn test_scatter_with_size_encoding() {
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
    let y_values = Float64Array::from(vec![2.0, 3.5, 2.8, 4.2, 5.1, 4.8, 6.2, 5.5]);
    let size_values = Float64Array::from(vec![50.0, 100.0, 75.0, 150.0, 200.0, 125.0, 175.0, 225.0]);

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

    let plot = Plot::new(Cartesian)
        .preferred_size(600.0, 400.0)
        .data(df)
        .scale_x(|scale| scale.scale_type("linear"))
        .scale_y(|scale| scale.scale_type("linear"))
        .axis_x(|axis| axis.title("X Value"))
        .axis_y(|axis| axis.title("Y Value"))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(col("size"))
                .fill("#ff6347")
                .stroke("#8b0000")
                .stroke_width(lit(2.0)),
        );

    assert_visual_match_default(plot, "symbol", "scatter_with_size").await;
}

#[tokio::test]
async fn test_scatter_with_angle() {
    // Create data for triangles with different rotations
    let x_values = Float64Array::from(vec![2.0, 4.0, 6.0, 8.0, 2.0, 4.0, 6.0, 8.0]);
    let y_values = Float64Array::from(vec![2.0, 2.0, 2.0, 2.0, 4.0, 4.0, 4.0, 4.0]);
    let angle_values = Float64Array::from(vec![0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0]);

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

    let plot = Plot::new(Cartesian)
        .preferred_size(600.0, 400.0)
        .data(df)
        .scale_x(|scale| scale.scale_type("linear").domain((0.0, 10.0)))
        .scale_y(|scale| scale.scale_type("linear").domain((0.0, 6.0)))
        .axis_x(|axis| axis.title("X Position"))
        .axis_y(|axis| axis.title("Y Position"))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .shape(lit("triangle-up").identity())
                .angle(col("angle"))
                .size(lit(300.0))
                .fill("#ff8c00")
                .stroke("#000000")
                .stroke_width(lit(2.0)),
        );

    assert_visual_match_default(plot, "symbol", "scatter_with_angle").await;
}