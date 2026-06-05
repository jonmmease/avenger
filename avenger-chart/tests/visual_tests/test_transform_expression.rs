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

fn expression_transform_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("source_a", DataType::Float64, false),
        Field::new("source_b", DataType::Float64, false),
        Field::new("actual", DataType::Float64, false),
        Field::new("predicted", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "Alpha", "Alpha", "Beta", "Beta", "Gamma", "Gamma", "Delta", "Delta",
            ])),
            Arc::new(Float64Array::from(vec![
                -4.0, -2.2, -0.6, 0.8, 1.9, 3.2, 4.8, 6.1,
            ])),
            Arc::new(Float64Array::from(vec![
                2.3, 1.6, 2.8, 3.4, 3.0, 4.2, 4.8, 5.3,
            ])),
            Arc::new(Float64Array::from(vec![
                2.0, 3.4, 4.1, 5.8, 5.2, 6.9, 7.4, 8.1,
            ])),
            Arc::new(Float64Array::from(vec![
                2.5, 3.0, 4.6, 4.8, 5.7, 6.1, 6.9, 7.2,
            ])),
        ],
    )
    .expect("expression transform batch");
    ctx.read_batch(batch)
        .expect("expression transform dataframe")
}

#[tokio::test]
async fn calculate_residual_scatter() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Calculate: residual")
        .canvas_size(560.0, 380.0)
        .data(expression_transform_data(&ctx))
        .mark(
            Symbol::new().transform_no_output(
                Calculate::new()
                    .expr("residual", col("actual") - col("predicted"))
                    .expr("fit_ratio", col("actual") / col("predicted")),
                |mark| {
                    mark.x_with(col("predicted"), |c| c.axis(|a| a.title("Predicted")))
                        .y_with(col("actual"), |c| c.axis(|a| a.title("Actual")))
                        .fill_with(col("residual"), |c| c.legend(|l| l.title("Residual")))
                        .size(col("fit_ratio") * lit(70.0))
                        .stroke("#111111")
                        .stroke_width(1.0)
                },
            ),
        );

    let compiled = plot.compile(&ctx).await.expect("compile calculate plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_expression",
        "calculate_residual_scatter",
    )
    .await;
}

#[tokio::test]
async fn filter_threshold_scatter() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Filter: source_a >= 0")
        .canvas_size(560.0, 380.0)
        .data(expression_transform_data(&ctx))
        .mark(
            Rule::new()
                .unit_data()
                .x(lit(0.0))
                .y(lit(1.0))
                .y2(lit(9.0))
                .stroke("#666666")
                .stroke_dash("dashed")
                .stroke_width(1.5),
        )
        .mark(Symbol::new().transform_no_output(
            Filter::new(col("source_a").gt_eq(lit(0.0))),
            |mark| {
                mark.x_with(col("source_a"), |c| {
                    c.scale_with::<Linear>(|s| s.domain_interval(lit(0.0), lit(6.5)))
                        .axis(|a| a.title("source_a"))
                })
                .y_with(col("actual"), |c| c.axis(|a| a.title("Actual")))
                .fill("#2f80ed")
                .size(110.0)
                .stroke("#ffffff")
                .stroke_width(1.0)
            },
        ));

    let compiled = plot.compile(&ctx).await.expect("compile filter plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_expression",
        "filter_threshold_scatter",
    )
    .await;
}

#[tokio::test]
async fn select_projected_scatter() {
    let ctx = SessionContext::new();
    let plot = Plot::<Cartesian>::new()
        .title("Select: projected columns")
        .canvas_size(560.0, 380.0)
        .data(expression_transform_data(&ctx))
        .mark(
            Symbol::new().transform_no_output(
                Select::new()
                    .expr(col("category"))
                    .expr(col("source_a").alias("a"))
                    .expr((col("actual") - col("predicted")).alias("residual")),
                |mark| {
                    mark.x_with(col("a"), |c| c.axis(|a| a.title("Projected a")))
                        .y_with(col("residual"), |c| c.axis(|a| a.title("Residual")))
                        .fill_with(col("category"), |c| c.legend(|l| l.title("Category")))
                        .size(120.0)
                        .stroke("#ffffff")
                        .stroke_width(1.0)
                },
            ),
        );

    let compiled = plot.compile(&ctx).await.expect("compile select plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_expression",
        "select_projected_scatter",
    )
    .await;
}
