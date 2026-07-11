use super::helpers::assert_visual_match;
use avenger_chart::prelude::*;
use datafusion::arrow::{
    array::{Float64Array, Int64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use std::sync::Arc;

fn missing_months_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("series", DataType::Utf8, false),
        Field::new("month", DataType::Int64, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "Alpha", "Alpha", "Alpha", "Beta", "Beta", "Beta",
            ])) as _,
            Arc::new(Int64Array::from(vec![1, 2, 4, 1, 3, 5])) as _,
            Arc::new(Float64Array::from(vec![3.0, 5.0, 2.0, 1.0, 4.0, 3.0])) as _,
        ],
    )
    .expect("missing months batch");
    ctx.read_batch(batch).expect("missing months dataframe")
}

#[tokio::test]
async fn line_missing_months() {
    let ctx = SessionContext::new();
    let impute = Impute::new(col("value"))
        .key(col("month"))
        .group_by([col("series")])
        .value(lit(0.0));
    let plot = Chart::<Cartesian>::new()
        .title("Impute: missing months to zero")
        .canvas_size(640.0, 380.0)
        .data(missing_months_data(&ctx))
        .mark(Line::new().transform(impute, |mark, imputed| {
            mark.x_with(col("month"), |c| c.axis(|a| a.title("Month")))
                .y_with(imputed.value(), |c| {
                    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(6.0))).nice(false))
                        .axis(|a| a.title("Value"))
                })
                .stroke_with(col("series"), |c| c.legend(|l| l.title("Series")))
                .order(col("month"))
                .stroke_width(3.0)
        }));

    let compiled = plot.compile(&ctx).await.expect("compile impute plot");
    assert_visual_match(
        &compiled,
        &ctx,
        None,
        "transform_impute",
        "line_missing_months",
        0.9998,
    )
    .await;
}
