# Marks

Marks are the visual building blocks of a plot. Each mark type turns channel inputs into a particular geometric representation. Avenger Chart currently ships three Cartesian mark types: `Symbol`, `Line`, and `Rect`.

## Available Mark Types

| Mark Type | Purpose | Key Channels | Use Cases |
|-----------|---------|--------------|-----------|
| **[Symbol](./symbol.md)** | Discrete points | x, y, size, fill, stroke, shape, angle | Scatter plots, dot plots |
| **[Line](./line.md)** | Ordered polylines | x, y, stroke, stroke_width, stroke_dash, opacity, defined | Time series, trend lines, multi-series charts |
| **[Rect](./rect.md)** | Axis-aligned rectangles | x, x2, y, y2, fill, stroke, corner_radius | Bar charts, heatmaps, interval plots |

Each mark type has a dedicated page with detailed channel documentation and examples.

## Generic Over Coordinate Systems

Marks are generic structs parametrized by coordinate system (`Mark<C: CoordinateSystem>`). This design allows the same mark type to work across different coordinate systems while exposing coordinate-appropriate channels.

**Position channels** are specific to each coordinate system:
- **Cartesian**: `x`, `y` (and `x2`, `y2` for rectangles)
- **Polar**: `r`, `theta` (radial distance and angle)

**Visual channels** (fill, stroke, size, shape, opacity, etc.) are shared across all coordinate systems.

When you construct a mark inside `.mark()`, it inherits the plot's coordinate system:

```rust
Plot::<Cartesian>::new()
    .mark(Symbol::new().x(col("a")).y(col("b")))  // Infers Cartesian

Plot::<Polar>::new()
    .mark(Symbol::new().r(col("distance")).theta(col("angle")))  // Infers Polar
```

For standalone construction, use explicit type parameters:

```rust
Symbol::<Cartesian>::new()
Symbol::<Polar>::new()
```

### Coordinate System Support

| Mark | Cartesian | Polar | Future Systems |
|------|-----------|-------|----------------|
| **[Symbol](./symbol.md)** | ✅ `x`, `y` | ✅ `r`, `theta` | Planned |
| **[Line](./line.md)** | ✅ `x`, `y` | 🔮 Planned | Planned |
| **[Rect](./rect.md)** | ✅ `x`, `x2`, `y`, `y2` | 🔮 Planned (arc/wedge marks) | Planned |

Symbol marks currently work in both Cartesian and Polar coordinate systems. Line and Rect marks are Cartesian-only in the current release, with polar variants planned for future versions.

## Layering Marks

Marks can be layered to combine encodings:

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


let plot = Plot::<Cartesian>::new()
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

`Plot` merges scale and legend configuration across marks that use the same channel names, so both layers share the same axes and color legend.

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

let plot = Plot::<Cartesian>::new()
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

This example shows scatter points with a separate trend line, each using different data sources.

## Additional Mark Options

All marks share a common builder API:
- `.data(...)` attaches a dedicated `DataFrame`.
- `.facet_strategy(...)` and `.broadcast_to_facets()` control how marks participate in faceting.
- `.details([...])` stores additional fields for future tooltip/interaction layers (the values are carried through evaluation but no tooltip UI ships yet).
- `.zindex(...)` sets explicit draw order when layers overlap.

## Planned Marks

Text annotations, area charts, path-based marks, and rule markers are documented in [future-work/text-mark.md](../../docs/future-work/text-mark.md) and related roadmap notes. They are not part of the current release.

## Next Steps

- Explore [Channels](../channels.md) to see how marks receive data.
- Learn how [Scales](../scales/index.md) transform channel expressions.
- Review [Legends](../legends.md) for automatically generated guides.
