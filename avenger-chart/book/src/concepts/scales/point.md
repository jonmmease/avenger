# Point Scale

> **Scale Type:** Discrete

Point scales map categorical data to evenly-spaced points in the range, ideal for scatter plots and line charts with categorical axes.

## Basic Example

Point scales place categories at discrete points. Use with glyph marks like `Symbol`:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create categorical scatter data
let batch = RecordBatch::try_from_iter(vec![
    (
        "group",
        Arc::new(StringArray::from(vec!["Group A", "Group B", "Group C", "Group A", "Group B", "Group C"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "measurement",
        Arc::new(Float64Array::from(vec![10.0, 15.0, 12.0, 14.0, 18.0, 11.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x_with(col("group"), |c| {
                c.scale_with::<Point>(|s| s.padding(0.5))
            })
            .y(col("measurement"))
            .size(250.0)
            .fill("#3498db")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Point scales center marks at category positions with configurable padding on the ends.

## Configuration Options

- **`padding(value)`**: Padding at the start and end of the range (as fraction of step)
- **`align(value)`**: Point alignment (0 = left, 0.5 = center, 1 = right)

## When to Use

**Use point scales when:**
- ✅ Creating categorical scatter plots
- ✅ Line charts with categorical x-axis
- ✅ Marks should be centered at category positions

**Avoid point scales when:**
- ❌ Creating bar charts (use Band instead)
- ❌ Discrete-to-discrete mapping for attributes (use Ordinal instead)

## See Also

- [Band Scales](./band.md) for interval-based categorical positioning
- [Ordinal Scales](./ordinal.md) for discrete value mapping
