use super::helpers::assert_visual_match_default;
use crate::test_data;
use avenger_chart::cartesian::Cartesian;
use avenger_chart::prelude::*;

use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::functions_aggregate::expr_fn::{count, sum};
use datafusion::functions_aggregate::average::avg;
use datafusion::prelude::*;
use std::sync::Arc;

/// Create a dataset for testing aggregation
/// This creates multiple rows per category to test aggregation
fn create_sales_data() -> DataFrame {
    // Create data with multiple rows per category
    let categories = StringArray::from(vec![
        "A", "A", "A", "B", "B", "B", "C", "C", "C", "D", "D", "D",
    ]);
    let sales = Float64Array::from(vec![
        100.0, 150.0, 200.0, // A: total=450
        80.0, 120.0, 160.0, // B: total=360
        200.0, 250.0, 300.0, // C: total=750
        50.0, 100.0, 150.0, // D: total=300
    ]);
    let profit = Float64Array::from(vec![
        20.0, 30.0, 40.0, // A: avg=30
        15.0, 25.0, 35.0, // B: avg=25
        40.0, 50.0, 60.0, // C: avg=50
        10.0, 20.0, 30.0, // D: avg=20
    ]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("sales", DataType::Float64, false),
        Field::new("profit", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(categories),
            Arc::new(sales),
            Arc::new(profit),
        ],
    )
    .expect("Failed to create RecordBatch");

    let ctx = SessionContext::new();
    ctx.read_batch(batch)
        .expect("Failed to read batch into DataFrame")
}

#[tokio::test]
async fn test_aggregate_sum_by_category() {
    let ctx = SessionContext::new();
    let df = create_sales_data();

    // Bar chart with aggregation: sum of sales by category
    // This should automatically group by category and sum sales
    let plot = Plot::<Cartesian>::new()
        .title("Total Sales by Category")
        .subtitle("Automatically aggregated using sum()")
        .data(df)
        .mark(
            Rect::new()
                .x(col("category")) // grouping dimension
                .x2_with(col("category"), |c| c.band(1.0)) // end of band
                .y(lit(0.0)) // baseline at 0
                .y2(sum(col("sales"))) // aggregate dimension
                .fill("#3498db"),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "aggregate", "sum_by_category").await;
}

#[tokio::test]
async fn test_aggregate_mean_by_category() {
    let ctx = SessionContext::new();
    let df = create_sales_data();

    // Bar chart with avg aggregation
    let plot = Plot::<Cartesian>::new()
        .title("Average Profit by Category")
        .subtitle("Automatically aggregated using avg()")
        .data(df)
        .mark(
            Rect::new()
                .x(col("category")) // grouping dimension
                .x2_with(col("category"), |c| c.band(1.0)) // end of band
                .y(lit(0.0)) // baseline at 0
                .y2(avg(col("profit"))) // aggregate dimension
                .fill("#e74c3c"),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "aggregate", "mean_by_category").await;
}

#[tokio::test]
async fn test_aggregate_multiple_aggregates() {
    let ctx = SessionContext::new();
    let df = create_sales_data();

    // Bar chart with multiple aggregated encodings
    // Both y and fill use aggregates, x is the grouping dimension
    let plot = Plot::<Cartesian>::new()
        .title("Sales with Profit-based Color")
        .subtitle("Multiple aggregate encodings (sum + avg)")
        .data(df)
        .mark(
            Rect::new()
                .x(col("category")) // grouping dimension
                .x2_with(col("category"), |c| c.band(1.0)) // end of band
                .y(lit(0.0)) // baseline at 0
                .y2(sum(col("sales"))) // aggregate dimension
                .fill_with(avg(col("profit")), |c| {
                    // Color by avg profit
                    c.legend(|l| l.title("Avg Profit"))
                }),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "aggregate",
        "multiple_aggregates",
    )
    .await;
}

#[tokio::test]
async fn test_aggregate_count() {
    let ctx = SessionContext::new();
    let df = create_sales_data();

    // Bar chart counting rows per category
    let plot = Plot::<Cartesian>::new()
        .title("Count of Records by Category")
        .subtitle("Using count() aggregation")
        .data(df)
        .mark(
            Rect::new()
                .x(col("category")) // grouping dimension
                .x2_with(col("category"), |c| c.band(1.0)) // end of band
                .y(lit(0.0)) // baseline at 0
                .y2(count(col("sales"))) // count aggregate
                .fill("#2ecc71"),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "aggregate", "count_by_category").await;
}

#[tokio::test]
async fn test_aggregate_no_grouping() {
    let ctx = SessionContext::new();
    let df = create_sales_data();

    // Single bar showing total of all sales (no grouping dimension)
    // This tests the empty group_by case
    // Add a constant column for the x position
    let df = df.with_column("label", lit("Total")).unwrap();

    let plot = Plot::<Cartesian>::new()
        .title("Total Sales (All Categories)")
        .subtitle("Full table aggregation with no GROUP BY")
        .data(df)
        .mark(
            Rect::new()
                .x(col("label")) // constant column with value "Total"
                .x2_with(col("label"), |c| c.band(1.0)) // end of band
                .y(lit(0.0)) // baseline at 0
                .y2(sum(col("sales"))) // aggregate dimension
                .fill("#9b59b6"),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "aggregate", "no_grouping").await;
}

#[tokio::test]
async fn test_aggregate_movies_by_mpaa_rating() {
    let ctx = SessionContext::new();
    let df = test_data::movies(&ctx)
        .await
        .expect("Failed to load movies dataset");

    // Bar chart: Average worldwide gross by MPAA rating, colored by average IMDB rating
    // This tests aggregation with a real-world dataset
    let plot = Plot::<Cartesian>::new()
        .title("Average Worldwide Gross by MPAA Rating")
        .subtitle("Colored by Average IMDB Rating")
        .data(df)
        .mark(
            Rect::new()
                .x(col("MPAA Rating")) // grouping dimension
                .x2_with(col("MPAA Rating"), |c| c.band(1.0)) // end of band
                .y_with(lit(0.0), |c| {
                    // baseline at 0 with formatted axis
                    c.axis(|a| a.format(".2s")) // SI prefix format with 2 significant digits
                })
                .y2(avg(col("Worldwide Gross"))) // aggregate dimension
                .fill_with(avg(col("IMDB Rating")), |c| {
                    // Color by avg IMDB rating
                    c.legend(|l| l.title("Avg IMDB Rating"))
                }),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "aggregate",
        "movies_by_mpaa_rating",
    )
    .await;
}

#[tokio::test]
async fn test_aggregate_movies_symbol_plot() {
    let ctx = SessionContext::new();
    let df = test_data::movies(&ctx)
        .await
        .expect("Failed to load movies dataset");

    // Filter to non-null MPAA Rating and Creative Type
    let df = df
        .filter(
            col("MPAA Rating")
                .is_not_null()
                .and(col("Creative Type").is_not_null())
                .and(col("Rotten Tomatoes Rating").is_not_null()),
        )
        .unwrap();

    // Symbol plot: MPAA Rating vs Creative Type
    // Size by count, color by avg Rotten Tomatoes Rating
    let plot = Plot::<Cartesian>::new()
        .title("Movie Count by MPAA Rating and Creative Type")
        .subtitle("Size: Count, Color: Avg Rotten Tomatoes Rating")
        .canvas_size(600.0, 300.0) // 50% wider than default (400x300)
        .data(df)
        .mark(
            Symbol::new()
                .x(col("MPAA Rating")) // grouping dimension
                .y(col("Creative Type")) // grouping dimension
                .size_with(count(col("MPAA Rating")), |c| {
                    // count aggregate for size
                    c.legend(|l| l.title("Count"))
                })
                .fill_with(avg(col("Rotten Tomatoes Rating")), |c| {
                    // color by avg rating
                    c.legend(|l| l.title("Avg RT Rating"))
                }),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "aggregate",
        "movies_symbol_plot",
    )
    .await;
}
