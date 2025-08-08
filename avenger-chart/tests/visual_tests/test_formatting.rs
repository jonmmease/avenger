use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::coords::Cartesian;
use avenger_chart::marks::symbol::Symbol;
use avenger_chart::plot::Plot;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

fn make_df_xy(x: &[f64], y: &[f64]) -> DataFrame {
    let x_values = Float64Array::from(x.to_vec());
    let y_values = Float64Array::from(y.to_vec());
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)]).unwrap();
    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

#[tokio::test]
async fn axis_y_currency_fixed() {
    let df = make_df_xy(
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        &[1200.0, 3400.0, 5600.0, 12345.0, 98765.0],
    );

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|s| s.domain((0.0, 6.0)))
        .scale_y(|s| s.domain((0.0, 100000.0)))
        .axis_y(|a| a.title("Revenue").format_number("$,.2f"))
        .legend_fill(|l| l.visible(false))
        .mark(Symbol::new().x(col("x")).y(col("y")).size(80.0).fill("#2ca25f"));

    assert_visual_match_default(plot, "layout", "format_axis_y_currency_fixed").await;
}

#[tokio::test]
async fn axis_y_percent() {
    let df = make_df_xy(&[1.0, 2.0, 3.0, 4.0, 5.0], &[0.1, 0.25, 0.5, 0.75, 0.95]);

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|s| s.domain((0.0, 6.0)))
        .scale_y(|s| s.domain((0.0, 1.0)))
        .axis_y(|a| a.title("Completion").format_number(".0%"))
        .legend_fill(|l| l.visible(false))
        .mark(Symbol::new().x(col("x")).y(col("y")).size(80.0).fill("#3182bd"));

    assert_visual_match_default(plot, "layout", "format_axis_y_percent").await;
}

#[tokio::test]
async fn axis_y_si_prefix() {
    let df = make_df_xy(
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        &[1.2e3, 4.5e4, 7.8e5, 2.3e6, 9.9e7],
    );

    let plot = Plot::new(Cartesian)
        .data(df)
        .scale_x(|s| s.domain((0.0, 6.0)))
        .scale_y(|s| s.domain((0.0, 1.0e8)))
        .axis_y(|a| a.title("Population").format_number(".2s"))
        .legend_fill(|l| l.visible(false))
        .mark(Symbol::new().x(col("x")).y(col("y")).size(80.0).fill("#e6550d"));

    assert_visual_match_default(plot, "layout", "format_axis_y_si_prefix").await;
}


