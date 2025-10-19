# Threshold Scale

> **Scale Type:** Discretizing

Threshold scales map continuous numeric values to discrete categories using explicit threshold boundaries. Unlike quantize scales that divide the domain evenly, threshold scales let you specify meaningful breakpoints based on domain knowledge.

## Basic Example: Color-Coded Performance Levels

Map continuous values to discrete color categories representing performance tiers:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create performance data with scores from 0-100
let batch = RecordBatch::try_from_iter(vec![
    (
        "category",
        Arc::new(StringArray::from(vec!["A", "B", "C", "D", "E", "F", "G", "H", "I"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "score",
        Arc::new(Float64Array::from(vec![25.0, 45.0, 55.0, 68.0, 75.0, 82.0, 88.0, 35.0, 92.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(col("category"))
            .x2_with(col("category"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("score"))
            .fill_with(col("score"), |c| {
                c.scale_with::<Threshold>(|s| {
                    s.domain_discrete(vec![lit(30.0), lit(50.0), lit(70.0), lit(85.0)])
                        .range_discrete(vec![
                            "#e74c3c", // Poor: < 30
                            "#e67e22", // Fair: 30-50
                            "#f39c12", // Good: 50-70
                            "#27ae60", // Great: 70-85
                            "#2ecc71", // Excellent: >= 85
                        ])
                })
                .legend(|l| l.title("Performance"))
            })
            .stroke("#2c3e50")
            .stroke_width(1.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The threshold values `[30, 50, 70, 85]` divide the continuous score range into five bins. Each bin maps to a different color representing a performance level.

## Understanding Threshold Binning

Threshold scales create bins based on boundary values. With `n` thresholds, you get `n+1` bins:

For thresholds `[30, 50, 70, 85]`:
- **Bin 1**: score < 30 → "Poor" (red)
- **Bin 2**: 30 ≤ score < 50 → "Fair" (orange)
- **Bin 3**: 50 ≤ score < 70 → "Good" (yellow)
- **Bin 4**: 70 ≤ score < 85 → "Great" (green)
- **Bin 5**: score ≥ 85 → "Excellent" (bright green)

**Important**: You must provide exactly `n+1` range values for `n` thresholds. Thresholds must be in ascending order.

## Shape Encoding with Thresholds

Threshold scales work with any discrete-valued channel, not just colors:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create data with magnitude values
let x_values = Float64Array::from(vec![
    1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0,
    1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5, 9.5, 10.5,
]);
let y_values = Float64Array::from(vec![
    2.0, 3.5, 2.8, 4.2, 5.1, 4.8, 6.2, 5.5, 7.0, 6.5,
    3.0, 2.5, 3.8, 4.5, 5.5, 5.2, 6.8, 6.0, 7.5, 7.2,
]);
let magnitude = Float64Array::from(vec![
    5.0, 15.0, 25.0, 35.0, 45.0, 55.0, 65.0, 75.0, 85.0, 95.0,
    10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 12.0,
]);

let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(x_values) as datafusion::arrow::array::ArrayRef),
    ("y", Arc::new(y_values) as datafusion::arrow::array::ArrayRef),
    ("magnitude", Arc::new(magnitude) as datafusion::arrow::array::ArrayRef),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .shape_with(col("magnitude"), |c| {
                c.scale_with::<Threshold>(|s| {
                    s.domain_discrete(vec![lit(20.0), lit(40.0), lit(60.0), lit(80.0)])
                        .range_discrete(vec!["circle", "square", "triangle-up", "diamond", "star"])
                })
                .legend(|l| l.title("Magnitude Range"))
            })
            .size(180.0)
            .fill("#3498db")
            .stroke("#2c3e50")
            .stroke_width(2.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This creates a scatter plot where symbol shapes indicate magnitude ranges: circles for low values, squares for medium-low, triangles for medium, diamonds for medium-high, and stars for high values.

## Temperature Categorization

A practical example using temperature data:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create hourly temperature data
let batch = RecordBatch::try_from_iter(vec![
    (
        "hour",
        Arc::new(Float64Array::from(vec![
            0.0, 3.0, 6.0, 9.0, 12.0, 15.0, 18.0, 21.0, 24.0
        ]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "temp_f",
        Arc::new(Float64Array::from(vec![
            28.0, 25.0, 30.0, 45.0, 62.0, 75.0, 68.0, 52.0, 35.0
        ]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Daily Temperature with Category Ranges")
    .mark(
        Symbol::new()
            .x_with(col("hour"), |c| {
                c.axis(|a| a.title("Hour of Day"))
            })
            .y_with(col("temp_f"), |c| {
                c.axis(|a| a.title("Temperature (°F)"))
            })
            .fill_with(col("temp_f"), |c| {
                c.scale_with::<Threshold>(|s| {
                    s.domain_discrete(vec![lit(32.0), lit(50.0), lit(70.0), lit(85.0)])
                        .range_discrete(vec![
                            "#3498db", // Freezing: < 32
                            "#5dade2", // Cold: 32-50
                            "#f39c12", // Mild: 50-70
                            "#e67e22", // Warm: 70-85
                            "#e74c3c", // Hot: >= 85
                        ])
                })
                .legend(|l| l.title("Temperature Range"))
            })
            .size(250.0)
            .stroke("#2c3e50")
            .stroke_width(2.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

The thresholds `[32, 50, 70, 85]` represent meaningful temperature breakpoints (freezing point, cold/mild boundary, comfortable/warm boundary, warm/hot boundary).

## Common Use Cases

Threshold scales are ideal when you have domain expertise about meaningful breakpoints:

- **Temperature ranges**: Below freezing, cold, mild, warm, hot
- **Performance tiers**: Poor, fair, good, excellent, outstanding
- **Risk levels**: Low risk, moderate, high, critical
- **Income brackets**: Tax brackets or economic classifications
- **Severity scales**: Medical severity, earthquake magnitude categories
- **Quality grades**: Letter grades (F, D, C, B, A) from numeric scores
- **Air quality index**: Good, moderate, unhealthy for sensitive groups, unhealthy, very unhealthy, hazardous

## Threshold vs. Other Discretizing Scales

| Scale Type | Input | Output | Boundaries | Best For |
|------------|-------|--------|------------|----------|
| **Threshold** | Continuous | Discrete | Explicitly specified | Custom categories with meaningful breakpoints |
| **Quantize** | Continuous | Discrete | Evenly divided | Equal-width bins across the domain |
| **Quantile** | Continuous | Discrete | Data percentiles | Equal-count bins (same number of values per bin) |
| **Ordinal** | Discrete | Discrete | N/A | Categorical data already discrete |

**Use threshold scales when:**
- You have domain knowledge about natural or meaningful breakpoints
- Categories have semantic significance (e.g., freezing point = 32°F, passing grade = 60%)
- Bins should reflect real-world boundaries rather than statistical divisions

**Avoid threshold scales when:**
- You want equal-width bins regardless of meaning (use Quantize instead)
- You want equal-count bins for statistical balance (use Quantile instead)
- Your data is already categorical (use Ordinal instead)

## API Reference

**Key methods for threshold scales:**

```rust
.scale_with::<Threshold>(|s| {
    s.domain_discrete(vec![lit(threshold1), lit(threshold2), ...])
     .range_discrete(vec!["output0", "output1", "output2", ...])
})
```

- **`.scale_with::<Threshold>(|s| ...)`**: Specify a threshold scale
- **`.domain_discrete(vec![...])`**: Set threshold boundary values (must be in ascending order)
- **`.range_discrete(vec![...])`**: Set output values (must have exactly n+1 elements for n thresholds)

**Important notes:**
- Domain values must be wrapped in `lit()` for DataFusion expressions
- Threshold values must be in strictly ascending order
- Range must have exactly one more element than domain (n thresholds → n+1 outputs)
- Mismatched array lengths will produce a compilation error

**See also:**
- [Ordinal Scales](./ordinal.md) for discrete-to-discrete mapping
- [Conditional Encoding](../channels.md#conditional-encoding) for alternative categorization approaches
- [Legends](./guides-axes-legends/legends.md) for customizing threshold scale legends
