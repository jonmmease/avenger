use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;

use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

fn make_test_data() -> DataFrame {
    let categories = StringArray::from(vec!["A", "B", "C", "A", "B", "C", "A", "B", "C"]);
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 10.0, 9.0]);
    let values = Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(categories),
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(values),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

#[tokio::test]
async fn test_larger_font_size() {
    let df = make_test_data();

    // Create a plot with larger base font size (18px instead of default 12px)
    let plot = Plot::<Cartesian>::new()
        .title("Large Font Size Example")
        .subtitle("All fonts scaled to 150%")
        .with_theme(|t| t.with_font_size(18.0))
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 10.0)))
                        .axis(|a| a.title("X Axis").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 12.0)))
                        .axis(|a| a.title("Y Axis").grid(true))
                })
                .size(150.0)
                .fill_with(col("category"), |c| {
                    c.legend(|l| l.title("Category").position(LegendPosition::Right))
                }),
        );

    assert_visual_match_default(plot, "font_scaling", "larger_font_size").await;
}

#[tokio::test]
async fn test_compact_font_scale() {
    let df = make_test_data();

    // Create a plot with compact font scaling
    let plot = Plot::<Cartesian>::new()
        .title("Compact Font Scale")
        .subtitle("Tighter visual hierarchy")
        .with_theme(|t| t.with_compact_fonts())
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 10.0)))
                        .axis(|a| a.title("X Axis").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 12.0)))
                        .axis(|a| a.title("Y Axis").grid(true))
                })
                .size(100.0)
                .fill_with(col("value"), |c| {
                    c.scale(|s| s.domain((0.0, 100.0)))
                        .legend(|l| l.title("Value").position(LegendPosition::Right))
                }),
        );

    assert_visual_match_default(plot, "font_scaling", "compact_font_scale").await;
}

#[tokio::test]
async fn test_dramatic_font_scale() {
    let df = make_test_data();

    // Create a plot with dramatic font scaling (more contrast)
    let plot = Plot::<Cartesian>::new()
        .title("Dramatic Font Scale")
        .subtitle("Higher contrast hierarchy")
        .with_theme(|t| t.with_dramatic_fonts())
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 10.0)))
                        .axis(|a| a.title("X Axis").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 12.0)))
                        .axis(|a| a.title("Y Axis").grid(true))
                })
                .size(100.0)
                .shape_with(col("category"), |c| c.legend(|l| l.title("Shape Category"))),
        );

    assert_visual_match_default(plot, "font_scaling", "dramatic_font_scale").await;
}

#[tokio::test]
async fn test_small_font_size() {
    let df = make_test_data();

    // Create a plot with smaller base font size (8px instead of default 12px)
    let plot = Plot::<Cartesian>::new()
        .title("Small Font Size Example")
        .subtitle("All fonts at 67% scale")
        .with_theme(|t| t.with_font_size(8.0))
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 10.0)))
                        .axis(|a| a.title("X Axis").grid(false))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 12.0)))
                        .axis(|a| a.title("Y Axis").grid(false))
                })
                .size(80.0)
                .fill_with(col("category"), |c| {
                    c.legend(|l| l.title("Category").position(LegendPosition::Bottom))
                }),
        );

    assert_visual_match_default(plot, "font_scaling", "small_font_size").await;
}

#[tokio::test]
async fn test_combined_scaling() {
    let df = make_test_data();

    // Combine larger base size with compact scale
    let plot = Plot::<Cartesian>::new()
        .title("Combined Scaling")
        .subtitle("Large base + compact scale")
        .with_theme(|t| t.with_font_size(16.0).with_compact_fonts())
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 10.0)))
                        .axis(|a| a.title("X Values").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 12.0)))
                        .axis(|a| a.title("Y Values").grid(true))
                })
                .size(120.0)
                .fill_with(col("category"), |c| c.legend(|l| l.title("Category")))
                .stroke_with(col("category"), |c| c.no_legend())
                .stroke_width(2.0),
        );

    assert_visual_match_default(plot, "font_scaling", "combined_scaling").await;
}
