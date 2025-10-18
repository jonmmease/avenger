# Time Scale

> **Scale Type:** Continuous

Time scales handle temporal data with calendar-aware formatting and intelligent tick generation for dates and times.

## Basic Example

Time scales understand temporal data and format axis ticks appropriately:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Date32Array, Float64Array};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create time series data with Date32
let batch = RecordBatch::try_from_iter(vec![
    (
        "date",
        Arc::new(Date32Array::from(vec![19723, 19754, 19784, 19815, 19845, 19876]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "temperature",
        Arc::new(Float64Array::from(vec![15.0, 18.0, 22.0, 25.0, 20.0, 16.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x_with(col("date"), |c| {
                c.scale_with::<Time>(|s| s.nice(true))
                    .axis(|a| a.title("Date"))
            })
            .y_with(col("temperature"), |c| {
                c.axis(|a| a.title("Temperature (°C)"))
            })
            .stroke("#2ecc71")
            .stroke_width(3.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Time scales automatically format dates and provide appropriate tick spacing.

## Configuration Options

- **`nice(true)`**: Rounds domain to nice time boundaries (start of day, month, year, etc.)
- **`clamp(true)`**: Clamps values outside the time range

## When to Use

**Use time scales when:**
- ✅ Data represents dates or timestamps
- ✅ You need calendar-aware axis labels
- ✅ Time-based tick spacing is important

**See Also:**
- [Linear Scales](./linear.md) for general numeric data
