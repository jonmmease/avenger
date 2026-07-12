# Guides, Axes, and Legends

Avenger Chart organizes visual reference elements into three distinct concepts: **Guides**, **Axes**, and **Legends**. Understanding how these concepts relate helps you build effective visualizations and customize their appearance.

## The Three Concepts

### Guides

A **Guide** is the coordinate-system-specific visual reference structure. Each coordinate system has its own guide type that determines how reference elements are rendered:

- **Cartesian coordinates** → `CartesianGuide` (rectangular axes, grid lines, plot background)
- **Polar coordinates** → `PolarGuide` (radial and angular axes, circular grid, plot background)
- **Zero-Dimensional coordinates** → `NoGuide` (no spatial reference needed)

Guides **contain** axes and coordinate-specific options. They are responsible for rendering the visual framework that helps readers understand the coordinate space.

### Axes

**Axes** correspond to **single independent dimensions** within a coordinate system. They provide scales, tick marks, labels, and grid lines for position channels:

- **Cartesian**: `x` and `y` axes (horizontal and vertical dimensions)
- **Polar**: `r` and `theta` axes (radial distance and angular dimensions)
- **Zero-Dimensional**: no axes (0D space has no dimensions)

Axes are configured at the **channel level** when you encode position data, then collected into the coordinate system's guide.

### Legends

**Legends** provide visual keys for **non-position channels**—visual encodings that are independent of the coordinate system:

- `fill` (color)
- `size` (marker area or thickness)
- `shape` (marker shape)
- `stroke` (outline color)
- `opacity` (transparency)
- `stroke_width` (line thickness)
- `stroke_dash` (line pattern)

Legends work identically across all coordinate systems. They are configured at the **mark/channel level** and render in their own layout area (typically to the right or top of the plot).

## Conceptual Relationships

```
┌─────────────────────────────────────────────────────┐
│                    Plot                             │
│                                                     │
│  ┌──────────────────────┐     ┌─────────────────┐ │
│  │  Guide               │     │  Legends        │ │
│  │  (coordinate-system) │     │  (visual        │ │
│  │                      │     │   channels)     │ │
│  │  ┌────────────────┐  │     │                 │ │
│  │  │ Axis (x)       │  │     │  • fill         │ │
│  │  │ Axis (y)       │  │     │  • size         │ │
│  │  └────────────────┘  │     │  • shape        │ │
│  │                      │     │  • stroke       │ │
│  │  + plot background   │     │  • opacity      │ │
│  │  + grid lines        │     │  • ...          │ │
│  │  + coord options     │     │                 │ │
│  └──────────────────────┘     └─────────────────┘ │
│                                                     │
└─────────────────────────────────────────────────────┘
```

**Key insight**: Guides contain axes and coordinate-specific visuals. Legends are separate and coordinate-independent.

## Comparison with Grammar of Graphics

Avenger Chart's terminology differs from the standard **Grammar of Graphics** (Wilkinson, Wickham):

| Concept | Standard GoG | Avenger Chart |
|---------|-------------|---------------|
| **Guide** | Umbrella term for both axes and legends | Coordinate-system-specific visual reference (contains axes) |
| **Axis** | A type of guide (for position scales) | A dimension within a coordinate system's guide |
| **Legend** | A type of guide (for non-position scales) | Independent system for visual channels |

**Why the difference?**

Avenger Chart's architecture enables coordinate systems without traditional axes. For example, a future **geographic coordinate system** will have a guide (the map/continents) but **no axes**—latitude and longitude are implicit in the geographic projection. The standard GoG terminology (where "guide" means "axis or legend") doesn't accommodate this pattern cleanly.

By making guides coordinate-system-specific containers, Avenger Chart can support diverse coordinate systems while keeping legends consistent across all systems.

## Examples

### Cartesian Plot: Axes + Legend

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
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
        Arc::new(Float64Array::from(vec![2.0, 4.0, 3.0, 5.0, 4.0, 6.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "A", "B", "B", "C", "C"]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;

let plot = Chart::<Cartesian>::new()
    .title("Cartesian: x/y axes + fill legend")
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))           // → x axis in CartesianGuide
            .y(col("y"))           // → y axis in CartesianGuide
            .size(300.0)
            .fill_with(col("category"), |c| {  // → Legend (independent)
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Category"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

**Structure**:
- **Guide**: `CartesianGuide` with `x` and `y` axes, grid lines, rectangular clip
- **Legends**: Fill legend for category (color)

### Polar Plot: Different Axes, Same Legend

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    (
        "r",
        Arc::new(Float64Array::from(vec![1.0, 1.5, 2.0, 2.5, 3.0, 3.5]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "theta",
        Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "A", "B", "B", "C", "C"]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;

let plot = Chart::<Polar>::new()
    .title("Polar: r/theta axes + fill legend")
    .data(df)
    .mark(
        Symbol::new()
            .r(col("r"))           // → r axis in PolarGuide
            .theta(col("theta"))   // → theta axis in PolarGuide
            .size(300.0)
            .fill_with(col("category"), |c| {  // → Legend (independent)
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Category"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

**Structure**:
- **Guide**: `PolarGuide` with `r` (radial) and `theta` (angular) axes, circular grid, circular clip
- **Legends**: Fill legend for category (identical to Cartesian example)

**Key observation**: The **legend system is identical** across coordinate systems. Only the guide and axes change.

### Zero-Dimensional: No Guide, Legends Only

```rust,render
use avenger_chart::prelude::*;
use avenger_chart::zerod::ZeroDCoord;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
let batch = RecordBatch::try_from_iter(vec![
    (
        "value",
        Arc::new(Float64Array::from(vec![100.0, 200.0, 300.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "B", "C"]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;

let plot = Chart::<ZeroDCoord>::new()
    .title("Zero-D: No axes, legends only")
    .data(df)
    .mark(
        Symbol::new()
            // No position channels (0D space)
            .size_with(col("value"), |c| {
                c.scale(|s| s.range_interval(lit(100.0), lit(500.0)))
                    .legend(|l| l.title("Value"))
            })
            .fill_with(col("category"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Category"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

**Structure**:
- **Guide**: `NoGuide` (no axes, no grid, no spatial reference)
- **Legends**: Size and fill legends (visual channels only)

All marks render at the plot center. Only visual encodings vary.

## Future: Geographic Coordinates

Planned geographic coordinate systems will demonstrate the architectural advantage of separating guides from axes:

```rust
// Future API (not yet implemented)
let plot = Chart::<Geographic>::new()
    .guide(|g| g
        .projection(Projection::Mercator)
        .show_countries(true)
        .show_graticule(false)
    )
    .data(df)
    .mark(
        Symbol::new()
            .longitude(col("lon"))  // No axis - implicit in projection
            .latitude(col("lat"))   // No axis - implicit in projection
            .fill_with(col("population"), |c| {
                c.legend(|l| l.title("Population"))  // Legend still works
            })
    );
```

The **guide** (map projection + country outlines) provides spatial context without traditional axes. Legends remain independent.

## Configuration

### Configuring Guides

Guide options are coordinate-system-specific and configured at the **plot level**:

```rust
// Cartesian guide configuration
Plot::<Cartesian>::new()
    .guide(|g| g.plot_background_color("#f5f5f5"))

// Polar guide configuration
Plot::<Polar>::new()
    .guide(|g| g.plot_background_color("#fff8f0"))
```

See the [Axes](./guides-axes-legends/axes.md) guide for detailed axis configuration.

### Configuring Axes

Axes are configured at the **channel level** using `*_with()` methods:

```rust
Symbol::new()
    .x_with(col("temperature"), |c| {
        c.axis(|a| a
            .title("Temperature (°C)")
            .grid(true)
        )
    })
```

See the [Axes](./guides-axes-legends/axes.md) guide for comprehensive examples.

### Configuring Legends

Legends are configured at the **mark/channel level** within channel builders:

```rust
Symbol::new()
    .fill_with(col("species"), |c| {
        c.scale_with::<Ordinal>(|s| s)
            .legend(|l| l
                .title("Species")
                .position(LegendPosition::Top)
            )
    })
```

See the [Legends](./legends.md) page for complete legend documentation.

## Summary

| Concept | Scope | Contains | Configured At | Works With |
|---------|-------|----------|---------------|------------|
| **Guide** | Coordinate-system-specific | Axes + coord options | Plot level | One coordinate system |
| **Axis** | Single dimension | Scale, ticks, labels, grid | Channel level | Position channels |
| **Legend** | Visual channel | Color/size/shape key | Mark/channel level | All coordinate systems |

**Mental model**:
- Guides define the **spatial reference framework** (coordinate-dependent)
- Axes provide **dimensional scales** within that framework
- Legends explain **visual encodings** (coordinate-independent)

## Next Steps

- See [Axes](./guides-axes-legends/axes.md) for comprehensive axis configuration
- Read [Legends](./legends.md) for complete legend documentation
- Explore [Coordinate Systems](./coordinate-systems/index.md) to understand guide implementations
- Review [Themes](./themes.md) for styling guides, axes, and legends
