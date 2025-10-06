use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

fn make_df_xy_category(x: &[f64], y: &[f64], category: &[&str]) -> DataFrame {
    let x_values = Float64Array::from(x.to_vec());
    let y_values = Float64Array::from(y.to_vec());
    let category_values = arrow::array::StringArray::from(category.to_vec());
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("category", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(x_values),
            Arc::new(y_values),
            Arc::new(category_values),
        ],
    )
    .unwrap();
    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

fn make_df_xyc(x: &[f64], y: &[f64], c: &[f64]) -> DataFrame {
    let x_values = Float64Array::from(x.to_vec());
    let y_values = Float64Array::from(y.to_vec());
    let c_values = Float64Array::from(c.to_vec());
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("c", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(x_values), Arc::new(y_values), Arc::new(c_values)],
    )
    .unwrap();
    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

#[tokio::test]
async fn symbol_legend_with_background() {
    let ctx = SessionContext::new();

    // Create data with categories for discrete legend
    let df = make_df_xy_category(
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        &[2.0, 4.0, 6.0, 3.0, 5.0, 7.0],
        &["A", "B", "C", "A", "B", "C"],
    );

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 8.0))))
            .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 8.0))))
            .size(100.0)
            .fill_with(col("category"), |c| {
                c.scale_with::<Ordinal>(|s| s).legend(|l| {
                    l.title("Category")
                        .background_padding(6.0)
                        .background_corner_radius(6.0)
                        .background_fill("rgba(255,255,255,0.75)")
                        .background_stroke("rgba(0,0,0,0.25)")
                })
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "legend_symbol_background").await;
}

#[tokio::test]
async fn line_legend_with_background() {
    // Create data with multiple series for line legend
    let ctx = SessionContext::new();
    let df = ctx
        .read_batch(
            RecordBatch::try_new(
                Arc::new(Schema::new(vec![
                    Field::new("x", DataType::Float64, false),
                    Field::new("y", DataType::Float64, false),
                    Field::new("series", DataType::Utf8, false),
                ])),
                vec![
                    Arc::new(Float64Array::from(vec![
                        1.0, 2.0, 3.0, 4.0, 1.0, 2.0, 3.0, 4.0,
                    ])),
                    Arc::new(Float64Array::from(vec![
                        2.0, 4.0, 3.0, 5.0, 3.0, 2.0, 4.0, 3.0,
                    ])),
                    Arc::new(arrow::array::StringArray::from(vec![
                        "Series A", "Series A", "Series A", "Series A", "Series B", "Series B",
                        "Series B", "Series B",
                    ])),
                ],
            )
            .unwrap(),
        )
        .unwrap();

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Line::new()
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 5.0))))
            .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 6.0))))
            .stroke_with(col("series"), |c| {
                c.scale(|s| s).legend(|l| {
                    l.title("Series")
                        // .background_padding(6.0)
                        .background_corner_radius(6.0)
                        .background_fill("rgba(255,255,255,0.75)")
                        .background_stroke("rgba(0,0,0,0.25)")
                })
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "legend_line_background").await;
}

#[tokio::test]
async fn symbol_legend_without_visible_background() {
    let ctx = SessionContext::new();

    // Test that layout is consistent even without visible background
    let df = make_df_xy_category(
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        &[2.0, 4.0, 6.0, 3.0, 5.0, 7.0],
        &["A", "B", "C", "A", "B", "C"],
    );

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 8.0))))
            .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 8.0))))
            .size(100.0)
            .fill_with(col("category"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Category")) // No background styling
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "layout",
        "legend_symbol_no_background",
    )
    .await;
}

#[tokio::test]
async fn colorbar_legend_with_background() {
    let ctx = SessionContext::new();

    // Create data with continuous values for colorbar legend
    let df = make_df_xyc(
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        &[2.0, 4.0, 3.0, 5.0, 6.0, 4.0],
        &[0.1, 0.3, 0.5, 0.7, 0.9, 0.2],
    );

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 7.0))))
            .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 7.0))))
            .size(120.0)
            .fill_with(col("c"), |c| {
                c.scale(|s| s.domain((0.0, 1.0))).legend(
                    |l| {
                        l.title("Intensity")
                            .background_corner_radius(6.0)
                            .background_fill("rgba(200,200,200,0.75)")
                    }, // .background_stroke("rgba(0,0,0,0.25)")
                )
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "layout",
        "legend_colorbar_background",
    )
    .await;
}
