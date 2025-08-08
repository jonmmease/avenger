use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::axis::AxisPosition;
use avenger_chart::coords::Cartesian;
use avenger_chart::LegendPosition;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

fn make_df_categories() -> DataFrame {
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

fn make_df_numeric() -> DataFrame {
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 10.0, 9.0]);
    let value = Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values), Arc::new(value)])
        .unwrap();

    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

#[tokio::test]
async fn title_basic_symbol() {
    let df = make_df_categories();
    let plot = Plot::new(Cartesian)
        .title("Basic Title")
        .data(df)
        .scale_x(|s| s.domain((0.0, 10.0)))
        .scale_y(|s| s.domain((0.0, 12.0)))
        .legend_fill(|l| l.visible(false))
        .mark(Symbol::new().x(col("x")).y(col("y")).size(100.0).fill("#2ca25f"));

    assert_visual_match_default(plot, "layout", "title_basic_symbol").await;
}

#[tokio::test]
async fn title_with_symbol_legend() {
    let df = make_df_categories();
    let plot = Plot::new(Cartesian)
        .title("Title With Legend")
        .data(df)
        .scale_x(|s| s.domain((0.0, 10.0)))
        .scale_y(|s| s.domain((0.0, 12.0)))
        .legend_fill(|l| l.title("Category").position(LegendPosition::Right))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(100.0)
                .fill(col("category")),
        );

    assert_visual_match_default(plot, "layout", "title_with_symbol_legend").await;
}

#[tokio::test]
async fn title_top_x_right_y() {
    let df = make_df_categories();
    let plot = Plot::new(Cartesian)
        .title("Top X & Right Y")
        .data(df)
        .scale_x(|s| s.domain((0.0, 10.0)))
        .scale_y(|s| s.domain((0.0, 12.0)))
        .axis_x(|a| a.position(AxisPosition::Top).title("Top X").grid(true))
        .axis_y(|a| a.position(AxisPosition::Right).title("Right Y").grid(true))
        .legend_fill(|l| l.visible(false))
        .mark(Symbol::new().x(col("x")).y(col("y")).size(100.0).fill("#2ca25f"));

    assert_visual_match_default(plot, "layout", "title_top_x_right_y").await;
}

#[tokio::test]
async fn title_with_colorbar_legend() {
    let df = make_df_numeric();
    let plot = Plot::new(Cartesian)
        .title("Title With Colorbar")
        .data(df)
        .scale_x(|s| s.domain((0.0, 10.0)))
        .scale_y(|s| s.domain((0.0, 12.0)))
        .scale_fill(|s| s.domain((0.0, 100.0)))
        .legend_fill(|l| l.title("Value").position(LegendPosition::Right))
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(100.0)
                .fill(col("value")),
        );

    assert_visual_match_default(plot, "layout", "title_with_colorbar_legend").await;
}


