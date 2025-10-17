# Scales

Scales transform channel expressions from **data space** (the domain) into **visual space** (the range). Every encoded channel either uses a scale inferred from the data or one that you explicitly configure.

## Continuous Scales

### Linear (default)

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


let plot = Plot::<Cartesian>::new()
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

Options:
- `zero(true)` expands the inferred domain so that 0 is included
- `nice(true)` rounds domain endpoints to human-friendly numbers
- `clamp(true)` pins out-of-domain values to the range bounds

### Logarithmic Scales

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


let plot = Plot::<Cartesian>::new()
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

### Time Scales

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

## Discrete Position Scales

### Band Scales

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

Options:
- `padding_inner` - space between bands (0.0 = no gap, 1.0 = band width)
- `padding_outer` - space before first and after last band

### Point Scales

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

## Ordinal Scales

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


let plot = Plot::<Cartesian>::new()
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

Ordinal scales use default color palettes but can be customized with `range_colors()`.

## Threshold Scales

Threshold scales map continuous numeric values to discrete categories using explicit threshold boundaries. Unlike quantize scales that divide the domain evenly, threshold scales let you specify meaningful breakpoints based on domain knowledge.

### Basic Example: Color-Coded Performance Levels

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

### Understanding Threshold Binning

Threshold scales create bins based on boundary values. With `n` thresholds, you get `n+1` bins:

For thresholds `[30, 50, 70, 85]`:
- **Bin 1**: score < 30 → "Poor" (red)
- **Bin 2**: 30 ≤ score < 50 → "Fair" (orange)
- **Bin 3**: 50 ≤ score < 70 → "Good" (yellow)
- **Bin 4**: 70 ≤ score < 85 → "Great" (green)
- **Bin 5**: score ≥ 85 → "Excellent" (bright green)

**Important**: You must provide exactly `n+1` range values for `n` thresholds. Thresholds must be in ascending order.

### Shape Encoding with Thresholds

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

### Temperature Categorization

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

### Common Use Cases

Threshold scales are ideal when you have domain expertise about meaningful breakpoints:

- **Temperature ranges**: Below freezing, cold, mild, warm, hot
- **Performance tiers**: Poor, fair, good, excellent, outstanding
- **Risk levels**: Low risk, moderate, high, critical
- **Income brackets**: Tax brackets or economic classifications
- **Severity scales**: Medical severity, earthquake magnitude categories
- **Quality grades**: Letter grades (F, D, C, B, A) from numeric scores
- **Air quality index**: Good, moderate, unhealthy for sensitive groups, unhealthy, very unhealthy, hazardous

### Threshold vs. Other Discretizing Scales

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

### API Reference

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
- [Ordinal Scales](#ordinal-scales) for discrete-to-discrete mapping
- [Conditional Encoding](./channels.md#conditional-encoding) for alternative categorization approaches
- [Legends](./legends.md) for customizing threshold scale legends

## Quantize Scales

Quantize scales divide a continuous numeric domain into **uniform bins of equal width**, mapping each bin to a discrete output value. Unlike threshold scales that use explicit breakpoints, quantize scales automatically create evenly-spaced bins.

### Basic Example: Temperature Ranges

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

### Understanding Quantize Binning

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

### Using Nice Domains

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

### Elevation Classification

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

### Common Use Cases

Quantize scales are ideal for creating uniform categories without domain expertise:

- **Elevation/depth ranges**: Equal-width altitude or ocean depth zones
- **Age groups**: When equal-width brackets make sense (0-20, 20-40, 40-60, etc.)
- **Price ranges**: Product pricing tiers with uniform width
- **Time periods**: Equal-duration time bins (hours, days, months)
- **Sensor readings**: Uniform value ranges for instrumentation
- **Generic categorization**: When you need bins but lack meaningful thresholds

### Quantize vs. Threshold vs. Quantile

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

### API Reference

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
- [Threshold Scales](#threshold-scales) for explicit breakpoints
- [Quantile Scales](#quantile-scales) for data-driven equal-count bins
- [Legends](./legends.md) for customizing quantize scale legends

## Quantile Scales

Quantile scales divide data into bins containing **approximately equal numbers of data points**, using percentile thresholds computed from the actual data distribution. This makes them ideal for skewed data where you want balanced categories.

### Basic Example: Income Distribution

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

### Understanding Quantile Binning

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

### Choropleth Map Colors

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

### Test Score Distribution

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

### Common Use Cases

Quantile scales excel with skewed data and percentile-based categorization:

- **Choropleth maps**: Color geographic regions when values vary widely (population, income, density)
- **Statistical analysis**: Creating quartiles, quintiles, deciles, or percentiles
- **Performance ranking**: Percentile-based grouping (top 10%, bottom 25%, etc.)
- **Resource allocation**: Ensure even distribution of resources across categories
- **Skewed data visualization**: Prevent empty bins when data has outliers or long tails
- **Fair comparisons**: Compare distributions with different ranges or scales

### Quantile vs. Quantize vs. Threshold

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

### How Data is Used for Quantiles

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

### API Reference

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
- [Threshold Scales](#threshold-scales) for explicit breakpoints
- [Quantize Scales](#quantize-scales) for equal-width bins
- [Legends](./legends.md) for customizing quantile scale legends

## Size Scales

Size scales control the area of symbol marks. A square-root scale (default for size) produces perceptually balanced circles:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create population data
let batch = RecordBatch::try_from_iter(vec![
    (
        "city",
        Arc::new(StringArray::from(vec!["City A", "City B", "City C", "City D", "City E"]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "x_coord",
        Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y_coord",
        Arc::new(Float64Array::from(vec![2.0, 3.0, 2.5, 3.5, 3.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "population",
        Arc::new(Float64Array::from(vec![5000.0, 15000.0, 8000.0, 25000.0, 12000.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("x_coord"))
            .y(col("y_coord"))
            .fill("#e67e22")
            .size_with(col("population"), |c| {
                c.scale(|s| s.range_interval(lit(100.0), lit(800.0)))
                    .legend(|l| l.title("Population"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Size channel values represent area in square pixels, so larger values produce proportionally larger circles.

## Custom Domain and Range

Explicitly set domain and range for precise control:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create temperature data
let batch = RecordBatch::try_from_iter(vec![
    (
        "hour",
        Arc::new(Float64Array::from(vec![0.0, 4.0, 8.0, 12.0, 16.0, 20.0, 24.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "temperature",
        Arc::new(Float64Array::from(vec![15.0, 12.0, 18.0, 25.0, 28.0, 22.0, 16.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Line::new()
            .x_with(col("hour"), |c| {
                c.scale(|s| {
                    s.domain_interval(lit(0.0), lit(24.0))
                })
                .axis(|a| a.title("Hour of Day"))
            })
            .y_with(col("temperature"), |c| {
                c.scale(|s| {
                    s.domain_interval(lit(0.0), lit(40.0))
                })
                .axis(|a| a.title("Temperature (°C)"))
            })
            .stroke("#e74c3c")
            .stroke_width(3.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Custom domains override automatic inference, useful for ensuring consistent scales across multiple plots.

## Plot-Level Scale Configuration

Configure a scale once at the plot level and let multiple marks inherit it:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::Float64Array;
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create data for multiple marks
let batch = RecordBatch::try_from_iter(vec![
    (
        "x",
        Arc::new(Float64Array::from(vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y1",
        Arc::new(Float64Array::from(vec![10.0, 15.0, 13.0, 18.0, 16.0, 20.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
    (
        "y2",
        Arc::new(Float64Array::from(vec![8.0, 12.0, 11.0, 15.0, 14.0, 17.0]))
            as datafusion::arrow::array::ArrayRef,
    ),
])
?;

let df = ctx.read_batch(batch)?;


let plot = Plot::<Cartesian>::new()
    .data(df.clone())
    .scale_with::<Linear>("x", |s| s.domain_interval(lit(0.0), lit(100.0)))
    .mark(
        Symbol::new()
            .x(col("x"))
            .y(col("y1"))
            .size(200.0)
            .fill("#3498db")
    )
    .mark(
        Line::new()
            .x(col("x"))
            .y(col("y2"))
            .stroke("#e74c3c")
            .stroke_width(2.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

During compilation, Avenger Chart merges channel-level scale requests with plot-level configuration. When different marks use the same channel name, the scale is shared automatically.

## Additional Options

### Padding

Control spacing for band scales with inner and outer padding:

```rust,render
use avenger_chart::prelude::*;
use datafusion::arrow::array::{Float64Array, StringArray};
use datafusion::arrow::record_batch::RecordBatch;
use std::sync::Arc;


let ctx = SessionContext::new();
// Create data for padding demonstration
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
    .title("Band Scale with Custom Padding")
    .mark(
        Rect::new()
            .x_with(col("category"), |c| {
                c.scale_with::<Band>(|s| {
                    s.padding_inner(0.3)   // 30% gap between bars
                        .padding_outer(0.1)  // 10% space at edges
                })
            })
            .x2_with(col("category"), |c| c.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
            .fill("#9b59b6")
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

Options:
- `padding_inner` - Gap between bars as fraction of band width (0.0 = no gap, 1.0 = bar width)
- `padding_outer` - Space at edges as fraction of band width

### Unknown Values

Specify a fallback for unmapped categories:

```rust,no_run
use avenger_chart::prelude::*;

# fn example() {
let _config = Ordinal::default()
    .unknown("#cccccc".to_string());  // Gray for unknown categories
# }
```

### Discrete Domains

Provide explicit, ordered lists of domain values:

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# fn example() {
let _config = Ordinal::default()
    .domain_discrete(vec![
        lit("low"),
        lit("medium"),
        lit("high"),
    ]);
# }
```

## Next Steps

- Review [Channels](./channels.md) to understand how scale configuration fits into encoding
- See [Legends](./legends.md) for how scale metadata is surfaced in guides
- Explore [Themes](./themes.md) to control how scales are styled
