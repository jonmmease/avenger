# Band Scale

> **Scale Type:** Discrete

Band scales divide the range into discrete bands for categorical data, with configurable padding between and around bands. Ideal for bar charts and other interval marks.

## Basic Example

Band scales arrange categorical positions with configurable padding. Use with interval marks like `Rect`:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create categorical data
let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![25.0, 40.0, 30.0, 55.0, 35.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .y_with(col("category"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.2).padding_outer(0.1))
            })
            .y2_with(col("category"), |c| c.band(1.0))
            .x(lit(0.0))
            .x2(col("value"))
            .fill("#9b59b6")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Configuration Options

- **`padding_inner(value)`**: Space between bands (0.0 = no gap, 1.0 = band width equals gap)
- **`padding_outer(value)`**: Space before first and after last band
- **`align(value)`**: Band alignment (0 = left, 0.5 = center, 1 = right)

## The `.band()` Method

Band scales support the `.band()` method for positioning marks within categorical bands. This is commonly used to control bar width in bar charts.

### What is `.band()`?

The `.band(fraction)` method positions a mark at a specific location within a band, expressed as a fraction from 0.0 to 1.0:

- **`band(0.0)`** - Start of the band (left edge for vertical bars, bottom edge for horizontal bars)
- **`band(0.5)`** - Middle of the band
- **`band(1.0)`** - End of the band (right edge for vertical bars, top edge for horizontal bars)

### Visual Representation

```
Band for "Category A" (with padding_inner=0.2):

├──────────────────────────────────────┤
↑            ↑            ↑             ↑
0.0         0.25         0.5           1.0
start                   middle         end

[   gap   ][ ← band width → ][   gap   ]
```

The band position is always relative to the band itself, not to the entire scale range. Padding affects the band size, but `.band()` positions remain consistent (0.0 = start, 1.0 = end).

### Common Patterns

**Full-Width Bars** (Default):
```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
Rect::new()
    .x(col("category"))                  // Start: band(0.0)
    .x2_with(col(":x"), |c| c.band(1.0)) // End: band(1.0)
# ;
# }
```

**Narrow Bars (70% width, centered)**:
```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
Rect::new()
    .x_with(col("category"), |c| c.band(0.15))   // Start: 15%
    .x2_with(col(":x"), |c| c.band(0.85))         // End: 85%
# ;
# }
```

**Left-Aligned Narrow Bars** (60% width):
```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
Rect::new()
    .x_with(col("category"), |c| c.band(0.0))    // Start: 0%
    .x2_with(col(":x"), |c| c.band(0.6))          // End: 60%
# ;
# }
```

**Center Point** (for point marks):
```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
Symbol::new()
    .x_with(col("category"), |c| c.band(0.5))    // Middle of band
# ;
# }
```

### Rendered Example

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![25.0, 40.0, 30.0, 55.0, 35.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;

// Create narrow bars (60% width, centered)
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.2))
                    .band(0.2)  // Start at 20%
            })
            .x2_with(col(":x"), |c| c.band(0.8))  // End at 80%
            .y(lit(0.0))
            .y2(col("value"))
            .fill("#e74c3c")
    )
    .title("Band Positioning: 60% Width Bars");

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This example creates bars that span from 20% to 80% of each band, resulting in bars that are 60% of the full band width, centered within their categories.

### Interaction with Padding

Band position (`.band()`) and band padding (`padding_inner`, `padding_outer`) work together:

**With `padding_inner=0.0`** (no gaps):
- Bands are immediately adjacent
- `.band(1.0)` of one category touches `.band(0.0)` of the next

**With `padding_inner=0.5`** (50% gaps):
- Bands are separated by gaps equal to 50% of the band width
- `.band()` positions are still relative to the band itself, not the gaps

**Example showing different padding**:
```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
// Same .band() values, different padding
Rect::new()
    .x_with(col("category"), |c| {
        c.scale_with::<Band>(|s| s.padding_inner(0.5))  // Large gaps
            .band(0.2)  // Still starts at 20% of band
    })
    .x2_with(col(":x"), |c| c.band(0.8))  // Still ends at 80% of band
# ;
# }
```

The bar is still 60% of the band width, but the bands themselves are smaller due to increased padding.

### When to Use `.band()`

**Use `.band()` when:**
- ✅ Creating bars or intervals in categorical charts
- ✅ Controlling the width of bars (narrower than full width)
- ✅ Positioning marks at specific locations within categories
- ✅ Creating centered or offset categorical marks

**Not needed when:**
- ❌ Using Point scale (positions at center automatically)
- ❌ Creating full-width bars (`.band(0.0)` to `.band(1.0)` is the default)

### See Also

- [Bar Charts](../../guides/bar-charts.md#controlling-bar-width) for practical applications
- [Channel References](../channels/index.md#channel-references) for using `:x` and `:y` with `.band()`

## When to Use

**Use band scales when:**
- ✅ Creating bar charts or similar interval visualizations
- ✅ Categories need visual separation (padding)
- ✅ Marks have width/height spanning a category range

**Avoid band scales when:**
- ❌ Categories should be centered at points (use Point instead)
- ❌ Discrete-to-discrete mapping for attributes like color (use Ordinal instead)

## See Also

- [Point Scales](./point.md) for point-based categorical positioning
- [Ordinal Scales](./ordinal.md) for discrete value mapping
