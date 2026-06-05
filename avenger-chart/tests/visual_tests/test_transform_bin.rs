use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::{
    array::{Float64Array, StringArray},
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

fn faceted_histogram_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("group", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "Low", "Low", "Low", "Low", "Low", "Low", "High", "High", "High", "High", "High",
                "High",
            ])),
            Arc::new(Float64Array::from(vec![
                0.2, 0.8, 1.6, 2.4, 3.2, 4.6, 90.0, 93.0, 96.0, 100.0, 103.0, 108.0,
            ])),
        ],
    )
    .expect("faceted histogram batch");
    ctx.read_batch(batch).expect("faceted histogram dataframe")
}

fn faceted_histogram_plot(
    df: DataFrame,
    transform_scope: Sharing,
    title: &str,
) -> Plot<FacetColumn> {
    let leaf = Plot::<Cartesian>::new().mark(Rect::new().transform_with_scope(
        transform_scope,
        Bin::new(col("value")).maxbins(4),
        |mark, bin| {
            mark.x(bin.start())
                .x2(bin.end())
                .y(lit(0.0))
                .y2(count(lit(1)))
                .fill("#4169e1")
                .stroke("#ffffff")
                .stroke_width(1.0)
        },
    ));

    Plot::<FacetColumn>::new()
        .data(df)
        .title(title)
        .canvas_size(820.0, 360.0)
        .mark(Subplot::new(leaf).col_with(col("group"), |c| c.guide(|g| g.title("Facet group"))))
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

#[tokio::test]
async fn faceted_histogram_free_bin_edges() {
    let ctx = SessionContext::new();
    let plot = faceted_histogram_plot(
        faceted_histogram_data(&ctx),
        Sharing::Free,
        "Free bin transform: each facet owns its bin edges",
    );

    let compiled = plot.compile(&ctx).await.expect("compile histogram");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_bin",
        "faceted_histogram_free_bin_edges",
    )
    .await;
}

#[tokio::test]
async fn faceted_histogram_shared_bin_edges() {
    let ctx = SessionContext::new();
    let plot = faceted_histogram_plot(
        faceted_histogram_data(&ctx),
        Sharing::Shared,
        "Shared bin transform: one binning table feeds every facet",
    );

    let compiled = plot.compile(&ctx).await.expect("compile histogram");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "transform_bin",
        "faceted_histogram_shared_bin_edges",
    )
    .await;
}
