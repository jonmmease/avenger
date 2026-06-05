use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::{
    array::{Float64Array, Int64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::dataframe::DataFrame;
use datafusion::functions_aggregate::sum::sum_udaf;
use datafusion::logical_expr::{Expr, WindowFunctionDefinition, col, expr::WindowFunction};
use datafusion::prelude::SessionContext;
use std::sync::Arc;

fn running_total_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("series", DataType::Utf8, false),
        Field::new("day", DataType::Int64, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "Alpha", "Alpha", "Alpha", "Alpha", "Beta", "Beta", "Beta", "Beta",
            ])) as _,
            Arc::new(Int64Array::from(vec![1, 2, 3, 4, 1, 2, 3, 4])) as _,
            Arc::new(Float64Array::from(vec![
                3.0, 1.0, 4.0, 2.0, 1.5, 3.5, 2.0, 4.5,
            ])) as _,
        ],
    )
    .expect("running total batch");
    ctx.read_batch(batch).expect("running total dataframe")
}

fn running_sum_window_expr() -> Expr {
    Expr::from(WindowFunction::new(
        WindowFunctionDefinition::AggregateUDF(sum_udaf()),
        vec![col("value")],
    ))
}

#[tokio::test]
async fn running_total_line() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Window: running total")
        .canvas_size(620.0, 380.0)
        .data(running_total_data(&ctx))
        .mark(
            Line::new().transform_no_output(
                Window::new()
                    .partition_by([col("series")])
                    .order_by([col("day").sort(true, false)])
                    .expr("running_total", running_sum_window_expr()),
                |mark| {
                    mark.x_with(col("day"), |c| c.axis(|a| a.title("Day")))
                        .y_with(col("running_total"), |c| {
                            c.axis(|a| a.title("Running total"))
                        })
                        .stroke_with(col("series"), |c| c.legend(|l| l.title("Series")))
                        .order(col("day"))
                        .stroke_width(3.0)
                },
            ),
        );

    let compiled = plot.compile(&ctx).await.expect("compile window plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_window",
        "running_total_line",
    )
    .await;
}
