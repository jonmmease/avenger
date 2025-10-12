# Scales

Scales transform channel expressions from **data space** (the domain) into **visual space** (the range). Every encoded channel either uses a scale inferred from the data or one that you explicitly configure.

## Configuring Scales on Channels

Channel builders expose a `scale` method that receives a `Scale<Auto>` and returns the modified scale. The example below maps temperature readings into pixel coordinates with a custom domain and range.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _plot = Plot::<Cartesian>::new().mark(
    Symbol::new().x_with(col("temperature_c"), |c| {
        c.scale(|s| {
            s.domain_interval(lit(-40.0), lit(50.0))
                .range_interval(lit(0.0), lit(600.0))
        })
    }),
);
# }
```

When a domain is omitted, Avenger Chart infers it from the data referenced in the channel expression. Ranges default to sensible values for each channel (e.g., pixel spans for positional channels and categorical palettes for color).

## Continuous Scales

### Linear (default)

Linear scales are used automatically for numeric columns. You can opt into typed configuration to access helpers such as `zero`, `nice`, and `clamp`.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _plot = Plot::<Cartesian>::new().mark(
    Rect::new().x_with(col("value"), |c| {
        c.scale_with::<Linear>(|s| s.zero(true).nice(true))
    }),
);
# }
```

- `zero(true)` expands the inferred domain so that 0 is included.
- `nice(true)` rounds domain endpoints to human-friendly numbers.
- `clamp(true)` pins out-of-domain values to the range bounds.

### Logarithmic and Power Scales

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _plot = Plot::<Cartesian>::new().mark(
    Symbol::new().x_with(col("gdp_per_capita"), |c| {
        c.scale_with::<Log>(|s| s.base(10.0).clamp(true))
    }),
);
# }
```

`Scale::scale_with` also accepts `Pow`, `Sqrt`, and `Symlog` for other continuous transformations.

### Time Scales

Time scales understand temporal data and format axis ticks accordingly.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _plot = Plot::<Cartesian>::new().mark(
    Line::new().x_with(col("timestamp"), |c| {
        c.scale_with::<Time>(|s| s.nice(true))
    }),
);
# }
```

## Discrete Position Scales

Band and point scales arrange categorical positions. Use band scales with interval marks such as `Rect`, and point scales with glyph marks like `Symbol`.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _plot = Plot::<Cartesian>::new().mark(
    Rect::new().y_with(col("category"), |c| {
        c.scale_with::<Band>(|s| s.padding_inner(0.1).padding_outer(0.05))
    }),
);
# }
```

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _plot = Plot::<Cartesian>::new().mark(
    Symbol::new().x_with(col("group"), |c| {
        c.scale_with::<Point>(|s| s.padding(0.5))
    }),
);
# }
```

## Ordinal and Categorical Scales

Ordinal scales map discrete values to colors, shapes, or other non-positional encodings. Provide a list of colors via `range_colors`.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use palette::Srgba;
# fn example() {
let _plot = Plot::<Cartesian>::new().mark(
    Symbol::new().fill_with(col("species"), |c| {
        c.scale_with::<Ordinal>(|s| {
            s.range_colors(vec![
                Srgba::new(0.121, 0.466, 0.705, 1.0),
                Srgba::new(0.173, 0.627, 0.173, 1.0),
                Srgba::new(0.882, 0.470, 0.0, 1.0),
            ])
        })
        .legend(|l| l.title("Species"))
    }),
);
# }
```

For shapes (`.shape_with`) and dashes (`.stroke_dash_with`), pass string values that correspond to Vega symbol or dash names. Unknown inputs fall back to the specified `unknown` option if you set one on the scale.

## Size Scales

Size scales control the area of symbol marks. Because the channel value represents area in square pixels, a linear range produces perceptually balanced circles.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _plot = Plot::<Cartesian>::new().mark(
    Symbol::new().size_with(col("population"), |c| {
        c.scale_with::<Linear>(|s| {
            s.domain_interval(lit(1_000.0), lit(50_000.0))
                .range_interval(lit(64.0), lit(512.0))
        })
        .legend(|l| l.title("Population"))
    }),
);
# }
```

## Plot-Level Scale Configuration

You can configure a scale once at the plot level and let multiple marks inherit it.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let _plot = Plot::<Cartesian>::new()
    .data(df)
    .scale_with::<Linear>("x", |s| s.domain_interval(lit(0.0), lit(100.0)))
    .mark(Symbol::new().x(col("x")).y(col("y1")))
    .mark(Line::new().x(col("x")).y(col("y2")));
# Ok(())
# }
```

During compilation, Avenger Chart merges channel-level scale requests with plot-level configuration. When different marks refer to the same channel name (`"x"`, `"y"`, `"fill"`, etc.), the resulting scale is shared automatically.

## Additional Options

- `padding` / `padding_inner` / `padding_outer` – control spacing for band and point scales.
- `align` – align discrete categories within the available range.
- `unknown` – specify a fallback value for unmapped categories (ordinal scales).
- `domain_discrete` / `range_discrete` – provide explicit, ordered lists of domain or range values.

All options are available through the typed `Scale` methods or, for dynamic situations, via `Scale<Auto>::option` using the underlying property name.

## Next Steps

- Review [Channels](./channels.md) to understand how scale configuration fits into encoding.
- See [Legends](./legends.md) for how scale metadata is surfaced in guides.
- Explore the [Themes](./themes.md) chapter to control how scales are styled.
