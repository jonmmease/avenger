# Linear Scale

> **Scale Type:** Continuous

Linear scales are the default for numeric data, providing a direct proportional mapping from data values to visual coordinates.

## Basic Example

Linear scales are used automatically for numeric columns:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create data with wide value range
let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E", "F"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![23.0, 45.0, 12.0, 67.0, 34.0, 56.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col("category"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2_with(col("value"), |c| {
                c.scale_with::<Linear>(|s| s.zero(true).nice(true))
            })
            .fill("#3498db")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

## Configuration Options

- **`zero(true)`**: Expands the inferred domain so that 0 is included
- **`nice(true)`**: Rounds domain endpoints to human-friendly numbers
- **`clamp(true)`**: Pins out-of-domain values to the range bounds
- **`padding(value)`**: Adds proportional padding to the domain

## When to Use

**Use linear scales when:**
- ✅ Data has a proportional relationship
- ✅ Zero is meaningful (e.g., temperatures above absolute zero, counts)
- ✅ Values span a reasonable range without extreme variation

**Avoid linear scales when:**
- ❌ Data spans multiple orders of magnitude (use Log instead)
- ❌ You need to emphasize changes in small values (use Pow or Sqrt)
- ❌ Data crosses zero with both positive and negative values of interest (consider Symlog)

## See Also

- [Log Scales](./log.md) for data spanning orders of magnitude
- [Pow Scales](./pow.md) for emphasizing large or small values
