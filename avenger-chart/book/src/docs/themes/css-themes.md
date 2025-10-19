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

## Cardinality-Based Palette Selection

The `[cardinality="N"]` attribute selector allows you to define different color palettes based on the number of unique categories in your data. This powerful feature enables automatic palette optimization, ensuring you use appropriate color sets that minimize visual ambiguity and color cycling.

### Why Cardinality Matters

When visualizing categorical data with colors, the number of available colors should ideally match or exceed the number of categories. If your palette has fewer colors than categories, the ordinal scale will cycle back to the beginning, reusing colors for different categories and creating visual ambiguity.

Cardinality-based selectors solve this by letting you define optimal palettes for different data sizes:

```css
mark[type="symbol"] {
    /* Base palette - used as fallback */
    fill-discrete: red, blue, green, yellow, purple, orange;
}

/* Optimized 3-color palette for exactly 3 categories */
mark[type="symbol"][cardinality="3"] {
    fill-discrete: #E69F00, #56B4E9, #009E73;
}

/* Expanded 5-color palette for 5 categories */
mark[type="symbol"][cardinality="5"] {
    fill-discrete: #E69F00, #56B4E9, #009E73, #F0E442, #0072B2;
}

/* Large 10-color palette for high cardinality data */
mark[type="symbol"][cardinality="10"] {
    fill-discrete: #1f77b4, #ff7f0e, #2ca02c, #d62728, #9467bd,
                   #8c564b, #e377c2, #7f7f7f, #bcbd22, #17becf;
}
```

### Fallback Logic

When selecting a cardinality-specific palette, Avenger Chart uses intelligent fallback logic to minimize color cycling:

1. **Primary**: Use the palette with the **smallest cardinality >= requested**
   - Example: 4 categories with palettes [3, 5, 10] → selects 5-color palette (no cycling)

2. **Secondary**: If no palette >= requested exists, use the **largest cardinality < requested**
   - Example: 12 categories with palettes [3, 5, 10] → selects 10-color palette (minimal cycling)

3. **Tertiary**: If no cardinality-specific palettes exist, use the **base palette** (no `[cardinality]` attribute)

This logic ensures you always get the shortest possible palette that avoids or minimizes color cycling.

### Complete Example: Adaptive Color Palettes

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

let ctx = SessionContext::new();

// Create scatter plot data with 4 categories (Type A, B, C, D)
let data = vec![
    (1.0, 2.0, "Type A"),
    (2.0, 5.0, "Type B"),
    (3.0, 3.0, "Type C"),
    (4.0, 8.0, "Type D"),
    (5.0, 4.0, "Type A"),
    (6.0, 9.0, "Type B"),
    (7.0, 6.0, "Type C"),
    (8.0, 7.0, "Type D"),
];

let x_array = Float64Array::from(data.iter().map(|(x, _, _)| *x).collect::<Vec<_>>());
let y_array = Float64Array::from(data.iter().map(|(_, y, _)| *y).collect::<Vec<_>>());
let category_array = StringArray::from(data.iter().map(|(_, _, c)| *c).collect::<Vec<_>>());

let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(x_array) as _),
    ("y", Arc::new(y_array) as _),
    ("category", Arc::new(category_array) as _),
])?;

let df = ctx.read_batch(batch)?;

// Define CSS with cardinality-specific palettes
let css = r#"
    :root {
        font-family: "Inter", sans-serif;
        font-size: 12px;
    }

    canvas {
        background-color: white;
        margin: 15px;
    }

    chart-title {
        font-size: 1.3rem;
        font-weight: 500;
        color: #374151;
    }

    axis title {
        font-size: 1.0rem;
        font-weight: 400;
        color: #6b7280;
    }

    axis label {
        font-size: 0.9rem;
        color: #9ca3af;
    }

    axis grid {
        stroke: #e5e7eb;
        opacity: 0.5;
    }

    legend title {
        font-size: 1.0rem;
        font-weight: 500;
        color: #374151;
    }

    legend label {
        font-size: 0.9rem;
        color: #6b7280;
    }

    /* 2-category palette - high contrast red/blue */
    mark[type="symbol"][cardinality="2"] {
        fill-discrete: #e74c3c, #3498db;
        size: 150px;
    }

    /* 3-category palette - warm spectrum */
    mark[type="symbol"][cardinality="3"] {
        fill-discrete: #f39c12, #e67e22, #d35400;
        size: 150px;
    }

    /* 5-category palette - vibrant rainbow (SELECTED for 4 categories) */
    mark[type="symbol"][cardinality="5"] {
        fill-discrete: #E91E63, #9C27B0, #673AB7, #3F51B5, #2196F3;
        size: 150px;
    }

    /* Base palette - grayscale fallback */
    mark[type="symbol"] {
        fill-discrete: #2c3e50, #34495e, #7f8c8d, #95a5a6, #bdc3c7, #ecf0f1;
        size: 150px;
    }
"#;

let theme = Theme::from_css(css)?;

let plot = Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .title("Cardinality-Based Palette: 4 Categories → 5-Color Palette")
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| c.axis(|a| a.title("X Value")))
            .y_with(col("y"), |c| c.axis(|a| a.title("Y Value")))
            .fill_with(col("category"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Category"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

In this example, we have 4 unique categories but define palettes for 2, 3, and 5 categories. Since there's no exact match for cardinality=4, the fallback logic selects the 5-color palette (smallest >= 4), providing all 4 categories with distinct colors without cycling.

### Multi-Channel Cardinality

You can define cardinality-specific ranges for multiple channels independently:

```css
/* Different palettes for fill and stroke based on cardinality */

/* 3-category palettes */
mark[type="symbol"][cardinality="3"] {
    fill-discrete: #E69F00, #56B4E9, #009E73;
    stroke-discrete: #000000, #666666, #999999;
}

/* 5-category palettes */
mark[type="symbol"][cardinality="5"] {
    fill-discrete: #ffb3ba, #ffdfba, #ffffba, #baffc9, #bae1ff;
    stroke-discrete: #8b4513, #a0522d, #d2691e, #cd853f, #deb887;
}

/* Base palettes for any cardinality */
mark[type="symbol"] {
    fill-discrete: red, blue, green, yellow, purple, orange;
    stroke-discrete: black, gray, silver;
    stroke-width: 2px;
}
```

When a mark uses both `fill` and `stroke` with categorical data, each channel independently applies cardinality-based palette selection based on its own domain cardinality.

### Color Palette Design Guidelines

When designing cardinality-specific palettes, consider these best practices:

1. **Perceptual Distinctness**: Ensure colors are easily distinguishable
   - Small palettes (2-3): Use high contrast complementary colors
   - Medium palettes (4-7): Use perceptually uniform color spaces
   - Large palettes (8+): Consider colorblind-safe palettes

2. **Consistent Progression**: Maintain visual coherence across cardinalities
   ```css
   /* Build larger palettes by extending smaller ones */
   mark[cardinality="3"] {
       fill-discrete: #E69F00, #56B4E9, #009E73;
   }

   mark[cardinality="5"] {
       /* Extends the 3-color palette with 2 more colors */
       fill-discrete: #E69F00, #56B4E9, #009E73, #F0E442, #0072B2;
   }
   ```

3. **Semantic Meaning**: Use color progression that matches data semantics
   - Ordinal data: Use gradients or related hues
   - Nominal data: Use distinct hues with similar lightness/saturation

4. **Base Palette Safety**: Always define a base palette with enough colors for typical use cases
   ```css
   mark[type="symbol"] {
       /* Safe default with 10 colors */
       fill-discrete: #1f77b4, #ff7f0e, #2ca02c, #d62728, #9467bd,
                      #8c564b, #e377c2, #7f7f7f, #bcbd22, #17becf;
   }
   ```

### When to Use Cardinality Selectors

Cardinality-based palette selection is most valuable when:

- **Variable Category Counts**: Your application visualizes datasets with different numbers of categories
- **Optimal Color Differentiation**: You want to provide the best possible color distinction for each cardinality
- **Professional Polish**: You're creating a polished theme where small details matter
- **Color Theory Application**: You want to apply perceptual color theory principles at different scales

For static visualizations with known category counts, explicit palette specification without cardinality selectors may be simpler.

### Example: ggplot2-Style Evenly Spaced Hues

This example demonstrates a theme inspired by ggplot2's default color palette, which uses evenly spaced hues around the HSL color wheel. Each cardinality gets colors distributed uniformly across the hue spectrum (0-360°), ensuring maximum perceptual distinction.

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

let ctx = SessionContext::new();

// Create data with 6 categories
let data = vec![
    (1.0, 2.0, "Category A"),
    (2.0, 5.0, "Category B"),
    (3.0, 3.0, "Category C"),
    (4.0, 8.0, "Category D"),
    (5.0, 4.0, "Category E"),
    (6.0, 9.0, "Category F"),
    (7.0, 6.0, "Category A"),
    (8.0, 7.0, "Category B"),
];

let x_array = Float64Array::from(data.iter().map(|(x, _, _)| *x).collect::<Vec<_>>());
let y_array = Float64Array::from(data.iter().map(|(_, y, _)| *y).collect::<Vec<_>>());
let category_array = StringArray::from(data.iter().map(|(_, _, c)| *c).collect::<Vec<_>>());

let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(x_array) as _),
    ("y", Arc::new(y_array) as _),
    ("category", Arc::new(category_array) as _),
])?;

let df = ctx.read_batch(batch)?;

// ggplot2-inspired theme with evenly spaced HSL hues
let css = r#"
    :root {
        font-family: "Inter", sans-serif;
        font-size: 12px;
    }

    canvas {
        background-color: white;
        margin: 15px;
    }

    plot {
        background-color: #f8f8f8;
    }

    chart-title {
        font-size: 1.4rem;
        font-weight: 500;
        color: #2c3e50;
    }

    axis title {
        font-size: 1.0rem;
        font-weight: 400;
        color: #34495e;
    }

    axis label {
        font-size: 0.9rem;
        color: #7f8c8d;
    }

    axis grid {
        stroke: #dfe6e9;
        opacity: 0.7;
    }

    legend title {
        font-size: 1.0rem;
        font-weight: 500;
        color: #2c3e50;
    }

    legend label {
        font-size: 0.9rem;
        color: #34495e;
    }

    /* Evenly spaced hues: 2 colors at 0° and 180° */
    mark[type="symbol"][cardinality="2"] {
        fill-discrete: hsl(15 65% 60%), hsl(195 65% 60%);
        size: 140px;
    }

    /* Evenly spaced hues: 3 colors at 0°, 120°, 240° */
    mark[type="symbol"][cardinality="3"] {
        fill-discrete: hsl(15 65% 60%), hsl(135 65% 60%), hsl(255 65% 60%);
        size: 140px;
    }

    /* Evenly spaced hues: 4 colors at 0°, 90°, 180°, 270° */
    mark[type="symbol"][cardinality="4"] {
        fill-discrete: hsl(15 65% 60%), hsl(105 65% 60%),
                       hsl(195 65% 60%), hsl(285 65% 60%);
        size: 140px;
    }

    /* Evenly spaced hues: 5 colors at 72° intervals */
    mark[type="symbol"][cardinality="5"] {
        fill-discrete: hsl(15 65% 60%), hsl(87 65% 60%), hsl(159 65% 60%),
                       hsl(231 65% 60%), hsl(303 65% 60%);
        size: 140px;
    }

    /* Evenly spaced hues: 6 colors at 60° intervals */
    mark[type="symbol"][cardinality="6"] {
        fill-discrete: hsl(15 65% 60%), hsl(75 65% 60%), hsl(135 65% 60%),
                       hsl(195 65% 60%), hsl(255 65% 60%), hsl(315 65% 60%);
        size: 140px;
    }

    /* Evenly spaced hues: 7 colors at ~51.4° intervals */
    mark[type="symbol"][cardinality="7"] {
        fill-discrete: hsl(15 65% 60%), hsl(66 65% 60%), hsl(117 65% 60%),
                       hsl(168 65% 60%), hsl(219 65% 60%), hsl(270 65% 60%),
                       hsl(321 65% 60%);
        size: 140px;
    }

    /* Evenly spaced hues: 8 colors at 45° intervals */
    mark[type="symbol"][cardinality="8"] {
        fill-discrete: hsl(15 65% 60%), hsl(60 65% 60%), hsl(105 65% 60%),
                       hsl(150 65% 60%), hsl(195 65% 60%), hsl(240 65% 60%),
                       hsl(285 65% 60%), hsl(330 65% 60%);
        size: 140px;
    }

    /* Evenly spaced hues: 9 colors at 40° intervals */
    mark[type="symbol"][cardinality="9"] {
        fill-discrete: hsl(15 65% 60%), hsl(55 65% 60%), hsl(95 65% 60%),
                       hsl(135 65% 60%), hsl(175 65% 60%), hsl(215 65% 60%),
                       hsl(255 65% 60%), hsl(295 65% 60%), hsl(335 65% 60%);
        size: 140px;
    }

    /* Evenly spaced hues: 10 colors at 36° intervals */
    mark[type="symbol"][cardinality="10"] {
        fill-discrete: hsl(15 65% 60%), hsl(51 65% 60%), hsl(87 65% 60%),
                       hsl(123 65% 60%), hsl(159 65% 60%), hsl(195 65% 60%),
                       hsl(231 65% 60%), hsl(267 65% 60%), hsl(303 65% 60%),
                       hsl(339 65% 60%);
        size: 140px;
    }

    /* Base palette - standard evenly spaced 8-color set */
    mark[type="symbol"] {
        fill-discrete: hsl(15 65% 60%), hsl(60 65% 60%), hsl(105 65% 60%),
                       hsl(150 65% 60%), hsl(195 65% 60%), hsl(240 65% 60%),
                       hsl(285 65% 60%), hsl(330 65% 60%);
        size: 140px;
    }
"#;

let theme = Theme::from_css(css)?;

let plot = Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .title("ggplot2-Style Evenly Spaced Hues")
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| c.axis(|a| a.title("X Value")))
            .y_with(col("y"), |c| c.axis(|a| a.title("Y Value")))
            .fill_with(col("category"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Category"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The theme uses HSL color notation with:
- **Hue**: Evenly distributed around the color wheel (0-360°), starting at 15°
- **Saturation**: Fixed at 65% for consistent vividness
- **Lightness**: Fixed at 60% for consistent brightness

Each cardinality gets optimal hue spacing:
- 2 colors: 180° apart (complementary)
- 3 colors: 120° apart (triadic)
- 4 colors: 90° apart (tetradic)
- 6 colors: 60° apart (hexadic)
- And so on...

This approach ensures maximum perceptual distinction between categories by distributing colors evenly across the hue spectrum, similar to ggplot2's default discrete color scale.

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

The following examples demonstrate production-ready themes built entirely with `Theme::from_css()`. Each showcases a cluster of theming capabilities applied to real datasets.

### Dark Professional Theme

A complete dark theme with warm accent colors, subtle grid, and polished typography. Perfect for dashboards and dark-mode applications.

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let stocks_path = format!("{}/../tests/data/stocks.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(stocks_path, ParquetReadOptions::default())
    .await
    ?;

let css = r#"
    :root {
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        --base-font-size: 12px;
        font-size: var(--base-font-size);
    }

    canvas {
        background-color: #0f172a;
        margin: 15px;
    }

    plot {
        background-color: #1e293b;
    }

    guide {
        background-color: transparent;
    }

    chart-title {
        color: #f1f5f9;
        font-weight: 600;
        font-size: 1.4rem;
    }

    chart-subtitle {
        color: #94a3b8;
        font-weight: 400;
        font-size: 1.0rem;
    }

    axis domain {
        stroke: #475569;
        stroke-width: 1.5;
    }

    axis tick {
        stroke: #475569;
        size: 5.0;
    }

    axis title {
        color: #e2e8f0;
        font-weight: 500;
        font-size: 1.05rem;
    }

    axis label {
        color: #94a3b8;
        font-weight: 400;
        font-size: 0.9rem;
        padding: 4;
    }

    axis grid {
        stroke: #334155;
        opacity: 0.4;
        stroke-width: 0.5;
    }

    legend title {
        color: #e2e8f0;
        font-weight: 500;
        font-size: 1.05rem;
    }

    legend label {
        color: #cbd5e1;
        font-weight: 400;
        font-size: 0.95rem;
    }

    legend background {
        fill: rgba(30, 41, 59, 1.0);
        stroke: #94a3b8;
        stroke-width: 2.0;
        corner-radius: 6;
        padding: 10;
    }

    mark[type="line"] {
        stroke-width: 2.5;
        stroke-cap: round;
        stroke-join: round;
        stroke-discrete: #f59e0b, #10b981, #3b82f6, #8b5cf6, #ef4444;
    }
"#;

let theme = Theme::from_css(css)?;

let plot = Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .title("Stock Prices - Dark Professional Theme")
    .subtitle("Warm accent colors with polished dark interface")
    .mark(
        Line::new()
            .x_with(col("date"), |c| c
                .scale_with::<Time>(|s| s)
                .axis(|a| a.title("Date"))
            )
            .y_with(col("price"), |c| c
                .axis(|a| a.title("Price (USD)").format("$.0f"))
            )
            .stroke_with(col("symbol"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Stock Symbol"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Light Minimalist Theme

A clean, airy theme with subtle colors and generous whitespace. Ideal for presentations and reports where clarity is paramount.

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
    :root {
        font-family: "SF Pro Display", -apple-system, sans-serif;
        --base-font-size: 12px;
        font-size: var(--base-font-size);
    }

    canvas {
        background-color: #ffffff;
        margin: 20px;
    }

    plot {
        background-color: #fafafa;
    }

    chart-title {
        color: #374151;
        font-weight: 300;
        font-size: 1.6rem;
    }

    chart-subtitle {
        color: #9ca3af;
        font-weight: 300;
        font-size: 1.0rem;
    }

    axis domain {
        stroke: #e5e7eb;
        stroke-width: 1.0;
    }

    axis tick {
        stroke: #e5e7eb;
        size: 4.0;
    }

    axis title {
        color: #6b7280;
        font-weight: 400;
        font-size: 0.95rem;
    }

    axis label {
        color: #9ca3af;
        font-weight: 300;
        font-size: 0.85rem;
        padding: 5;
    }

    axis grid {
        stroke: #f3f4f6;
        opacity: 0.6;
        stroke-width: 0.5;
    }

    legend title {
        color: #6b7280;
        font-weight: 400;
        font-size: 0.95rem;
    }

    legend label {
        color: #9ca3af;
        font-weight: 300;
        font-size: 0.9rem;
    }

    legend background {
        fill: #ffffff;
        stroke: #d1d5db;
        stroke-width: 2.0;
        corner-radius: 8;
        padding: 12;
    }

    mark[type="symbol"] {
        size: 140;
        shape: circle;
        stroke-width: 0;
        opacity: 0.75;
        fill-discrete: #60a5fa, #34d399, #fbbf24;
    }
"#;

let theme = Theme::from_css(css)?;

let plot = Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .title("Iris Dataset - Light Minimalist Theme")
    .subtitle("Clean design with generous whitespace")
    .mark(
        Symbol::new()
            .x_with(col("sepal_length"), |c| c
                .axis(|a| a.title("Sepal Length (cm)"))
            )
            .y_with(col("sepal_width"), |c| c
                .axis(|a| a.title("Sepal Width (cm)"))
            )
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### High Contrast Accessibility Theme

WCAG-compliant high contrast theme with bold colors and increased sizing. Designed for maximum readability and accessibility.

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use datafusion::functions_aggregate::expr_fn::*;

let ctx = SessionContext::new();
# let movies_path = format!("{}/../tests/data/movies.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(movies_path, ParquetReadOptions::default())
    .await
    ?;

let aggregated = df
    .aggregate(
        vec![col("MPAA Rating")],
        vec![sum(col("Worldwide Gross")).alias("total_gross")],
    )
    ?
    .filter(col("MPAA Rating").is_not_null())
    ?
    .sort(vec![col("total_gross").sort(false, false)])
    ?;

let css = r#"
    :root {
        font-family: "Arial", "Helvetica Neue", sans-serif;
        --base-font-size: 14px;
        font-size: var(--base-font-size);
    }

    canvas {
        background-color: #000000;
        margin: 15px;
    }

    plot {
        background-color: #000000;
    }

    chart-title {
        color: #ffffff;
        font-weight: 700;
        font-size: 1.6rem;
    }

    chart-subtitle {
        color: #ffffff;
        font-weight: 600;
        font-size: 1.1rem;
    }

    axis domain {
        stroke: #ffffff;
        stroke-width: 3.0;
    }

    axis tick {
        stroke: #ffffff;
        stroke-width: 2.5;
        size: 8.0;
    }

    axis title {
        color: #ffffff;
        font-weight: 700;
        font-size: 1.15rem;
    }

    axis label {
        color: #ffffff;
        font-weight: 600;
        font-size: 1.0rem;
        padding: 6;
    }

    axis grid {
        stroke: #555555;
        opacity: 1.0;
        stroke-width: 1.5;
    }

    legend title {
        color: #ffffff;
        font-weight: 700;
        font-size: 1.15rem;
    }

    legend label {
        color: #ffffff;
        font-weight: 600;
        font-size: 1.05rem;
    }

    legend background {
        fill: #1a1a1a;
        stroke: #ffffff;
        stroke-width: 3.0;
        corner-radius: 4;
        padding: 12;
    }

    mark[type="rect"] {
        stroke: #000000;
        stroke-width: 3.0;
        fill-discrete: #00ff00, #ffff00, #ff00ff, #00ffff, #ff0000;
    }
"#;

let theme = Theme::from_css(css)?;

let plot = Plot::<Cartesian>::new()
    .theme(theme)
    .data(aggregated)
    .title("Movie Revenue by Rating - High Contrast Theme")
    .subtitle("WCAG-compliant colors with bold strokes")
    .mark(
        Rect::new()
            .x_with(col("MPAA Rating"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.2))
            })
            .x2_with(col(":x"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| c
                .axis(|a| a.title("Total Gross Revenue").format(".2s"))
            )
            .y2(col("total_gross"))
            .fill_with(col("MPAA Rating"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("MPAA Rating"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Scientific Publication Theme

Grayscale with a single accent color, serif typography, and precise grid control. Print-ready theme suitable for academic publications.

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use datafusion::functions::expr_fn::*;
use datafusion::functions_aggregate::expr_fn::*;

let ctx = SessionContext::new();
# let weather_path = format!("{}/../tests/data/seattle-weather.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(weather_path, ParquetReadOptions::default())
    .await
    ?;

// Extract month and aggregate average temperature
let monthly = df
    .select(vec![
        date_part(lit("month"), col("date")).alias("month"),
        col("temp_max"),
        col("temp_min"),
    ])
    ?
    .aggregate(
        vec![col("month")],
        vec![
            avg(col("temp_max")).alias("avg_temp_max"),
            avg(col("temp_min")).alias("avg_temp_min"),
        ],
    )
    ?
    .sort(vec![col("month").sort(true, false)])
    ?;

let css = r#"
    :root {
        font-family: "Georgia", "Times New Roman", serif;
        --base-font-size: 11px;
        font-size: var(--base-font-size);
    }

    canvas {
        background-color: #ffffff;
        margin: 12px;
    }

    plot {
        background-color: #ffffff;
    }

    chart-title {
        color: #000000;
        font-weight: 400;
        font-size: 1.4rem;
    }

    chart-subtitle {
        color: #404040;
        font-weight: 400;
        font-size: 1.0rem;
        font-style: italic;
    }

    axis domain {
        stroke: #000000;
        stroke-width: 1.0;
    }

    axis tick {
        stroke: #000000;
        stroke-width: 1.0;
        size: 5.0;
    }

    axis title {
        color: #000000;
        font-weight: 400;
        font-size: 1.05rem;
    }

    axis label {
        color: #202020;
        font-weight: 400;
        font-size: 0.95rem;
        padding: 3;
    }

    axis grid {
        stroke: #d0d0d0;
        opacity: 0.5;
        stroke-width: 0.5;
    }

    legend title {
        color: #000000;
        font-weight: 400;
        font-size: 1.05rem;
    }

    legend label {
        color: #202020;
        font-weight: 400;
        font-size: 0.95rem;
    }

    legend background {
        fill: none;
        stroke: #000000;
        stroke-width: 0.75;
        corner-radius: 0;
        padding: 8;
    }

    mark[type="line"] {
        stroke-width: 1.5;
        stroke-cap: butt;
        stroke-join: miter;
        stroke-discrete: #000000, #606060;
    }
"#;

let theme = Theme::from_css(css)?;

let plot = Plot::<Cartesian>::new()
    .theme(theme)
    .data(monthly.clone())
    .title("Monthly Temperature in Seattle")
    .subtitle("Average daily high and low temperatures (2012-2015)")
    .mark(
        Line::new()
            .x_with(col("month"), |c| c
                .axis(|a| a.title("Month"))
            )
            .y_with(col("avg_temp_max"), |c| c
                .axis(|a| a.title("Temperature (°F)"))
            )
            .stroke(lit("High"))
    )
    .mark(
        Line::new()
            .x(col("month"))
            .y_with(col("avg_temp_min"), |c| {
                c.scale_with::<Linear>(|s| s.zero(false))
            })
            .stroke_with(lit("Low"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Daily Temperature"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Modern Dashboard Theme

Corporate-friendly theme with professional blue palette and compact layout. Optimized for business dashboards and executive reports.

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
    :root {
        font-family: "Segoe UI", "Roboto", sans-serif;
        --base-font-size: 11px;
        font-size: var(--base-font-size);

        --corporate-blue: #0066cc;
        --corporate-light: #4d94ff;
        --corporate-dark: #004080;
    }

    canvas {
        background-color: #f5f7fa;
        margin: 10px;
    }

    plot {
        background-color: #ffffff;
    }

    chart-title {
        color: #1a2332;
        font-weight: 600;
        font-size: 1.35rem;
    }

    chart-subtitle {
        color: #5a6c7d;
        font-weight: 400;
        font-size: 0.95rem;
    }

    axis domain {
        stroke: #cbd5e0;
        stroke-width: 1.0;
    }

    axis tick {
        stroke: #cbd5e0;
        size: 4.0;
    }

    axis title {
        color: #2d3748;
        font-weight: 500;
        font-size: 0.95rem;
    }

    axis label {
        color: #4a5568;
        font-weight: 400;
        font-size: 0.85rem;
        padding: 3;
    }

    axis grid {
        stroke: #e2e8f0;
        opacity: 0.6;
        stroke-width: 0.5;
    }

    legend {
        spacing: 8;
        label-padding: 4;
    }

    legend title {
        color: #2d3748;
        font-weight: 500;
        font-size: 0.95rem;
    }

    legend label {
        color: #4a5568;
        font-weight: 400;
        font-size: 0.85rem;
    }

    legend background {
        fill: rgba(247, 250, 252, 0.95);
        stroke: #a0aec0;
        stroke-width: 1.5;
        corner-radius: 4;
        padding: 8;
    }

    mark[type="symbol"] {
        stroke: white;
        stroke-width: 1.0;
        opacity: 0.8;
        fill-discrete: var(--corporate-blue), #16a34a, #ea580c, #7c3aed, #dc2626;
        size-continuous: 60, 240;
    }
"#;

let theme = Theme::from_css(css)?;

let plot = Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .title("Iris Dataset - Dashboard Theme")
    .subtitle("Corporate styling with compact, information-dense layout")
    .mark(
        Symbol::new()
            .x_with(col("sepal_length"), |c| c
                .axis(|a| a.title("Sepal Length"))
            )
            .y_with(col("sepal_width"), |c| c
                .axis(|a| a.title("Sepal Width"))
            )
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
            .size(120.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### CSS Variables & color-mix() Theme

Demonstrates extensive use of CSS variables and `color-mix()` for maintainable, derived color palettes. All colors stem from a single base hue.

```rust,render
use avenger_chart::prelude::*;

let ctx = SessionContext::new();

let css = r#"
    :root {
        font-family: "Inter", sans-serif;
        --base-font-size: 12px;
        font-size: var(--base-font-size);

        /* Base palette - single hue with variations using color-mix */
        --primary: #7c3aed;
        --bg-light: #faf5ff;
        --bg-dark: #3b0764;

        /* Derived colors using color-mix */
        --primary-light: color-mix(in srgb, var(--primary) 40%, white 60%);
        --primary-lighter: color-mix(in srgb, var(--primary) 20%, white 80%);
        --primary-dark: color-mix(in srgb, var(--primary) 80%, black 20%);

        /* Text colors derived from background */
        --text-on-light: color-mix(in srgb, var(--bg-light) 0%, black 90%);
        --text-muted: color-mix(in srgb, var(--text-on-light) 60%, var(--bg-light) 40%);

        /* Grid and borders */
        --grid: color-mix(in srgb, var(--bg-light) 90%, var(--text-on-light) 10%);
        --border: color-mix(in srgb, var(--bg-light) 70%, var(--primary) 30%);

        /* Categorical palette derived from primary */
        --cat-1: var(--primary);
        --cat-2: color-mix(in srgb, var(--primary) 70%, #f59e0b 30%);
        --cat-3: color-mix(in srgb, var(--primary) 70%, #10b981 30%);
        --cat-4: color-mix(in srgb, var(--primary) 70%, #3b82f6 30%);
        --cat-5: color-mix(in srgb, var(--primary) 70%, #ef4444 30%);
    }

    canvas {
        background-color: var(--bg-light);
        margin: 15px;
    }

    plot {
        background-color: white;
    }

    chart-title {
        color: var(--text-on-light);
        font-weight: 600;
        font-size: 1.5rem;
    }

    chart-subtitle {
        color: var(--text-muted);
        font-weight: 400;
        font-size: 1.0rem;
    }

    axis domain {
        stroke: var(--border);
        stroke-width: 1.5;
    }

    axis tick {
        stroke: var(--border);
        size: 5.0;
    }

    axis title {
        color: var(--text-on-light);
        font-weight: 500;
        font-size: 1.0rem;
    }

    axis label {
        color: var(--text-muted);
        font-weight: 400;
        font-size: 0.9rem;
    }

    axis grid {
        stroke: var(--grid);
        opacity: 0.7;
        stroke-width: 0.5;
    }

    legend title {
        color: var(--text-on-light);
        font-weight: 500;
        font-size: 1.0rem;
    }

    legend label {
        color: var(--text-muted);
        font-weight: 400;
        font-size: 0.9rem;
    }

    legend background {
        fill: var(--primary-lighter);
        stroke: var(--border);
        stroke-width: 1.5;
        corner-radius: 8;
        padding: 10;
    }

    mark[type="rect"] {
        stroke: white;
        stroke-width: 2.0;
        fill-discrete: var(--cat-1), var(--cat-2), var(--cat-3), var(--cat-4), var(--cat-5);
    }
"#;

let theme = Theme::from_css(css)?;

let plot = Plot::<Cartesian>::new()
    .theme(theme)
    .data(datasets::categorical_bars(&ctx))
    .title("CSS Variables & color-mix() - Maintainable Theme")
    .subtitle("All colors derived from a single base hue using color-mix()")
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
            .fill_with(col("category"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Category"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Responsive Media Queries Theme

Adaptive theme that responds to canvas height with compact vs. spacious styling. Demonstrates CSS media queries for responsive design with parameters. The legend position moves to the bottom when height exceeds the threshold.

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use datafusion::common::ScalarValue;
use indexmap::IndexMap;

let ctx = SessionContext::new();
# let stocks_path = format!("{}/../tests/data/stocks.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(stocks_path, ParquetReadOptions::default())
    .await
    ?;

let css = r#"
    :root {
        font-family: "Inter", sans-serif;
        font-size: 12px;
    }

    canvas {
        background-color: #ffffff;
        margin: 15px;
    }

    chart-title {
        color: #1f2937;
        font-weight: 500;
        font-size: 1.4rem;
    }

    chart-subtitle {
        color: #6b7280;
        font-weight: 400;
        font-size: 1.0rem;
    }

    axis domain {
        stroke: #d1d5db;
        stroke-width: 1.0;
    }

    axis title {
        color: #374151;
        font-weight: 500;
        font-size: 1.0rem;
    }

    axis label {
        color: #6b7280;
        font-weight: 400;
        font-size: 0.9rem;
    }

    axis grid {
        stroke: #e5e7eb;
        opacity: 0.5;
    }

    legend title {
        color: #374151;
        font-weight: 500;
        font-size: 1.0rem;
    }

    legend label {
        color: #6b7280;
        font-weight: 400;
        font-size: 0.9rem;
    }

    legend background {
        fill: rgba(255, 255, 255, 0.95);
        stroke: #6b7280;
        stroke-width: 2.0;
        corner-radius: 6;
        padding: 10;
    }

    legend {
        spacing: 8;
        label-padding: 4;
    }

    mark[type="line"] {
        stroke-width: 2.0;
        stroke-cap: round;
        stroke-discrete: #3b82f6, #10b981, #f59e0b, #ef4444, #8b5cf6;
    }

    /* Compact layout for short canvases */
    @media (height < 300px) {
        :root {
            font-size: 10px;
        }

        chart-title {
            color: #ef4444;
        }

        legend {
            spacing: 2;
            label-padding: 2;
        }

        guide {
            background-color: rgba(254, 202, 202, 0.3);
        }
    }

    /* Spacious layout for tall canvases */
    @media (height >= 300px) {
        :root {
            font-size: 14px;
        }

        chart-title {
            color: #10b981;
        }

        legend {
            position: bottom;
            spacing: 12;
            label-padding: 6;
        }

        guide {
            background-color: rgba(187, 247, 208, 0.3);
        }
    }
"#;

let theme = Theme::from_css(css)?;

// Define width and height parameters with default values
let width = Param::new("width", ScalarValue::from(600.0));
let height = Param::new("height", ScalarValue::from(300.0));

let plot = Plot::<Cartesian>::new()
    .theme(theme)
    .data(df)
    .add_param(width.clone())
    .add_param(height.clone())
    .canvas_size(&width, &height)
    .title("Responsive Theme - Height-Based Styling")
    .subtitle("Font sizes, spacing, and legend position adapt to canvas height")
    .mark(
        Line::new()
            .x_with(col("date"), |c| c
                .scale_with::<Time>(|s| s)
                .axis(|a| a.title("Date"))
            )
            .y_with(col("price"), |c| c
                .axis(|a| a.title("Price (USD)"))
            )
            .stroke_with(col("symbol"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Stock"))
            })
    );

let compiled = plot.compile(&ctx).await?;

// Render at short height - compact styling
let mut params_compact = IndexMap::new();
params_compact.insert("width".to_string(), ScalarValue::from(600.0));
params_compact.insert("height".to_string(), ScalarValue::from(250.0));
let compact = compiled.evaluate(&ctx, Some(params_compact)).await?;

// Render at tall height - spacious styling
let mut params_spacious = IndexMap::new();
params_spacious.insert("width".to_string(), ScalarValue::from(600.0));
params_spacious.insert("height".to_string(), ScalarValue::from(400.0));
let spacious = compiled.evaluate(&ctx, Some(params_spacious)).await?;

Ok((compact, spacious))
```

These complete theme examples showcase the full power of CSS-based theming in Avenger Chart. Each theme is production-ready and can be adapted to your specific needs by modifying the CSS variables and selectors.

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
| `mark[cardinality="N"]` | Marks with N unique categories (adaptive palette selection) |

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
- Explore the default [Themes](../themes.md)
