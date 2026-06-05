use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::{
    array::TimestampMillisecondArray,
    datatypes::{DataType, Field, Schema, TimeUnit as ArrowTimeUnit},
    record_batch::RecordBatch,
};
use datafusion::dataframe::DataFrame;
use datafusion::functions_aggregate::expr_fn::count;
use datafusion::prelude::SessionContext;
use std::sync::Arc;

fn monthly_event_data(ctx: &SessionContext) -> DataFrame {
    const DAY_MS: i64 = 86_400_000;
    let schema = Arc::new(Schema::new(vec![Field::new(
        "timestamp",
        DataType::Timestamp(ArrowTimeUnit::Millisecond, None),
        false,
    )]));
    let values = vec![
        2 * DAY_MS,
        5 * DAY_MS,
        12 * DAY_MS,
        35 * DAY_MS,
        40 * DAY_MS,
        63 * DAY_MS,
        65 * DAY_MS,
        68 * DAY_MS,
        72 * DAY_MS,
    ];
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(TimestampMillisecondArray::from(values)) as _],
    )
    .expect("monthly event batch");
    ctx.read_batch(batch).expect("monthly event dataframe")
}

#[tokio::test]
async fn monthly_rects() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("TimeUnit: monthly counts")
        .canvas_size(640.0, 380.0)
        .data(monthly_event_data(&ctx))
        .mark(
            Rect::new().transform(
                TimeUnit::new(col("timestamp"))
                    .unit(TimeUnitPart::Month)
                    .name("month"),
                |mark, time| {
                    mark.x_with(time.start(), |c| c.axis(|a| a.title("Month").tick_count(3)))
                        .x2(time.end())
                        .y(lit(0.0))
                        .y2_with(count(lit(1)), |c| c.axis(|a| a.title("Events")))
                        .fill("#3b82f6")
                        .stroke("#ffffff")
                        .stroke_width(1.0)
                },
            ),
        );

    let compiled = plot.compile(&ctx).await.expect("compile timeunit plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_time_unit",
        "monthly_rects",
    )
    .await;
}
