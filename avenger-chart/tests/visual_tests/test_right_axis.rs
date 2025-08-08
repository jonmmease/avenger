use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::LegendPosition;
use avenger_chart::axis::AxisPosition;
use avenger_chart::coords::Cartesian;
use avenger_chart::marks::line::Line;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

fn make_df_for_symbol() -> DataFrame {
    let categories = StringArray::from(vec!["A", "B", "C", "A", "B", "C", "A", "B", "C"]);
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 10.0, 9.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(x_values), Arc::new(y_values)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

fn make_df_for_line() -> DataFrame {
    let series = StringArray::from(vec![
        "S1", "S1", "S1", "S1", "S1", "S2", "S2", "S2", "S2", "S2",
    ]);
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
    let y_values = Float64Array::from(vec![
        10.0, 15.0, 12.0, 18.0, 20.0, 8.0, 12.0, 9.0, 14.0, 16.0,
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("series", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(series), Arc::new(x_values), Arc::new(y_values)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

#[tokio::test]
async fn right_axis_no_legend_symbol_mark() {
    let df = make_df_for_symbol();

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|s| s.domain((0.0, 10.0)))
        .scale_y(|s| s.domain((0.0, 12.0)))
        .axis_y(|a| a.position(AxisPosition::Right).title("Y Right").grid(true))
        .legend_fill(|l| l.visible(false))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(100.0)
                .fill("#2ca25f"),
        );

    assert_visual_match_default(plot, "layout", "right_axis_no_legend_symbol").await;
}

#[tokio::test]
async fn right_axis_with_symbol_legend() {
    let df = make_df_for_symbol();

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|s| s.domain((0.0, 10.0)))
        .scale_y(|s| s.domain((0.0, 12.0)))
        .axis_y(|a| a.position(AxisPosition::Right).title("Y Right").grid(true))
        .legend_fill(|l| l.title("Category").position(LegendPosition::Right))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(100.0)
                .fill(col("category")),
        );

    assert_visual_match_default(plot, "layout", "right_axis_with_symbol_legend").await;
}

#[tokio::test]
async fn right_axis_with_line_legend() {
    let df = make_df_for_line();

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|s| s.domain((0.0, 6.0)))
        .scale_y(|s| s.domain((0.0, 22.0)))
        .axis_y(|a| a.position(AxisPosition::Right).title("Y Right").grid(true))
        .legend_stroke(|l| l.title("Series").position(LegendPosition::Right))
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke(col("series"))
                .stroke_width(2.0),
        );

    assert_visual_match_default(plot, "layout", "right_axis_with_line_legend").await;
}

#[tokio::test]
async fn right_axis_with_colorbar_legend() {
    let df = make_df_for_symbol();

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|s| s.domain((0.0, 10.0)))
        .scale_y(|s| s.domain((0.0, 12.0)))
        .scale_fill(|s| s.domain((0.0, 100.0)))
        .axis_y(|a| a.position(AxisPosition::Right).title("Y Right").grid(true))
        .legend_fill(|l| l.title("Temp").position(LegendPosition::Right))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(100.0)
                .fill(lit(42.0)),
        );

    assert_visual_match_default(plot, "layout", "right_axis_with_colorbar_legend").await;
}

#[tokio::test]
async fn top_x_axis_with_right_y_no_legend() {
    let df = make_df_for_symbol();

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|s| s.domain((0.0, 10.0)))
        .scale_y(|s| s.domain((0.0, 12.0)))
        .axis_x(|a| a.position(AxisPosition::Top).title("X Top").grid(true))
        .axis_y(|a| a.position(AxisPosition::Right).title("Y Right").grid(true))
        .legend_fill(|l| l.visible(false))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(100.0)
                .fill("#2ca25f"),
        );

    assert_visual_match_default(plot, "layout", "top_x_axis_with_right_y_no_legend").await;
}
