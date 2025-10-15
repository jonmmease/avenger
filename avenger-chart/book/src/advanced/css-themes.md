# Custom Themes with CSS

Avenger Chart supports CSS-based theming, allowing complete control over visual styling with familiar web standards.

## Why CSS Themes?

CSS theming provides:

- **Familiar syntax** for web developers
- **Cascading rules** for efficient styling
- **Attribute selectors** for targeting mark types
- **Descendant selectors** for styling sub-elements
- **CSS variables** with runtime parameter overrides
- **Modern CSS functions** like `light-dark()` and `color-mix()`

## Loading a CSS Theme

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = datafusion::execution::context::SessionContext::new();
let css = std::fs::read_to_string("my-theme.css")?;
let theme = Theme::from_css(&css)?;

let df = ctx
    .read_csv("data.csv", CsvReadOptions::default())
    .await?;

let plot = Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .mark(Symbol::new().x(col("x")).y(col("y")));
# Ok(())
# }
```

## Basic CSS Theme Structure

```css
/* Root-level variables and defaults */
:root {
    font-family: "Inter", sans-serif;
    --base-font-size: 12px;
    font-size: var(--base-font-size);

    /* Color palette */
    --bg-color: white;
    --text-color: black;
    --grid-color: #e0e0e0;
    --accent-color: #0072B2;
}

/* Canvas (outer container with margins) */
canvas {
    background-color: var(--bg-color);
    margin: 10px;
}

/* Plot area (the plotting region) */
plot {
    background-color: var(--bg-color);
}

/* Chart titles */
chart-title {
    color: var(--text-color);
    font-weight: 500;
    font-size: 1.5rem;
    text-align: left;
    width: canvas;  /* or: plot-area */
}

chart-subtitle {
    color: #666;
    font-weight: 200;
    font-size: 1.167rem;
    text-align: left;
    width: canvas;
}

/* Axis elements using descendant selectors */
axis domain {
    stroke: var(--text-color);
    stroke-width: 1.0;
}

axis tick {
    stroke: var(--text-color);
    size: 5.0;
}

axis title {
    color: var(--text-color);
    font-weight: 400;
    font-size: 1.0rem;
}

axis label {
    color: #666;
    font-weight: 300;
    font-size: 0.833rem;
    padding: 3;
}

axis grid {
    stroke: var(--grid-color);
    opacity: 0.5;
    stroke-width: 0.5;
}

/* Legend elements using descendant selectors */
legend {
    spacing: 10;
    label-padding: 5;
    columns: 1;
    label-limit: 0;  /* 0 means no limit */
}

legend title {
    color: var(--text-color);
    font-weight: 400;
    font-size: 1.0rem;
}

legend label {
    color: #666;
    font-weight: 300;
    font-size: 0.917rem;
}

legend tick {
    color: #666;
    font-weight: 300;
    font-size: 0.833rem;
    stroke: var(--text-color);
}

legend background {
    padding: 8;
}

/* Marks using attribute selectors */
mark[type="symbol"] {
    fill: var(--accent-color);
    stroke: var(--bg-color);
    stroke-width: 0.5;
    size: 72;
    shape: circle;
    opacity: 1.0;
}

mark[type="line"] {
    stroke: var(--accent-color);
    stroke-width: 2.0;
    stroke-dash: solid;
    stroke-cap: round;
    stroke-join: round;
    opacity: 1.0;
}

mark[type="rect"] {
    fill: var(--accent-color);
    stroke: var(--bg-color);
    stroke-width: 0.5;
    corner-radius: 0;
    opacity: 1.0;
}

mark[type="text"] {
    fill: var(--text-color);
    font-size: 1.0rem;
}
```

## Attribute Selectors for Mark Types

Use `mark[type="..."]` to target specific mark types:

```css
/* All marks */
mark {
    opacity: 1.0;
}

/* Only symbols */
mark[type="symbol"] {
    size: 100;
    shape: circle;
}

/* Only lines */
mark[type="line"] {
    stroke-width: 2.5;
    stroke-cap: round;
}

/* Only rectangles */
mark[type="rect"] {
    corner-radius: 4;
}

/* Only text marks */
mark[type="text"] {
    font-weight: 600;
}
```

## Descendant Selectors

Use space-separated selectors to target sub-elements:

### Axis Sub-elements

```css
/* Axis parts */
axis domain {      /* The axis line itself */
    stroke: black;
    stroke-width: 1.0;
}

axis tick {        /* Tick marks */
    stroke: black;
    size: 5.0;
}

axis title {       /* Axis title text */
    font-size: 1.2rem;
    font-weight: 600;
}

axis label {       /* Tick labels */
    font-size: 0.9rem;
    padding: 3;
}

axis grid {        /* Grid lines */
    stroke: #e0e0e0;
    stroke-width: 0.5;
    opacity: 0.5;
}
```

### Legend Sub-elements

```css
legend title {
    font-weight: 600;
    font-size: 1.1rem;
}

legend label {
    font-size: 0.9rem;
}

legend tick {
    font-size: 0.8rem;
    stroke: #666;
}

legend background {
    fill: white;
    stroke: #ccc;
    stroke-width: 1px;
    padding: 8;
    corner-radius: 4;
}
```

## Targeting Axis and Legend Types

Use attribute selectors to style specific axis or legend types:

### Axis Type Selectors

```css
/* Style x-axis specifically */
axis[type="x"] title {
    color: blue;
}

/* Style y-axis specifically */
axis[type="y"] title {
    color: red;
}

/* Grid lines for each axis type */
axis[type="x"] grid {
    stroke: #e0e0ff;
}

axis[type="y"] grid {
    stroke: #ffe0e0;
}
```

### Legend Type Selectors

```css
/* Symbol legends */
legend[type="symbol"] {
    symbol-size: 64;
}

/* Continuous color bar legends */
legend[type="colorbar"] {
    gradient-thickness: 15;
}

/* Rectangle legends */
legend[type="rect"] {
    symbol-size: 64;
}

/* Line legends */
legend[type="line"] {
    symbol-size: 64;
}
```

### Guide Type Selectors

Target coordinate systems (Cartesian vs Polar):

```css
/* Cartesian coordinate systems */
guide[type="cartesian"] {
    background-color: #f0f0f0;
}

guide[type="cartesian"] axis title {
    color: #1976d2;
}

/* Polar coordinate systems */
guide[type="polar"] {
    background-color: #fff3e0;
}

guide[type="polar"] axis title {
    color: #e65100;
}
```

## Discrete and Continuous Scale Ranges

Use `-discrete` and `-continuous` suffixes for scale range properties:

### Color Scales

```css
mark {
    /* Discrete color palette (categorical data) */
    fill-discrete: #0072B2, #E69F00, #009E73, #F0E442, #D55E00;

    /* Continuous color gradient (quantitative data) */
    fill-continuous: #440154, #3b528b, #21918c, #5ec962, #FDE725;

    /* Same for stroke */
    stroke-discrete: #0072B2, #E69F00, #009E73;
    stroke-continuous: #440154, #FDE725;
}
```

### Size Scales

```css
mark {
    /* Discrete size steps */
    size-discrete: 30, 80, 140, 200, 260;

    /* Continuous size range (min, max) */
    size-continuous: 30, 200;
}
```

### Shape Scales

```css
mark {
    /* Discrete shape palette */
    shape-discrete: circle, cross, diamond, square, star, triangle-up, wye, cushion;
}
```

### Stroke Properties

```css
mark {
    /* Discrete stroke dash patterns */
    stroke-dash-discrete: solid, dashed, dotted, long-dash, dash-dot, long-short, even-short, double-dash;

    /* Discrete stroke width steps */
    stroke-width-discrete: 0.5, 1.0, 2.0, 3.0, 5.0;
}
```

### Opacity Scales

```css
mark {
    /* Discrete opacity steps */
    opacity-discrete: 0.2, 0.4, 0.6, 0.8, 1.0;

    /* Continuous opacity range */
    opacity-continuous: 0.1, 1.0;
}
```

## CSS Variables and Runtime Parameters

Define variables in `:root` and override them at runtime:

```css
:root {
    --primary-color: #0072B2;
    --secondary-color: #E69F00;
    --base-font-size: 12px;
}

mark[type="symbol"] {
    fill: var(--primary-color);
}

mark[type="line"] {
    stroke: var(--secondary-color);
}
```

Override variables at runtime with parameters. Parameters can be set on the `Plot` (as defaults) and then overridden at render time:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::common::ScalarValue;
# use indexmap::IndexMap;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = datafusion::execution::context::SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::default()).await?;
# let css = r#"
# :root { --primary-color: #0072B2; --base-font-size: 12px; }
# mark[type="symbol"] { fill: var(--primary-color); }
# "#;
let theme = Theme::from_css(css)?;

// Step 1: Define plot with default parameter values
let plot = Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .mark(Symbol::new().x(col("x")).y(col("y")))
    .add_param(Param::new("--primary-color", ScalarValue::from("#0072B2")))
    .add_param(Param::new("--base-font-size", ScalarValue::from("12px")));

// Step 2: Compile the plot
let compiled = plot.compile(&ctx).await?;

// Step 3: Render with default parameters
let result1 = compiled.evaluate(&ctx, None).await?;

// Step 4: Render again with overridden parameters
let mut custom_params = IndexMap::new();
custom_params.insert("--primary-color".to_string(), ScalarValue::from("#FF5733"));
custom_params.insert("--base-font-size".to_string(), ScalarValue::from("14px"));
let result2 = compiled.evaluate(&ctx, Some(custom_params)).await?;
# Ok(())
# }
```

This workflow allows you to compile a plot once and render it multiple times with different parameter values, which is efficient for interactive applications or generating multiple variants.

## Modern CSS Functions

### Adaptive Color Schemes with `light-dark()`

```css
:root {
    /* Define background and text that adapt to color scheme */
    --bg-color: light-dark(white, #121212);
    --text-color: light-dark(black, white);
}

canvas {
    background-color: var(--bg-color);
}

axis label {
    color: var(--text-color);
}
```

Set the color scheme at runtime:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::common::ScalarValue;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = datafusion::execution::context::SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::default()).await?;
# let theme = Theme::light();
let plot = Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .mark(Symbol::new().x(col("x")).y(col("y")))
    .add_param(Param::new("color-scheme", ScalarValue::from("dark")));  // Switch to dark mode
# Ok(())
# }
```

Or use built-in theme presets:

```rust,no_run
# use avenger_chart::prelude::*;
# fn example1() {
let light_theme = Theme::light();
# }

# fn example2() {
let dark_theme = Theme::dark();
# }
```

### Color Mixing with `color-mix()`

Generate semantic colors from base colors:

```css
:root {
    --bg-color: light-dark(white, #121212);
    --text-color: light-dark(black, white);

    /* Derived colors using color-mix */
    --grid-color: color-mix(in srgb, var(--bg-color) 88%, var(--text-color) 12%);
    --text-secondary: color-mix(in srgb, var(--text-color) 80%, var(--bg-color) 20%);
    --text-tertiary: color-mix(in srgb, var(--text-color) 65%, var(--bg-color) 35%);
    --border-color: color-mix(in srgb, var(--text-color) 90%, var(--bg-color) 10%);
}

axis grid {
    stroke: var(--grid-color);
}

axis label {
    color: var(--text-tertiary);
}

legend label {
    color: var(--text-secondary);
}
```

## Complete Theme Examples

### Professional Light Theme

```css
:root {
    font-family: "Inter", -apple-system, sans-serif;
    --base-font-size: 12px;
    font-size: var(--base-font-size);

    /* Okabe-Ito color palette (colorblind-friendly) */
    --categorical-colors: #0072B2, #E69F00, #009E73, #F0E442, #D55E00, #56B4E9, #CC79A7, #999999;
    --viridis-colors: #440154, #3b528b, #21918c, #5ec962, #FDE725;

    --bg-color: white;
    --text-color: black;
    --grid-color: #e5e5e5;
}

canvas {
    background-color: var(--bg-color);
    margin: 10px;
}

chart-title {
    color: var(--text-color);
    font-weight: 500;
    font-size: 1.5rem;
}

axis domain {
    stroke: var(--text-color);
    stroke-width: 1.0;
}

axis title {
    font-weight: 400;
    font-size: 1.0rem;
}

axis label {
    color: #666;
    font-size: 0.833rem;
}

axis grid {
    stroke: var(--grid-color);
    opacity: 0.5;
}

legend title {
    font-weight: 400;
    font-size: 1.0rem;
}

mark {
    fill-discrete: var(--categorical-colors);
    fill-continuous: var(--viridis-colors);
    stroke-discrete: var(--categorical-colors);
    size-continuous: 30, 200;
}

mark[type="symbol"] {
    size: 72;
    shape: circle;
}

mark[type="line"] {
    stroke-width: 2.0;
    stroke-cap: round;
}
```

### Dark Theme

```css
:root {
    --base-font-size: 12px;
    font-size: var(--base-font-size);

    --bg-color: #1e1e1e;
    --text-color: #e0e0e0;
    --grid-color: #404040;
    --categorical-colors: #4fc3f7, #ffb74d, #4db6ac, #fff59d, #ff8a65;
}

canvas {
    background-color: var(--bg-color);
}

plot {
    background-color: #252525;
}

chart-title {
    color: var(--text-color);
}

axis domain {
    stroke: var(--text-color);
}

axis label {
    color: #b0b0b0;
}

axis grid {
    stroke: var(--grid-color);
    opacity: 0.3;
}

legend background {
    fill: #2a2a2a;
    stroke: #404040;
    padding: 8;
    corner-radius: 4;
}

mark {
    fill-discrete: var(--categorical-colors);
}
```

### Adaptive Theme (Light/Dark)

```css
:root {
    --base-font-size: 12px;

    /* Adaptive colors */
    --bg-color: light-dark(white, #121212);
    --text-color: light-dark(black, white);

    /* Derived colors */
    --grid-color: color-mix(in srgb, var(--bg-color) 88%, var(--text-color) 12%);
    --text-secondary: color-mix(in srgb, var(--text-color) 80%, var(--bg-color) 20%);
    --border-color: color-mix(in srgb, var(--text-color) 90%, var(--bg-color) 10%);

    --categorical-colors: #0072B2, #E69F00, #009E73, #F0E442, #D55E00;
}

canvas {
    background-color: var(--bg-color);
}

axis domain {
    stroke: var(--border-color);
}

axis label {
    color: var(--text-secondary);
}

axis grid {
    stroke: var(--grid-color);
}

mark {
    fill-discrete: var(--categorical-colors);
}
```

Use with runtime parameter:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::common::ScalarValue;
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = datafusion::execution::context::SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::default()).await?;
# let css = r#"
# :root {
#     --bg-color: light-dark(white, #121212);
#     --text-color: light-dark(black, white);
# }
# canvas { background-color: var(--bg-color); }
# "#;
let theme = Theme::from_css(css)?;

// Light mode (default)
let plot_light = Plot::<Cartesian>::new()
    .theme(theme.clone())
    .data(df.clone())
    .mark(Symbol::new().x(col("x")).y(col("y")));

// Dark mode via parameter
let plot_dark = Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .mark(Symbol::new().x(col("x")).y(col("y")))
    .add_param(Param::new("color-scheme", ScalarValue::from("dark")));
# Ok(())
# }
```

## Selector Reference

### Top-Level Elements

| Selector | Description |
|----------|-------------|
| `canvas` | Outer container with margins |
| `plot` | Plot area background |
| `chart-title` | Main title |
| `chart-subtitle` | Subtitle |

### Mark Type Selectors

| Selector | Description |
|----------|-------------|
| `mark` | All marks |
| `mark[type="symbol"]` | Symbol marks (scatter plots) |
| `mark[type="line"]` | Line marks |
| `mark[type="rect"]` | Rectangle marks (bar charts) |
| `mark[type="text"]` | Text marks |
| `mark[type="arc"]` | Arc marks (pie/donut charts) |
| `mark[type="area"]` | Area marks |
| `mark[type="rule"]` | Rule marks (reference lines) |

### Axis Selectors

| Selector | Description |
|----------|-------------|
| `axis` | All axes |
| `axis[type="x"]` | X-axis only |
| `axis[type="y"]` | Y-axis only |
| `axis domain` | Axis line |
| `axis tick` | Tick marks |
| `axis title` | Axis title |
| `axis label` | Tick labels |
| `axis grid` | Grid lines |

### Legend Selectors

| Selector | Description |
|----------|-------------|
| `legend` | All legends |
| `legend[type="symbol"]` | Symbol legends |
| `legend[type="colorbar"]` | Continuous color legends |
| `legend[type="rect"]` | Rectangle legends |
| `legend[type="line"]` | Line legends |
| `legend title` | Legend title |
| `legend label` | Legend item labels |
| `legend tick` | Color bar tick labels |
| `legend background` | Legend background box |

### Guide Selectors

| Selector | Description |
|----------|-------------|
| `guide[type="cartesian"]` | Cartesian coordinate systems |
| `guide[type="polar"]` | Polar coordinate systems |

## Property Reference

### Color Properties

| Property | Values | Applies To |
|----------|--------|------------|
| `fill` | color | Marks |
| `fill-discrete` | color list | Marks (categorical scales) |
| `fill-continuous` | color list | Marks (continuous scales) |
| `stroke` | color | Marks, axis elements |
| `stroke-discrete` | color list | Marks (categorical scales) |
| `stroke-continuous` | color list | Marks (continuous scales) |
| `color` | color | Text elements |
| `background-color` | color | Canvas, plot, guides |

### Size Properties

| Property | Values | Applies To |
|----------|--------|------------|
| `size` | number | Symbols |
| `size-discrete` | number list | Symbol scales (categorical) |
| `size-continuous` | min, max | Symbol scales (continuous) |
| `stroke-width` | number | Lines, strokes |
| `stroke-width-discrete` | number list | Stroke width scales |
| `font-size` | size (px, rem, em) | Text |

### Shape Properties

| Property | Values | Applies To |
|----------|--------|------------|
| `shape` | circle, cross, diamond, square, star, triangle-up, wye, cushion | Symbols |
| `shape-discrete` | shape list | Symbol scales |
| `corner-radius` | number | Rectangles, legend backgrounds |

### Stroke Properties

| Property | Values | Applies To |
|----------|--------|------------|
| `stroke-dash` | solid, dashed, dotted, long-dash, dash-dot, etc. | Lines |
| `stroke-dash-discrete` | pattern list | Line scales |
| `stroke-cap` | round, square, butt | Lines |
| `stroke-join` | round, bevel, miter | Lines |

### Layout Properties

| Property | Values | Applies To |
|----------|--------|------------|
| `margin` | number or quad | Canvas |
| `padding` | number or quad | Legend background, labels |
| `spacing` | number | Legend items |
| `label-padding` | number | Legend |
| `columns` | number | Legend |
| `width` | canvas, plot-area | Titles |
| `text-align` | left, center, right | Titles |

### Other Properties

| Property | Values | Applies To |
|----------|--------|------------|
| `opacity` | 0.0 - 1.0 | Marks |
| `opacity-discrete` | number list | Opacity scales |
| `opacity-continuous` | min, max | Opacity scales |
| `font-family` | font name | Text |
| `font-weight` | 100-900 | Text |
| `label-limit` | number (0 = no limit) | Legend |
| `gradient-thickness` | number | Color bar legends |
| `symbol-size` | number | Legends |

## Next Steps

- Learn about [Parameters](./parameters.md) for dynamic theming
- Explore the default [Themes](../concepts/themes.md)
