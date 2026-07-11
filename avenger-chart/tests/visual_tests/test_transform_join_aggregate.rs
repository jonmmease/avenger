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

fn revenue_mix_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("department", DataType::Utf8, false),
        Field::new("segment", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "Hardware", "Hardware", "Hardware", "Services", "Services", "Services",
            ])) as _,
            Arc::new(StringArray::from(vec![
                "Core",
                "Accessories",
                "Licensing",
                "Cloud",
                "Support",
                "Consulting",
            ])) as _,
            Arc::new(Float64Array::from(vec![45.0, 15.0, 30.0, 20.0, 50.0, 30.0])) as _,
        ],
    )
    .expect("revenue mix batch");
    ctx.read_batch(batch).expect("revenue mix dataframe")
}

#[tokio::test]
async fn percent_of_group_bar() {
    let ctx = SessionContext::new();
    let leaf = Plot::<Cartesian>::new().mark(
        Rect::new().transform_shared_no_output(
            JoinAggregate::new()
                .group_by([col("department")])
                .sum("department_total", col("value")),
            |mark| {
                mark.x_with(col("segment"), |c| {
                    c.scale_with::<Band>(|s| {
                        s.domain_discrete(vec![
                            lit("Accessories"),
                            lit("Cloud"),
                            lit("Consulting"),
                            lit("Core"),
                            lit("Licensing"),
                            lit("Support"),
                        ])
                    })
                    .axis(|a| a.title("Segment").grid(false))
                })
                .x2_with(col(":x"), |c| c.band(1.0))
                .y_with(lit(0.0), |c| {
                    c.scale_with::<Linear>(|s| s.domain_interval(lit(0.0), lit(0.55)))
                        .axis(|a| a.title("Share of department"))
                })
                .y2(col("value") / col("department_total"))
                .fill_with(col("segment"), |c| c.legend(|l| l.title("Segment")))
                .stroke("#ffffff")
                .stroke_width(1.0)
            },
        ),
    );
    let plot = Chart::<FacetColumn>::new()
        .title("JoinAggregate: percent of department")
        .canvas_size(760.0, 420.0)
        .data(revenue_mix_data(&ctx))
        .mark(
            Subplot::new(leaf)
                .column_with(col("department"), |c| c.guide(|g| g.title("Department"))),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile joinaggregate plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_join_aggregate",
        "percent_of_group_bar",
    )
    .await;
}
