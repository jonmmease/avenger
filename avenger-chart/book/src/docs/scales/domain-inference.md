# Domain Inference

When you don't explicitly specify a scale's domain, Avenger Chart automatically infers it from your data. This page explains how domain inference works for different scale types and how you can control the behavior with options like `nice()`, `zero()`, and automatic visual padding.

## What is Domain Inference?

A scale's **domain** defines the range of input data values it maps from. For example:
- A Linear scale might have domain `[0.0, 100.0]` for percentage data
- A Band scale might have domain `["A", "B", "C"]` for three categories

When you omit the domain specification, Avenger Chart analyzes your data to determine appropriate domain boundaries automatically.

## Continuous Domain Inference

For continuous scales (Linear, Log, Pow, Sqrt, Symlog, Time), domain inference computes the minimum and maximum values from your data.

### Basic Example

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Data ranges from 15 to 85
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![15.0, 45.0, 25.0, 65.0, 85.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Automatic Domain Inference")
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))  // Domain automatically inferred from data
            .size(150.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The y-axis domain is automatically computed from the data (approximately 15-85) with additional padding for the symbol size.

### How It Works

For each continuous scale channel:
1. **Scan the data**: Find min and max values across all rows
2. **Apply adjustments**: Add padding for marks, extend to zero if requested, round to nice values
3. **Set domain**: Use the computed boundaries as the scale domain

This happens during the compilation phase, ensuring domains adapt to your data.

## Categorical Domain Inference

For categorical scales (Band, Point, Ordinal), domain inference extracts unique values from your data and preserves their order.

### Basic Example

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Categories appear in data order: C, A, B, D
let batch = RecordBatch::try_from_iter(vec![
    ("category", Arc::new(StringArray::from(vec!["C", "A", "B", "D", "C", "A"])) as _),
    ("value", Arc::new(Float64Array::from(vec![25.0, 40.0, 30.0, 55.0, 35.0, 45.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Categorical Domain: Data Order Preserved")
    .mark(
        Rect::new()
            .x(col("category"))  // Domain automatically: ["C", "A", "B", "D"]
            .x2_with(col(":x"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Notice the categories appear in **data order** (C, A, B, D), not alphabetical order. The first occurrence of each unique value determines its position.

### Order Control

To control categorical order:

**Use DataFusion's `sort()`** before plotting:
```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::*;

// Sort alphabetically
let sorted = df.sort(vec![col("category").sort(true, false)])?;

// Or sort by value (descending)
let sorted = df.sort(vec![col("value").sort(false, false)])?;
```

**Specify explicit domain** for fixed order:
```rust,no_run
use avenger_chart::prelude::*;

.x_with(col("category"), |c| {
    c.scale_with::<Band>(|s| {
        s.domain_discrete(vec![lit("A"), lit("B"), lit("C"), lit("D")])
    })
})
```

## The `nice()` Option

The `nice()` option extends continuous domains to "nice" round numbers, creating cleaner axis tick values.

### Example: With and Without Nice

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Data ranges from 17.3 to 94.8
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![17.3, 45.2, 28.1, 67.9, 94.8])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Without nice() - Precise Domain")
    .mark(
        Symbol::new()
            .x(col("x"))
            .y_with(col("y"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false))  // Precise boundaries
            })
            .size(120.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Compare with nice rounding:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Same data ranges from 17.3 to 94.8
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![17.3, 45.2, 28.1, 67.9, 94.8])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("With nice() - Round Tick Values")
    .mark(
        Symbol::new()
            .x(col("x"))
            .y_with(col("y"), |c| {
                c.scale_with::<Linear>(|s| s.nice(true))  // Round to clean values
            })
            .size(120.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### When to Use Nice

**Use `nice(true)` (the default) when:**
- ✅ Tick readability matters (dashboards, reports)
- ✅ Exact domain boundaries aren't critical
- ✅ You want professional-looking axes

**Use `nice(false)` when:**
- ✅ Domain boundaries must be exact
- ✅ Comparing plots with fixed scales
- ✅ Minimal whitespace is needed

## The `zero()` Option

The `zero()` option forces the domain to include zero, even if data doesn't reach it. Common for bar charts where bars should extend from a baseline.

### Example: Including Zero

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Values range from 50 to 90 (no zeros)
let batch = RecordBatch::try_from_iter(vec![
    ("category", Arc::new(StringArray::from(vec!["A", "B", "C", "D"])) as _),
    ("value", Arc::new(Float64Array::from(vec![50.0, 70.0, 60.0, 90.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Without zero() - Data Range Only")
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y_with(col("value"), |c| {
                c.scale_with::<Linear>(|s| s.zero(false))  // Don't include zero
            })
            .y2(lit(0.0))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Without `zero()`, the y-axis starts near 50, making bar heights misleading. Compare with zero included:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Same values: 50 to 90
let batch = RecordBatch::try_from_iter(vec![
    ("category", Arc::new(StringArray::from(vec!["A", "B", "C", "D"])) as _),
    ("value", Arc::new(Float64Array::from(vec![50.0, 70.0, 60.0, 90.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("With zero() - Includes Baseline")
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y_with(col("value"), |c| {
                c.scale_with::<Linear>(|s| s.zero(true))  // Include zero
            })
            .y2(lit(0.0))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Now the y-axis extends to zero, making bar heights proportional to values.

### When to Use Zero

**Use `zero(true)` when:**
- ✅ Creating bar charts (bars from baseline)
- ✅ Proportional comparison matters
- ✅ Zero is a meaningful reference point

**Use `zero(false)` when:**
- ✅ Data doesn't cross zero
- ✅ Emphasizing variation in a narrow range
- ✅ Zero isn't meaningful (e.g., temperature in Fahrenheit)

## Automatic Visual Padding

Avenger Chart automatically expands position scale domains to prevent mark clipping at data boundaries. This intelligent padding considers mark geometry (symbol size, stroke width) to ensure marks are fully visible.

### How Automatic Padding Works

When marks extend to the edges of your data range, the chart automatically:

1. **Analyzes mark dimensions**: Calculates the visual extent of each mark
   - For symbols: radius + stroke width
   - For lines: stroke width / 2 on each side
2. **Computes required padding**: Determines how much to expand the domain in data space
3. **Expands the domain**: Adjusts the scale domain so marks don't get clipped

**Current limitation**: Automatic padding currently works only for **Linear scales**. Other scale types (Log, Pow, Sqrt, Time, etc.) will be supported in future releases.

### Example: Symbol Padding

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Data points exactly at domain boundaries (0, 0) and (100, 100)
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![0.0, 50.0, 100.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![0.0, 50.0, 100.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Automatic Padding (Symbols Fully Visible)")
    .mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false))
            })
            .y_with(col("y"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false))
            })
            .size(300.0)  // Large symbols to show padding effect
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Notice how symbols at (0, 0) and (100, 100) are fully visible despite being at the data boundaries. The scale domain was automatically expanded to accommodate their visual size.

### Example: Line Stroke Padding

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Line touches domain boundaries at y=0 and y=100
let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0, 4.0])) as _),
    ("y", Arc::new(Float64Array::from(vec![0.0, 50.0, 25.0, 75.0, 100.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Automatic Padding Prevents Stroke Clipping")
    .mark(
        Line::new()
            .x_with(col("x"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false))
            })
            .y_with(col("y"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false))
            })
            .stroke("#e74c3c")
            .stroke_width(12.0)  // Thick stroke to show padding effect
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The line stroke is fully visible at y=0 and y=100. Without automatic padding, half the stroke width would be clipped at these boundaries.

### Why Automatic Padding Matters

Without padding, marks with visual extent get "cut off" at data extremes:
- A symbol at y=100 would have its top half clipped if the domain ended at 100
- A 10px stroke at the maximum y-value would extend beyond the plot area and be invisible
- The visual would be incomplete and misleading

Automatic padding ensures the entire mark geometry is rendered within the visible plot region.

### Interaction with Nice Scales

The `nice()` setting affects how domain expansion works:

**With nice scales** (`nice(true)`, the default):
- Domain is rounded to "nice" numbers (e.g., 0, 25, 50, 75, 100)
- Provides clean tick values
- May add extra breathing room beyond what's needed for padding

**With precise scales** (`nice(false)`):
- Domain expands exactly enough to fit mark geometry
- Tick values may not be round numbers
- Minimal whitespace around data

### Scale Type Support

**Automatic padding support by scale type**:
- ✅ **Linear scales**: Full automatic padding support
- ⏳ **Other scales** (Log, Pow, Sqrt, Time, Band, Point, etc.): Planned for future release

For scales without automatic padding, consider adding manual padding to your domain boundaries or using `nice(true)` to ensure some whitespace.

## Combining Domain Options

You can combine `nice()`, `zero()`, and automatic padding for complete control:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

// Values from 30 to 85
let batch = RecordBatch::try_from_iter(vec![
    ("category", Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E"])) as _),
    ("value", Arc::new(Float64Array::from(vec![30.0, 55.0, 40.0, 75.0, 85.0])) as _),
])?;
let df = ctx.read_batch(batch)?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Combined: nice + zero + padding")
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))
            .y_with(lit(0.0), |c| {
                c.scale_with::<Linear>(|s| {
                    s.nice(true)    // Round to clean tick values
                        .zero(true)  // Include zero baseline
                })
                .axis(|a| a.title("Value"))
            })
            .y2(col("value"))
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates a bar chart with:
- Zero baseline (for proportional comparison)
- Nice tick values (for readability)
- Automatic padding (if marks were at boundaries)

## Manual Domain Specification

For complete control, you can always specify domains explicitly, which disables automatic inference:

```rust,no_run
use avenger_chart::prelude::*;

// Fixed domain: 0 to 100
.y_with(col("value"), |c| {
    c.scale_with::<Linear>(|s| s.domain((0.0, 100.0)).nice(false))
})

// Or using expressions for dynamic boundaries
.y_with(col("value"), |c| {
    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(100.0))))
})
```

See [Scales Overview > Domain Specification](./index.md#domain-specification) for complete details on manual domain specification.

## Summary

**Domain inference automatically determines scale boundaries from your data:**

| Feature | Continuous Scales | Categorical Scales |
|---------|-------------------|-------------------|
| **Inference Method** | Compute min/max | Extract unique values |
| **Order** | Numeric | First occurrence |
| **nice()** | Rounds to clean values | N/A |
| **zero()** | Includes zero if absent | N/A |
| **Padding** | Linear scales only | N/A |

**Best practices:**
- ✅ Use automatic inference for exploratory analysis
- ✅ Enable `zero(true)` for bar charts
- ✅ Use `nice(true)` for readable axes (the default)
- ✅ Trust automatic padding to prevent clipping on Linear scales
- ✅ Specify explicit domains for fixed scales across multiple plots

## See Also

- [Scales Overview](./index.md) - Complete guide to scale types and usage
- [Domain Specification](./index.md#domain-specification) - Manual domain specification methods
- [Symbol Mark](../marks/symbol.md) - Scatter plots with automatic padding
- [Line Mark](../marks/line.md) - Line charts with stroke padding
- [Band Scale](./band.md) - Categorical positioning for bar charts
