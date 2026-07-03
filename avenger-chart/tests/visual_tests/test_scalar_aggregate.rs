use super::helpers::assert_visual_match_default;
use avenger_chart::cartesian::Cartesian;
use avenger_chart::prelude::*;

use datafusion::arrow::array::Float64Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

fn create_scatter_data(ctx: &SessionContext) -> DataFrame {
    let x = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
    let y = Float64Array::from(vec![2.5, 4.0, 3.0, 6.5, 5.0, 8.0, 6.0, 9.5]);
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    let batch =
        RecordBatch::try_new(schema, vec![Arc::new(x), Arc::new(y)]).expect("record batch");
    ctx.read_batch(batch).expect("read batch")
}

/// Scatter plot with a horizontal mean rule and scalar-normalized symbol
/// sizes, both driven by eager `ScalarAggregate` derived scalars.
#[tokio::test]
async fn test_scalar_aggregate_mean_rule_over_scatter() {
    let ctx = SessionContext::new();
    let df = create_scatter_data(&ctx);

    let plot = Plot::<Cartesian>::new()
        .title("Mean rule from ScalarAggregate")
        .data(df)
        .mark(
            Symbol::new()
                .transform(ScalarAggregate::new().max("max_y", col("y")), |mark, s| {
                    mark.size_with(col("y") / s.scalar("max_y") * lit(160.0), |c| {
                        c.no_scale()
                    })
                })
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 9.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .fill("#4682b4"),
        )
        .mark(
            Rule::new().transform(ScalarAggregate::new().mean("mean_y", col("y")), |mark, s| {
                mark.x(lit(0.0))
                    .x2(lit(9.0))
                    .y(s.scalar("mean_y"))
                    .y2(s.scalar("mean_y"))
                    .stroke("#d62728")
                    .stroke_width(2.0)
            }),
        );

    let compiled = plot.compile(&ctx).await.expect("compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "scalar_aggregate",
        "mean_rule_over_scatter",
    )
    .await;
}
