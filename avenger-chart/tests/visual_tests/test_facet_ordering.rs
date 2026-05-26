use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::{
    arrow::{
        array::{Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    functions_aggregate::expr_fn::sum,
    prelude::*,
};
use std::sync::Arc;

const BASELINE_CATEGORY: &str = "facet_ordering";

fn single_facet_ordering_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("score", DataType::Float64, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "Large", "Large", "Medium", "Medium", "Tiny", "Tiny",
            ])),
            Arc::new(Float64Array::from(vec![1.0, 3.0, 4.0, 5.0, 7.0, 8.0])),
            Arc::new(Float64Array::from(vec![0.2, 0.8, 0.2, 0.8, 0.2, 0.8])),
            Arc::new(Float64Array::from(vec![0.25, 0.75, 0.30, 0.70, 0.35, 0.65])),
        ],
    )
    .expect("facet ordering batch");
    ctx.read_batch(batch).expect("facet ordering dataframe")
}

fn nested_facet_ordering_data(ctx: &SessionContext) -> DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, false),
        Field::new("species", DataType::Utf8, false),
        Field::new("score", DataType::Float64, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "East", "East", "East", "East", "West", "West", "West", "West",
            ])),
            Arc::new(StringArray::from(vec![
                "Setosa",
                "Setosa",
                "Versicolor",
                "Versicolor",
                "Versicolor",
                "Versicolor",
                "Virginica",
                "Virginica",
            ])),
            Arc::new(Float64Array::from(vec![
                1.0, 1.0, 3.0, 4.0, 4.0, 5.0, 7.0, 8.0,
            ])),
            Arc::new(Float64Array::from(vec![
                0.25, 0.75, 0.25, 0.75, 0.25, 0.75, 0.25, 0.75,
            ])),
            Arc::new(Float64Array::from(vec![
                0.30, 0.70, 0.35, 0.65, 0.35, 0.65, 0.40, 0.60,
            ])),
        ],
    )
    .expect("nested facet ordering batch");
    ctx.read_batch(batch)
        .expect("nested facet ordering dataframe")
}

fn leaf_plot() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .size(70.0)
            .fill("#2f80ed"),
    )
}

#[tokio::test]
async fn facet_row_order_by_sum_desc() {
    let ctx = SessionContext::new();
    let df = single_facet_ordering_data(&ctx);

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .canvas_size(520.0, 620.0)
        .mark(Subplot::new(leaf_plot()).row_with(col("category"), |c| {
            c.order_by(sum(col("score")))
                .order_desc()
                .facet(|f| f.title("Aggregate desc"))
        }));

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        BASELINE_CATEGORY,
        "facet_row_order_by_sum_desc",
    )
    .await;
}

#[tokio::test]
async fn facet_col_order_by_sum_asc() {
    let ctx = SessionContext::new();
    let df = single_facet_ordering_data(&ctx);

    let plot = Plot::<FacetColumn>::new()
        .data(df)
        .canvas_size(760.0, 300.0)
        .mark(Subplot::new(leaf_plot()).col_with(col("category"), |c| {
            c.order_by(sum(col("score")))
                .order_asc()
                .facet(|f| f.title("Aggregate asc"))
        }));

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        BASELINE_CATEGORY,
        "facet_col_order_by_sum_asc",
    )
    .await;
}

async fn run_nested_ordering_baseline(name: &str, sharing: Option<ScaleSharing>) {
    let ctx = SessionContext::new();
    let df = nested_facet_ordering_data(&ctx);

    let plot = Plot::<FacetRow>::new()
        .data(df)
        .canvas_size(920.0, 500.0)
        .mark(
            Subplot::new(
                Plot::<FacetColumn>::new().mark(Subplot::new(leaf_plot()).col_with(
                    col("species"),
                    |c| {
                        let c = c.order_by(sum(col("score"))).order_desc();
                        match sharing {
                            Some(mode) => c.facet(|f| {
                                f.title("Species")
                                    .with_slot_sharing(mode)
                                    .empty_cells_as_subplots()
                            }),
                            None => c.facet(|f| f.title("Species")),
                        }
                    },
                )),
            )
            .row_with(col("region"), |c| c.facet(|f| f.title("Region"))),
        );

    let compiled = plot.compile(&ctx).await.expect("compile");
    assert_visual_match_default(&compiled, &ctx, None, BASELINE_CATEGORY, name).await;
}

#[tokio::test]
async fn nested_facet_free_order_by_sum_desc() {
    run_nested_ordering_baseline("nested_facet_free_order_by_sum_desc", None).await;
}

#[tokio::test]
async fn nested_facet_shared_order_by_sum_desc() {
    run_nested_ordering_baseline(
        "nested_facet_shared_order_by_sum_desc",
        Some(ScaleSharing::Shared),
    )
    .await;
}
