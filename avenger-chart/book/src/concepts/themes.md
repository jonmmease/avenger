# Themes

Themes control the visual presentation of a plot: typography, colors, grid lines, mark defaults, legend layout, and more. Avenger Chart themes are CSS-based, so selectors, cascading rules, and variables all work the way they do on the web.

## Built-in Themes

### Light (default)

`Theme::light()` returns the adaptive default theme with the light color scheme selected. A plot uses this theme automatically when no explicit theme is supplied.

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;


let plot = Plot::<Cartesian>::new()
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
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Dark

Switching to `Theme::dark()` selects the dark palette while keeping the same responsive CSS rules.

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;


let plot = Plot::<Cartesian>::new()
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
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Extending a Theme with CSS

You can append additional CSS to tweak specific elements. Because the API consumes a `Theme`, build and modify it before passing it into the plot.

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;

let mut theme = Theme::light();
theme.append_css(
    r#"
    legend background {
        fill: #f0f9ff;  /* Light blue background */
        stroke: #3b82f6;  /* Blue border */
        stroke-width: 2px;
        corner-radius: 8px;
        padding: 10px;
    }

    mark[type="symbol"] {
        stroke-width: 1.5px;
        stroke: #1e293b;
    }
    "#,
)?;


let plot = Plot::<Cartesian>::new()
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
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

`append_css` keeps previously appended styles, so you can layer multiple overrides if needed.
In the rendered example above the legend background is styled purely through CSS, so the mark configuration only declares the legend title.

## Loading a Theme from CSS

To start from scratch, construct a theme directly from a CSS string.

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;

let css = r#"
    canvas, plot, guide {
        background-color: #111827;
    }

    chart-title, chart-subtitle {
        color: #f8fafc;
    }

    axis label, axis title {
        color: #f8fafc;
    }

    axis domain, axis tick {
        stroke: #f8fafc;
    }

    axis grid {
        stroke: #334155;
    }

    legend label, legend title {
        color: #f8fafc;
    }

    legend background {
        fill: #0f172a;
        stroke: #f8fafc;
        stroke-width: 1px;
        padding: 8px;
    }

    mark[type="symbol"] {
        fill: #4ec9b0;
        stroke: #f8fafc;
        stroke-width: 1.5px;
    }
    "#;

let theme = Theme::from_css(css)?;

let plot = Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .title("Custom Theme from CSS")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .size(180.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
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
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

// Create simple data
let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "B", "C"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![25.0, 40.0, 35.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

let mut theme = Theme::light();
theme.append_css(
    r#"
    /* Universal mark selector sets stroke for all marks using !important */
    mark {
        stroke: #ff5722 !important;  /* Deep orange */
        stroke-width: 3px !important;
        fill: #14b8a6;  /* Teal */
    }

    /* Type-specific selector overrides fill (higher specificity wins for non-!important) */
    mark[type="symbol"] {
        fill: #f472b6;  /* Pink */
    }
    "#,
)?;

Plot::<Cartesian>::new()
    .theme(theme)
    .data(df.clone())
    .title("CSS !important and Specificity")
    .subtitle("Using !important to apply universal styles across mark types")
    .mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.3))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
    )
    .mark(
        Symbol::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.3))
            })
            .y(col("value"))
            .size(300.0)
    )
```

- CSS `!important`: The `mark` selector uses `!important` for stroke properties, allowing it to override higher-specificity rules.
- Without `!important`, the `mark[type="symbol"]` selector (higher specificity) would normally override the `mark` selector.
- The fill property doesn't use `!important`, so normal CSS specificity applies: symbols get pink fill from the type-specific rule.
- Result: both mark types get deep orange stroke from `!important`, rects keep teal fill, symbols get pink fill.

### Color Utilities

The CSS parser recognises modern color functions, so you can build palettes with `color-mix`, `hsl()`, `hsla()`, or `lab()`/`lch()` values:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;

let mut theme = Theme::light();
theme.append_css(
    r#"
    /* Modern color functions with transparency and mixing */
    mark[type="symbol"] {
        fill-discrete:
            hsla(200, 80%, 50%, 0.75),
            hsla(30, 90%, 55%, 0.75),
            hsla(160, 70%, 45%, 0.75);
        stroke: color-mix(in srgb, #1e293b 65%, white);
        stroke-width: 1.5px;
    }
    "#,
)?;


let plot = Plot::<Cartesian>::new()
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
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Media Queries and Responsive Styling

Because themes use standard CSS, you can respond to canvas size, device pixel ratio, or user-preference media queries. Combine media queries with parameters to render a single compiled plot at multiple breakpoints.

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;

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

let plot = Plot::<Cartesian>::new()
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
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The media queries above adjust the plot-area background as the canvas width changes. The rendered example shows the medium width breakpoint (green tint). See [Parameters](../advanced/parameters.md) for how to render the same compiled plot at multiple sizes.

## Next Steps

- See [Marks](./marks.md) to learn which channels a theme can style by default.
- Review [Legends](./legends.md) to customize guide appearance with CSS selectors.
- Explore [CSS Themes](../advanced/css-themes.md) for more comprehensive styling patterns and best practices.
