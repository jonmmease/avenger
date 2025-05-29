# Avenger Scales

A high-performance Rust library for data visualization scales, providing mappings between data domains and visual ranges. Built on Apache Arrow for efficient data processing.

## Overview

Avenger Scales provides a comprehensive set of scale types commonly used in data visualization:

- **Continuous Scales**: Linear, Logarithmic, Power, Symlog
- **Categorical Scales**: Band, Point, Ordinal
- **Quantization Scales**: Quantile, Quantize, Threshold
- **Color Interpolation**: Multi-color gradients with various color spaces (SRGBA, HSLA, LABA)

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
avenger-scales = "0.1.0"
```

### Basic Linear Scale

```rust
use avenger_scales::scales::linear::LinearScale;
use arrow::array::{ArrayRef, Float32Array};
use std::sync::Arc;

// Create a linear scale mapping [0, 100] → [0, 500]
let scale = LinearScale::new((0.0, 100.0), (0.0, 500.0));

// Scale some values
let input = Arc::new(Float32Array::from(vec![0.0, 25.0, 50.0, 75.0, 100.0])) as ArrayRef;
let output = scale.scale_to_numeric(&input).unwrap();

// Results: [0.0, 125.0, 250.0, 375.0, 500.0]
```

### Logarithmic Scale with Color Mapping

```rust
use avenger_scales::scales::log::LogScale;
use arrow::array::{ArrayRef, Float32Array};
use datafusion_common::utils::arrays_into_list_array;
use std::sync::Arc;

// Create color range: Red → Yellow → Blue
let red = [1.0, 0.0, 0.0, 1.0];
let yellow = [1.0, 1.0, 0.0, 1.0];
let blue = [0.0, 0.0, 1.0, 1.0];

let color_arrays = vec![
    Arc::new(Float32Array::from(Vec::from(red))) as ArrayRef,
    Arc::new(Float32Array::from(Vec::from(yellow))) as ArrayRef,
    Arc::new(Float32Array::from(Vec::from(blue))) as ArrayRef,
];
let color_range = Arc::new(arrays_into_list_array(color_arrays).unwrap()) as ArrayRef;

// Create log scale with domain [1, 100] and color range
let scale = LogScale::new((1.0, 100.0), (0.0, 1.0)).with_range(color_range);

// Scale values to colors
let values = Arc::new(Float32Array::from(vec![1.0, 10.0, 100.0])) as ArrayRef;
let colors = scale.scale_to_color(&values).unwrap();

// Generate CSS gradient stops
let gradient_stops = scale.color_range_as_gradient_stops(5).unwrap();
```

### Categorical Scales

```rust
use avenger_scales::scales::{band::BandScale, ordinal::OrdinalScale};
use arrow::array::{ArrayRef, StringArray};
use std::sync::Arc;

// Band scale for bar charts
let domain = Arc::new(StringArray::from(vec!["A", "B", "C", "D"])) as ArrayRef;
let band_scale = BandScale::new(domain).with_range_interval((0.0, 300.0));

// Get positions and bandwidth
let categories = Arc::new(StringArray::from(vec!["A", "B"])) as ArrayRef;
let positions = band_scale.scale_to_numeric(&categories).unwrap();
let bandwidth = band_scale.bandwidth();

// Ordinal scale for categorical mappings
let sizes = Arc::new(StringArray::from(vec!["small", "medium", "large"])) as ArrayRef;
let caps = Arc::new(StringArray::from(vec!["round", "square", "butt"])) as ArrayRef;
let ordinal_scale = OrdinalScale::new(sizes).with_range(caps);
```

## Scale Types

### Continuous Scales

#### Linear Scale
Maps a continuous domain to a continuous range using linear interpolation.
```rust
let scale = LinearScale::new((0.0, 100.0), (0.0, 1.0));
```

#### Logarithmic Scale
Maps a continuous domain to a continuous range using logarithmic transformation.
```rust
let scale = LogScale::new((1.0, 1000.0), (0.0, 1.0))
    .with_option("base", 10.0);
```

#### Power Scale
Maps using power transformation with configurable exponent.
```rust
let scale = PowScale::new((0.0, 100.0), (0.0, 1.0))
    .with_option("exponent", 0.5); // Square root scale
```

#### Symlog Scale
Symmetric log scale that handles positive, negative, and zero values.
```rust
let scale = SymlogScale::new((-1000.0, 1000.0), (0.0, 1.0))
    .with_option("constant", 1.0);
```

### Categorical Scales

#### Band Scale
For bar charts and other visualizations where categories need bandwidth.
```rust
let scale = BandScale::new(domain)
    .with_range_interval((0.0, 300.0))
    .with_option("padding", 0.1)
    .with_option("round", true);
```

#### Point Scale
For scatter plots and line charts where categories map to points.
```rust
let scale = PointScale::new(domain)
    .with_range_interval((0.0, 300.0))
    .with_option("padding", 0.5);
```

#### Ordinal Scale
Maps discrete domain values to discrete range values.
```rust
let scale = OrdinalScale::new(domain).with_range(range);
```

### Quantization Scales

#### Quantile Scale
Maps a continuous domain to a discrete range using quantiles.
```rust
let scale = QuantileScale::new(data_values, output_range);
```

#### Quantize Scale
Maps a continuous domain to a discrete range using uniform intervals.
```rust
let scale = QuantizeScale::new((0.0, 100.0), discrete_range);
```

#### Threshold Scale
Maps a continuous domain to a discrete range using custom thresholds.
```rust
let scale = ThresholdScale::new(thresholds, output_range);
```

## Color Interpolation

Avenger Scales supports multiple color spaces for interpolation:

- **SRGBA**: Standard RGB with alpha
- **HSLA**: Hue, Saturation, Lightness, Alpha
- **LABA**: Perceptually uniform Lab color space

```rust
use avenger_scales::color_interpolator::{HslaColorInterpolator, LabaColorInterpolator};

// Use different color interpolator
let scale = LinearScale::new((0.0, 1.0), (0.0, 1.0))
    .with_range(color_range)
    .with_color_interpolator(Arc::new(HslaColorInterpolator));
```

## Advanced Features

### Nice Domains
Automatically adjust domains to "nice" round numbers:
```rust
let scale = LinearScale::new((1.23, 8.97), (0.0, 1.0))
    .with_option("nice", true); // Domain becomes (0.0, 10.0)
```

### Clamping
Constrain output values to the range:
```rust
let scale = LinearScale::new((0.0, 100.0), (0.0, 1.0))
    .with_option("clamp", true);
```

### Inversion
Map from range back to domain:
```rust
let domain_value = scale.invert_scalar(0.5).unwrap(); // 50.0
```

### Ticks
Generate tick marks for axes:
```rust
let ticks = scale.ticks(Some(5.0)).unwrap(); // 5 evenly spaced ticks
```

### Pan and Zoom
Interactive scale manipulation:
```rust
let panned_scale = scale.pan(10.0).unwrap();
let zoomed_scale = scale.zoom(0.5, 2.0).unwrap(); // Zoom 2x around center
```

## Number Formatting

Built-in number formatting with D3-style format strings:
```rust
use avenger_scales::format_num::NumberFormat;

let formatter = NumberFormat::new();
assert_eq!(formatter.format(".2f", 3.14159), "3.14");
assert_eq!(formatter.format(".0%", 0.123), "12%");
assert_eq!(formatter.format(".2s", 42000000), "42M");
```

## Examples

The `examples/` directory contains comprehensive examples:

- `linear_scale.rs` - Basic linear scale usage
- `color_scales.rs` - Color interpolation and gradients
- `categorical_scales.rs` - Band, point, and ordinal scales
- `logarithmic_scale.rs` - Logarithmic transformations
- `quantization_scales.rs` - Quantile, quantize, and threshold scales
- `advanced_features.rs` - Pan, zoom, nice domains, and formatting

Run examples with:
```bash
cargo run --example linear_scale
cargo run --example color_scales
```

## Contributing

Contributions are welcome! Please see the main Avenger repository for contribution guidelines.