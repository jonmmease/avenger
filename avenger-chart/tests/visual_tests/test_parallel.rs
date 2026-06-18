use std::sync::Arc;

use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::{
    arrow::{
        array::{ArrayRef, Float64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    prelude::{SessionContext, col},
};

fn numeric_parallel_data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("group", DataType::Utf8, false),
        Field::new("speed", DataType::Float64, false),
        Field::new("efficiency", DataType::Float64, false),
        Field::new("stability", DataType::Float64, false),
        Field::new("cost", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "a0", "a1", "a2", "a3", "b0", "b1", "b2", "b3",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "alpha", "alpha", "alpha", "alpha", "beta", "beta", "beta", "beta",
            ])),
            Arc::new(Float64Array::from(vec![
                48.0, 53.0, 59.0, 64.0, 42.0, 47.0, 52.0, 57.0,
            ])),
            Arc::new(Float64Array::from(vec![
                0.62, 0.68, 0.71, 0.76, 0.57, 0.61, 0.66, 0.70,
            ])),
            Arc::new(Float64Array::from(vec![
                69.0, 73.0, 78.0, 81.0, 75.0, 77.0, 80.0, 83.0,
            ])),
            Arc::new(Float64Array::from(vec![
                118.0, 126.0, 134.0, 145.0, 99.0, 108.0, 116.0, 124.0,
            ])),
        ],
    )
    .expect("numeric parallel test data");
    ctx.read_batch(batch).expect("numeric parallel dataframe")
}

fn mixed_parallel_data(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("tier", DataType::Utf8, false),
        Field::new("latency", DataType::Float64, false),
        Field::new("quality", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "bronze", "silver", "gold", "bronze", "silver", "gold", "bronze", "silver", "gold",
            ])),
            Arc::new(Float64Array::from(vec![
                94.0, 82.0, 71.0, 88.0, 74.0, 62.0, 101.0, 86.0, 68.0,
            ])),
            Arc::new(Float64Array::from(vec![
                0.48, 0.61, 0.78, 0.55, 0.66, 0.84, 0.44, 0.58, 0.80,
            ])),
        ],
    )
    .expect("mixed parallel test data");
    ctx.read_batch(batch).expect("mixed parallel dataframe")
}

#[tokio::test]
async fn parallel_points_overlay() {
    let ctx = SessionContext::new();
    let coord = Parallel::new()
        .dimension_with("speed", col("speed"), |d| d.axis(|a| a.title("Speed")))
        .dimension_with("efficiency", col("efficiency"), |d| {
            d.axis(|a| a.title("Efficiency"))
        })
        .dimension_with("stability", col("stability"), |d| {
            d.axis(|a| a.title("Stability"))
        })
        .dimension_with("cost", col("cost"), |d| d.axis(|a| a.title("Cost")));

    let plot = Plot::with_coord(coord)
        .canvas_size(640.0, 360.0)
        .plot_size(500.0, 210.0)
        .data(numeric_parallel_data(&ctx))
        .mark(
            ParallelLine::new()
                .stroke("#94a3b8")
                .stroke_width(1.3)
                .opacity(0.45),
        )
        .mark(
            ParallelSymbol::new()
                .fill_with(col("group"), |fill| fill.no_legend())
                .stroke("#111827")
                .stroke_width(0.8)
                .size(82.0)
                .opacity(0.92),
        );

    let compiled = plot.compile(&ctx).await.expect("compile parallel points");
    assert_visual_match_default(&compiled, &ctx, None, "parallel", "parallel_points_overlay").await;
}

#[tokio::test]
async fn parallel_points_categorical_axis() {
    let ctx = SessionContext::new();
    let coord = Parallel::new()
        .dimension_with("latency", col("latency"), |d| {
            d.axis(|a| a.title("Latency"))
        })
        .dimension_with("tier", col("tier"), |d| {
            d.scale_with::<Point>(|s| s).axis(|a| a.title("Tier"))
        })
        .dimension_with("quality", col("quality"), |d| {
            d.axis(|a| a.title("Quality"))
        });

    let plot = Plot::with_coord(coord)
        .canvas_size(560.0, 340.0)
        .plot_size(430.0, 205.0)
        .data(mixed_parallel_data(&ctx))
        .mark(
            ParallelLine::new()
                .stroke_with(col("tier"), |stroke| stroke.no_legend())
                .stroke_width(1.5)
                .opacity(0.38),
        )
        .mark(
            ParallelSymbol::new()
                .fill_with(col("tier"), |fill| fill.no_legend())
                .stroke("#1f2937")
                .stroke_width(0.75)
                .size(74.0),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile parallel categorical points");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "parallel",
        "parallel_points_categorical_axis",
    )
    .await;
}
