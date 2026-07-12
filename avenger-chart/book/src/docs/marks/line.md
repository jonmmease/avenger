# Line Mark

`Line` marks draw ordered polylines, making them ideal for time series, trend lines, and connecting sequential data points.

**Coordinate System Support:** Currently Cartesian only. Polar support (radial lines, spirals) is planned for future releases.

## Basic Example

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
// Create temperature data over time
let batch = RecordBatch::try_from_iter(vec![
    (
        "day",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "temperature",
        Arc::new(Float64Array::from(vec![15.0, 18.0, 16.0, 22.0, 20.0, 24.0, 23.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("day"))
            .y(col("temperature"))
            .stroke("#dc143c")
            .stroke_width(3.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Channels

### Position Channels (Cartesian)
- **`x`**, **`y`** – Sampled along the polyline path

### Stroke Channels
- **`stroke`** – Line color (supports hex colors, CSS colors, expressions)
- **`stroke_width`** – Line width in pixels
- **`stroke_dash`** – Dash pattern array (e.g., `[5.0, 3.0]` for dashed lines) or theme-defined pattern name (e.g., `"dashed"`)
- **`stroke_cap`** – Line cap style (`"butt"`, `"round"`, or `"square"`)
- **`stroke_join`** – Corner join style (`"miter"`, `"round"`, or `"bevel"`)

### Other Channels
- **`opacity`** – Transparency applied to the entire line (0.0 to 1.0)
- **`defined`** – Boolean expression that allows gaps in the line (undefined segments are not drawn)
- **`order`** – Explicit ordering for multi-series stroke encodings

## Stroke Styling

### Line Color and Width

Customize stroke color and width for emphasis:

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

let plot = Chart::<Cartesian>::new()
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

### Dashed Lines

Use dash patterns to differentiate lines or indicate estimated/projected data:

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

let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .stroke("#ff6b6b")
            .stroke_width(3.0)
            .stroke_dash("dashed")  // Theme-defined dash pattern
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Multi-Series Lines

### Using Color Encoding

When your data contains multiple series identified by a categorical column, use stroke encoding to automatically create separate lines for each category:

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

let plot = Chart::<Cartesian>::new()
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

This automatically creates separate lines for each value in the 'series' column with distinct colors and a legend.

## Gaps in Data

Lines can have gaps where data points are excluded. There are two ways to create gaps:

### Using NULL Values

DataFusion automatically handles NULL values - lines will have gaps where data is missing. To connect across gaps, filter nulls from the data using `.filter(col("column").is_not_null())`.

### Using the `defined` Channel

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
?;

let df = ctx.read_batch(batch)?;

let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y"))
            .defined(col("defined"))  // 0 creates gaps, 1 includes points
            .stroke("#2e8b57")
            .stroke_width(2.5)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates three separate line segments where `defined` equals 1, with gaps at positions 3-4 and 8.

## Automatic Visual Padding

Linear scales automatically expand their domains to prevent line stroke clipping at data boundaries. The padding calculation considers stroke width to ensure the entire line is fully visible even at domain edges.

For complete details on how automatic padding works, how it interacts with `nice()` scales, and scale type support, see [Domain Inference > Automatic Visual Padding](../scales/domains.md#automatic-visual-padding)

## Combining Lines and Points

Layer symbols over lines to show both the trend and individual data points:

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

let plot = Chart::<Cartesian>::new()
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

## Complete Example: Stock Prices

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let df = ctx
    .read_parquet(avenger_sample_data::stocks_path(), ParquetReadOptions::default())
    .await
    ?;

let plot = Chart::<Cartesian>::new()
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

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Next Steps

- See the [Marks Overview](./index.md) for layering and composition
- Try the [Line Patterns recipes](../patterns/common-plot-patterns.md#line-patterns) for a quick-start guide
- Learn about [Channels](../channels/index.md) for data binding
