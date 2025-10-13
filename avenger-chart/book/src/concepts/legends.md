# Legends

Legends provide visual keys that explain how data is encoded in your visualization. Avenger Chart automatically generates legends for encoded channels.

## Enabling Legends

Configure legends via the channel builder closure with `.legend()`:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
            .size_with(col("petal_length"), |c| {
                c.scale(|s| s.range_interval(lit(80.0), lit(300.0)))
                    .legend(|l| l.title("Petal Length (cm)"))
            })
    )
```

This creates two legend entries: one for color (species) and one for size (petal length).

## Legend Titles

Customize legend titles with `.title()`:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .title("Custom Legend Titles")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(150.0)
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Iris Species"))  // Custom title
            })
    )
```

**Default**: If no title is provided, Avenger Chart uses the referenced column name when possible, or falls back to the channel name.

## Legend Types

### Categorical Legends

For discrete values, legends display colored markers for each category:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("petal_length"))
            .y(col("petal_width"))
            .size(180.0)
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
    )
```

Displays:
- Colored circles for each category
- Category labels
- Title

### Continuous Legends (Colorbars)

For quantitative color scales, legends display as gradient colorbars:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(180.0)
            .fill_with(col("petal_length"), |c| {
                c.scale_with::<Linear>(|s| s)
                    .legend(|l| l.title("Petal Length (cm)"))
            })
    )
```

Displays:
- Vertical color gradient bar
- Domain values at endpoints
- Title

### Size Legends

For symbol sizes, legends show circles at representative sizes:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .fill("#3498db")
            .size_with(col("petal_length"), |c| {
                c.scale(|s| s.range_interval(lit(80.0), lit(350.0)))
                    .legend(|l| l.title("Petal Length (cm)"))
            })
    )
```

Displays:
- Circles at representative sizes
- Corresponding data values
- Title

## Multiple Legends

When multiple channels have legends, they stack in the legend area:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
            .size_with(col("petal_length"), |c| {
                c.scale(|s| s.range_interval(lit(80.0), lit(280.0)))
                    .legend(|l| l.title("Petal Length"))
            })
            .shape_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species (shape)"))
            })
    )
```

This creates three separate legend sections stacked vertically on the right side.

## Legend Position

Control where legends appear with `.position()`:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(150.0)
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species").position(LegendPosition::Top))
            })
    )
```

**Available positions**:
- `Right` - Right side of the plot (default)
- `Left` - Left side of the plot
- `Top` - Above the plot
- `Bottom` - Below the plot

## Colorbar Styling

Continuous color encodings render as colorbars. You can customize their appearance:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(180.0)
            .fill_with(col("petal_length"), |c| {
                c.scale_with::<Linear>(|s| s)
                    .legend(|l| {
                        l.title("Petal Length (cm)")
                            .background_fill("#f3f4f6")
                            .background_stroke("#9ca3af")
                            .background_corner_radius(6.0)
                            .background_padding(10.0)
                            .gradient_thickness(18.0)
                    })
            })
    )
```

Colorbar options:
- `gradient_thickness` - Width/height of the gradient bar
- `background_fill` - Background color
- `background_stroke` - Border color
- `background_corner_radius` - Rounded corners
- `background_padding` - Padding around the gradient

## Selective Legends

Disable legends for specific channels with `.visible(false)`:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .title("Legend for Color Only")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species").visible(true))  // Show
            })
            .size_with(col("petal_length"), |c| {
                c.scale(|s| s.range_interval(lit(80.0), lit(280.0)))
                    .legend(|l| l.visible(false))  // Hide
            })
    )
```

Only the color legend appears; size variation is visible but not explained in the legend.

## Legend Formatting

### Number Formatting

For continuous scales, format legend values:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(180.0)
            .fill_with(col("petal_length"), |c| {
                c.scale_with::<Linear>(|s| s)
                    .legend(|l| {
                        l.title("Petal Length")
                            .format_number(".2f")  // Two decimal places
                    })
            })
    )
```

Format strings follow standard number formatting patterns (e.g., `.2f` for two decimal places, `.2s` for SI notation).

## Legend Ordering

Legends appear in the order channels are defined. Here's an example with color defined before size:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

// Color legend appears first, then size
Plot::<Cartesian>::new()
    .data(df)
    .title("Color First, Then Size")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
            .size_with(col("petal_length"), |c| {
                c.scale(|s| s.range_interval(lit(80.0), lit(280.0)))
                    .legend(|l| l.title("Petal Length"))
            })
    )
```

And here's the same plot with size defined before color:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

// Size legend appears first, then color
Plot::<Cartesian>::new()
    .data(df)
    .title("Size First, Then Color")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size_with(col("petal_length"), |c| {
                c.scale(|s| s.range_interval(lit(80.0), lit(280.0)))
                    .legend(|l| l.title("Petal Length"))
            })
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
    )
```

## Legend Styling

Legends inherit styling from the current theme. See [Themes](./themes.md) for customization options.

## Interactive Legends

**Planned feature**: Click legend entries to filter data or highlight marks.

## Hiding All Legends

Global legend toggles are not yet available; hide legends per-channel via `.legend(|l| l.visible(false))`.

## Next Steps

- Learn about [Themes](./themes.md) for complete styling control
- See legend examples in [Scatter Plots](../guides/scatter-plots.md)
- Explore [CSS Themes](../advanced/css-themes.md) for advanced customization
