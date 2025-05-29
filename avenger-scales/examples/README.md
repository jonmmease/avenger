# Avenger Scales Examples

This directory contains examples demonstrating how to use the various scale types and features available in the `avenger-scales` crate.

## Running Examples

To run any example, use:

```bash
cargo run --example <example_name>
```

For example:
```bash
cargo run --example linear_scale
```

## Example Overview

### 1. [linear_scale.rs](./linear_scale.rs)
**Basic linear scale usage**
- Creating a simple linear scale with domain and range
- Scaling values from domain to range  
- Inverse scaling (range back to domain)
- Accessing domain/range information

### 2. [color_scales.rs](./color_scales.rs)
**Color interpolation and gradients**
- Linear scale with 2-color gradient (blue → red)
- Log scale with 3-color gradient (red → yellow → blue)
- Generating gradient stops for CSS/SVG
- Working with RGBA color values

### 3. [categorical_scales.rs](./categorical_scales.rs)
**Categorical and ordinal mappings**
- **Band scales**: For bar charts with bandwidth calculation
- **Point scales**: For scatter plots and line charts
- **Ordinal scales**: Categorical to categorical mappings
- Range inversion for finding categories from positions

### 4. [logarithmic_scales.rs](./logarithmic_scales.rs)
**Non-linear scale transformations**
- **Log scales**: Base 10 and base 2 logarithmic scaling
- **Symlog scales**: Handling negative values and zero
- **Power scales**: Square root and other power transformations
- Scale inversion and tick generation

### 5. [quantile_threshold.rs](./quantile_threshold.rs)
**Data-driven and threshold-based scales**
- **Quantile scales**: Data-driven quantile-based categorization
- **Quantize scales**: Uniform interval categorization
- **Threshold scales**: Custom breakpoint categorization
- Comparing different categorization approaches

### 6. [formatting_and_ticks.rs](./formatting_and_ticks.rs)
**Number formatting and axis generation**
- Number formatting with precision control
- Date and timestamp formatting
- Timezone-aware timestamp formatting
- Tick generation for axes
- Scale options (clamp, round, nice)

## Key Concepts Demonstrated

### Scale Types
- **Continuous scales**: Linear, Log, Power, Symlog
- **Categorical scales**: Band, Point, Ordinal
- **Discrete scales**: Quantile, Quantize, Threshold

### Features Showcased
- **Color interpolation**: Multi-color gradients in different color spaces
- **Data transformation**: Linear, logarithmic, and power transformations  
- **Formatting**: Numbers, dates, timestamps with various formats
- **Tick generation**: Automatic axis tick generation
- **Inversion**: Converting from range back to domain values
- **Configuration**: Scale options like clamping, rounding, and nice domains

### Common Use Cases
- **Data visualization**: Mapping data values to visual properties
- **Chart axes**: Generating tick marks and labels
- **Color mapping**: Creating color gradients for heatmaps
- **Categorical mapping**: Assigning positions to categories
- **Data binning**: Grouping continuous data into discrete categories

## Scale Configuration Options

Most scales support these common options:
- `clamp`: Keep output within range bounds
- `round`: Round output to integers
- `nice`: Extend domain to nice round numbers
- `padding`: Add spacing (for band/point scales)
- `base`: Logarithm base (for log scales)
- `exponent`: Power exponent (for power scales)
- `constant`: Linear threshold (for symlog scales)

## Advanced Features

### Builder Pattern
```rust
let scale = LinearScale::new((0.0, 100.0), (0.0, 500.0))
    .with_option("clamp", true)
    .with_option("round", true)
    .with_range_colors(vec![
        [1.0, 0.0, 0.0, 1.0], // Red
        [0.0, 1.0, 0.0, 1.0], // Green  
        [0.0, 0.0, 1.0, 1.0], // Blue
    ])?;
```

### Color Interpolation
```rust
let color_result = scale.scale_to_color(&values)?;
let gradient_stops = scale.color_range_as_gradient_stops(10)?;
```

### Multiple Output Types
```rust
let numeric_output = scale.scale_to_numeric(&values)?;
let string_output = scale.scale_to_string(&values)?;
let color_output = scale.scale_to_color(&values)?;
```

These examples provide a comprehensive introduction to the capabilities of the avenger-scales crate and should help you get started with scale-based data transformations in your projects.