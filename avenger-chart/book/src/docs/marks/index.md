# Marks

Marks are the fundamental visual building blocks of a plot, transforming data into geometric representations. This concept comes from the Grammar of Graphics (Wilkinson, 1999), popularized in visualization libraries like ggplot2. Marks turn channel encodings into geometric primitives—points, lines, rectangles—that appear on screen.

## How Marks Work

Marks consume data through **channels**, which bind data columns or expressions to visual properties. Each mark type defines which channels it accepts:

- **Position channels** determine **where** marks appear (e.g., `x` and `y` in Cartesian coordinates, `r` and `theta` in Polar)
- **Visual channels** control **appearance** (e.g., `fill` color, `size`, `stroke`, `opacity`, `shape`)

When you create a mark with channel encodings, Avenger Chart compiles these specifications into visual geometry. The data flows through: **Data** → **Channels** → **Mark** → **Visual Representation**.

## Basic Example

Here's a simple scatter plot demonstrating the mark pattern:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create sample data
let batch = RecordBatch::try_from_iter(vec![
    (
        "temperature",
        Arc::new(Float64Array::from(vec![15.0, 18.0, 22.0, 25.0, 28.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "humidity",
        Arc::new(Float64Array::from(vec![65.0, 70.0, 55.0, 50.0, 45.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("temperature"))   // Position channel (where)
            .y(col("humidity"))      // Position channel (where)
            .size(300.0)             // Visual channel (appearance)
            .fill("#4682b4")         // Visual channel (appearance)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates a Symbol mark with position channels (`x`, `y`) mapping data columns to positions and visual channels (`size`, `fill`) controlling appearance.

## Mark capability matrix

| Mark | Position channels & default scales | Visual channels & defaults | Notable limitations | Reference |
|------|------------------------------------|----------------------------|---------------------|-----------|
| **Symbol** | `x`, `y` (Linear in Cartesian); `r`, `theta` (Polar radial/angle). In ZeroD all marks collapse to the plot center. | `size` (numeric → Sqrt scale), `fill`/`stroke` (string → Ordinal, numeric → Linear), `shape`, `angle`, `stroke_width`, `opacity`. | Only mark that currently runs in Polar/ZeroD coordinates. Auto radius padding assumes Linear scales. | [Details](./symbol.md) |
| **Line** | `x`, `y` (Linear). Use the `order` channel to control drawing order; `defined` (boolean) creates gaps. | `stroke`, `stroke_width`, `stroke_dash`, `stroke_cap`, `stroke_join`, `stroke_opacity`, `opacity`. Defaults come from the theme (solid 2px line). | Requires array-backed columns (no scalar-only lines). Automatic padding currently supported for Linear scales only. | [Details](./line.md) |
| **Rect** | `x`, `x2`, `y`, `y2` (Linear for numeric intervals; Band for categorical axes inferred from data type). | `fill`, `stroke`, `stroke_width`, `corner_radius`, `opacity`. | Cartesian only. Band positioning expects matched `x`/`x2` (or `y`/`y2`) pairs—use `_with` helpers such as `.x2_with(col(":x"), |c| c.band(1.0))`. | [Details](./rect.md) |

Default scale choices come from `Scale::preferred_scale_type` implementations: numeric columns map to `Linear`, string columns to `Ordinal` (with `Nominal` available once the upstream scale lands), and temporal columns to `Time`. Numbers feeding the symbol `size` channel default to a square-root scale for perceptual reasons. You can always override with `scale_with::<Type>` when you need a different mapping.

Each mark page linked above contains exhaustive channel documentation and live examples.

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

## Common Capabilities

All marks share a common builder API with these key methods:

- **`.data(DataFrame)`** – Attach a dedicated data source to this mark, overriding plot-level data. Enables layering marks with different datasets.

- **`.zindex(i32)`** – Control draw order when marks overlap. Higher values draw on top. Useful when layering marks with specific stacking requirements.

- **`.details(Vec<Expr>)`** – Carry additional data fields through evaluation for future tooltip and interaction systems. Values are preserved but no tooltip UI currently ships.

- **`.facet_strategy(...)`** and **`.broadcast_to_facets()`** – Control how marks participate in faceted plots (data-driven subplots).

These capabilities work consistently across all mark types, enabling flexible composition patterns. See the [Layering](./layering.md) page for detailed examples.

## Composition and Layering

Multiple marks can be combined in a single plot to create rich composite visualizations. See the **[Layering](./layering.md)** page for:

- How to layer marks with shared or independent data sources
- Scale and legend merging across layers
- Draw order control with `.zindex()`
- Common layering patterns (line + points, reference bands, multi-series comparisons)

## Planned Marks

Text annotations, area charts, path-based marks, and rule markers are documented in [future-work/text-mark.md](../../docs/future-work/text-mark.md) and related roadmap notes. They are not part of the current release.

## Next Steps

- Explore [Channels](../channels/index.md) to see how marks receive data.
- Learn how [Scales](../scales/index.md) transform channel expressions.
- Review [Legends](../guides-axes-legends/legends.md) for automatically generated guides.
- Understand [Coordinate Systems](../coordinate-systems/index.md) for positional mapping.
