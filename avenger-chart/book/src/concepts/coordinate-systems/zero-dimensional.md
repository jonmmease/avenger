# Zero-Dimensional Coordinates

The Zero-Dimensional coordinate system collapses all positional channels, rendering marks at a single point in the center of the plot. This enables visualizations where **only visual encodings matter**—perfect for KPI dashboards, legend-style galleries, and compact data callouts.

## Basic Example

```rust,render
use avenger_chart::prelude::*;
use avenger_chart::zerod::ZeroDCoord;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();
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
])?;

let df = ctx.read_batch(batch)?;


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

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Position Channels

**Zero-Dimensional coordinates have no position channels.** All marks render at the center of the plot area, with visual channels providing the only variation between data points.

## Declaration

Create a Zero-Dimensional plot using the type parameter:

```rust
use avenger_chart::zerod::ZeroDCoord;

let plot = Plot::<ZeroDCoord>::new()
    .data(df)
    .mark(Symbol::new().size(col("value")).fill(col("category")));
```

When constructing marks inside `.mark()` for a Zero-Dimensional plot, they inherit the coordinate system automatically.

## Use Cases

### Dashboard KPI Tiles

Zero-dimensional plots work well for dashboard tiles where you want legends and visual encodings without spatial positioning:

```rust
Plot::<ZeroDCoord>::new()
    .title("Sales Performance")
    .mark(
        Symbol::new()
            .size_with(col("sales"), |c| {
                c.scale(|s| s.range_interval(lit(100.0), lit(500.0)))
                    .legend(|l| l.title("Sales ($1000s)"))
            })
            .fill_with(col("region"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Region"))
            })
    )
```

The marks appear centered with legends providing the reading guide.

### Legend Galleries

Create legend-style visualizations showing all combinations of visual encodings:

```rust
Plot::<ZeroDCoord>::new()
    .mark(
        Symbol::new()
            .shape_with(col("shape_type"), |c| {
                c.legend(|l| l.title("Shape"))
            })
            .fill_with(col("color_category"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Color"))
            })
            .size(300.0)
    )
```

### Compact Data Callouts

Display multi-dimensional data in minimal space where position isn't meaningful:

```rust
Plot::<ZeroDCoord>::new()
    .title("Product Metrics")
    .mark(
        Symbol::new()
            .size_with(col("market_share"), |c| {
                c.legend(|l| l.title("Market Share %"))
            })
            .fill_with(col("growth_rate"), |c| {
                c.scale_with::<Linear>(|s| s)
                    .legend(|l| l.title("Growth Rate"))
            })
    )
```

## Visual Encodings Only

In Zero-Dimensional plots, all variation comes from visual channels:

- **`size`** – Area of symbols
- **`fill`** – Fill color
- **`stroke`** – Outline color
- **`stroke_width`** – Outline thickness
- **`shape`** – Symbol shape
- **`opacity`** – Transparency

These channels still support full scale transformations and legend generation, just like in positional coordinate systems.

## Details and Future Interactivity

The `.details(...)` method works in Zero-Dimensional plots, carrying additional data fields through evaluation for future tooltip and interaction systems:

```rust
Symbol::new()
    .size(col("value"))
    .fill(col("category"))
    .details(vec![col("description"), col("timestamp")])
```

While no tooltip UI currently ships, this lays groundwork for interactive dashboard widgets.

## Comparison with Positional Systems

| Aspect | Cartesian/Polar | Zero-Dimensional |
|--------|-----------------|------------------|
| Position channels | ✅ Required (`x`/`y` or `r`/`theta`) | ❌ None |
| Visual channels | ✅ Supported | ✅ Supported |
| Legends | ✅ Generated | ✅ Generated |
| Scales | ✅ Applied | ✅ Applied |
| Use case | Spatially positioned data | Visual encoding only |
| Marks render | At data-driven positions | At plot center |

## Next Steps

- Learn about [Cartesian](./cartesian.md) coordinates for standard rectangular plots
- Explore [Polar](./polar.md) coordinates for radial visualizations
- Review [Marks](../marks/index.md) that work in all coordinate systems
- See [Legends](../legends.md) for visual encoding guides
