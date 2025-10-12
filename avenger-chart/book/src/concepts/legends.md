# Legends

Legends provide visual keys that explain how data is encoded in your visualization. Avenger Chart automatically generates legends for encoded channels.

## Enabling Legends

Configure legends via the channel builder closure:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use palette::Srgba;
# fn example() {
let _plot = Plot::<Cartesian>::new().mark(
    Symbol::new()
        .x(col("gdp"))
        .y(col("life_expectancy"))
        .fill_with(col("continent"), |c| {
            c.scale(|s| {
                s.range_colors(vec![
                    Srgba::new(0.121, 0.466, 0.705, 1.0),
                    Srgba::new(0.173, 0.627, 0.173, 1.0),
                    Srgba::new(0.882, 0.470, 0.0, 1.0),
                ])
            })
            .legend(|l| l.title("Continent"))
        })
        .size_with(col("population"), |c| {
            c.scale(|s| {
                s.domain_interval(lit(0.0), lit(1_000_000.0))
                    .range_interval(lit(50.0), lit(500.0))
            })
            .legend(|l| l.title("Population"))
        }),
);
# }
```

This creates two legend entries: one for color and one for size.

## Legend Titles

Customize legend titles:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use palette::Srgba;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .fill_with(col("continent"), |c| {
        c.scale(|s| {
            s.range_colors(vec![
                Srgba::new(0.596, 0.306, 0.639, 1.0),
                Srgba::new(0.204, 0.596, 0.859, 1.0),
                Srgba::new(0.984, 0.604, 0.600, 1.0),
            ])
        })
        .legend(|l| l.title("Geographic Region"))
    });
# }
```

**Default**: If no title is provided, Avenger Chart uses the referenced column name when possible, or falls back to the channel name.

## Legend Types

### Categorical Legends

For discrete values:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use palette::Srgba;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .fill_with(col("species"), |c| {
        c.scale(|s| {
            s.range_colors(vec![
                Srgba::new(0.121, 0.466, 0.705, 1.0),
                Srgba::new(0.173, 0.627, 0.173, 1.0),
                Srgba::new(0.882, 0.470, 0.0, 1.0),
            ])
        })
        .legend(|l| l.visible(true))
    });
# }
```

Displays:
- Colored squares/circles for each category
- Category labels

### Continuous Legends

For quantitative scales:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use palette::Srgba;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .fill_with(col("temperature"), |c| {
        c.scale(|s| s
            .domain_interval(lit(0.0), lit(100.0))
            .range_colors(vec![
                Srgba::new(0.267, 0.004, 0.329, 1.0),
                Srgba::new(0.255, 0.295, 0.741, 1.0),
                Srgba::new(0.993, 0.906, 0.144, 1.0),
            ])
        )
        .legend(|l| l.visible(true))
    });
# }
```

Displays:
- Color gradient bar
- Domain values at endpoints
- Optional tick marks

### Size Legends

For symbol sizes:

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
        .legend(|l| l.visible(true))
    });
# }
```

Displays:
- Circles at representative sizes
- Corresponding data values

## Legend Position

Control where legends appear:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# use palette::Srgba;
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let _plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .fill_with(col("category"), |c| {
                c.scale(|s| s.range_colors(vec![
                    Srgba::new(0.121, 0.466, 0.705, 1.0),
                    Srgba::new(0.173, 0.627, 0.173, 1.0),
                    Srgba::new(0.882, 0.470, 0.0, 1.0),
                ]))
                .legend(|l| l.title("Category"))
            })
    )
    .legend("fill", |legend| legend.position(LegendPosition::Right));
# Ok(())
# }
```

**Available positions**:
- `Right` - Right side of the plot (default)
- `Left` - Left side of the plot
- `Top` - Above the plot
- `Bottom` - Below the plot

### Colorbar Legends

Continuous color encodings render as colorbars. You configure them the same way as other legends, but the `Legend` builder also exposes colorbar-specific options such as background, padding, and gradient thickness.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("matrix.csv", CsvReadOptions::new()).await?;
let _plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("temperature"), |c| {
                c.scale(|s| s)
                    .legend(|l| {
                        l.title("Temperature (°C)")
                            .position(LegendPosition::Left)
                            .background_fill("#f3f4f6")
                            .background_stroke("#9ca3af")
                            .background_corner_radius(6.0)
                            .background_padding(10.0)
                            .gradient_thickness(18.0)
                    })
            })
            .size(90.0),
    );
# Ok(())
# }
```

Colorbars honor the same four positions (`Top`, `Right`, `Bottom`, `Left`) and inherit typography from the theme's `legend[type="colorbar"]` selector. Use `gradient_thickness` to adjust their width/height, and `background_*` properties to add cards or gutters behind the bar.

## Multiple Legends

When multiple channels have legends, they stack in the legend area:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .x(col("x"))
    .y(col("y"))
    .fill_with(col("species"), |c| c.legend(|l| l.title("Species")))
    .shape_with(col("gender"), |c| c.legend(|l| l.title("Gender")))
    .size_with(col("weight"), |c| c.legend(|l| l.title("Weight (kg)")));
# }
```

This creates three separate legend sections.

## Legend Formatting

### Number Formatting

For continuous scales:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
# let _symbol =
Symbol::<Cartesian>::new()
    .fill_with(col("value"), |c| {
        c.scale_with::<Linear>(|s| s)
            .legend(|l| l.visible(true).format_number(".2f"))
    });
# }
```

Format strings follow the same patterns as axis labels.

### Custom Labels

Legend label customization is planned but not yet exposed.


## Legend Styling

Legends inherit styling from the current theme:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let _plot = Plot::<Cartesian>::new()
    .theme(Theme::dark())
    .data(df)
    .mark(
        Symbol::new()
            .fill_with(col("category"), |c| c.legend(|l| l.visible(true)))
    );
# Ok(())
# }
```

See [Themes](./themes.md) for customization options.

## Selective Legends

Disable legends for specific channels:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .x(col("x"))
    .y(col("y"))
    .fill_with(col("category"), |c| c
        .legend(|l| l.visible(true))      // Show legend
    )
    .size_with(col("value"), |c| c
        .legend(|l| l.visible(false))     // Hide legend
    );
# }
```

## Legend Ordering

Legends appear in the order they're defined:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
// Color legend appears first, then size
# fn example1() {
let _symbol = Symbol::<Cartesian>::new()
    .fill_with(col("species"), |c| c.legend(|l| l.visible(true)))
    .size_with(col("weight"), |c| c.legend(|l| l.visible(true)));
# }

// Size legend appears first, then color
# fn example2() {
let _symbol = Symbol::<Cartesian>::new()
    .size_with(col("weight"), |c| c.legend(|l| l.visible(true)))
    .fill_with(col("species"), |c| c.legend(|l| l.visible(true)));
# }
```

## Interactive Legends

**Planned feature**: Click legend entries to filter data or highlight marks.

## Legend Layout Options

Legend layout configuration will be surfaced alongside the future legend builder enhancements.


## Hiding All Legends

Global legend toggles are not yet available; hide legends per-channel via the builder (e.g. `legend(|l| l.visible(false))`).


## Next Steps

- Learn about [Themes](./themes.md) for complete styling control
- See legend examples in [Scatter Plots](../guides/scatter-plots.md)
- Explore [CSS Themes](../advanced/css-themes.md) for advanced customization
