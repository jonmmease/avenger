# Channels

Channels map data to visual properties. Avenger Chart provides two ways to set channels: **direct values** and **data-driven encoding**.

## Direct Values

Set a channel to a constant value:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .x(col("temperature"))
    .y(col("humidity"))
    .fill("steelblue")    // All points are blue
    .size(100.0);         // All points are 100 square pixels
# }
```

Direct values work for both expressions and literals:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .x(col("x") + lit(10))  // Expression: shift x by 10
    .y(lit(50.0));          // Literal expression: still scaled
# }
```

Any `Expr` (from `col`, `lit`, arithmetic, etc.) is considered data to be scaled by the corresponding scales. Passing primitive literals (`"steelblue"`, `100.0`, `true`) sets raw values that bypass scaling.

## Data-Driven Encoding

Use `*_with()` methods to encode data with scales and legends:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use palette::Srgba;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .x(col("temperature"))
    .y(col("humidity"))
    .fill_with(col("region"), |c| {
        c.scale(|s| {
            s.range_colors(vec![
                Srgba::new(0.121, 0.466, 0.705, 1.0),
                Srgba::new(0.173, 0.627, 0.173, 1.0),
                Srgba::new(0.882, 0.470, 0.0, 1.0),
            ])
        })
        .legend(|l| l.title("Region"))
    })
    .size_with(col("population"), |c| {
        c.scale(|s| s.range_interval(lit(50.0), lit(500.0)))
            .legend(|l| l.title("Population"))
    });
# }
```

This creates:
- **Color encoding**: Maps region to colors via categorical scale
- **Size encoding**: Maps population to point sizes via a square-root scale (default for numeric sizes to better match perceived area—use `scale_with::<Linear>` if you need a linear mapping)
- **Legends**: Shows both encodings in the legend

## Conditional Encodings

Channels can branch on boolean expressions using the `when_value` and `when_scaled` helpers. They allow you to highlight subsets or swap in alternate scale inputs without rebuilding the chart.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use palette::Srgba;
# fn example() {
let highlight = when(col("status").eq(lit("error")), lit(true))
    .otherwise(lit(false))
    .unwrap();

let _symbol = Symbol::<Cartesian>::new()
    .x(col("x"))
    .y(col("y"))
    .fill_with(col("value"), |c| {
        c.when_value(highlight.clone(), lit("#ff6b6b"))
            .scale_with::<Linear>(|s| {
                s.range_colors(vec![
                    Srgba::new(0.121, 0.466, 0.705, 1.0),
                    Srgba::new(0.173, 0.627, 0.173, 1.0),
                ])
            })
            .legend(|l| l.title("Reading"))
    })
    .size_with(col("size"), |c| {
        c.when_scaled(highlight, col("size") * lit(1.5))
            .scale(|s| s.range_interval(lit(40.0), lit(160.0)))
    });
# }
```

- `when_value(condition, literal)` injects an immediate value whenever the boolean expression is true (bypassing the scale).
- `when_scaled(condition, expr)` swaps in an alternate expression that still flows through the configured scale.

Chain multiple calls to build ordered fallbacks—for example, mark errors red, warnings orange, and everything else use the base scale.

## Channel Types

### Position Channels

Control spatial location. These are **scaled** by default:

- `x`, `y` - Primary position (Cartesian)
- `x2`, `y2` - Secondary position for ranges
- `r`, `theta` - Polar coordinates

> Polar marks currently provide `r` and `theta`; range variants (`r2`, `theta2`) are not yet available.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _rect = Rect::new()
    .x(col("start_date"))
    .x2(col("end_date"))
    .y(col("task"));
# }
```

### Visual Channels

Control appearance. Like all channels, expressions (e.g., `col(...)` or other `Expr` values) are scaled by default, while plain primitive literals bypass scaling. Use the `_with` variants to configure or disable that scaling when needed:

- `fill` - Fill color
- `stroke` - Stroke color
- `size` - Symbol size or stroke width
- `opacity` - Transparency
- `shape` - Symbol shape
- `stroke_width` - Line width
- `stroke_dash` - Dash pattern

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .x(col("x"))
    .y(col("y"))
    .fill("rgba(70, 130, 180, 0.85)")
    .stroke("white")
    .stroke_width(1.0);
# }
```

## Scale Configuration

When using `*_with()`, you can configure the scale:

### Linear Scales

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .size_with(col("population"), |c| {
        c.scale(|s| s
            .domain_interval(lit(0.0), lit(1_000_000.0))
            .range_interval(lit(50.0), lit(500.0))
        )
    });
# }
```

### Categorical Scales

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use palette::Srgba;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .fill_with(col("category"), |c| {
        c.scale(|s| {
            s.range_colors(vec![
                Srgba::new(0.596, 0.306, 0.639, 1.0),
                Srgba::new(0.204, 0.596, 0.859, 1.0),
                Srgba::new(0.984, 0.604, 0.600, 1.0),
            ])
        })
    });
# }
```

### Log Scales

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .x_with(col("gdp"), |c| c.scale_with::<Log>(|s| s.base(10.0)));
# }
```

### Time Scales

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .x_with(col("date"), |c| c.scale_with::<Time>(|s| s));
# }
```

## Legends

Enable legends for encoded channels:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use palette::Srgba;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .fill_with(col("species"), |c| {
        c.scale(|s| {
            s.range_colors(vec![
                Srgba::new(0.204, 0.596, 0.859, 1.0),
                Srgba::new(0.984, 0.604, 0.600, 1.0),
                Srgba::new(0.169, 0.506, 0.337, 1.0),
            ])
        })
        .legend(|l| l.title("Species"))
    });
# }
```

To drive legends, scales, or channel expressions from runtime values, combine channels with [Parameters](../advanced/parameters.md). Parameters let you compile once and render with different thresholds, color overrides, or axis behaviours without rebuilding the plot.

Legends automatically display:
- Color scales (fill/stroke)
- Size scales
- Shape encodings (when implemented)

## Multiple Channels

Encode multiple channels from the same column:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .x(col("x"))
    .y(col("y"))
    .fill_with(col("temperature"), |c| {
        c.scale(|s| s
            .domain_interval(lit(0.0), lit(100.0))
            .range_interval(lit(0.0), lit(1.0))
        )
        .legend(|l| l.title("Temperature (color)"))
    })
    .size_with(col("temperature"), |c| {
        c.scale(|s| s
            .domain_interval(lit(0.0), lit(100.0))
            .range_interval(lit(50.0), lit(500.0))
        )
        .legend(|l| l.title("Temperature (size)"))
    });
# }
```

This creates redundant encoding where temperature controls both color and size.

## Expression Channels

Channels can use DataFusion expressions:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use palette::Srgba;
# fn example() -> Result<(), Box<dyn std::error::Error>> {
let status = when(col("value").gt(lit(100)), lit("high"))
    .otherwise(lit("low"))?;

let _symbol = Symbol::<Cartesian>::new()
    .x(col("x"))
    .y(col("y1") + col("y2"))
    .fill_with(status, |c| {
        c.scale(|s| {
            s.range_colors(vec![
                Srgba::new(0.8, 0.2, 0.2, 1.0),
                Srgba::new(0.2, 0.4, 0.8, 1.0),
            ])
        })
    });
# Ok(())
# }
```

## Literal vs. Column Values

**Key distinction**:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .x(col("date"))   // Expr -> scaled by default
    .y(50.0);         // Primitive literal -> bypasses scaling
# }
```

All channels follow the same rule: expressions are routed through scales unless you call `.no_scale()` or supply a primitive literal directly; plain primitives (strings, numbers, booleans) are treated as raw values.

## Next Steps

- Learn about [Scales](./scales.md) in detail
- Understand [Legends](./legends.md) configuration
- See channel examples in [Guides](../guides/scatter-plots.md)
