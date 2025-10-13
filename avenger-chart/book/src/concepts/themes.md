# Themes

Themes control the visual presentation of a plot: typography, colors, grid lines, mark defaults, legend layout, and more. Avenger Chart themes are CSS-based, so selectors, cascading rules, and variables all work the way they do on the web.

## Built-in Themes

### Light (default)

`Theme::light()` returns the adaptive default theme with the light color scheme selected. A plot uses this theme automatically when no explicit theme is supplied.

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .theme(Theme::light())
    .data(df)
    .title("Light Theme (Default)")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(180.0)
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
    )
```

### Dark

Switching to `Theme::dark()` selects the dark palette while keeping the same responsive CSS rules.

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

Plot::<Cartesian>::new()
    .theme(Theme::dark())
    .data(df)
    .title("Dark Theme")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(180.0)
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
    )
```

## Extending a Theme with CSS

You can append additional CSS to tweak specific elements. Because the API consumes a `Theme`, build and modify it before passing it into the plot.

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

let mut theme = Theme::light();
theme.append_css(
    r#"
    legend background {
        fill: color-mix(in srgb, white 90%, black 10%);
        stroke: #64748b;
        corner-radius: 6;
        padding: 8;
    }

    symbol {
        stroke-width: 1.5px;
        stroke: #1e293b;
    }
    "#,
)?;

Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .title("Extended Theme with Custom CSS")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(200.0)
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
    )
```

`append_css` keeps previously appended styles, so you can layer multiple overrides if needed.

## Loading a Theme from CSS

To start from scratch, construct a theme directly from a CSS string.

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

let css = r#"
    :root {
        --bg-color: #1e1e1e;
        --text-color: #f5f5f5;
        --accent: #4ec9b0;
    }

    canvas {
        background-color: var(--bg-color);
    }

    axis label, axis title, plot title {
        color: var(--text-color);
    }

    axis line, axis tick, axis domain {
        stroke: var(--text-color);
    }

    symbol {
        fill: var(--accent);
        stroke: var(--text-color);
        stroke-width: 1px;
    }
    "#;

let theme = Theme::from_css(css)?;
Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .title("Custom Theme from CSS")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(180.0)
    )
```

Because the CSS parser runs at build time, syntax errors are surfaced immediately via the returned `Result`.

## Working with CSS Selectors

Themes leverage standard CSS selectors:

```css
:root {
    --accent: #2563eb;
}

legend[type="symbol"] {
    symbol-size: 72;
}

axis.x label {
    font-size: 1.1rem;
}

symbol.highlighted {
    fill: var(--accent);
    stroke-width: 2px;
}
```

- `:root` defines theme-wide variables (`--accent` above).
- Attribute selectors such as `legend[type="symbol"]` let you target specific legend renderers.
- Class selectors (`.highlighted`) apply to marks that set a matching CSS class during compilation.

Inspect the generated scene graph to discover additional selectors and properties; every guide, axis component, and mark exposes CSS hooks.

### Guide and Mark Subtypes

Plots annotate guides, marks, and legends with descriptive attributes. You can scope styles to the coordinate system or mark type without writing additional Rust code.

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

let mut theme = Theme::light();
theme.append_css(
    r#"
    /* Target all symbols in the plot */
    mark[type="symbol"] {
        stroke-width: 1.5px;
        stroke: #1e293b;
    }

    /* Cardinality-based palette - iris has 3 species */
    mark[type="symbol"][cardinality="3"] {
        fill-discrete: #f472b6, #ec4899, #be185d;
    }
    "#,
)?;

Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .title("Mark Type and Cardinality Selectors")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(200.0)
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
    )
```

- `guide[type="cartesian"]` and `guide[type="polar"]` let you style axes and backgrounds differently per coordinate system.
- `mark[type="symbol"]` scopes properties to a specific mark implementation. When Avenger Chart computes the number of discrete values flowing into a mark it also annotates `cardinality="N"`, enabling palette selection based on data size.

### Color Utilities

The CSS parser recognises modern color functions, so you can build palettes with `color-mix`, `hsl()`, `hsla()`, or `lab()`/`lch()` values:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

let mut theme = Theme::light();
theme.append_css(
    r#"
    /* Modern color functions with transparency and mixing */
    mark[type="symbol"][cardinality="3"] {
        fill-discrete:
            hsla(200, 80%, 50%, 0.75),
            hsla(30, 90%, 55%, 0.75),
            hsla(160, 70%, 45%, 0.75);
    }

    symbol {
        stroke: color-mix(in srgb, #1e293b 65%, white);
        stroke-width: 1.5px;
    }
    "#,
)?;

Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .title("Modern Color Functions")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(200.0)
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
    )
```

### Media Queries and Responsive Styling

Because themes use standard CSS, you can respond to canvas size, device pixel ratio, or user-preference media queries. Combine media queries with parameters to render a single compiled plot at multiple breakpoints.

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    .expect("load iris dataset");

let css = r#"
    /* Base styles */
    guide { background-color: transparent; }

    /* Small screens - blue tint */
    @media (width < 600px) {
        guide { background-color: rgba(59, 130, 246, 0.12); }
    }

    /* Medium screens - green tint */
    @media (width >= 600px) and (width < 1200px) {
        guide { background-color: rgba(34, 197, 94, 0.12); }
    }

    /* Large screens - red tint */
    @media (width >= 1200px) {
        guide { background-color: rgba(248, 113, 113, 0.12); }
    }
"#;

let theme = Theme::from_css(css)?;
Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .title("Responsive Styling with Media Queries")
    .subtitle("Plot area background changes based on width")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(150.0)
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
    )
```

The media queries above adjust the plot-area background as the canvas width changes. The rendered example shows the medium width breakpoint (green tint). See [Parameters](../advanced/parameters.md) for how to render the same compiled plot at multiple sizes.

## Next Steps

- See [Marks](./marks.md) to learn which channels a theme can style by default.
- Review [Legends](./legends.md) to customize guide appearance with CSS selectors.
- Explore [CSS Themes](../advanced/css-themes.md) for more comprehensive styling patterns and best practices.
