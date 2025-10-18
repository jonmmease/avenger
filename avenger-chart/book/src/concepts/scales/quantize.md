# Quantize Scale

> **Scale Type:** Discretizing

Quantize scales divide a continuous numeric domain into **uniform bins of equal width**, mapping each bin to a discrete output value. Unlike threshold scales that use explicit breakpoints, quantize scales automatically create evenly-spaced bins.

## Basic Example: Temperature Ranges

Create equal-width temperature categories from continuous data:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create temperature data ranging from 0-100°F
let batch = RecordBatch::try_from_iter(vec![
    (
        "location",
        Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E", "F", "G", "H", "I", "J"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "temp",
        Arc::new(Float64Array::from(vec![15.0, 28.0, 42.0, 56.0, 65.0, 73.0, 82.0, 88.0, 94.0, 38.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("location"))
            .x2_with(col("location"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("temp"))
            .fill_with(col("temp"), |c| {
                c.scale_with::<Quantize>(|s| {
                    s.domain((lit(0.0), lit(100.0)))
                        .range_discrete(vec![
                            "#2166ac", // Cold: 0-20
                            "#4393c3", // Cool: 20-40
                            "#92c5de", // Mild: 40-60
                            "#f4a582", // Warm: 60-80
                            "#d6604d", // Hot: 80-100
                        ])
                })
                .legend(|l| l.title("Temperature (°F)"))
            })
            .stroke("#2c3e50")
            .stroke_width(1.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The domain `[0, 100]` with 5 range values creates 5 bins of width 20 each: `[0-20, 20-40, 40-60, 60-80, 80-100]`.

## Understanding Quantize Binning

Quantize scales divide the domain into equal-width intervals. The bin width is calculated as:

**bin_width = (domain_max - domain_min) / n**

where `n` is the number of range values.

For domain `[0, 100]` with 5 range values:
- **Bin 1**: 0 ≤ temp < 20 → Cold (dark blue)
- **Bin 2**: 20 ≤ temp < 40 → Cool (blue)
- **Bin 3**: 40 ≤ temp < 60 → Mild (light blue)
- **Bin 4**: 60 ≤ temp < 80 → Warm (light red)
- **Bin 5**: 80 ≤ temp ≤ 100 → Hot (red)

**Key characteristics**:
- All bins have **equal width** (20°F in this example)
- Bins may contain **different numbers of data points**
- Last bin is **inclusive** on the upper bound

## Using Nice Domains

The `nice()` option extends the domain to round values before creating bins:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create data with irregular range: 12-97
let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E", "F", "G", "H"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "value",
        Arc::new(Float64Array::from(vec![12.0, 28.0, 45.0, 58.0, 67.0, 79.0, 88.0, 97.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Quantize Scale with Nice Domain Extension")
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col("category"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2_with(col("value"), |c| {
                c.scale_with::<Quantize>(|s| {
                    s.domain((lit(0.0), lit(100.0)))
                        .range_discrete(vec![
                            "#8dd3c7",
                            "#ffffb3",
                            "#bebada",
                            "#fb8072",
                        ])
                })
                .axis(|a| a.title("Value"))
            })
            .fill_with(col("value"), |c| {
                c.scale_with::<Quantize>(|s| {
                    s.domain((lit(0.0), lit(100.0)))
                        .range_discrete(vec![
                            "#8dd3c7",
                            "#ffffb3",
                            "#bebada",
                            "#fb8072",
                        ])
                })
                .legend(|l| l.title("Value Range"))
            })
            .stroke("#333333")
            .stroke_width(1.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

By setting explicit domain `[0, 100]`, we create clean bins of width 25 each.

## Elevation Classification

A practical example categorizing elevation data:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Elevation data in meters
let x_values = Float64Array::from(vec![
    1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0,
    11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 19.0, 20.0,
]);
let y_values = Float64Array::from(vec![
    2.0, 3.0, 2.5, 3.5, 3.0, 4.0, 3.5, 4.5, 4.0, 5.0,
    4.5, 5.5, 5.0, 6.0, 5.5, 6.5, 6.0, 7.0, 6.5, 7.5,
]);
let elevation = Float64Array::from(vec![
    150.0, 380.0, 520.0, 890.0, 1200.0, 1450.0, 1680.0, 2100.0, 2400.0, 2850.0,
    3100.0, 3500.0, 3800.0, 4200.0, 4500.0, 280.0, 650.0, 1050.0, 1750.0, 2500.0,
]);

let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(x_values) as datafusion::arrow::array::ArrayRef),
    ("y", Arc::new(y_values) as datafusion::arrow::array::ArrayRef),
    ("elevation", Arc::new(elevation) as datafusion::arrow::array::ArrayRef),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Elevation Zones with Equal-Width Ranges")
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("elevation"), |c| {
                c.scale_with::<Quantize>(|s| {
                    s.domain((lit(0.0), lit(5000.0)))
                        .range_discrete(vec![
                            "#006d2c", // 0-1000m: Lowland
                            "#31a354", // 1000-2000m: Hill
                            "#74c476", // 2000-3000m: Mountain
                            "#bae4b3", // 3000-4000m: High mountain
                            "#edf8e9", // 4000-5000m: Alpine
                        ])
                })
                .legend(|l| l.title("Elevation (m)"))
            })
            .size(200.0)
            .stroke("#2c3e50")
            .stroke_width(1.5)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates 5 elevation zones with 1000m width each, useful for topographic visualization.

## Common Use Cases

Quantize scales are ideal for creating uniform categories without domain expertise:

- **Elevation/depth ranges**: Equal-width altitude or ocean depth zones
- **Age groups**: When equal-width brackets make sense (0-20, 20-40, 40-60, etc.)
- **Price ranges**: Product pricing tiers with uniform width
- **Time periods**: Equal-duration time bins (hours, days, months)
- **Sensor readings**: Uniform value ranges for instrumentation
- **Generic categorization**: When you need bins but lack meaningful thresholds

## Quantize vs. Threshold vs. Quantile

**Use quantize scales when:**
- ✅ You want uniform, evenly-spaced bins
- ✅ Visual simplicity is important (equal bin widths)
- ✅ You don't have domain knowledge for specific breakpoints
- ✅ Data distribution is relatively uniform

**Avoid quantize scales when:**
- ❌ Data is highly skewed (some bins may be empty or overcrowded)
- ❌ You have meaningful threshold values (use Threshold instead)
- ❌ You want balanced bin populations (use Quantile instead)
- ❌ Bin boundaries need semantic meaning

## API Reference

**Key methods for quantize scales:**

```rust
.scale_with::<Quantize>(|s| {
    s.domain((lit(min), lit(max)))
     .range_discrete(vec!["value1", "value2", ...])
})
```

- **`.domain((min, max))`**: Continuous domain interval (min, max)
- **`.range_discrete(vec![...])`**: n discrete output values → creates n equal-width bins

**Important notes:**
- Domain is a tuple of two values (min, max), not an array
- Range determines the number of bins: n range values → n bins
- Bin width = (max - min) / n
- Domain can be inferred from data if not explicitly set

**See also:**
- [Threshold Scales](./threshold.md) for explicit breakpoints
- [Quantile Scales](./quantile.md) for data-driven equal-count bins
- [Legends](../legends.md) for customizing quantize scale legends
