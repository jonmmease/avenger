use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::axis::AxisPosition;
use avenger_chart::cartesian::Cartesian;

use avenger_chart::legend::LegendPosition;
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

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(x_values), Arc::new(y_values), Arc::new(value)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

#[tokio::test]
async fn title_basic_symbol() {
    let df = make_df_categories();
    let plot = Plot::<Cartesian>::new().title("Basic Title").data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
            .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
            .size(100.0)
            .fill_with("#2ca25f", |c| c.no_legend()),
    );

    assert_visual_match_default(plot, "layout", "title_basic_symbol").await;
}

#[tokio::test]
async fn title_with_symbol_legend() {
    let df = make_df_categories();
    let plot = Plot::<Cartesian>::new()
        .title("Title With Legend")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
                .size(100.0)
                .fill_with(col("category"), |c| {
                    c.legend(|l| l.title("Category").position(LegendPosition::Right))
                }),
        );

    assert_visual_match_default(plot, "layout", "title_with_symbol_legend").await;
}

#[tokio::test]
async fn title_top_x_right_y() {
    let df = make_df_categories();
    let plot = Plot::<Cartesian>::new()
        .title("Top X & Right Y")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 10.0)))
                        .axis(|a| a.position(AxisPosition::Top).title("Top X").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 12.0)))
                        .axis(|a| a.position(AxisPosition::Right).title("Right Y").grid(true))
                })
                .size(100.0)
                .fill_with("#2ca25f", |c| c.no_legend()),
        );

    assert_visual_match_default(plot, "layout", "title_top_x_right_y").await;
}

#[tokio::test]
async fn title_with_colorbar_legend() {
    let df = make_df_numeric();
    let plot = Plot::<Cartesian>::new()
        .title("Title With Colorbar")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
                .size(100.0)
                .fill_with(col("value"), |c| {
                    c.scale(|s| s.domain((0.0, 100.0)))
                        .legend(|l| l.title("Value").position(LegendPosition::Right))
                }),
        );

    assert_visual_match_default(plot, "layout", "title_with_colorbar_legend").await;
}

#[tokio::test]
async fn subtitle_basic_symbol() {
    let df = make_df_categories();
    let plot = Plot::<Cartesian>::new()
        .title("Main Title")
        .subtitle("This is a subtitle")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
                .size(100.0)
                .fill_with("#2ca25f", |c| c.no_legend()),
        );

    assert_visual_match_default(plot, "layout", "subtitle_basic_symbol").await;
}

#[tokio::test]
async fn subtitle_with_legend() {
    let df = make_df_numeric();
    let plot = Plot::<Cartesian>::new()
        .title("Main Title")
        .subtitle("This is a subtitle with a legend")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
                .size(100.0)
                .fill_with(col("value"), |c| {
                    c.scale(|s| s.domain((0.0, 100.0)))
                        .legend(|l| l.title("Value").position(LegendPosition::Right))
                }),
        );

    assert_visual_match_default(plot, "layout", "subtitle_with_legend").await;
}

// Note: The current API doesn't support custom title properties like color, font_size, or align
// These tests are removed since those features don't exist yet

#[tokio::test]
async fn title_with_axes_positions() {
    let df = make_df_categories();
    let plot = Plot::<Cartesian>::new()
        .title("Title With Different Axes Positions")
        .subtitle("Demonstrating layout")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 10.0)))
                        .axis(|a| a.position(AxisPosition::Top).title("Top X").grid(false))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 12.0)))
                        .axis(|a| a.position(AxisPosition::Right).title("Right Y").grid(false))
                })
                .size(100.0)
                .fill_with("#c0392b", |c| c.no_legend()),
        );

    assert_visual_match_default(plot, "layout", "title_with_axes_positions").await;
}

#[tokio::test]
async fn multiline_title_subtitle() {
    let df = make_df_categories();
    let plot = Plot::<Cartesian>::new()
        .title("This is a Very Long Title That Should\nSpan Multiple Lines")
        .subtitle("And this is also a long subtitle\nthat spans multiple lines\nfor demonstration")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
                .size(100.0)
                .fill_with("#34495e", |c| c.no_legend()),
        );

    assert_visual_match_default(plot, "layout", "multiline_title_subtitle").await;
}
