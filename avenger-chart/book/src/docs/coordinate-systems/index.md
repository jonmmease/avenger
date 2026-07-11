# Coordinate Systems

Coordinate systems define how data values map to visual positions. Avenger Chart's architecture makes coordinate systems a first-class concept, allowing the same mark types to work across different coordinate systems.

## Available Coordinate Systems

| System | Position Channels | Declaration | Common Uses |
|--------|-------------------|-------------|-------------|
| **[Cartesian](./cartesian.md)** | `x`, `y` | `Plot::<Cartesian>::new()` | Scatter plots, bar charts, line charts, heatmaps |
| **[Polar](./polar.md)** | `r`, `theta` | `Plot::<Polar>::new()` | Radial plots, circular distributions, wind roses |
| **[Zero-Dimensional](./zero-dimensional.md)** | (none) | `Plot::<ZeroDCoord>::new()` | KPI tiles, dashboard callouts, legend galleries |
| **[Concat Containers](./concat.md)** | child subplots | `Plot::with_coord(GridConcat::new())` | Dashboards, manual grids, wrapped galleries |
| **[Repeat Containers](./repeat.md)** | repeated child subplots | `Plot::with_coord(RepeatGrid::new())` | Scatterplot matrices, wrapped variable galleries |

Each coordinate system has a dedicated page with detailed documentation and examples.

## Coordinate System Design

Avenger Chart's separation of coordinate systems from marks provides several benefits:

### 1. Reusable Marks

The same mark type works in multiple coordinate systems. Marks inherit the plot's coordinate system when supplied to `.mark()`, so they can surface channels that only exist in that system (e.g., `r`/`theta` for polar, `x`/`y` for Cartesian).

You can still construct marks explicitly when building them standalone:
```rust
Symbol::<Cartesian>::new()  // Explicit Cartesian
Symbol::<Polar>::new()      // Explicit Polar
```

### 2. Type Safety

The coordinate system is enforced at compile time. You cannot use polar-specific channels like `theta` in a Cartesian plot, or Cartesian channels like `x` in a Polar plot. This prevents runtime errors from coordinate system mismatches.

### 3. Consistent API

Once you learn one coordinate system, the pattern applies to others. Just change the position channels and plot declaration—everything else (scales, legends, marks, themes) works identically.

## Comparison Example

The same visualization pattern works across coordinate systems by changing position channels:

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
?;

let df = ctx.read_batch(batch)?;

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

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The same `Symbol` mark can render in different coordinate systems by using system-specific position channels.

## Additional Coordinate Systems

The following coordinate systems live in sibling crates or are planned for
future releases:

- **Multi-Dimensional** - For parallel coordinates and advanced radar charts
- **Treemap** - Available from `avenger-chart-treemap`
- **Sankey** - For flow diagrams with topological positioning
- **Hierarchical** - Future shared foundations for sunburst, icicle, and
  related hierarchy charts

See the [roadmap](../../roadmap.md) for details.

## Next Steps

- Learn about **[Cartesian](./cartesian.md)** coordinates for standard rectangular plots
- Explore **[Polar](./polar.md)** coordinates for radial visualizations
- Discover **[Zero-Dimensional](./zero-dimensional.md)** coordinates for KPI dashboards and callouts
- Compose child plots with **[Concat Containers](./concat.md)**
- Generate scatterplot matrices and variable galleries with **[Repeat Containers](./repeat.md)**
- Understand how **[Guides, Axes, and Legends](./guides-axes-legends/index.md)** relate to coordinate systems
- Understand [Marks](../marks/index.md) that render in coordinate systems
- Review [Channels](../channels/index.md) for encoding data
