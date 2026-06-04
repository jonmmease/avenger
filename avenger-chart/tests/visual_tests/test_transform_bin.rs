use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::{
    array::Float64Array,
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::dataframe::DataFrame;
use datafusion::functions_aggregate::expr_fn::count;
use datafusion::prelude::SessionContext;
use std::sync::Arc;

fn messy_histogram_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "value",
        DataType::Float64,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(Float64Array::from(vec![
            0.37, 0.64, 0.91, 1.12, 1.45, 1.82, 2.03, 2.36, 2.91, 3.14, 3.77, 4.08, 4.26, 4.52,
            4.83, 5.19, 5.44, 5.78, 6.02, 6.31, 6.88, 7.04, 7.33, 7.61, 7.95, 8.22, 8.57, 8.91,
            9.18, 9.91,
        ]))],
    )
    .expect("histogram batch");
    ctx.read_batch(batch).expect("histogram dataframe")
}

#[tokio::test]
async fn histogram_exact_maxbins_messy_edges() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Exact maxbins histogram")
        .subtitle("Bin edges are raw min/max divided into seven bins")
        .canvas_size(640.0, 420.0)
        .data(messy_histogram_data(&ctx))
        .mark(
            Rect::new().transform(Bin::new(col("value")).maxbins(7), |mark, bin| {
                mark.x(bin.start())
                    .x2(bin.end())
                    .y(lit(0.0))
                    .y2(count(col("value")))
                    .fill("#2f80ed")
                    .stroke("#ffffff")
                    .stroke_width(1.0)
            }),
        );

    let compiled = plot.compile(&ctx).await.expect("compile histogram");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_bin",
        "histogram_exact_maxbins_messy_edges",
    )
    .await;
}
