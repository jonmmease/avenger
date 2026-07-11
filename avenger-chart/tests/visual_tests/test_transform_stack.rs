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

fn stacked_sales_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("series", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "A", "A", "A", "B", "B", "B", "C", "C", "C", "D", "D", "D",
            ])),
            Arc::new(StringArray::from(vec![
                "North", "South", "West", "North", "South", "West", "North", "South", "West",
                "North", "South", "West",
            ])),
            Arc::new(Float64Array::from(vec![
                18.0, 26.0, 12.0, 34.0, 16.0, 22.0, 14.0, 30.0, 42.0, 28.0, 20.0, 18.0,
            ])),
        ],
    )
    .expect("stacked sales batch");
    ctx.read_batch(batch).expect("stacked sales dataframe")
}

fn stacked_rect_mark(offset: StackOffset) -> Rect<Cartesian> {
    Rect::new().transform(
        Aggregate::new()
            .group_by([col("category"), col("series")])
            .sum("total_value", col("value")),
        move |mark, aggregate| {
            mark.transform(
                Stack::new(aggregate.output("total_value"))
                    .group_by([col("category")])
                    .sort_by_exprs([col("series")])
                    .offset(offset)
                    .name("total_value_stack"),
                |mark, stack| {
                    mark.x_with(col("category"), |c| {
                        c.scale_with::<Band>(|s| s)
                            .axis(|a| a.title("Category").grid(false))
                    })
                    .x2_with(col(":x"), |c| c.band(1.0))
                    .y_with(stack.start(), |c| c.axis(|a| a.title("Value")))
                    .y2(stack.end())
                    .fill_with(col("series"), |c| c.legend(|l| l.title("Series")))
                    .stroke("#ffffff")
                    .stroke_width(1.0)
                },
            )
        },
    )
}

#[tokio::test]
async fn stacked_bar_zero_offset_transform() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Stacked Sales")
        .data(stacked_sales_data(&ctx))
        .mark(stacked_rect_mark(StackOffset::Zero));

    let compiled = plot.compile(&ctx).await.expect("compile stacked bar");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_stack",
        "stacked_bar_zero_offset_transform",
    )
    .await;
}

#[tokio::test]
async fn stacked_bar_normalized_transform() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Normalized Stacked Sales")
        .data(stacked_sales_data(&ctx))
        .mark(stacked_rect_mark(StackOffset::Normalize));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile normalized stacked bar");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_stack",
        "stacked_bar_normalized_transform",
    )
    .await;
}

#[tokio::test]
async fn stacked_bar_center_offset_transform() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("Centered Stacked Sales")
        .data(stacked_sales_data(&ctx))
        .mark(stacked_rect_mark(StackOffset::Center));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile centered stacked bar");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_stack",
        "stacked_bar_center_offset_transform",
    )
    .await;
}
