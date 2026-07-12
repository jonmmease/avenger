# Logarithmic Scale

> **Scale Type:** Continuous

Logarithmic scales compress large ranges and make multiplicative relationships linear, ideal for data spanning multiple orders of magnitude.

## Basic Example

Use log scales for exponential data:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create exponential GDP data
let batch = RecordBatch::try_from_iter(vec![
    (
        "country",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "gdp_per_capita",
        Arc::new(Float64Array::from(vec![500.0, 2000.0, 8000.0, 15000.0, 35000.0, 80000.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x_with(col("gdp_per_capita"), |c| {
                c.scale_with::<Log>(|s| s.base(10.0))
                    .axis(|a| a.title("GDP per Capita (log scale)"))
            })
            .y(col("country"))
            .size(300.0)
            .fill("#e74c3c")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Logarithmic scales compress large ranges and make multiplicative relationships linear. Use base 10 for data spanning orders of magnitude.

## Configuration Options

- **`base(value)`**: Set the logarithm base (default: 10). Common values are 10, 2, and e (~2.718)
- **`nice(true)`**: Rounds domain to nice values in log space
- **`clamp(true)`**: Clamps out-of-domain values

## When to Use

**Use log scales when:**
- ✅ Data spans multiple orders of magnitude (e.g., 1 to 1,000,000)
- ✅ Multiplicative relationships matter (percentage changes)
- ✅ Visualizing exponential growth or decay
- ✅ All values are positive (log of zero/negative is undefined)

**Avoid log scales when:**
- ❌ Data includes zero or negative values (use Symlog instead)
- ❌ Additive differences are more important than ratios
- ❌ Audience unfamiliar with logarithmic interpretation

## See Also

- [Symlog Scales](./symlog.md) for data that includes zero or negative values
- [Linear Scales](./linear.md) for proportional data
