# Aggregations

Aggregations summarize data by grouping and computing statistics. Avenger Chart provides a convenient syntax for performing aggregations directly in channel expressions.

## Aggregate Encodings

Use aggregate functions like `sum()`, `avg()`, or `count()` directly in channel expressions. Avenger Chart automatically groups by non-aggregated columns:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::functions_aggregate::expr_fn::*;
use datafusion::prelude::*;
use std::sync::Arc;

let ctx = SessionContext::new();
// Create data with multiple rows per category
let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "A", "B", "B", "C", "C"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "sales",
        Arc::new(Float64Array::from(vec![100.0, 150.0, 200.0, 180.0, 120.0, 140.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

// Use sum() in a channel - automatic aggregation by category!
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("category"))                  // Non-aggregated: GROUP BY
            .x2_with(col("category"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(sum(col("sales")))               // Aggregated: SUM
            .fill("#3498db")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

When you use `sum(col("sales"))` in the `y2` channel, Avenger Chart:
1. Detects the aggregate function
2. Groups by non-aggregated columns (`category`)
3. Computes `SUM(sales)` for each group

This is equivalent to SQL: `SELECT category, SUM(sales) FROM data GROUP BY category`

## Common Aggregate Functions

### Sum

Total values per group:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::functions_aggregate::expr_fn::*;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    (
        "region",
        Arc::new(StringArray::from(vec!["North", "North", "South", "South", "West", "West"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "revenue",
        Arc::new(Float64Array::from(vec![1200.0, 1500.0, 1800.0, 2100.0, 900.0, 1100.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("region"))
            .x2_with(col("region"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(sum(col("revenue")))
            .fill("#e74c3c")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Average (Mean)

Compute averages per group:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::functions_aggregate::average::avg;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    (
        "country",
        Arc::new(StringArray::from(vec!["USA", "USA", "UK", "UK", "Japan", "Japan"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "temperature",
        Arc::new(Float64Array::from(vec![15.0, 18.0, 12.0, 14.0, 20.0, 22.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("country"))
            .x2_with(col("country"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(avg(col("temperature")))
            .fill("#2ecc71")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Count

Count rows per group:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::StringArray;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::functions_aggregate::expr_fn::*;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    (
        "status",
        Arc::new(StringArray::from(vec!["Active", "Active", "Active", "Pending", "Pending", "Complete"]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("status"))
            .x2_with(col("status"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(count(col("status")))
            .fill("#9b59b6")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Multiple Aggregate Encodings

Use different aggregate functions in different channels:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::functions_aggregate::average::avg;
use datafusion::functions_aggregate::expr_fn::*;
use std::sync::Arc;

let ctx = SessionContext::new();
// Multiple values per category
let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "A", "A", "B", "B", "B", "C", "C", "C"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "sales",
        Arc::new(Float64Array::from(vec![100.0, 150.0, 200.0, 80.0, 120.0, 160.0, 200.0, 250.0, 300.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "profit",
        Arc::new(Float64Array::from(vec![20.0, 30.0, 40.0, 15.0, 25.0, 35.0, 40.0, 50.0, 60.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

// Height from sum(sales), color from avg(profit)
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col("category"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(sum(col("sales")))              // SUM for height
            .fill_with(avg(col("profit")), |c| {  // AVG for color
                c.legend(|l| l.title("Avg Profit"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Both `sum(col("sales"))` and `avg(col("profit"))` are computed for each group defined by `category`.

## Multi-Dimensional Aggregations

Group by multiple columns using Symbol plots:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::functions_aggregate::average::avg;
use datafusion::functions_aggregate::expr_fn::*;
use std::sync::Arc;

let ctx = SessionContext::new();
// Data with two grouping dimensions and varying counts per group
let batch = RecordBatch::try_from_iter(vec![
    (
        "region",
        Arc::new(StringArray::from(vec![
            "North", "North", "South", "South", "South", "South", "South",
            "North", "North", "North", "North", "South", "South"
        ]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "product",
        Arc::new(StringArray::from(vec![
            "A", "A", "A", "A", "A", "A", "A",
            "B", "B", "B", "B", "B", "B"
        ]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "sales",
        Arc::new(Float64Array::from(vec![
            100.0, 120.0, 150.0, 160.0, 140.0, 155.0, 145.0,
            200.0, 220.0, 210.0, 230.0, 250.0, 260.0
        ]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "rating",
        Arc::new(Float64Array::from(vec![
            4.5, 4.7, 4.2, 4.3, 4.1, 4.4, 4.6,
            4.8, 4.9, 4.7, 4.6, 4.4, 4.5
        ]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

// Groups by BOTH region AND product
let plot = Plot::<Cartesian>::new()
    .canvas_size(500.0, 300.0)
    .data(df)
    .mark(
        Symbol::new()
            .x(col("region"))                     // GROUP BY region
            .y(col("product"))                    // GROUP BY product
            .size_with(count(col("sales")), |c| {  // Aggregate: COUNT
                c.scale(|s| s.range_interval(lit(400.0), lit(1200.0)))
                    .legend(|l| l.title("Count"))
            })
            .fill_with(avg(col("rating")), |c| {   // Aggregate: AVG
                c.legend(|l| l.title("Avg Rating"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Full-Table Aggregation

Aggregate all rows with no grouping:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::functions_aggregate::expr_fn::*;
use datafusion::prelude::*;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    (
        "values",
        Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

// Add constant column for x position
let df = df.with_column("label", lit("Total")).unwrap();

// Single bar showing total
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("label"))                 // Constant column
            .x2_with(col("label"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(sum(col("values")))          // Sum all rows
            .fill("#3498db")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Real-World Example: Movies Dataset

```rust,render
use avenger_chart::prelude::*;
use datafusion::functions_aggregate::average::avg;
use datafusion::prelude::*;

let ctx = SessionContext::new();

let df = ctx
    .read_parquet(avenger_sample_data::movies_path(), ParquetReadOptions::default())
    .await
    ?;

// Filter to non-null ratings
let df = df
    .filter(
        col("MPAA Rating")
            .is_not_null()
            .and(col("IMDB Rating").is_not_null())
    )
    ?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("MPAA Rating"))
            .x2_with(col("MPAA Rating"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| c.axis(|a| a.format(".2s")))
            .y2(avg(col("Worldwide Gross")))      // Average gross by rating
            .fill_with(avg(col("IMDB Rating")), |c| {  // Color by avg IMDB
                c.legend(|l| l.title("Avg IMDB Rating"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## How It Works

When you use an aggregate function in a channel expression:

1. **Detection**: Avenger Chart scans all channel expressions
2. **Grouping**: Non-aggregated columns become GROUP BY dimensions
3. **Aggregation**: Aggregate functions are computed per group
4. **Evaluation**: Results are used for mark positioning/styling

Example:
```rust
Rect::new()
    .x(col("category"))      // Non-aggregated → GROUP BY category
    .y2(sum(col("sales")))   // Aggregated → SUM(sales)
    .fill_with(avg(col("profit")), ...) // Aggregated → AVG(profit)
```

Becomes equivalent to:
```sql
SELECT category, SUM(sales), AVG(profit)
FROM data
GROUP BY category
```

## Available Aggregate Functions

From `datafusion::functions_aggregate::expr_fn`:
- `sum(expr)` - Sum values
- `count(expr)` - Count non-null values
- `min(expr)` - Minimum value
- `max(expr)` - Maximum value
- `stddev(expr)` - Standard deviation
- `variance(expr)` - Variance
- `approx_percentile_cont(expr, percentile, None)` - Approximate percentile

From `datafusion::functions_aggregate::average`:
- `avg(expr)` - Average (mean)

## Manual DataFusion Aggregation

For more complex scenarios, you can also perform aggregations manually in DataFusion before passing data to the plot:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, Int32Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::functions_aggregate::average::avg;
use datafusion::functions_aggregate::expr_fn::*;
use datafusion::prelude::*;
use std::sync::Arc;

let ctx = SessionContext::new();
// Create sales data with year, category, region
let batch = RecordBatch::try_from_iter(vec![
    (
        "year",
        Arc::new(Int32Array::from(vec![2023, 2023, 2023, 2023, 2023, 2023, 2023, 2023, 2023, 2023, 2023, 2023]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "A", "A", "A", "B", "B", "B", "B", "C", "C", "C", "C"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "region",
        Arc::new(StringArray::from(vec!["North", "North", "South", "South", "North", "North", "South", "South", "North", "North", "South", "South"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "sales",
        Arc::new(Float64Array::from(vec![100.0, 120.0, 150.0, 140.0, 200.0, 210.0, 180.0, 190.0, 300.0, 280.0, 320.0, 310.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "rating",
        Arc::new(Float64Array::from(vec![4.5, 4.6, 4.7, 4.8, 4.2, 4.3, 4.4, 4.5, 4.8, 4.9, 4.7, 4.6]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

// Manual aggregation with filtering
let aggregated = df
    .filter(col("year").eq(lit(2023)))
    ?
    .aggregate(
        vec![col("category")],
        vec![
            sum(col("sales")).alias("total_sales"),
            avg(col("rating")).alias("avg_rating"),
            count(lit(1)).alias("count"),
        ]
    )
    ?
    .filter(col("count").gt(lit(2)))
    ?;

let plot = Plot::<Cartesian>::new()
    .data(aggregated)
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))  // Can use :x with pre-aggregated data
            .y(lit(0.0))
            .y2(col("total_sales"))              // Already aggregated
            .fill_with(col("avg_rating"), |c| {
                c.legend(|l| l.title("Avg Rating"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This approach gives you full control over:
- Pre-aggregation filtering (WHERE clause)
- Complex grouping logic
- Post-aggregation filtering (HAVING clause)
- Multiple aggregation passes

## Next Steps

- Learn about [Bar Charts](./bar-charts.md) for categorical comparisons
- Explore [Scatter Plots](./scatter-plots.md) for relationship visualization
- See [Working with DataFusion](./datafusion-expressions.md) for expression capabilities
- See [DataFusion documentation](https://docs.rs/datafusion) for more aggregation functions
