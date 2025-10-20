# Line Charts

Line charts visualize trends and patterns in continuous data over time or ordered categories. They excel at showing relationships, trends, and changes between data points. All line charts in Avenger Chart use the [Line mark](../marks/line.md).

## Basic Line Chart

The simplest line chart connects data points in order:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![2.0, 5.0, 3.0, 8.0, 6.0, 9.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(Line::new().x(col("x")).y(col("y")));

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Styling Lines

Customize line appearance with color, width, and dash patterns:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![2.0, 5.0, 3.0, 8.0, 6.0, 9.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke("#ff6b6b")
            .stroke_width(3.0)
            .stroke_dash("dashed")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Multi-Series Lines

Display multiple series with automatic color encoding and legends:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![
            1.0, 2.0, 3.0, 4.0, 5.0,
            1.0, 2.0, 3.0, 4.0, 5.0,
            1.0, 2.0, 3.0, 4.0, 5.0,
        ])) as _,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![
            2.0, 4.0, 3.0, 5.0, 6.0,
            1.0, 3.0, 2.5, 4.0, 3.5,
            3.0, 5.5, 4.5, 7.0, 8.0,
        ])) as _,
    ),
    (
        "series",
        Arc::new(StringArray::from(vec![
            "A", "A", "A", "A", "A",
            "B", "B", "B", "B", "B",
            "C", "C", "C", "C", "C",
        ])) as _,
    ),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke_with(col("series"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Series"))
            })
            .stroke_width(2.5)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Lines with Markers

Combine lines with point markers for emphasis:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![2.0, 5.0, 3.0, 8.0, 6.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .title("Line with markers")
    .mark(
        Line::new()
            .data(df.clone())
            .x(col("x"))
            .y(col("y"))
            .stroke("#2563eb")
            .stroke_width(2.5)
    )
    .mark(
        Symbol::new()
            .data(df)
            .x(col("x"))
            .y(col("y"))
            .fill("#1d4ed8")
            .size(180.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Time Series

Line charts are ideal for temporal data:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let df = ctx
    .read_parquet(avenger_sample_data::stocks_path(), ParquetReadOptions::default())
    .await
    ?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x_with(col("date"), |c| c
                .scale_with::<Time>(|s| s)
                .axis(|axis| axis.title("Date"))
            )
            .y_with(col("price"), |c| c
                .axis(|axis| axis.title("Stock Price ($)"))
            )
            .stroke_with(col("symbol"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Stock"))
            })
            .stroke_width(2.0)
    )
    .title("Stock Prices Over Time");

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Line Styles and Patterns

### Dash Patterns

Use dash patterns to distinguish different line types or indicate estimates:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0])) as _,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.5, 5.5])) as _,
    ),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke("#e11d48")
            .stroke_width(2.5)
            .stroke_dash("dotted")  // Can be "solid", "dashed", or "dotted"
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Line Caps and Joins

Control how lines terminate and connect at corners:

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke("#dc2626")
            .stroke_width(4.0)
            .stroke_cap("round")    // "butt", "round", or "square"
            .stroke_join("round")   // "miter", "round", or "bevel"
    );
# Ok(())
# }
```

## Handling Gaps in Data

Control how lines handle missing or undefined values:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, Int32Array};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![10.0, 25.0, 35.0, 30.0, 45.0, 60.0, 55.0, 70.0, 65.0, 80.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "defined",
        Arc::new(Int32Array::from(vec![1, 1, 1, 0, 0, 1, 1, 1, 0, 1]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .defined(col("defined"))  // 0 creates gaps
            .stroke("#10b981")
            .stroke_width(3.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Advanced Techniques

### Smoothed Lines

For smoother curves, consider preprocessing your data or using interpolation:

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
// Apply smoothing in DataFusion before visualization
let smoothed_df = ctx.sql("
    SELECT x,
           AVG(y) OVER (ORDER BY x ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING) as y_smooth
    FROM data
").await?;

let plot = Plot::<Cartesian>::new()
    .data(smoothed_df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y_smooth"))
    );
# Ok(())
# }
```

### Visual Emphasis

Emphasize important lines with bold styling:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![2.0, 5.0, 3.0, 8.0, 6.0, 9.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke("#7c3aed")
            .stroke_width(4.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Best Practices

1. **Order matters**: Lines connect points in the order they appear in your data
2. **Avoid overplotting**: Too many lines can obscure patterns - consider faceting or filtering
3. **Use consistent scales**: When comparing multiple lines, ensure y-axes are comparable
4. **Handle missing data explicitly**: Use the `defined` channel or filter nulls
5. **Choose appropriate interpolation**: Linear works for most cases, but consider alternatives for specific domains

## Common Pitfalls

- **Unordered data**: Lines will zigzag if x-values aren't sorted
- **Too many series**: More than 5-7 lines become hard to distinguish
- **Inappropriate for categories**: Use bar charts for unordered categorical data
- **Misleading smoothing**: Over-smoothing can hide important variations

## Next Steps

- Explore the [Line mark reference](../marks/line.md) for all available options
- Learn about [Area charts](./area-charts.md) for filled regions under lines
- See [Time series patterns](../guides/time-series.md) for temporal data best practices
- Understand [Scale configuration](../scales/index.md) for axis customization