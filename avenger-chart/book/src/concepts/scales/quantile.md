# Quantile Scale

> **Scale Type:** Discretizing

Quantile scales divide data into bins containing **approximately equal numbers of data points**, using percentile thresholds computed from the actual data distribution. This makes them ideal for skewed data where you want balanced categories.

## Basic Example: Income Distribution

Categorize skewed income data into equal-population quartiles:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Income data with realistic skew: many low values, few high values
let batch = RecordBatch::try_from_iter(vec![
    (
        "person",
        Arc::new(StringArray::from(vec![
            "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L"
        ]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "income",
        Arc::new(Float64Array::from(vec![
            20.0, 25.0, 28.0, 30.0, 35.0, 40.0, 45.0, 52.0, 80.0, 120.0, 180.0, 250.0
        ]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Income Distribution by Quartile")
    .mark(
        Rect::new()
            .x(col("person"))
            .x2_with(col("person"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("income"))
            .fill_with(col("income"), |c| {
                c.scale_with::<Quantile>(|s| {
                    s.range_discrete(vec![
                        "#d7191c", // Q1: Lowest 25%
                        "#fdae61", // Q2: Lower-middle 25%
                        "#abdda4", // Q3: Upper-middle 25%
                        "#2b83ba", // Q4: Highest 25%
                    ])
                })
                .legend(|l| l.title("Income Quartile"))
            })
            .stroke("#2c3e50")
            .stroke_width(1.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Notice how each color appears on exactly 3 bars despite the wide range in income values (20-250k). The bins adapt to the data distribution.

## Understanding Quantile Binning

Quantile scales compute thresholds from **actual data percentiles**, ensuring each bin contains approximately the same number of values.

**Algorithm**:
1. Collect all data values from the domain
2. Sort the values
3. Compute n-1 thresholds at percentile positions
4. Assign each value to a bin based on threshold comparisons

**Example with 9 values and 3 bins (terciles)**:
- Data: `[1, 1, 2, 3, 3, 3, 4, 4, 5]` (sorted)
- Threshold 1 (33rd percentile): `3.0`
- Threshold 2 (67th percentile): `4.0`
- **Bin 1** (< 3.0): `[1, 1, 2]` → 3 values
- **Bin 2** (3.0 ≤ x < 4.0): `[3, 3, 3]` → 3 values
- **Bin 3** (≥ 4.0): `[4, 4, 5]` → 3 values

**Key characteristics**:
- Each bin contains **approximately equal counts**
- Bin **widths vary** based on data distribution
- Thresholds are **data-driven**, not predetermined

## Choropleth Map Colors

A classic use case: coloring geographic regions with skewed population density:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Population density: highly skewed (most counties low density, few very high)
let batch = RecordBatch::try_from_iter(vec![
    (
        "county",
        Arc::new(StringArray::from(vec![
            "Rural A", "Rural B", "Rural C", "Town D", "Town E", "Town F",
            "Suburb G", "Suburb H", "City I", "Metro J", "Metro K", "Metro L"
        ]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "density",
        Arc::new(Float64Array::from(vec![
            5.0, 8.0, 12.0, 45.0, 62.0, 85.0, 210.0, 380.0, 1200.0, 3500.0, 5800.0, 9200.0
        ]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Population Density by Quantile (Choropleth Style)")
    .mark(
        Rect::new()
            .x(col("county"))
            .x2_with(col("county"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("density"))
            .fill_with(col("density"), |c| {
                c.scale_with::<Quantile>(|s| {
                    s.range_discrete(vec![
                        "#f7fbff", // Lowest density
                        "#deebf7",
                        "#c6dbef",
                        "#9ecae1",
                        "#6baed6",
                        "#3182bd", // Highest density
                    ])
                })
                .legend(|l| l.title("Density (per km²)"))
            })
            .stroke("#333333")
            .stroke_width(1.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Quantile binning ensures each color is used by exactly 2 counties, making the visualization balanced despite the 5-9200 range.

## Test Score Distribution

Categorizing student performance using percentiles:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Test scores: slightly skewed toward middle/high
let x_vals = Float64Array::from(vec![
    1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0,
    11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 19.0, 20.0,
]);
let y_vals = Float64Array::from(vec![
    2.0, 3.0, 4.0, 2.5, 3.5, 4.5, 3.0, 4.0, 5.0, 3.5,
    4.5, 5.5, 4.0, 5.0, 6.0, 4.5, 5.5, 6.5, 5.0, 6.0,
]);
let scores = Float64Array::from(vec![
    42.0, 55.0, 58.0, 62.0, 65.0, 68.0, 71.0, 74.0, 76.0, 78.0,
    80.0, 82.0, 84.0, 86.0, 88.0, 90.0, 92.0, 94.0, 96.0, 98.0,
]);

let batch = RecordBatch::try_from_iter(vec![
    ("x", Arc::new(x_vals) as datafusion::arrow::array::ArrayRef),
    ("y", Arc::new(y_vals) as datafusion::arrow::array::ArrayRef),
    ("score", Arc::new(scores) as datafusion::arrow::array::ArrayRef),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Test Scores by Quintile")
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y"))
            .fill_with(col("score"), |c| {
                c.scale_with::<Quantile>(|s| {
                    s.range_discrete(vec![
                        "#ca0020", // Bottom 20%
                        "#f4a582", // 20-40%
                        "#f7f7f7", // 40-60%
                        "#92c5de", // 60-80%
                        "#0571b0", // Top 20%
                    ])
                })
                .legend(|l| l.title("Performance Quintile"))
            })
            .size(180.0)
            .stroke("#2c3e50")
            .stroke_width(1.5)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Each quintile color represents exactly 20% of students, ensuring fair distribution across performance levels.

## Common Use Cases

Quantile scales excel with skewed data and percentile-based categorization:

- **Choropleth maps**: Color geographic regions when values vary widely (population, income, density)
- **Statistical analysis**: Creating quartiles, quintiles, deciles, or percentiles
- **Performance ranking**: Percentile-based grouping (top 10%, bottom 25%, etc.)
- **Resource allocation**: Ensure even distribution of resources across categories
- **Skewed data visualization**: Prevent empty bins when data has outliers or long tails
- **Fair comparisons**: Compare distributions with different ranges or scales

## Quantile vs. Quantize vs. Threshold

**Use quantile scales when:**
- ✅ Data is skewed or has outliers
- ✅ You want each category to represent equal population
- ✅ Statistical categorization matters (quartiles, percentiles)
- ✅ Creating choropleth maps with varied distributions
- ✅ Fairness in binning is more important than interpretability

**Avoid quantile scales when:**
- ❌ Bin boundaries need to be consistent across different datasets
- ❌ Interpretability of bin ranges is critical (quantile thresholds change with data)
- ❌ Data is uniformly distributed (quantize is simpler and equivalent)
- ❌ You have meaningful threshold values (use Threshold instead)

## How Data is Used for Quantiles

**Important**: Quantile scales require the actual data distribution to compute percentile thresholds. The scale automatically uses the domain data from your dataset:

```rust
// Quantile scale automatically infers domain from data
.fill_with(col("values"), |c| {
    c.scale_with::<Quantile>(|s| {
        s.range_discrete(vec!["low", "medium", "high"])
        // Domain is automatically inferred from col("values") data
    })
})
```

The scale will:
1. Collect all values from the `col("values")` column
2. Sort them to compute percentile thresholds
3. Create bins with equal counts based on those thresholds

## API Reference

**Key methods for quantile scales:**

```rust
.scale_with::<Quantile>(|s| {
    s.range_discrete(vec!["value1", "value2", ...])
})
```

- **`.range_discrete(vec![...])`**: n discrete output values → creates n quantile bins with equal counts
- **Domain is inferred**: The scale automatically uses data from the channel's column

**Configuration options:**
- Quantile scales have **no configuration options** (`nice`, `zero`, etc. not supported)
- Quantiles are purely **data-driven**

**Key differences from other scales:**
- **No explicit domain**: Uses actual data for percentile calculation
- **No normalization**: Thresholds determined entirely by data distribution
- **Population-based**: Focuses on equal counts, not equal widths

**See also:**
- [Threshold Scales](./threshold.md) for explicit breakpoints
- [Quantize Scales](./quantize.md) for equal-width bins
- [Legends](../legends.md) for customizing quantile scale legends
