# Ordinal Scale

> **Scale Type:** Discrete

Ordinal scales map discrete domain values to discrete range values, commonly used for color, shape, or other categorical attribute encodings.

## Basic Example

Ordinal scales map discrete values to colors, shapes, or other non-positional encodings:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create species data
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y",
        Arc::new(Float64Array::from(vec![10.0, 15.0, 12.0, 18.0, 14.0, 20.0, 16.0, 22.0, 19.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "species",
        Arc::new(StringArray::from(vec!["setosa", "setosa", "setosa", "versicolor", "versicolor", "versicolor", "virginica", "virginica", "virginica"]))
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
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Ordinal scales use default color palettes but can be customized with `range_colors()` or `range_discrete()`.

## Configuration Options

- **`range_discrete(vec![...])`**: Explicit mapping of domain values to range values
- **`range_colors(vec![...])`**: Specify custom color palette for color encodings

## When to Use

**Use ordinal scales when:**
- ✅ Mapping categories to colors
- ✅ Mapping categories to shapes or sizes
- ✅ Both domain and range are discrete

**Avoid ordinal scales when:**
- ❌ Mapping categories to positions (use Band or Point instead)
- ❌ Continuous data needs discretization (use Threshold, Quantize, or Quantile)

## See Also

- [Threshold Scales](./threshold.md) for continuous-to-discrete mapping
- [Band/Point Scales](./band.md) for categorical positioning
