# Scales

Scales are functions that transform data values from **data space** (the domain) into **visual space** (the range). They control how quantitative values map to positions, sizes, colors, and other visual properties.

## Quick Example

```rust
// Map data values (0-100) to pixel positions (0-400)
.x_with(col("value"), |c| {
    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(100.0))))
})

// Map categories to colors
.fill_with(col("category"), |c| {
    c.scale_with::<Ordinal>(|s| s)
})
```

## Scale Categories

Avenger-Chart provides 12 scale types organized into five categories:

### Continuous Scales

Map continuous numeric domains to continuous numeric ranges:

- **[Linear](./linear.md)** - Proportional mapping (default for numeric data)
- **[Log](./log.md)** - Logarithmic transformation for exponential data
- **[Pow](./pow.md)** - Power transformation with configurable exponent
- **[Sqrt](./sqrt.md)** - Square root transformation (pow with exponent=0.5)
- **[Symlog](./symlog.md)** - Symmetric log for data crossing zero

### Temporal Scales

Map temporal data with calendar-aware formatting:

- **[Time](./time.md)** - Dates, timestamps, and time series

### Categorical Position Scales

Map categorical data to spatial positions:

- **[Band](./band.md)** - Categorical positioning with bands (bar charts)
- **[Point](./point.md)** - Categorical positioning at points (scatter plots)

### Categorical Mapping Scales

Map categorical data to discrete visual properties:

- **[Ordinal](./ordinal.md)** - Discrete-to-discrete mapping (colors, shapes)

### Discretizing Scales

Map continuous domains to discrete outputs:

- **[Threshold](./threshold.md)** - Explicit threshold boundaries
- **[Quantize](./quantize.md)** - Equal-width bins
- **[Quantile](./quantile.md)** - Equal-count bins (percentiles)

## Scale Comparison

| Scale | Domain | Range | Transform | Best For |
|-------|--------|-------|-----------|----------|
| **Linear** | Continuous | Continuous | None (proportional) | General numeric data, positions |
| **Log** | Continuous (>0) | Continuous | Logarithmic | Exponential data, orders of magnitude |
| **Pow** | Continuous | Continuous | x^exponent | Emphasizing ranges, custom transforms |
| **Sqrt** | Continuous | Continuous | x^0.5 | Area encodings (circles, bubbles) |
| **Symlog** | Continuous | Continuous | Symmetric log | Data crossing zero, +/- values |
| **Time** | Temporal | Continuous | None | Dates, timestamps, time series |
| **Band** | Categorical | Intervals | Ordinal | Bar charts, categorical axes |
| **Point** | Categorical | Points | Ordinal | Scatter plots with categories |
| **Ordinal** | Categorical | Discrete | 1:1 mapping | Colors, shapes, sizes |
| **Threshold** | Continuous | Discrete | Custom thresholds | Custom binning, choropleth maps |
| **Quantize** | Continuous | Discrete | Equal-width bins | Uniform binning, heatmaps |
| **Quantile** | Continuous | Discrete | Equal-count bins | Distribution-based binning |

## Choosing a Scale

### For Position Encoding (x, y)

```
Is your data temporal?
  └─ Yes → Time
  └─ No
      └─ Is it categorical?
          ├─ Yes (bars) → Band
          ├─ Yes (dots) → Point
          └─ No (continuous)
              ├─ Normal range → Linear
              ├─ Wide range (orders of magnitude) → Log
              ├─ Crosses zero with wide range → Symlog
              └─ Custom emphasis → Pow
```

### For Size Encoding

```
What are you sizing?
  ├─ Area (circles, bubbles) → Sqrt
  ├─ Length (bars, lines) → Linear
  └─ Custom scaling → Pow
```

### For Color Encoding

```
What kind of data?
  ├─ Categories → Ordinal
  ├─ Continuous gradient → Linear, Pow, Sqrt
  ├─ Custom bins → Threshold
  ├─ Uniform bins → Quantize
  └─ Distribution bins → Quantile
```

## Common Patterns

### Continuous Color Gradient

```rust
.fill_with(col("temperature"), |c| {
    c.scale_with::<Linear>(|s| s)
        .range_colors(vec!["#blue", "#white", "#red"])
})
```

### Categorical Colors

```rust
.fill_with(col("species"), |c| {
    c.scale_with::<Ordinal>(|s| s)
        .range_colors(vec!["#ff0000", "#00ff00", "#0000ff"])
})
```

### Log-scaled Axis

```rust
.y_with(col("population"), |c| {
    c.scale_with::<Log>(|s| s.base(10.0).nice(true))
})
```

### Custom Thresholds

```rust
.fill_with(col("score"), |c| {
    c.scale_with::<Threshold>(|s| {
        s.domain_discrete(vec![lit(60.0), lit(80.0), lit(90.0)])
            .range_discrete(vec!["#F", "#D", "#C", "#B", "#A"])
    })
})
```

## Configuration Methods

Most scales support these common configuration methods:

- **`domain(...)`** - Set the input data range
- **`range(...)`** - Set the output visual range
- **`nice(true)`** - Extend domain to nice round values
- **`zero(true)`** - Include zero in the domain
- **`clamp(true)`** - Clamp out-of-range values

See individual scale pages for scale-specific options.

## See Also

- [Channels](../channels.md) - How to apply scales to visual channels
- [Understanding Expressions vs Literals](../channels.md#understanding-expressions-vs-literals) - When values are scaled vs bypass scaling
- [Legends](../legends.md) - Automatically generated scale legends
- [Axes](../axes.md) - Scale-aware coordinate axes
