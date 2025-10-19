# Line Charts

Line charts show trends over continuous domains, typically time or ordered categories, using the [Line mark](../marks/line.md). This guide provides a quick tutorial on creating line charts.

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
?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates a simple line chart connecting points in data order.

## Styling Lines

Customize line appearance with stroke color and width:

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
?;

let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke("#ff6b6b")
            .stroke_width(3.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Multi-Series Lines

Encode a categorical variable to color to create multiple series:

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
?;

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

## Combining Lines and Points

Layer symbols over lines to show both trends and data points:

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
?;

let df = ctx.read_batch(batch)?;

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

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Learn More

For comprehensive coverage of Line mark capabilities, see the [Line Mark Reference](../marks/line.md), which includes:

- **[Stroke Styling](../marks/line.md#stroke-styling)**: Color, width, and dash patterns
- **[Multi-Series Lines](../marks/line.md#multi-series-lines)**: Detailed color encoding examples
- **[Gaps in Data](../marks/line.md#gaps-in-data)**: NULL handling and the `defined` channel
- **[Automatic Visual Padding](../marks/line.md#automatic-visual-padding)**: How Avenger prevents stroke clipping
- **[Combining Lines and Points](../marks/line.md#combining-lines-and-points)**: Layering techniques
- **[Channels](../marks/line.md#channels)**: Complete list of position, stroke, and other channels
- **[Complete Example](../marks/line.md#complete-example-stock-prices)**: Stock prices over time with time scales

## Next Steps

- Learn about [Bar Charts](./bar-charts.md)
- Explore [Aggregations](../data/aggregations.md) for data summaries
- Review [Scales](../scales/index.md) for data transformation options
