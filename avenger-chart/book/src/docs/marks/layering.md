# Layering Marks

Marks can be layered to combine multiple visual encodings in a single plot. This enables rich composite visualizations like line charts with points, scatter plots with trend lines, and more complex multi-layer graphics.

## Basic Layering

Multiple `.mark()` calls on a plot stack marks in drawing order (first mark drawn first, last mark drawn on top):

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create time series data
let batch = RecordBatch::try_from_iter(vec![
    (
        "time",
        Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![10.0, 15.0, 13.0, 18.0, 16.0, 22.0, 20.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Chart::<Cartesian>::new()
    .data(df.clone())
    .mark(
        Line::new()
            .x(col("time"))
            .y(col("value"))
            .stroke("#1f2933")
            .stroke_width(2.0)
    )
    .mark(
        Symbol::new()
            .x(col("time"))
            .y(col("value"))
            .size(200.0)
            .fill("#38bdf8")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

In this example, the line is drawn first, then symbols are drawn on top.

## Shared Scales and Legends

`Plot` automatically merges scale and legend configuration across marks that use the same channel names. This means:

- Both marks share the same `x` and `y` axes
- Both marks use the same `x` and `y` scale domains and ranges
- If multiple marks encode the same channel to different data (e.g., both use `fill` with different columns), legends are generated for each

This automatic merging ensures visual consistency across layers without manual scale coordination.

## Mark-Specific Data

Each mark can source its own `DataFrame`. When a mark omits `.data(...)`, it inherits the plot-level data:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create scatter point data
let points_batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![2.5, 4.0, 3.0, 5.5, 4.5, 6.0, 5.5, 7.5]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let points = ctx.read_batch(points_batch)?;

// Create trend line data
let trend_batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 8.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y_pred",
        Arc::new(Float64Array::from(vec![2.0, 8.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let trend = ctx.read_batch(trend_batch)?;

let plot = Chart::<Cartesian>::new()
    .mark(
        Symbol::new()
            .data(points)
            .x(col("x"))
            .y(col("y"))
            .size(200.0)
            .fill("#2563eb")
    )
    .mark(
        Line::new()
            .data(trend)
            .x(col("x"))
            .y(col("y_pred"))
            .stroke("#dc2626")
            .stroke_width(3.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This example shows scatter points with a separate trend line, each using different data sources. The `x` scales are still shared, ensuring proper alignment.

## Draw Order Control

By default, marks are drawn in the order they're added to the plot. You can explicitly control z-ordering with `.zindex()`:

```rust
Plot::<Cartesian>::new()
    .mark(
        Symbol::new()
            .zindex(2)  // Draw on top
            // ... channels ...
    )
    .mark(
        Rect::new()
            .zindex(1)  // Draw below
            // ... channels ...
    )
```

Higher z-index values draw on top of lower values.

## Common Layering Patterns

### Line Chart with Points

Combine Line and Symbol marks to show both trend and individual data points:

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(Line::new().x(col("x")).y(col("y")).stroke("#333"))
    .mark(Symbol::new().x(col("x")).y(col("y")).size(150.0).fill("#e74c3c"))
```

### Reference Lines

Layer Rect marks to create reference bands or regions:

```rust
Plot::<Cartesian>::new()
    .mark(
        Rect::new()
            .data(reference_data)
            .x(lit(x_min))
            .x2(lit(x_max))
            .y(col("lower_bound"))
            .y2(col("upper_bound"))
            .fill("#e0e0e0")
            .opacity(0.3)
            .zindex(0)  // Draw behind
    )
    .mark(
        Line::new()
            .data(actual_data)
            .x(col("x"))
            .y(col("y"))
            .stroke("#2563eb")
            .zindex(1)  // Draw on top
    )
```

### Multi-Series Comparison

Layer multiple marks to compare different data series:

```rust
Plot::<Cartesian>::new()
    .mark(
        Line::new()
            .data(series_a)
            .x(col("date"))
            .y(col("value"))
            .stroke("#1f77b4")
    )
    .mark(
        Line::new()
            .data(series_b)
            .x(col("date"))
            .y(col("value"))
            .stroke("#ff7f0e")
            .stroke_dash(vec![5.0, 3.0])
    )
```

## Additional Mark Options

All marks share common options that affect layering:

- **`.data(...)`** – Attaches a dedicated `DataFrame` to the mark
- **`.zindex(...)`** – Sets explicit draw order (higher values draw on top)
- **`.facet_data_scope(...)`**, **`.facet_data_level(...)`**, and **`.broadcast_to_facets()`** – Control how much faceted data a mark sees
- **`.details([...])`** – Stores additional fields for future tooltip/interaction layers (values are carried through evaluation but no tooltip UI ships yet)

## Next Steps

- See the [Marks Overview](./index.md) for available mark types
- Learn about [Channels](../channels/index.md) for encoding data
- Review [Scales](../scales/index.md) for understanding how scales are merged
- Explore specific mark pages: [Symbol](./symbol.md), [Line](./line.md), [Rect](./rect.md)
