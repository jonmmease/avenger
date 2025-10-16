# Line Charts

Line charts show trends over continuous domains, typically time or ordered categories. This guide covers single and multi-series line charts.

## Basic Line Chart

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
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
    );

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
```

This creates a simple line chart connecting points in data order.

## Styling Lines

### Line Color and Width

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
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke("#ff6b6b")
            .stroke_width(3.0)
    );

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
```

### Dashed Lines

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
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke("#ff6b6b")
            .stroke_width(3.0)
            .stroke_dash("dashed")  // Theme-defined dash pattern
    );

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
```

### Line Interpolation

Control how points are connected:

```rust,ignore
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
let _line = Line::new()
    .x(col("x"))
    .y(col("y"))
    .interpolate(Interpolate::Linear);  // Default - not yet implemented
```

Options (planned):
- `Linear` - Straight lines between points
- `Step` - Step function (horizontal then vertical)
- `StepBefore` - Vertical then horizontal
- `StepAfter` - Horizontal then vertical
- `Monotone` - Smooth cubic spline (future)

## Multi-Series Line Charts

### Using Color Encoding

For data in long format with a group column:

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
        ])) as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![
            2.0, 4.0, 3.0, 5.0, 6.0,
            1.0, 3.0, 2.5, 4.0, 3.5,
            3.0, 5.5, 4.5, 7.0, 8.0,
        ])) as datafusion::arrow::array::ArrayRef,
    ),
    (
        "series",
        Arc::new(StringArray::from(vec![
            "A", "A", "A", "A", "A",
            "B", "B", "B", "B", "B",
            "C", "C", "C", "C", "C",
        ])) as datafusion::arrow::array::ArrayRef,
    ),
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

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

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
```

This automatically creates separate lines for each value in the 'series' column.

## Combining Lines and Points

Layer symbols over lines:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![2.0, 5.0, 4.0, 7.0, 6.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

let plot = Plot::<Cartesian>::new()
    .data(df.clone())
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke("#4682b4")
            .stroke_width(2.5)
    )
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill("#4682b4")
            .size(120.0)
    );

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
```

## Area Charts

Fill below a line:

```rust,ignore
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Area::new()  // Area mark not yet implemented
            .x(col("date"))
            .y(col("value"))
            .y2(lit(0.0))  // Baseline at 0
            .fill("lightblue")
            .opacity(0.5)
    )
    .mark(
        Line::new()
            .x(col("date"))
            .y(col("value"))
            .stroke("steelblue")
            .stroke_width(2.0)
    )
# ;
# Ok(())
# }
```

## Stacked Area Charts

Stacked areas will land with the planned transform system (see `docs/future-work/transform-system.md`).

## Time Series

For temporal data, use appropriate scales and formatting. Time scales will automatically format axis labels appropriately (hours, days, months, years) when implemented. Currently, use linear scales for numeric time representations.

## Reference Lines

Add horizontal or vertical reference lines:

```rust,ignore
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("date"))
            .y(col("temperature"))
            .stroke("steelblue")
    )
    .mark(
        Rule::new()  // Rule mark not yet implemented
            .y(lit(32.0))  // Freezing point
            .stroke("red")
            .stroke_dash(vec![5.0, 5.0])
    )
# ;
# Ok(())
# }
```

## Gaps in Data

Lines can have gaps where data points are excluded. There are two ways to create gaps:

### Using NULL values

DataFusion automatically handles NULL values - lines will have gaps where data is missing. To connect across gaps, filter nulls from the data using `.filter(col("column").is_not_null())`.

### Using the `defined` channel

Control exactly which points to include using the `defined` channel with a boolean or numeric (0/1) column:

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
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .defined(col("defined"))  // 0 creates gaps, 1 includes points
            .stroke("#2e8b57")
            .stroke_width(2.5)
    );

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
```

This creates three separate line segments where `defined` equals 1, with gaps at positions 3-4 and 8.

## Complete Example

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let stocks_path = format!(
#     "{}/../tests/data/stocks.parquet",
#     env!("CARGO_MANIFEST_DIR")
# );
let df = ctx
    .read_parquet(stocks_path, ParquetReadOptions::default())
    .await
    .expect("load stocks dataset");

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x_with(col("date"), |c| c
                .scale_with::<Time>(|s| s)
                .axis(|axis| axis.title("Date"))
            )
            .y_with(col("price"), |c| c
                .axis(|axis| axis.title("Price ($)"))
            )
            .stroke_with(col("symbol"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Stock"))
            })
            .stroke_width(2.0)
    )
    .title("Stock Prices Over Time");

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
```

## Next Steps

- Learn about [Bar Charts](./bar-charts.md)
- Explore [Aggregations](./aggregations.md) for data summaries
