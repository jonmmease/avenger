# Rect Mark

`Rect` marks render axis-aligned rectangles and power bar charts, heatmaps, and interval plots.

## Basic Example

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create horizontal bar chart data
let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![25.0, 40.0, 30.0, 55.0, 35.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .y(col("category"))
            .y2_with(col("category"), |c| c.band(1.0))
            .x(lit(0.0))
            .x2(col("value"))
            .fill("#ffa500")
            .opacity(0.9)
            .corner_radius(3.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Channels

### Position Channels

Rectangles are defined by four position channels that specify opposite corners:

- **`x`**, **`x2`** – Horizontal extent (left and right edges)
- **`y`**, **`y2`** – Vertical extent (bottom and top edges)

### Visual Encoding Channels

- **`fill`** – Fill color (supports hex colors, CSS colors, expressions)
- **`stroke`** – Outline color
- **`stroke_width`** – Outline width in pixels
- **`opacity`** – Transparency (0.0 to 1.0)
- **`corner_radius`** – Radius for rounded corners in pixels

## Usage Patterns

### Bar Charts

For bar charts, typically use a categorical scale for one axis and a quantitative scale for the other:

```rust
Rect::new()
    .x(col("category"))               // Categorical position
    .x2_with(col("category"), |c| c.band(1.0))  // Band width
    .y(lit(0.0))                       // Baseline at 0
    .y2(col("value"))                  // Bar height from data
    .fill("#1f77b4")
```

### Heatmaps

For heatmaps, use categorical scales on both axes and encode a quantitative value to fill color:

```rust
Rect::new()
    .x(col("x_category"))
    .x2_with(col("x_category"), |c| c.band(1.0))
    .y(col("y_category"))
    .y2_with(col("y_category"), |c| c.band(1.0))
    .fill_with(col("value"), |c| {
        c.scale_with::<Linear>(|s| s)
    })
```

### Interval Plots

For interval plots showing ranges, use both position channels with quantitative data:

```rust
Rect::new()
    .x(col("start_time"))
    .x2(col("end_time"))
    .y(col("task"))
    .y2_with(col("task"), |c| c.band(0.8))  // 80% of band width
    .fill("#2ca02c")
```

## Next Steps

- See the [Marks Overview](./index.md) for layering and composition
- Explore [Bar Charts Guide](../../guides/bar-charts.md) for more examples
- Learn about [Band Scales](../scales/band.md) for categorical positioning
