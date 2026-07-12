# Channels

## Before you start: align data types with your intent

The scale Avenger chooses for a channel depends on the underlying DataFusion column types. If you classify your columns early (casting strings to timestamps, distinguishing between ordered and unordered categories) you can lean on the defaults and only tweak scales when you want stylistic changes. See [Data Types and Channel Mapping](../data-types.md) for a quick reference.

Channels map data to visual properties. Avenger Chart provides two ways to set channels: **direct values** and **data-driven encoding**.

## Direct Values

Set a channel to a constant value:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create temperature and humidity data
let batch = RecordBatch::try_from_iter(vec![
    (
        "temperature",
        Arc::new(Float64Array::from(vec![15.0, 18.0, 22.0, 25.0, 28.0, 20.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "humidity",
        Arc::new(Float64Array::from(vec![65.0, 70.0, 55.0, 50.0, 45.0, 75.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("temperature"))
            .y(col("humidity"))
            .fill("steelblue")    // All points are blue
            .size(250.0)          // All points are 250 square pixels
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Direct values apply the same styling to all points in the visualization.

## Mapping data vs setting constants

Every channel setter comes in two flavours. The plain method (e.g., `.fill("#4682b4")`) **sets** a constant after scales run. The `_with` variant (e.g., `.fill_with(col("species"), |c| { … })`) **maps** data through a scale and also lets you customize that scale.

| Pattern | Method | Scaled? | Legend eligible? | Description |
|---------|--------|---------|------------------|-------------|
| Set constant | `.fill("#4682b4")` | ❌ | ❌ | All rows share the same value; no scale involved. |
| Map with defaults | `.fill(col("species"))` | ✅ | ✅ | Uses the default scale inferred from the column type (`Ordinal` for strings, `Linear` for numbers, etc.). |
| Map with configuration | `.fill_with(col("species"), |c| { … })` | ✅ | ✅ | Same as above, but you can tune the scale (domain, range, legend title, etc.). |

The rest of this guide uses `_with` forms heavily so you can see how scale configuration works, but remember that the short form is always available when defaults are sufficient.

## Data-driven encoding

Use `*_with()` methods to encode data with scales and legends:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create data with region and population
let batch = RecordBatch::try_from_iter(vec![
    (
        "temperature",
        Arc::new(Float64Array::from(vec![15.0, 18.0, 22.0, 25.0, 28.0, 20.0, 16.0, 24.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "humidity",
        Arc::new(Float64Array::from(vec![65.0, 70.0, 55.0, 50.0, 45.0, 75.0, 80.0, 60.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "region",
        Arc::new(StringArray::from(vec!["North", "North", "South", "South", "West", "West", "North", "South"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "population",
        Arc::new(Float64Array::from(vec![100000.0, 150000.0, 80000.0, 200000.0, 120000.0, 90000.0, 180000.0, 110000.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("temperature"))
            .y(col("humidity"))
            .fill_with(col("region"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Region"))
            })
            .size_with(col("population"), |c| {
                c.scale(|s| s.range_interval(lit(100.0), lit(600.0)))
                    .legend(|l| l.title("Population"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates:
- **Color encoding**: Maps region to colors via categorical scale
- **Size encoding**: Maps population to point sizes via a square-root scale (default for numeric sizes to better match perceived area—use `scale_with::<Linear>` if you need a linear mapping)
- **Legends**: Shows both encodings in the legend

## Understanding Expressions vs Literals

A fundamental concept in Avenger Chart is how channel values are processed: whether they go through **scales** (transformations) or bypass them as **raw values**.

### The Three Forms

When setting a channel value, you have three options:

| Form | Example | Scaled? | Use Case |
|------|---------|---------|----------|
| **Column Expression** | `col("temperature")` | ✅ Yes | Map data values through a scale |
| **Literal Expression** | `lit(50.0)` | ✅ Yes | Use a constant value, but still apply scale transformation |
| **Primitive Literal** | `50.0` or `"red"` | ❌ No | Bypass scaling entirely, use raw value directly |

### Scaling Behavior

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
Symbol::<Cartesian>::new()
    // Expressions (col and lit) go through scales:
    .x(col("temperature"))        // Data values → scale → pixel positions
    .y(lit(50.0))                 // Constant 50.0 → scale → pixel position

    // Primitive literals bypass scales:
    .size(200.0)                  // Directly sets size to 200 square pixels
    .fill("steelblue")            // Directly sets color (no scale applied)
# ;
# }
```

**Key Point**: The difference between `lit(50.0)` and `50.0` is subtle but important:
- `lit(50.0)` creates a DataFusion expression that will be processed by the scale
- `50.0` is a Rust primitive that bypasses scaling entirely

### When Each Form is Scaled (Default Behavior)

**Important**: The behavior below describes the *default* for each input type. You can always override this with:
- `.no_scale()` - Force bypassing of scale (treat expression result as raw value)
- `.scale_with::<ScaleType>(...)` - Force specific scale type

By default, the rule is consistent across all channels:

```
┌─────────────────────────┬──────────────┬──────────────────────────┐
│ Input Type              │ Scaled By    │ Result                   │
│                         │ Default?     │                          │
├─────────────────────────┼──────────────┼──────────────────────────┤
│ col("column_name")      │ YES*         │ Data values transformed  │
│ lit(value)              │ YES*         │ Constant value transformed│
│ Any DataFusion Expr     │ YES*         │ Expression result transformed│
│ Primitive (50.0, "red") │ NO           │ Used directly as-is      │
└─────────────────────────┴──────────────┴──────────────────────────┘

* Unless .no_scale() is called or a specific scale is set with .scale_with::<T>()
```

### Decision Guide: Which Form Should I Use?

Use this guide to choose the right form for your use case:

**For Position Channels (x, y)**:
- **Column data** → `col("column_name")` - Map data to positions through scale
- **Fixed position in data space** → `lit(value)` - E.g., reference line at y=0
- **Fixed position in pixel space** → `value` - Rarely needed for positions

**For Size Channels**:
- **Data-driven sizes** → `col("column_name")` with `.size_with(...)` - Scale data to sizes
- **Fixed size in data units** → `lit(value)` - Rarely useful
- **Fixed size in pixels** → `value` - Common for constant mark sizes

**For Color Channels**:
- **Data-driven colors** → `col("column_name")` with `.fill_with(...)` - Map to color scale
- **Fixed color from scale** → `lit(value)` - Pick color from scale palette
- **Fixed color directly** → `"colorname"` or `"#rrggbb"` - Common for constant colors

### Example: Comparing Scaled vs Unscaled

Here's a concrete example showing the difference:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
// Scenario: Y-axis ranges from 0 to 100 in data space,
// and maps to 0-400 pixels in visual space

let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))

            // Option 1: lit(50.0) - goes through scale
            // Data value 50.0 → scale → ~200 pixels from bottom
            .y(lit(50.0))

            // Option 2: 50.0 - bypasses scale
            // Directly sets y to 50 pixels from bottom (not what you usually want!)
            // .y(50.0)  // Uncomment to see difference
    );
# Ok(())
# }
```

**In practice**:
- `lit(50.0)` positions the mark at data value 50 (middle of 0-100 range)
- `50.0` positions the mark at 50 pixels from the bottom edge

For position channels, you almost always want `col()` or `lit()` to work in data space, not pixel space.

### The `.no_scale()` Method

When you need to bypass scaling for an expression, use `.no_scale()`:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# async fn example(ctx: &SessionContext, df: DataFrame) -> Result<(), Box<dyn std::error::Error>> {
let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            // Use data values directly as pixel sizes (no scale transformation)
            .size_with(col("size_in_pixels"), |c| c.no_scale())
    );
# Ok(())
# }
```

This is useful when:
- Your data already contains pixel values, colors, or other visual properties
- You want to use computed expressions without scale transformation
- You need precise control over visual output

### Common Patterns

**Pattern 1: Constant value in data space**
```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
// Add a reference line at y = 0 (in data coordinates)
Rect::new()
    .y(lit(0.0))      // Scaled: data value 0
    .y2(lit(0.0))
    .height(lit(2.0)) // Line thickness in data units
# ;
# }
```

**Pattern 2: Constant value in visual space**
```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
// All symbols are exactly 100 square pixels
Symbol::new()
    .x(col("x"))
    .y(col("y"))
    .size(100.0)      // Unscaled: 100 square pixels
# ;
# }
```

**Pattern 3: Data-driven with scale**
```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
// Map population values to sizes via sqrt scale (default for size)
Symbol::new()
    .x(col("x"))
    .y(col("y"))
    .size_with(col("population"), |c| {
        c.scale(|s| s.range_interval(lit(50.0), lit(500.0)))
    })
# ;
# }
```

**Pattern 4: Override default scale type**
```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
// Size channel defaults to Sqrt scale, but we can override to Linear
Symbol::new()
    .x(col("x"))
    .y(col("y"))
    .size_with(col("population"), |c| {
        c.scale_with::<Linear>(|s| s.range_interval(lit(50.0), lit(500.0)))
    })
# ;
# }
```

**Pattern 5: Override to bypass scaling**
```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
// Use expression but bypass scaling (data already contains pixel values)
Symbol::new()
    .x(col("x"))
    .y(col("y"))
    .size_with(col("size_pixels"), |c| c.no_scale())
# ;
# }
```

## Conditional Encodings

Channels can branch on boolean expressions using the `when_value` and `when_scaled` helpers:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create data with status
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![10.0, 15.0, 12.0, 18.0, 14.0, 20.0, 16.0, 22.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![5.0, 10.0, 8.0, 15.0, 12.0, 18.0, 14.0, 20.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "status",
        Arc::new(StringArray::from(vec!["ok", "ok", "error", "ok", "ok", "error", "ok", "ok"]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

let highlight = when(col("status").eq(lit("error")), lit(true))
    .otherwise(lit(false))
    .unwrap();


let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("value"), |c| {
                c.when_value(highlight.clone(), lit("#ff6b6b"))
                    .scale_with::<Linear>(|s| s)
                    .legend(|l| l.title("Value"))
            })
            .size_with(col("value"), |c| {
                c.when_scaled(highlight, col("value") * lit(2.0))
                    .scale(|s| s.range_interval(lit(100.0), lit(400.0)))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

**How conditional encoding works**:
- `when_value(condition, literal)` - Inject an immediate value when condition is true (bypasses the scale)
- `when_scaled(condition, expr)` - Swap in an alternate expression when condition is true (still uses the same scale)

**Important limitation**: Only a **single scale** is supported per channel. All branches (both the default and conditional cases) must use the same scale configuration. You can choose whether each branch is scaled or not, but you cannot use different scale types (e.g., Linear vs Log) for different branches.

```
✅ Allowed: Same scale, branches either use it or bypass it
   .fill_with(col("value"), |c| {
       c.when_value(condition, "#ff0000")  // Bypass scale
           .scale_with::<Linear>(...)       // Scale for default case
   })

❌ Not allowed: Different scales for different branches
   .fill_with(col("value"), |c| {
       c.when_scaled(condition, ...)        // Would need its own scale
           .scale_with::<Linear>(...)       // Only this scale exists
   })
```

In the example above, error points appear in red (bypassing the color scale) and are larger (using the same size scale with a doubled value), while normal points follow the configured color and size scales.

## Channel References

Channels can reference other channels' expressions using the `:channel_name` syntax. This allows you to derive one channel from another without repeating complex expressions.

### What are Channel References?

A channel reference uses a colon prefix to refer to another channel:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
// Reference syntax
col(":x")      // References the x channel's expression
col(":y")      // References the y channel's expression
col(":color")  // References the color channel's expression
col(":size")   // References the size channel's expression
# ;
# }
```

During plot compilation, channel references are resolved to the actual expressions from the referenced channels.

### How It Works

When you use a channel reference, the system:

1. **Finds the referenced channel** - Looks up the channel by name (e.g., `":x"` → `x` channel)
2. **Extracts its expression** - Gets the expression from that channel
3. **Replaces the reference** - Substitutes the reference with the actual expression
4. **Applies transformations** - Applies any additional operations (like `.band()`)

**Example**:
```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
Symbol::new()
    .x(col("temperature"))                  // Step 1: x = col("temperature")
    .y(col(":x") + lit(10.0))               // Step 2: ":x" → col("temperature")
                                             // Result: y = col("temperature") + 10
# ;
# }
```

### Common Use Case: Bar Charts

The most common use of channel references is in bar charts with band scales:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
Rect::new()
    .x(col("category"))                     // x = col("category")
    .x2_with(col(":x"), |c| c.band(1.0))   // x2: ":x" → col("category"), then band(1.0)
    .y(lit(0.0))
    .y2(col("value"))
# ;
# }
```

**What happens**:
1. `x` channel is set to `col("category")`
2. `x2` channel references `:x`, which resolves to `col("category")`
3. `.band(1.0)` is applied to position at the end of the band
4. Result: bars span from the start (`x`) to the end (`x2`) of each category

See [Bar Charts](../bar-charts.md) for detailed examples of this pattern.

### Why Use Channel References?

**1. DRY Principle** - Don't repeat yourself:
```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
// With reference (DRY) - define expression once
Rect::new()
    .x(col("category"))
    .x2_with(col(":x"), |c| c.band(1.0))

// Without reference (repetitive)
Rect::new()
    .x(col("category"))
    .x2_with(col("category"), |c| c.band(1.0))  // Repeats col("category")
# ;
# }
```

**2. Maintain consistency** - Change once, updates everywhere:
```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# use datafusion::functions::expr_fn::lower;
# fn example() {
// If you need to change the expression (e.g., add lower()), only update x
Rect::new()
    .x(lower(col("category")))              // Change here...
    .x2_with(col(":x"), |c| c.band(1.0))   // ...automatically applied here too
# ;
# }
```

**3. Create derived channels** - Build one channel from another:
```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
Symbol::new()
    .x(col("value"))
    .y(col(":x") * lit(2.0))  // y is double the x value
# ;
# }
```

### Available Channel Names

You can reference any defined channel:

**Position**: `:x`, `:y`, `:x2`, `:y2`, `:r`, `:theta`
**Visual**: `:color`, `:fill`, `:stroke`, `:opacity`
**Size/Shape**: `:size`, `:shape`, `:stroke_width`
**Other**: `:angle`, `:defined`, `:order`

### Dependency Resolution

Channel references support dependency chains:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
Symbol::new()
    .x(col("a"))           // x = col("a")
    .y(col(":x"))          // y = col("a")  (via :x)
    .size(col(":y"))       // size = col("a") (via :y via :x)
# ;
# }
```

Need metadata for future interactive views? Marks expose a `.details([...])` method that records extra columns alongside the visual encodings. The renderer stores these fields but does not yet display tooltips—integrated interaction layers are tracked on the roadmap.

The system uses **topological sorting** to resolve dependencies in the correct order, ensuring all references are resolved before evaluation.

**Circular references are detected** and will cause a compilation error:
```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
// ❌ This will fail - circular dependency
Symbol::new()
    .x(col(":y"))  // x depends on y
    .y(col(":x"))  // y depends on x - circular!
# ;
# }
```

### Alternative: Repeating Expressions

You can always repeat the expression instead of using a reference:

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# fn example() {
// Approach 1: With channel reference
Rect::new()
    .x(col("category"))
    .x2_with(col(":x"), |c| c.band(1.0))

// Approach 2: Without channel reference (explicit)
Rect::new()
    .x(col("category"))
    .x2_with(col("category"), |c| c.band(1.0))
# ;
# }
```

**When to use each**:
- **Use references** when the expression might change or is complex
- **Repeat explicitly** when you want to be very clear about what's happening
- Both compile to the same result

### Complete Example

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

let ctx = SessionContext::new();

let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "B", "C", "D"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![30.0, 50.0, 40.0, 60.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])?;

let df = ctx.read_batch(batch)?;

// Using channel references for bar positioning
let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col(":x"), |c| c.band(1.0))  // Reference :x to span full width
            .y(lit(0.0))
            .y2(col("value"))
            .fill("#3498db")
    )
    .title("Channel References: :x in Bar Chart");

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This example shows `:x` being used to reference the `x` channel's expression (`col("category")`), then applying `.band(1.0)` to position `x2` at the end of each categorical band.

## Channel Types

### Position Channels

Control spatial location:

- `x`, `y` - Primary position (Cartesian)
- `x2`, `y2` - Secondary position for ranges
- `r`, `theta` - Polar coordinates

Position channels follow the same scaling rules as all other channels: expressions (e.g., `col("date")` or `lit(100)`) are scaled, while primitive literals (e.g., `50.0`) bypass scaling.

> Polar marks currently provide `r` and `theta`; range variants (`r2`, `theta2`) are not yet available.

Example using range positions for interval marks:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create task schedule data
let batch = RecordBatch::try_from_iter(vec![
    (
        "task",
        Arc::new(StringArray::from(vec!["Task A", "Task B", "Task C", "Task D"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "start_day",
        Arc::new(Float64Array::from(vec![1.0, 3.0, 2.0, 5.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "end_day",
        Arc::new(Float64Array::from(vec![4.0, 7.0, 5.0, 8.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("start_day"))
            .x2(col("end_day"))
            .y(col("task"))
            .y2_with(col("task"), |c| c.band(1.0))
            .fill("#3498db")
            .corner_radius(2.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Visual Channels

Control appearance. Expressions (e.g., `col(...)`) are scaled by default, while plain primitive literals bypass scaling:

- `fill` - Fill color
- `stroke` - Stroke color
- `size` - Symbol size or stroke width
- `opacity` - Transparency
- `shape` - Symbol shape
- `stroke_width` - Line width
- `stroke_dash` - Dash pattern

## Scale Configuration

When using `*_with()`, you can configure the scale type and parameters.

### Categorical Scales

Map discrete values to colors:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create categorical data
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![10.0, 15.0, 12.0, 18.0, 14.0, 16.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "B", "C", "A", "B", "C"]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .size(300.0)
            .fill_with(col("category"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Category"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Log Scales

Use logarithmic scaling for exponential data:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create exponential data
let batch = RecordBatch::try_from_iter(vec![
    (
        "index",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![10.0, 100.0, 1000.0, 10000.0, 100000.0, 1000000.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("index"))
            .y_with(col("value"), |c| {
                c.scale_with::<Log>(|s| s.base(10.0))
            })
            .size(300.0)
            .fill("#e74c3c")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Log scales compress large ranges and make exponential relationships linear.

## Multiple Channels

Encode multiple channels from the same column for redundant encoding:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create temperature data
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![1.0, 1.2, 1.1, 1.3, 1.2, 1.4, 1.3, 1.5]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "temperature",
        Arc::new(Float64Array::from(vec![0.0, 15.0, 30.0, 45.0, 60.0, 75.0, 90.0, 100.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("temperature"), |c| {
                c.scale_with::<Linear>(|s| s)
                    .legend(|l| l.title("Temperature (color)"))
            })
            .size_with(col("temperature"), |c| {
                c.scale(|s| s.range_interval(lit(100.0), lit(600.0)))
                    .legend(|l| l.title("Temperature (size)"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates redundant encoding where temperature controls both color and size, making the pattern easier to perceive.

## Expression Channels

Channels can use DataFusion expressions for computed values:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create data for expression example
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y1",
        Arc::new(Float64Array::from(vec![10.0, 15.0, 12.0, 18.0, 14.0, 20.0, 16.0, 22.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y2",
        Arc::new(Float64Array::from(vec![5.0, 8.0, 6.0, 12.0, 9.0, 15.0, 11.0, 18.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![50.0, 120.0, 80.0, 150.0, 95.0, 180.0, 110.0, 200.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;

let status = when(col("value").gt(lit(100.0)), lit("high"))
    .otherwise(lit("low"))
    .unwrap();


let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y1") + col("y2"))  // Expression: sum of two columns
            .size(300.0)
            .fill_with(status, |c| {    // Expression: conditional category
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Status"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

DataFusion expressions enable complex data transformations within channel mappings.

For a comprehensive guide to DataFusion's expression capabilities, including scalar functions, string operations, date manipulation, and more, see [Working with DataFusion](../data/datafusion-expressions.md).

## Next Steps

- Learn about [Scales](../scales/index.md) in detail.
- Understand [Legends](../guides-axes-legends/legends.md) configuration.
- See end-to-end recipes in [Common Plot Patterns](../patterns/common-plot-patterns.md).
- Use [Parameters](../parameters.md) for runtime control.
