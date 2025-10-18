# Power Scale

> **Scale Type:** Continuous

Power scales apply exponential transformation (x^exponent) to compress or expand portions of the domain, useful for emphasizing differences or matching perceptual scaling.

## Basic Example

Power scales transform data using a configurable exponent:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create data with wide range
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![1.0, 4.0, 9.0, 16.0, 25.0, 36.0, 49.0, 64.0]))
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
            .y_with(col("y"), |c| {
                c.scale_with::<Pow>(|s| s.exponent(0.5).nice(true))
                    .axis(|a| a.title("Square Root Scale"))
            })
            .size(250.0)
            .fill("#e74c3c")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Using `exponent(0.5)` applies a square root transformation, compressing large values and expanding small values.

## Understanding Power Scales

Power scales apply the transformation `y = x^exponent` where:
- **exponent < 1** (e.g., 0.5): Compresses large values, expands small values
- **exponent = 1**: Linear scale (no transformation)
- **exponent > 1** (e.g., 2): Expands large values, compresses small values

For negative values, the scale preserves the sign: `sign(x) * |x|^exponent`.

## Examples

### Emphasizing Differences (exponent > 1)

Using exponent 2 to emphasize larger differences in color mapping:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create categorical data with varying values
let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E", "F", "G", "H"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0]))
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
            .stroke("#333")
            .stroke_width(0.5)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The `exponent(2.0)` transformation makes the color gradient accelerate more strongly toward high values.

### Area-Based Encoding (exponent = 0.5)

When encoding data as area (e.g., circle size), use square root scaling for perceptual accuracy:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create population data
let batch = RecordBatch::try_from_iter(vec![
    (
        "city",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "population",
        Arc::new(Float64Array::from(vec![100000.0, 500000.0, 1000000.0, 2000000.0, 5000000.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("city"))
            .y(lit(50.0))
            .size_with(col("population"), |c| {
                c.scale_with::<Pow>(|s| {
                    s.exponent(0.5)
                        .range_interval(lit(100.0), lit(5000.0))
                })
                .legend(|l| l.title("Population"))
            })
            .fill("#3498db")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Square root scaling (`exponent(0.5)`) ensures that perceived circle area matches the data values.

## Configuration Options

- **`exponent(value)`**: Set the power exponent (default: 1.0). Must not be 0.
- **`zero(true)`**: Extends domain to include zero
- **`nice(true)`**: Rounds domain to nice values in transformed space
- **`clamp(true)`**: Clamps values outside the domain

## When to Use

**Use power scales when:**
- ✅ You need to emphasize or de-emphasize certain value ranges
- ✅ Encoding quantitative data as area (use exponent 0.5)
- ✅ Creating perceptually balanced color scales
- ✅ All values have the same sign (or you understand signed behavior)

**Avoid power scales when:**
- ❌ Linear relationships are more appropriate
- ❌ Data spans multiple orders of magnitude (use Log instead)
- ❌ Domain crosses zero in unexpected ways (use Symlog instead)

## See Also

- [Sqrt Scales](./sqrt.md) for the common case of exponent=0.5
- [Linear Scales](./linear.md) for proportional mapping
- [Log Scales](./log.md) for exponential data
