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


let plot = Plot::<Cartesian>::new()
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
- **`stroke_dash`** – Dash pattern array (e.g., `[5.0, 3.0]` for dashed lines)
- **`stroke_cap`** – Line cap style (`"butt"`, `"round"`, or `"square"`)
- **`stroke_join`** – Corner join style (`"miter"`, `"round"`, or `"bevel"`)

### Other Channels
- **`opacity`** – Transparency applied to the entire line (0.0 to 1.0)
- **`defined`** – Boolean expression that allows gaps in the line (undefined segments are not drawn)
- **`order`** – Explicit ordering for multi-series stroke encodings

## Usage Patterns

### Time Series

Line marks excel at visualizing data over time:

```rust
Line::new()
    .x(col("timestamp"))
    .y(col("value"))
    .stroke("#1f77b4")
    .stroke_width(2.0)
```

### Multi-Series Lines

When encoding a categorical variable to `stroke`, each category gets its own line with automatic legend generation:

```rust
Line::new()
    .x(col("date"))
    .y(col("price"))
    .stroke_with(col("stock_symbol"), |c| {
        c.scale_with::<Ordinal>(|s| s)
            .legend(|l| l.title("Stock"))
    })
```

### Conditional Gaps

Use the `defined` channel to create gaps in lines when data is missing or conditions aren't met:

```rust
Line::new()
    .x(col("x"))
    .y(col("y"))
    .defined(col("is_valid"))  // Only draw where is_valid is true
    .stroke("#2ca02c")
```

## Next Steps

- See the [Marks Overview](./index.md) for layering and composition
- Explore [Line Charts Guide](../../guides/line-charts.md) for more examples
- Learn about [Channels](../channels.md) for data binding
