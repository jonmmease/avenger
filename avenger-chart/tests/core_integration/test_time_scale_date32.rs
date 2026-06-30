#[cfg(feature = "doc-render")]
use avenger_chart::doc::render::render_evaluated_plot_to_png;
use avenger_chart::param::Param;
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Date32Array, Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;
#[cfg(feature = "doc-render")]
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::test]
async fn test_time_scale_with_date32_simple() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();

    // Create a batch with Date32 column
    let batch = RecordBatch::try_from_iter(vec![
        (
            "date",
            Arc::new(Date32Array::from(vec![19723, 19724, 19725, 19726, 19727])) // 2024-01-01 through 2024-01-05
                as datafusion::arrow::array::ArrayRef,
        ),
        (
            "value",
            Arc::new(Float64Array::from(vec![10.0, 15.0, 12.0, 18.0, 14.0]))
                as datafusion::arrow::array::ArrayRef,
        ),
        (
            "symbol",
            Arc::new(StringArray::from(vec![
                "AAPL", "AAPL", "AAPL", "AAPL", "AAPL",
            ])) as datafusion::arrow::array::ArrayRef,
        ),
    ])?;

    let df = ctx.read_batch(batch)?;

    // Try to create a plot with Time scale
    let plot = Plot::<Cartesian>::new().data(df).mark(
        Line::new()
            .x_with(col("date"), |c| {
                c.scale_with::<Time>(|s| s).axis(|axis| axis.title("Date"))
            })
            .y_with(col("value"), |c| c.axis(|axis| axis.title("Value"))),
    );

    // Compile the plot
    let _compiled = plot.compile(&ctx).await?;

    println!("Simple plot compiled successfully with Time scale and Date32!");

    Ok(())
}

#[tokio::test]
async fn test_time_scale_with_date32_datetime_format() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();

    let batch = RecordBatch::try_from_iter(vec![
        (
            "date",
            Arc::new(Date32Array::from(vec![19723, 19724, 19725, 19726, 19727]))
                as datafusion::arrow::array::ArrayRef,
        ),
        (
            "value",
            Arc::new(Float64Array::from(vec![10.0, 15.0, 12.0, 18.0, 14.0]))
                as datafusion::arrow::array::ArrayRef,
        ),
    ])?;

    let df = ctx.read_batch(batch)?;
    let plot = Plot::<Cartesian>::new().data(df).mark(
        Line::new()
            .x_with(col("date"), |c| {
                c.scale_with::<Time>(|s| s)
                    .axis(|axis| axis.title("Date").datetime_format("MMM d"))
            })
            .y_with(col("value"), |c| c.axis(|axis| axis.title("Value"))),
    );

    let _compiled = plot.compile(&ctx).await?;
    Ok(())
}

#[tokio::test]
async fn test_time_scale_with_date32_and_expression() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();

    // Create a batch with Date32 column (like the parameters.md example)
    let batch = RecordBatch::try_from_iter(vec![
        (
            "date",
            Arc::new(Date32Array::from(vec![19723, 19724, 19725, 19726, 19727]))
                as datafusion::arrow::array::ArrayRef,
        ),
        (
            "price",
            Arc::new(Float64Array::from(vec![140.0, 155.0, 145.0, 160.0, 148.0]))
                as datafusion::arrow::array::ArrayRef,
        ),
        (
            "symbol",
            Arc::new(StringArray::from(vec![
                "AAPL", "AAPL", "AAPL", "AAPL", "AAPL",
            ])) as datafusion::arrow::array::ArrayRef,
        ),
    ])?;

    let df = ctx.read_batch(batch)?;

    // Create a parameter and expression (like parameters.md example)
    let threshold = Param::new("price_threshold", ScalarValue::from(150.0));
    let status = when(col("price").gt(threshold.expr()), lit("above")).otherwise(lit("within"))?;

    // Try to create a plot with Time scale and expression encoding
    let plot = Plot::<Cartesian>::new()
        .data(df)
        .add_param(threshold.clone())
        .mark(
            Symbol::new()
                .x_with(col("date"), |c| {
                    c.scale_with::<Time>(|s| s).axis(|a| a.title("Date"))
                })
                .y_with(col("price"), |c| c.axis(|a| a.title("Price ($)")))
                .fill_with(status, |c| {
                    c.scale_with::<Ordinal>(|s| s)
                        .legend(|l| l.title("Threshold Status"))
                }),
        );

    // Compile the plot
    let _compiled = plot.compile(&ctx).await?;

    println!("Plot with parameter and expression compiled successfully!");

    Ok(())
}

#[tokio::test]
#[cfg(feature = "doc-render")]
async fn test_time_scale_with_stocks_parquet() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();

    // Load stocks.parquet (same as in the mdbook example)
    let stocks_path = format!("{}/tests/data/stocks.parquet", env!("CARGO_MANIFEST_DIR"));
    let df = ctx
        .read_parquet(stocks_path, ParquetReadOptions::default())
        .await?;

    // Filter to AAPL like the example
    let aapl = df.filter(col("symbol").eq(lit("AAPL")))?;

    // Create parameter and expression
    let threshold = Param::new("price_threshold", ScalarValue::from(150.0));
    let status = when(col("price").gt(threshold.expr()), lit("above")).otherwise(lit("within"))?;

    // Create plot with Time scale
    let plot = Plot::<Cartesian>::new()
        .data(aapl.clone())
        .add_param(threshold.clone())
        .mark(
            Symbol::new()
                .x_with(col("date"), |c| {
                    c.scale_with::<Time>(|s| s).axis(|a| a.title("Date"))
                })
                .y_with(col("price"), |c| c.axis(|a| a.title("Price ($)")))
                .fill_with(status, |c| {
                    c.scale_with::<Ordinal>(|s| s)
                        .legend(|l| l.title("Threshold Status"))
                }),
        );

    // Compile and evaluate the plot
    let compiled = plot.compile(&ctx).await?;
    let evaluated = compiled.evaluate(&ctx, None).await?;

    // Render to PNG
    let output_path = PathBuf::from("/tmp/test_stocks_time_scale.png");
    render_evaluated_plot_to_png(&evaluated, &output_path)
        .await
        .map_err(|e| format!("Render error: {}", e))?;

    println!("Stocks parquet plot with Time scale rendered to PNG successfully!");
    println!("Output: {}", output_path.display());

    Ok(())
}
