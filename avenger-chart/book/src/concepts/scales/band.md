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
