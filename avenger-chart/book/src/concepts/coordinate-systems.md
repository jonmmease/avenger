# Coordinate Systems

Coordinate systems define how data values map to visual positions. Avenger Chart's architecture makes coordinate systems a first-class concept, allowing the same mark types to work across different coordinate systems.

## The Cartesian System

The most common coordinate system uses rectangular x/y coordinates:

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
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("temperature"))
            .y(col("humidity"))
            .size(200.0)
            .fill("#3498db")
    );

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
```

### Position Channels

- `x` - Horizontal position
- `y` - Vertical position

### Scale Types

- Linear scales (default for numeric data)
- Logarithmic scales (via `scale_with::<Log>`)
- Time scales (for temporal columns)
- Point or band scales for categorical data (mark-dependent: scatter plots use point scales, rect/interval marks use band scales)

## The Polar System

Polar coordinates use radius and angle:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create polar data - circular pattern
let batch = RecordBatch::try_from_iter(vec![
    (
        "radius",
        Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "angle",
        Arc::new(Float64Array::from(vec![
            0.0, 0.785, 1.571, 2.356, 3.142, 3.927, 4.712, 5.498
        ]))  // 0, π/4, π/2, 3π/4, π, 5π/4, 3π/2, 7π/4 radians
            as datafusion::arrow::array::ArrayRef,
    ),
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");


let plot = Plot::<Polar>::new()
    .data(df)
    .mark(
        Symbol::<Polar>::new()
            .r(col("radius"))
            .theta(col("angle"))
            .size(200.0)
            .fill("#e74c3c")
    );

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
```

### Position Channels

- `r` - Radial distance from origin
- `theta` - Angular position (in radians)

### Common Uses

- Radial scatter plots and circular distributions using `Symbol` marks
- Planned extensions (arc, radial bar, radar charts) tracked in [future work](../../docs/future-work/README.md)

## Coordinate System Design

Avenger Chart's separation of coordinate systems from marks provides several benefits:

Marks inherit the plot's coordinate system when supplied to `.mark()`, so they can surface channels that only exist in that system (e.g., `r`/`theta` for polar). You can still construct `Symbol::<Cartesian>::new()` explicitly when building marks standalone.

### 1. Reusable Marks

The same mark type works in multiple coordinate systems. Here's the same dataset visualized in both Cartesian and Polar coordinates:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Angular data: 8 points around a circle
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 0.707, 0.0, -0.707, -1.0, -0.707, 0.0, 0.707]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![0.0, 0.707, 1.0, 0.707, 0.0, -0.707, -1.0, -0.707]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

// Same data, Cartesian coordinates
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .size(300.0)
            .fill("#9b59b6")
    );

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
```

The same `Symbol` mark can render in different coordinate systems by using system-specific position channels.

### 2. Type Safety

The coordinate system is enforced at compile time. You cannot use polar-specific channels like `theta` in a Cartesian plot, or Cartesian channels like `x` in a Polar plot. This prevents runtime errors from coordinate system mismatches.

### 3. Zero-Dimensional Callouts

Zero-dimensional coordinates collapse all positional channels so marks render at a single point. This is useful for KPI tiles, compact dashboards, or legend-style galleries where only the visual encodings matter:

```rust,render,ignore
use avenger_chart::prelude::*;
use avenger_chart::zerod::ZeroDCoord;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

// Revenue data by business unit
let batch = RecordBatch::try_from_iter(vec![
    (
        "unit",
        Arc::new(StringArray::from(vec!["Sales", "Marketing", "Engineering", "Operations"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "revenue",
        Arc::new(Float64Array::from(vec![1200.0, 800.0, 1500.0, 950.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");


let plot = Plot::<ZeroDCoord>::new()
    .title("Revenue by Business Unit")
    .data(df)
    .mark(
        Symbol::new()
            .size_with(col("revenue"), |c| {
                c.scale(|s| s.range_interval(lit(200.0), lit(800.0)))
                    .legend(|l| l.title("Revenue ($)"))
            })
            .fill_with(col("unit"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Business Unit"))
            })
            .stroke("#2c3e50")
            .stroke_width(2.0)
    );

let compiled = plot.compile(&ctx).await.expect("compile");
let evaluated = compiled.evaluate(&ctx, None).await.expect("evaluate");
evaluated
```

Even though everything renders at the centre of the plot, the mark still encodes data via size, color, shape, legends, and tooltips. This is perfect for dashboard KPIs or legend-style visualizations.

## Cartesian vs Polar Comparison

The same visualization pattern works across coordinate systems by changing position channels:

| Aspect | Cartesian | Polar |
|--------|-----------|-------|
| Position channels | `x`, `y` | `r`, `theta` |
| Plot declaration | `Plot::<Cartesian>::new()` | `Plot::<Polar>::new()` |
| Mark example | `Symbol::new().x(col("a")).y(col("b"))` | `Symbol::<Polar>::new().r(col("radius")).theta(col("angle"))` |
| Common uses | Scatter plots, bar charts, line charts | Radial plots, circular distributions, wind roses |

## Future Coordinate Systems

The following coordinate systems are planned for future releases:

- **Multi-Dimensional** - For parallel coordinates and advanced radar charts
- **Sankey** - For flow diagrams with topological positioning
- **Hierarchical** - For treemaps and sunburst charts

See the [future work documentation](../../docs/future-work/README.md) for details.

## Next Steps

- Learn about [Marks](./marks.md) that render in coordinate systems
- Understand [Channels](./channels.md) for encoding data
- Explore [Scales](./scales.md) for data transformations
