# Coordinate Systems

Coordinate systems define how data values map to visual positions. Avenger Chart's architecture makes coordinate systems a first-class concept, allowing the same mark types to work across different coordinate systems.

## The Cartesian System

The most common coordinate system uses rectangular x/y coordinates:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        // Coordinate system can be inferred from the plot when passed via `.mark`
        Symbol::new()
            .x(col("temperature"))
            .y(col("humidity")),
    );
# Ok(())
# }
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

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let plot = Plot::<Polar>::new()
    .data(df)
    .mark(
        Symbol::<Polar>::new()
            .r(col("value"))
            .theta(col("angle")),
    );
# Ok(())
# }
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

The same mark type works in multiple coordinate systems. For example, `Symbol` works in both Cartesian and Polar:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn shared_mark_examples() {
let cartesian_plot = Plot::<Cartesian>::new()
    .mark(Symbol::new().x(col("x")).y(col("y")));

let polar_plot = Plot::<Polar>::new()
    .mark(Symbol::new().r(col("radius")).theta(col("angle")));
# let _ = (cartesian_plot, polar_plot);
# }
```

### 2. Type Safety

The coordinate system is enforced at compile time:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
// ✓ Valid - x is a Cartesian channel
let plot = Plot::<Cartesian>::new()
    .mark(Symbol::new().x(col("value")));
# let _ = plot;
```

```rust,compile_fail
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
// ✗ Compile error - theta is not valid for Cartesian
Plot::<Cartesian>::new().mark(Symbol::new().theta(col("angle")));
```

### 3. Zero-Dimensional Callouts

Zero-dimensional coordinates collapse all positional channels so marks render at a single point. This is useful for KPI tiles, compact dashboards, or legend-style galleries where only the visual encodings matter.

```rust,no_run
# use avenger_chart::prelude::*;
# use avenger_chart::zerod::ZeroDCoord;
# use datafusion::prelude::*;
# fn zerod_example(df: DataFrame) {
let _plot = Plot::<ZeroDCoord>::new()
    .title("Revenue by Unit")
    .data(df)
    .mark(
        Symbol::new()
            .size_with(col("revenue"), |c| c.legend(|l| l.title("Revenue")))
            .fill_with(col("unit"), |c| c.legend(|l| l.title("Business Unit")))
            .stroke_width(2.0)
    );
# }
```

Even though everything renders at the centre of the plot, the mark still encodes data via size, color, shape, legends, and tooltips.

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
