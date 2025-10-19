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

## Composition and Layering

Multiple marks can be combined in a single plot to create rich composite visualizations. See the **[Layering](./layering.md)** page for:

- How to layer marks with shared or independent data sources
- Scale and legend merging across layers
- Draw order control with `.zindex()`
- Common layering patterns (line + points, reference bands, multi-series comparisons)

## Planned Marks

Text annotations, area charts, path-based marks, and rule markers are documented in [future-work/text-mark.md](../../docs/future-work/text-mark.md) and related roadmap notes. They are not part of the current release.

## Next Steps

- Explore [Channels](../channels.md) to see how marks receive data.
- Learn how [Scales](../scales/index.md) transform channel expressions.
- Review [Legends](../legends.md) for automatically generated guides.
