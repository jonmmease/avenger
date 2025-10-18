# Square Root Scale

> **Scale Type:** Continuous

Square root scales apply square root transformation (x^0.5), ensuring that visual area proportions match data values. Essential for perceptually accurate size encodings.

## Basic Example

Sqrt scales are commonly used when encoding quantitative data as area (circle size, bubble charts):

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create data with varying magnitudes
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![20.0, 40.0, 60.0, 80.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![50.0, 50.0, 50.0, 50.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![100.0, 400.0, 900.0, 2500.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .size_with(col("value"), |c| {
                c.scale_with::<Sqrt>(|s| {
                    s.range_interval(lit(200.0), lit(400.0))
                })
                .legend(|l| l.title("Value"))
            })
            .fill("#3498db")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The square root transformation ensures that circle area (not radius) is proportional to data values, creating perceptually accurate encodings.

## Understanding Sqrt Scales

When encoding data as area (e.g., circle size), using a linear scale on size would make visual area grow quadratically, creating misleading visualizations. Sqrt scales solve this:

- **Without sqrt**: A value of 100 → size 100 → area π×100² = 31,416
- **With sqrt**: A value of 100 → size √100 = 10 → area π×10² = 314

This makes the perceived visual magnitude match the data magnitude.

## Examples

### Bubble Chart with Population Data

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create city population data
let batch = RecordBatch::try_from_iter(vec![
    (
        "city",
        Arc::new(StringArray::from(vec!["Town A", "Town B", "City C", "City D", "Metro E"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "x",
        Arc::new(Float64Array::from(vec![10.0, 30.0, 50.0, 70.0, 90.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![40.0, 60.0, 20.0, 80.0, 45.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "population",
        Arc::new(Float64Array::from(vec![50000.0, 200000.0, 500000.0, 1000000.0, 3000000.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .size_with(col("population"), |c| {
                c.scale_with::<Sqrt>(|s| {
                    s.range_interval(lit(200.0), lit(400.0))
                })
                .legend(|l| l.title("Population"))
            })
            .fill("#e74c3c")
            .stroke("#c0392b")
            .stroke_width(1.5)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

### Color Gradient with Sqrt Transformation

Sqrt scales can also create non-linear color gradients that compress large values:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create data with wide range
let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E", "F", "G", "H"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![10.0, 25.0, 45.0, 70.0, 100.0, 135.0, 175.0, 220.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.1))
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
            .fill(col("value"))
            .stroke("#222")
            .stroke_width(0.75)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Configuration Options

Sqrt is a specialized version of [Pow](./pow.md) with `exponent=0.5`. It supports the same configuration methods:

- **`zero(true)`**: Extends domain to include zero
- **`nice(true)`**: Rounds domain to nice values in transformed space
- **`clamp(true)`**: Clamps values outside the domain

## When to Use

**Use sqrt scales when:**
- ✅ Encoding data as circle/symbol size
- ✅ Creating bubble charts where area should match data
- ✅ Any area-based visual encoding (not length/position)
- ✅ Creating perceptually balanced gradients

**Avoid sqrt scales when:**
- ❌ Encoding data as position or length (use Linear instead)
- ❌ You want linear relationships in visual perception
- ❌ Working with bar charts or line charts

## See Also

- [Pow Scales](./pow.md) for configurable power transformations
- [Linear Scales](./linear.md) for proportional mapping
- [Symbol Mark](../../marks/symbol.md) for circle/bubble charts
