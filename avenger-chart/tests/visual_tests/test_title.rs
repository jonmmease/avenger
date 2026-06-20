use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;

use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

fn make_df_categories() -> DataFrame {
    let categories = StringArray::from(vec!["A", "B", "C", "A", "B", "C", "A", "B", "C"]);
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 10.0, 9.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("category", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(categories), Arc::new(x_values), Arc::new(y_values)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

fn make_df_numeric() -> DataFrame {
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    let y_values = Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 7.0, 6.0, 8.0, 10.0, 9.0]);
    let value = Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0]);

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("value", DataType::Float64, false),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(x_values), Arc::new(y_values), Arc::new(value)],
    )
    .unwrap();

    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

#[tokio::test]
async fn title_basic_symbol() {
    let ctx = SessionContext::new();
    let df = make_df_categories();
    let plot = Plot::<Cartesian>::new().title("Basic Title").data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
            .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
            .size(100.0)
            .fill_with("#2ca25f", |c| c.no_legend()),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "title_basic_symbol").await;
}

#[tokio::test]
async fn title_with_symbol_legend() {
    let ctx = SessionContext::new();
    let df = make_df_categories();
    let plot = Plot::<Cartesian>::new()
        .title("Title With Legend")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
                .size(100.0)
                .fill_with(col("category"), |c| {
                    c.legend(|l| l.title("Category").position(LegendPosition::Right))
                }),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "title_with_symbol_legend").await;
}

#[tokio::test]
async fn title_top_x_right_y() {
    let ctx = SessionContext::new();
    let df = make_df_categories();
    let plot = Plot::<Cartesian>::new()
        .title("Top X & Right Y")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 10.0)))
                        .axis(|a| a.position(AxisPosition::Top).title("Top X").grid(true))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 12.0)))
                        .axis(|a| a.position(AxisPosition::Right).title("Right Y").grid(true))
                })
                .size(100.0)
                .fill_with("#2ca25f", |c| c.no_legend()),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "title_top_x_right_y").await;
}

#[tokio::test]
async fn title_with_colorbar_legend() {
    let ctx = SessionContext::new();
    let df = make_df_numeric();
    let plot = Plot::<Cartesian>::new()
        .title("Title With Colorbar")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
                .size(100.0)
                .fill_with(col("value"), |c| {
                    c.scale(|s| s.domain((0.0, 100.0)))
                        .legend(|l| l.title("Value").position(LegendPosition::Right))
                }),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "layout",
        "title_with_colorbar_legend",
    )
    .await;
}

#[tokio::test]
async fn subtitle_basic_symbol() {
    let ctx = SessionContext::new();
    let df = make_df_categories();
    let plot = Plot::<Cartesian>::new()
        .title("Main Title")
        .subtitle("This is a subtitle")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
                .size(100.0)
                .fill_with("#2ca25f", |c| c.no_legend()),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "subtitle_basic_symbol").await;
}

#[tokio::test]
async fn subtitle_with_legend() {
    let ctx = SessionContext::new();
    let df = make_df_numeric();
    let plot = Plot::<Cartesian>::new()
        .title("Main Title")
        .subtitle("This is a subtitle with a legend")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
                .size(100.0)
                .fill_with(col("value"), |c| {
                    c.scale(|s| s.domain((0.0, 100.0)))
                        .legend(|l| l.title("Value").position(LegendPosition::Right))
                }),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "subtitle_with_legend").await;
}

// Note: The current API doesn't support custom title properties like color, font_size, or align
// These tests are removed since those features don't exist yet

#[tokio::test]
async fn title_with_axes_positions() {
    let ctx = SessionContext::new();
    let df = make_df_categories();
    let plot = Plot::<Cartesian>::new()
        .title("Title With Different Axes Positions")
        .subtitle("Demonstrating layout")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| {
                    c.scale(|s| s.domain((0.0, 10.0)))
                        .axis(|a| a.position(AxisPosition::Top).title("Top X").grid(false))
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 12.0)))
                        .axis(|a| a.position(AxisPosition::Right).title("Right Y").grid(false))
                })
                .size(100.0)
                .fill_with("#c0392b", |c| c.no_legend()),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "title_with_axes_positions").await;
}

#[tokio::test]
async fn subtitle_with_symbol_legend() {
    let ctx = SessionContext::new();
    let df = make_df_categories();
    let plot = Plot::<Cartesian>::new()
        .title("Main Title")
        .subtitle("Subtitle with legend")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
                .size(100.0)
                .fill_with(col("category"), |c| {
                    c.legend(|l| l.title("Category").position(LegendPosition::Right))
                }),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "layout",
        "subtitle_with_symbol_legend",
    )
    .await;
}

#[tokio::test]
async fn subtitle_with_colorbar() {
    let ctx = SessionContext::new();
    let df = make_df_numeric();
    let plot = Plot::<Cartesian>::new()
        .title("Temperature Distribution")
        .subtitle("Measured across different locations")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
                .size(100.0)
                .fill_with(col("value"), |c| {
                    c.scale(|s| s.domain((0.0, 100.0)))
                        .legend(|l| l.title("Value").position(LegendPosition::Right))
                }),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "subtitle_with_colorbar").await;
}

#[tokio::test]
async fn subtitle_only() {
    let ctx = SessionContext::new();
    let df = make_df_categories();
    let plot = Plot::<Cartesian>::new()
        .subtitle("Only a subtitle, no title")
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
                .size(100.0)
                .fill_with("#2ca25f", |c| c.no_legend()),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "subtitle_only").await;
}

#[tokio::test]
async fn title_plot_area_only() {
    let ctx = SessionContext::new();
    let df = make_df_categories();
    let plot = Plot::<Cartesian>::new()
        .configure_title("Plot Area Only Title", |t| t.span(TitleSpan::PlotArea))
        .configure_subtitle("Plot Area Only Subtitle", |s| s.span(TitleSpan::PlotArea))
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
                .size(100.0)
                .fill_with("#2ca25f", |c| c.no_legend()),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "title_plot_area_only").await;
}

#[tokio::test]
async fn title_plot_area_only_with_legend() {
    let ctx = SessionContext::new();
    let df = make_df_categories();
    let plot = Plot::<Cartesian>::new()
        .configure_title("Plot Area Title", |t| t.span(TitleSpan::PlotArea))
        .configure_subtitle("With Right Legend", |s| s.span(TitleSpan::PlotArea))
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 10.0))))
                .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
                .size(100.0)
                .fill_with(col("category"), |c| {
                    c.legend(|l| l.title("Category").position(LegendPosition::Right))
                }),
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "layout",
        "title_plot_area_only_with_legend",
    )
    .await;
}
