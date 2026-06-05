use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::{
    array::{Float64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::dataframe::DataFrame;
use datafusion::prelude::SessionContext;
use std::sync::Arc;

fn medal_counts_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("country", DataType::Utf8, false),
        Field::new("gold", DataType::Float64, false),
        Field::new("silver", DataType::Float64, false),
        Field::new("bronze", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["Canada", "Japan", "Norway"])) as _,
            Arc::new(Float64Array::from(vec![4.0, 7.0, 9.0])) as _,
            Arc::new(Float64Array::from(vec![8.0, 5.0, 6.0])) as _,
            Arc::new(Float64Array::from(vec![3.0, 6.0, 4.0])) as _,
        ],
    )
    .expect("medal counts batch");
    ctx.read_batch(batch).expect("medal counts dataframe")
}

#[tokio::test]
async fn medal_counts_faceted_bar() {
    let ctx = SessionContext::new();
    let leaf = Plot::<Cartesian>::new().mark(
        Rect::new().transform(
            Fold::new()
                .field("Gold", col("gold"))
                .field("Silver", col("silver"))
                .field("Bronze", col("bronze"))
                .as_key("medal")
                .as_value("count"),
            |mark, fold| {
                mark.x_with(fold.key(), |c| {
                    c.scale_with::<Band>(|s| {
                        s.domain_discrete(vec![lit("Gold"), lit("Silver"), lit("Bronze")])
                    })
                    .axis(|a| a.title("Medal"))
                })
                .x2_with(col(":x"), |c| c.band(1.0))
                .y(lit(0.0))
                .y2_with(fold.value(), |c| c.axis(|a| a.title("Count")))
                .fill_with(fold.key(), |c| {
                    c.scale_with::<Ordinal>(|s| {
                        s.domain_discrete(vec![lit("Gold"), lit("Silver"), lit("Bronze")])
                    })
                    .legend(|l| l.title("Medal"))
                })
                .stroke("#ffffff")
                .stroke_width(1.0)
            },
        ),
    );

    let plot = Plot::<FacetColumn>::new()
        .title("Folded medal counts")
        .canvas_size(760.0, 380.0)
        .data(medal_counts_data(&ctx))
        .mark(Subplot::new(leaf).col_with(col("country"), |c| c.guide(|g| g.title("Country"))));

    let compiled = plot.compile(&ctx).await.expect("compile fold bar plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_fold",
        "medal_counts_faceted_bar",
    )
    .await;
}
