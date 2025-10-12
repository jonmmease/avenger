# Aggregations

Aggregations summarize data by grouping and computing statistics. This guide shows how to use DataFusion's aggregation capabilities with Avenger Chart.

## DataFusion Aggregations

Currently, you perform aggregations in DataFusion before passing data to Avenger Chart:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let aggregated = df.aggregate(
    vec![col("category")],           // Group by
    vec![sum(col("sales")).alias("total_sales")]  // Aggregate
)?;

let _plot = Plot::<Cartesian>::new()
    .data(aggregated)
    .mark(
        Rect::new()
            .x(col("category"))
            .y(lit(0.0))
            .y2(col("total_sales"))
    );
# Ok(())
# }
```

## Common Aggregation Functions

### Count

Count rows in each group:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let aggregated = df.aggregate(
    vec![col("category")],
    vec![count(lit(1)).alias("count")]
)?;
# Ok(())
# }
```

### Sum

Sum values in each group:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let aggregated = df.aggregate(
    vec![col("region")],
    vec![sum(col("revenue")).alias("total_revenue")]
)?;
# Ok(())
# }
```

### Average (Mean)

Compute average:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let aggregated = df.aggregate(
    vec![col("country")],
    vec![avg(col("temperature")).alias("avg_temp")]
)?;
# Ok(())
# }
```

### Min and Max

Find minimum and maximum:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let aggregated = df.aggregate(
    vec![col("year")],
    vec![
        min(col("price")).alias("min_price"),
        max(col("price")).alias("max_price"),
    ]
)?;
# Ok(())
# }
```

### Standard Deviation

Compute standard deviation:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let aggregated = df.aggregate(
    vec![col("group")],
    vec![stddev(col("value")).alias("std_dev")]
)?;
# Ok(())
# }
```

## Multiple Aggregations

Compute multiple statistics in one operation:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let stats = df.aggregate(
    vec![col("species")],
    vec![
        count(lit(1)).alias("count"),
        avg(col("sepal_length")).alias("avg_length"),
        min(col("sepal_length")).alias("min_length"),
        max(col("sepal_length")).alias("max_length"),
    ]
)?;
# Ok(())
# }
```

## Multi-Column Grouping

Group by multiple columns:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let aggregated = df.aggregate(
    vec![col("year"), col("quarter")],
    vec![sum(col("sales")).alias("total_sales")]
)?;
# Ok(())
# }
```

## Histograms

Create histograms by binning:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# use datafusion::functions::math::expr_fn::floor;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
// Bin data using floor division
let binned = df
    .with_column(
        "bin",
        floor(col("value") / lit(10.0)) * lit(10.0)
    )?;

// Count per bin
let histogram = binned.aggregate(
    vec![col("bin")],
    vec![count(lit(1)).alias("count")]
)?;

// Visualize
let plot = Plot::<Cartesian>::new()
    .data(histogram)
    .mark(
        Rect::new()
            .x(col("bin"))
            .x2(col("bin") + lit(10.0))
            .y(lit(0.0))
            .y2(col("count"))
    );
# Ok(())
# }
```

## Future: Built-in Transform System

A built-in transform system is planned for future releases:

```rust,ignore
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
// Future API - not yet implemented
Rect::new()
    .data(df)
    .transform(  // Transform system not yet implemented
        Bin::x("price")
            .bins(20)
            .aggregate(count(lit(1)))
    )
# ;
# Ok(())
# }
```

See [transform-system.md](../../docs/future-work/transform-system.md) for the design.

## Aggregated Scatter Plots

Visualize aggregated statistics:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let stats = df.aggregate(
    vec![col("species")],
    vec![
        avg(col("sepal_length")).alias("mean_length"),
        stddev(col("sepal_length")).alias("std_length"),
    ]
)?;

Plot::<Cartesian>::new()
    .data(stats)
    .mark(
        Symbol::<Cartesian>::new()
            .x(col("species"))
            .y(col("mean_length"))
            .size(200.0)
    )
    // Future: Add error bars with Rule marks
# ;
# Ok(())
# }
```

## Time-Based Aggregations

Aggregate by time periods:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
// Assume date_trunc function is available
let monthly = df
    .with_column(
        "month",
        // Use appropriate date truncation
        col("date")
    )?
    .aggregate(
        vec![col("month")],
        vec![avg(col("temperature")).alias("avg_temp")]
    )?;

Plot::<Cartesian>::new()
    .data(monthly)
    .mark(
        Line::new()
            .x_with(col("month"), |c| c.scale_with::<Time>(|s| s))
            .y(col("avg_temp"))
    )
# ;
# Ok(())
# }
```

## Percentiles

Compute percentiles:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
// DataFusion supports percentile approximation
let percentiles = df.aggregate(
    vec![col("category")],
    vec![
        approx_percentile_cont(col("value").sort(true, false), lit(0.25), None).alias("p25"),
        approx_percentile_cont(col("value").sort(true, false), lit(0.50), None).alias("p50"),
        approx_percentile_cont(col("value").sort(true, false), lit(0.75), None).alias("p75"),
    ]
)?;
# Ok(())
# }
```

## Filtering Before Aggregation

Filter data before aggregating:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let filtered = df.filter(col("year").eq(lit(2023)))?;

let aggregated = filtered.aggregate(
    vec![col("category")],
    vec![sum(col("sales")).alias("total_sales")]
)?;
# Ok(())
# }
```

## Filtering After Aggregation

Filter aggregated results (HAVING clause):

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions_aggregate::expr_fn::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let aggregated = df.aggregate(
    vec![col("category")],
    vec![count(lit(1)).alias("count")]
)?;

let filtered = aggregated.filter(col("count").gt(lit(100)))?;
# Ok(())
# }
```

## Complete Example

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::functions_aggregate::expr_fn::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let movies_path = format!(
    "{}/../tests/data/movies.parquet",
    env!("CARGO_MANIFEST_DIR")
);
let df = ctx
    .read_parquet(movies_path, ParquetReadOptions::default())
    .await
    .expect("load movies dataset");

let by_year = df
    .aggregate(
        vec![col("year")],
        vec![
            avg(col("rating")).alias("avg_rating"),
            count(lit(1)).alias("num_movies"),
        ],
    )
    .expect("aggregate movies by year");

let filtered = by_year
    .filter(col("num_movies").gt_eq(lit(10)))
    .expect("filter by movie count");

let plot = Plot::<Cartesian>::new()
    .data(filtered)
    .mark(
        Symbol::<Cartesian>::new()
            .x(col("year"))
            .y(col("avg_rating"))
            .size_with(col("num_movies"), |c| {
                c.scale(|s| s
                    .domain_interval(lit(0.0), lit(100.0))
                    .range_interval(lit(50.0), lit(500.0))
                )
                .legend(|l| l.title("Number of Movies"))
            })
            .fill("#4682b4")
    );

plot
```

## Next Steps

- Explore the future [Transform System](../../docs/future-work/transform-system.md)
- See [DataFusion documentation](https://docs.rs/datafusion) for more aggregation functions
