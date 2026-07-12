# Cartesian Coordinates

The Cartesian coordinate system is the most common visualization system, using rectangular x/y coordinates to map data values to visual positions. It powers scatter plots, bar charts, line charts, heatmaps, and most standard statistical graphics.

## Basic Example

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create temperature and humidity data
let batch = RecordBatch::try_from_iter(vec![
    (
        "temperature",
        Arc::new(Float64Array::from(vec![15.0, 18.0, 22.0, 25.0, 28.0, 20.0, 16.0, 24.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "humidity",
        Arc::new(Float64Array::from(vec![65.0, 70.0, 55.0, 50.0, 45.0, 75.0, 80.0, 60.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("temperature"))
            .y(col("humidity"))
            .size(200.0)
            .fill("#3498db")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Position Channels

Cartesian coordinates provide two fundamental position channels:

- **`x`** – Horizontal position (maps to the horizontal axis)
- **`y`** – Vertical position (maps to the vertical axis)

Additional position channels exist for specific mark types:
- **`x2`**, **`y2`** – Secondary positions for [Rect](../marks/rect.md) marks to define rectangles

## Declaration

Create a Cartesian plot using the type parameter:

```rust
let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(Symbol::new().x(col("a")).y(col("b")));
```

When marks are constructed inside `.mark()`, they inherit the plot's coordinate system automatically. For standalone construction, use the explicit form:

```rust
let mark = Symbol::<Cartesian>::new()
    .x(col("a"))
    .y(col("b"));
```

## Supported Scale Types

Cartesian coordinates work with all scale types, allowing flexible data transformations:

### Quantitative Scales
- **[Linear](../scales/linear.md)** – Default for numeric data with uniform spacing
- **[Log](../scales/log.md)** – Logarithmic transformation for exponential relationships
- **[Pow](../scales/pow.md)** – Power transformation with configurable exponent
- **[Sqrt](../scales/sqrt.md)** – Square root transformation (area-based encodings)
- **[Symlog](../scales/symlog.md)** – Symmetric logarithmic scale for data with positive and negative values

### Temporal Scales
- **[Time](../scales/time.md)** – For temporal/date columns with intelligent tick formatting

### Categorical Scales
- **[Band](../scales/band.md)** – For categorical data with spacing (used in bar charts)
- **[Point](../scales/point.md)** – For categorical data without spacing (used in scatter plots)
- **[Ordinal](../scales/ordinal.md)** – For discrete color/shape mappings

### Discretizing Scales
- **[Threshold](../scales/threshold.md)** – Manual breakpoints
- **[Quantize](../scales/quantize.md)** – Uniform bins
- **[Quantile](../scales/quantile.md)** – Quantile-based bins

## Common Usage Patterns

### Scatter Plots

Use quantitative scales on both axes:

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("gdp_per_capita"))
            .y(col("life_expectancy"))
            .size(200.0)
            .fill("#2ecc71")
    )
```

See the [Scatter Plots Guide](../../guides/scatter-plots.md) for more examples.

### Line Charts

Ideal for time series and sequential data:

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("date"))
            .y(col("value"))
            .stroke("#e74c3c")
            .stroke_width(2.0)
    )
```

See the [Line Charts Guide](../../guides/line-charts.md) for more examples.

### Bar Charts

Use categorical scales on one axis and quantitative on the other:

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col("category"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
            .fill("#3498db")
    )
```

See the [Bar Charts Guide](../../guides/bar-charts.md) for more examples.

### Heatmaps

Use categorical scales on both axes with color encoding:

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("x_category"))
            .x2_with(col("x_category"), |c| c.band(1.0))
            .y(col("y_category"))
            .y2_with(col("y_category"), |c| c.band(1.0))
            .fill_with(col("value"), |c| {
                c.scale_with::<Linear>(|s| s)
                    .legend(|l| l.title("Value"))
            })
    )
```

## Layering in Cartesian Coordinates

Multiple marks can be layered in the same Cartesian plot. Scales and legends are automatically merged across marks that use the same channels:

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

Both the line and symbols share the same `x` and `y` scales, ensuring consistent positioning.

## Axes and Guides

Cartesian plots automatically generate axes for the `x` and `y` channels based on the configured scales. See the [Axes Guide](../../guides/axes.md) for customization options.

## Next Steps

- Explore [Polar](./polar.md) coordinates for radial visualizations
- Learn about [Marks](../marks/index.md) that work in Cartesian coordinates
- Review [Scales](../scales/index.md) for data transformations
- See the [Bar Charts](../../guides/bar-charts.md), [Line Charts](../../guides/line-charts.md), and [Scatter Plots](../../guides/scatter-plots.md) guides
