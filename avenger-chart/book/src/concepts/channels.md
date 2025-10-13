# Channels

Channels map data to visual properties. Avenger Chart provides two ways to set channels: **direct values** and **data-driven encoding**.

## Direct Values

Set a channel to a constant value:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

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
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("temperature"))
            .y(col("humidity"))
            .fill("steelblue")    // All points are blue
            .size(250.0)          // All points are 250 square pixels
    )
```

Direct values apply the same styling to all points in the visualization.

## Data-Driven Encoding

Use `*_with()` methods to encode data with scales and legends:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

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
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

Plot::<Cartesian>::new()
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
    )
```

This creates:
- **Color encoding**: Maps region to colors via categorical scale
- **Size encoding**: Maps population to point sizes via a square-root scale (default for numeric sizes to better match perceived area—use `scale_with::<Linear>` if you need a linear mapping)
- **Legends**: Shows both encodings in the legend

## Conditional Encodings

Channels can branch on boolean expressions using the `when_value` and `when_scaled` helpers:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

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
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

let highlight = when(col("status").eq(lit("error")), lit(true))
    .otherwise(lit(false))
    .unwrap();

Plot::<Cartesian>::new()
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
    )
```

- `when_value(condition, literal)` injects an immediate value whenever the boolean expression is true (bypassing the scale)
- `when_scaled(condition, expr)` swaps in an alternate expression that still flows through the configured scale

Error points appear in red and are larger, while normal points follow the color scale.

## Channel Types

### Position Channels

Control spatial location:

- `x`, `y` - Primary position (Cartesian)
- `x2`, `y2` - Secondary position for ranges
- `r`, `theta` - Polar coordinates

Position channels follow the same scaling rules as all other channels: expressions (e.g., `col("date")` or `lit(100)`) are scaled, while primitive literals (e.g., `50.0`) bypass scaling.

> Polar marks currently provide `r` and `theta`; range variants (`r2`, `theta2`) are not yet available.

Example using range positions for interval marks:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

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
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("start_day"))
            .x2(col("end_day"))
            .y(col("task"))
            .y2_with(col("task"), |c| c.band(1.0))
            .fill("#3498db")
            .corner_radius(2.0)
    )
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

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

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
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

Plot::<Cartesian>::new()
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
    )
```

### Log Scales

Use logarithmic scaling for exponential data:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

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
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("index"))
            .y_with(col("value"), |c| {
                c.scale_with::<Log>(|s| s.base(10.0))
            })
            .size(300.0)
            .fill("#e74c3c")
    )
```

Log scales compress large ranges and make exponential relationships linear.

## Multiple Channels

Encode multiple channels from the same column for redundant encoding:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;

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
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

Plot::<Cartesian>::new()
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
    )
```

This creates redundant encoding where temperature controls both color and size, making the pattern easier to perceive.

## Expression Channels

Channels can use DataFusion expressions for computed values:

```rust,render,ignore
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::prelude::*;
use std::sync::Arc;

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
.expect("create batch");

let df = ctx.read_batch(batch).expect("read batch");

let status = when(col("value").gt(lit(100.0)), lit("high"))
    .otherwise(lit("low"))
    .unwrap();

Plot::<Cartesian>::new()
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
    )
```

DataFusion expressions enable complex data transformations within channel mappings.

## Literal vs. Column Values

**Key distinction**:

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# fn example() {
let _symbol = Symbol::<Cartesian>::new()
    .x(col("date"))   // Expr -> scaled by default
    .y(50.0);         // Primitive literal -> bypasses scaling
# }
```

All channels follow the same rule: expressions are routed through scales unless you call `.no_scale()` or supply a primitive literal directly; plain primitives (strings, numbers, booleans) are treated as raw values.

## Next Steps

- Learn about [Scales](./scales.md) in detail
- Understand [Legends](./legends.md) configuration
- See channel examples in [Guides](../guides/scatter-plots.md)
- Use [Parameters](../advanced/parameters.md) for runtime control
