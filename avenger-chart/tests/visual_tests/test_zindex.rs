//! Visual tests for zindex behavior

use avenger_chart::coords::Cartesian;
use avenger_chart::marks::line::Line;
use avenger_chart::marks::rect::Rect;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::{Float64Array, Int32Array};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

use super::helpers::assert_visual_match_default;

#[tokio::test]
async fn test_zindex_ordering() {
    // Create data for overlapping marks
    let x_values = Int32Array::from(vec![1, 2, 3, 4, 5]);
    let y_values = Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Int32, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)])
        .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::new(Cartesian)
        
        .data(df)
        .scale_x(|scale| scale.scale_type("linear").domain((0.0, 6.0)))
        .scale_y(|scale| scale.scale_type("linear").domain((0.0, 60.0)))
        .axis_x(|axis| axis.title("X"))
        .axis_y(|axis| axis.title("Y"))
        // First: Light blue line (with high zindex=10, should be drawn last/on top)
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke("#6699ff")
                .stroke_width(8.0)
                .zindex(10),
        )
        // Second: Red line offset (with middle zindex=5, should be drawn in middle)
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y") + lit(5.0))
                .stroke("#ff6666")
                .stroke_width(8.0)
                .zindex(5),
        )
        // Third: Green rect (with low zindex=1, should be drawn first/bottom)
        .mark(
            Rect::new()
                .x(lit(2.0))
                .x2(lit(4.0))
                .y(lit(20.0))
                .y2(lit(40.0))
                .fill("#66ff66")
                .opacity(0.8)
                .zindex(1),
        );

    assert_visual_match_default(plot, "zindex", "zindex_ordering").await;
}

#[tokio::test]
async fn test_zindex_default_order() {
    // Create data for overlapping marks - SAME AS ABOVE
    let x_values = Int32Array::from(vec![1, 2, 3, 4, 5]);
    let y_values = Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Int32, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)])
        .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(batch)
        .expect("Failed to read batch into DataFrame");

    let plot = Plot::new(Cartesian)
        
        .data(df)
        .scale_x(|scale| scale.scale_type("linear").domain((0.0, 6.0)))
        .scale_y(|scale| scale.scale_type("linear").domain((0.0, 60.0)))
        .axis_x(|axis| axis.title("X"))
        .axis_y(|axis| axis.title("Y"))
        // SAME marks, SAME order, but NO zindex - should render in declaration order
        // First: Light blue line (no zindex, should be drawn first/bottom)
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke("#6699ff")
                .stroke_width(8.0),
        )
        // Second: Red line offset (no zindex, should be drawn second/middle)
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y") + lit(5.0))
                .stroke("#ff6666")
                .stroke_width(8.0),
        )
        // Third: Green rect (no zindex, should be drawn last/on top)
        .mark(
            Rect::new()
                .x(lit(2.0))
                .x2(lit(4.0))
                .y(lit(20.0))
                .y2(lit(40.0))
                .fill("#66ff66")
                .opacity(0.8),
        );

    assert_visual_match_default(plot, "zindex", "zindex_default_order").await;
}
