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

fn sql_stacked_rect_mark() -> Rect<Cartesian> {
    Rect::new().transform(
        Sql::new(
            r#"
            WITH agg AS (
                SELECT category, series, sum(value) AS total_value
                FROM input
                GROUP BY category, series
            ),
            stacked AS (
                SELECT
                    category,
                    series,
                    total_value,
                    sum(total_value) OVER (
                        PARTITION BY category
                        ORDER BY series
                        ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
                    ) AS cumulative_value,
                    sum(total_value) OVER (
                        PARTITION BY category
                        ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING
                    ) AS category_total
                FROM agg
            )
            SELECT
                category,
                series,
                total_value,
                category_total - cumulative_value AS stack_start,
                category_total - cumulative_value + total_value AS stack_end
            FROM stacked
            "#,
        ),
        |mark, sql| {
            mark.x_with(sql.field("category"), |c| {
                c.scale_with::<Band>(|s| s)
                    .axis(|a| a.title("Category").grid(false))
            })
            .x2_with(col(":x"), |c| c.band(1.0))
            .y_with(sql.field("stack_start"), |c| c.axis(|a| a.title("Value")))
            .y2(sql.field("stack_end"))
            .fill_with(sql.field("series"), |c| c.legend(|l| l.title("Series")))
            .stroke("#ffffff")
            .stroke_width(1.0)
        },
    )
}

#[tokio::test]
async fn sql_stacked_rect() {
    let ctx = SessionContext::new();
    let plot = Chart::<Cartesian>::new()
        .title("SQL Stacked Sales")
        .data(stacked_sales_data(&ctx))
        .mark(sql_stacked_rect_mark());

    let compiled = plot.compile(&ctx).await.expect("compile SQL stacked bar");
    assert_visual_match_default(&compiled, &ctx, None, "transform_sql", "sql_stacked_rect").await;
}
