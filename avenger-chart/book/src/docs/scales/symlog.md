# Symmetric Log Scale

> **Scale Type:** Continuous

Symmetric log (symlog) scales provide smooth transitions between linear and logarithmic behavior, ideal for data that crosses zero or includes values close to zero. Unlike standard log scales, symlog can handle negative values and zero.

## Basic Example

Symlog scales handle data crossing zero with a smooth linear-to-log transition:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create data spanning negative to positive
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![-100.0, -10.0, -1.0, 0.0, 1.0, 10.0, 100.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x(col("x"))
            .y_with(col("y"), |c| {
                c.scale_with::<Symlog>(|s| s.constant(1.0).nice(true))
                    .axis(|a| a.title("Symlog Scale"))
            })
            .stroke("#3498db")
            .stroke_width(3.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The scale behaves linearly near zero and logarithmically for larger absolute values.

## Understanding Symlog Scales

Symlog uses the transformation: `sign(x) * log(1 + |x|/C)` where C is the `constant` parameter.

- **Within [-C, C]**: Scale behaves approximately linearly
- **Outside [-C, C]**: Scale behaves logarithmically
- **At zero**: Transformation is smooth and defined (unlike log scales)

The `constant` parameter controls the transition point between linear and logarithmic behavior.

## Examples

### Financial Data with Gains and Losses

Symlog scales excel at visualizing profit/loss data that crosses zero:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create quarterly profit/loss data
let batch = RecordBatch::try_from_iter(vec![
    (
        "quarter",
        Arc::new(StringArray::from(vec!["Q1", "Q2", "Q3", "Q4", "Q5", "Q6", "Q7", "Q8"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "profit",
        Arc::new(Float64Array::from(vec![-500.0, -100.0, -20.0, 50.0, 200.0, 800.0, 1500.0, 3000.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x_with(col("quarter"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.2))
            })
            .x2_with(col("quarter"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2_with(col("profit"), |c| {
                c.scale_with::<Symlog>(|s| s.constant(100.0).nice(true))
                    .axis(|a| a.title("Profit/Loss ($K)"))
            })
            .fill_with(col("profit"), |c| {
                c.scale_with::<Threshold>(|s| {
                    s.domain_discrete(vec![lit(0.0)])
                        .range_discrete(vec!["#e74c3c", "#27ae60"])
                })
            })
            .stroke("#333")
            .stroke_width(1.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The `constant(100.0)` parameter means values between -$100K and +$100K are displayed linearly, while larger profits/losses use logarithmic scaling.

### Diverging Color Scale

Use symlog for color mappings when data has meaningful zero-crossing:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create temperature anomaly data
let batch = RecordBatch::try_from_iter(vec![
    (
        "year",
        Arc::new(StringArray::from(vec!["2015", "2016", "2017", "2018", "2019", "2020", "2021", "2022"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "anomaly",
        Arc::new(Float64Array::from(vec![-2.5, -0.8, -0.2, 0.1, 0.5, 1.2, 2.8, 5.5]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x_with(col("year"), |c| {
                c.scale_with::<Band>(|s| s.padding_inner(0.1))
            })
            .x2_with(col("year"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(lit(30.0))
            .fill(col("anomaly"))
            .stroke("#666")
            .stroke_width(0.5)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The symlog scale ensures balanced color distribution across zero, with smooth transitions from negative to positive values.

## Configuration Options

- **`constant(value)`**: Sets the linear threshold (default: 1.0). Controls where linear-to-log transition occurs. Must be positive.
- **`nice(true)`**: Rounds domain to nice values in transformed space
- **`clamp(true)`**: Clamps values outside the domain

## When to Use

**Use symlog scales when:**
- ✅ Data crosses zero or includes zero
- ✅ Data spans both positive and negative values
- ✅ You need log-like scaling but have values near zero
- ✅ Visualizing diverging data (profit/loss, temperature anomalies, etc.)

**Avoid symlog scales when:**
- ❌ All values are strictly positive and far from zero (use Log instead)
- ❌ Linear relationships are more appropriate
- ❌ The transition complexity would confuse your audience

## See Also

- [Log Scales](./log.md) for strictly positive data
- [Linear Scales](./linear.md) for proportional mapping
- [Threshold Scales](./threshold.md) for discrete color mappings across zero
