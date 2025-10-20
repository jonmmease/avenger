# Scales

Scales are functions that transform data values from **data space** (the domain) into **visual space** (the range). They control how quantitative values map to positions, sizes, colors, and other visual properties.

Avenger Chart provides [12 scale types](./scale-types.md) organized into five categories. See the [Scale Types Reference](./scale-types.md) for a complete comparison and decision guide.

## What is a Scale?

In the Grammar of Graphics, scales bridge the gap between your data and what appears on screen:

```
Data Space         Scale Transform         Visual Space
──────────         ────────────────         ────────────
0 to 100     →     Linear Scale      →     0px to 400px
Categories   →     Ordinal Scale     →     Discrete Colors
Timestamps   →     Time Scale        →     Pixel Positions
```

Every visual encoding channel (x, y, color, size, etc.) can have a scale that determines how data values are mapped to visual properties.

### Example: Position Encoding

```rust
// Without a scale, you'd need to manually compute pixel positions
let pixel_x = (value / max_value) * width;  // Manual calculation

// With a scale, the transformation is declarative
.x_with(col("value"), |c| {
    c.scale_with::<Linear>(|s| s.domain((lit(0.0), lit(100.0))))
})
```

The scale automatically handles:
- Domain-to-range mapping
- Interpolation between values
- Handling out-of-bounds values (with `clamp()`)
- Generating axis tick values and labels

## The Scale API

Avenger Chart uses a builder pattern for configuring scales on channels. The basic syntax is:

```rust
.channel_with(data_column, |c| {
    c.scale_with::<ScaleType>(|s| s.option1().option2())
})
```

Breaking this down:

1. **Channel method** (`.x_with()`, `.y_with()`, `.fill_with()`, etc.) - Specifies which visual property to control
2. **Data column** (`col("value")`) - The data column to encode
3. **Scale type** (`Linear`, `Ordinal`, `Log`, etc.) - The transformation function
4. **Scale configuration** (`.domain()`, `.nice()`, etc.) - Optional customization

### Short Form

For simple cases with default settings, use the short form:

```rust
.x(col("value"))  // Uses default Linear scale with automatic domain
```

This is equivalent to:

```rust
.x_with(col("value"), |c| {
    c.scale_with::<Linear>(|s| s)
})
```

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

## Configuring Scales

Scales support various configuration options depending on their type. Common configuration methods include:

### Domain Configuration

The **domain** defines the input data range. See the [Domains guide](./domains.md) for complete details on:

- **Explicit domains**: Fixed or computed boundaries using `domain()`, `domain_interval()`, or `domain_discrete()`
- **Automatic domains**: Let the library infer from your data (default)
- **Domain refinement**: Use `nice()`, `zero()`, and automatic visual padding

Example:
```rust
.x_with(col("value"), |c| {
    c.scale_with::<Linear>(|s| {
        s.domain((lit(0.0), lit(100.0)))  // Explicit domain
            .nice(true)                     // Round to clean tick values
            .zero(true)                     // Include zero
    })
})
```

### Range Configuration

The **range** defines the output visual values. For position channels (x, y), ranges are typically set by the plot dimensions. For other channels, use:

```rust
// Color ranges
.fill_with(col("value"), |c| {
    c.scale_with::<Linear>(|s| s)
        .range_colors(vec!["#low", "#mid", "#high"])
})

// Size ranges
.size_with(col("value"), |c| {
    c.scale_with::<Linear>(|s| s.range((lit(50.0), lit(500.0))))
})
```

### Common Options

Most continuous scales support:

- **`domain(...)`** - Set the input data range ([Domains guide](./domains.md))
- **`range(...)`** - Set the output visual range
- **`nice(true)`** - Extend domain to nice round values ([Details](./domains.md#the-nice-option))
- **`zero(true)`** - Include zero in the domain ([Details](./domains.md#the-zero-option))
- **`clamp(true)`** - Clamp out-of-range values instead of extrapolating

See individual [scale type pages](./scale-types.md) for scale-specific options like `base()` for Log scales or `exponent()` for Pow scales.

## See Also

- [Domains](./domains.md) - Complete guide to domain specification, inference, and refinement
- [Scale Types](./scale-types.md) - Reference guide to all 12 scale types with decision trees
- [Channels](../channels/index.md) - How to apply scales to visual channels
- [Understanding Expressions vs Literals](../channels/index.md#understanding-expressions-vs-literals) - When values are scaled vs bypass scaling
- [Legends](../guides-axes-legends/legends.md) - Automatically generated scale legends
- [Axes](../guides-axes-legends/axes.md) - Scale-aware coordinate axes
