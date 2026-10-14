# Avenger Scales

A high-performance Rust library for data visualization scales, providing mappings between data domains and visual ranges. Built on Apache Arrow for efficient data processing.

## Overview

Avenger Scales provides a comprehensive set of scale types commonly used in data visualization:

- **Continuous Scales**: Linear, Logarithmic, Power, Symlog
- **Categorical Scales**: Band, Point, Ordinal
- **Nested Categorical Scales**: Nested band, for struct-valued paths with one field per level. Levels nest freely, with space for the children each parent has, or share their slots across parents. Padding is set per level in leaf steps, and the options the band scale also has mean the same thing
- **Quantization Scales**: Quantile, Quantize, Threshold
- **Domain inference**: every scale that can infer its domain summarizes data, and fits its domain to the summary for its range. Summaries merge, so data from several tables or plots fits one domain. Fits keep the marks' pixel extents inside the range, by widening continuous domains and by setting the clip padding of band and point scales
- **DataFusion** (`datafusion` feature): summarize data, fit domains, and apply scales in DataFusion queries, so one plan goes from rows to pixels
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
use arrow::array::{ArrayRef, Float64Array};
use std::sync::Arc;

// Create a linear scale mapping [0, 100] → [0, 500]
let scale = LinearScale::new((0.0, 100.0), (0.0, 500.0));

// Scale some values
let input = Arc::new(Float64Array::from(vec![0.0, 25.0, 50.0, 75.0, 100.0])) as ArrayRef;
let output = scale.scale_to_numeric(&input).unwrap();

// Results: [0.0, 125.0, 250.0, 375.0, 500.0]
```

### Logarithmic Scale with Color Mapping

```rust
use avenger_scales::scales::log::LogScale;
use avenger_scales::scalar::Scalar;
use arrow::array::{ArrayRef, Float32Array, Float64Array};
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
let color_range = Scalar::arrays_into_list_array(color_arrays).unwrap();

// Create log scale with domain [1, 100] and color range
let scale = LogScale::new((1.0, 100.0), (0.0, 1.0)).with_range(color_range);

// Scale values to colors
let values = Arc::new(Float64Array::from(vec![1.0, 10.0, 100.0])) as ArrayRef;
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
    .with_option("padding_inner", 0.1)
    .with_option("padding_outer", 0.1)
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

### Domain Inference
A scale's `summarize` summarizes data, with the pixel extents of the marks drawn at it, and
`fit_domain` fits the scale's domain to that summary for its range. `DomainOptions` has one
variant for each kind of summary: `IntervalOptions` set `zero`, `nice`, and the domain for no
data, and `CategoryOptions` set the category order. A single value spans 10% of its
magnitude, one power of the base for log scales, or one day for time scales. Continuous scales
map their configured domain as given, so a domain from elsewhere stays exact.
```rust
use avenger_scales::scales::domain_summary::{DomainOptions, Extents, IntervalOptions};

// A scale with no domain yet, which fit_domain fills.
let scale = LinearScale::unfitted((0.0, 400.0));
// Circles of radius 10 at each value.
let summary = scale.summarize(&values, Extents::Constant { lower: 10.0, upper: 10.0 })?;
let options = DomainOptions::Interval(IntervalOptions {
    zero: true,
    nice: Some(10.0),
    ..Default::default()
});
let scale = scale.fit_domain(&options, &summary)?;
```
Summaries `merge`, so marks from several tables or plots can share a domain. To share a continuous
domain between plots of different widths, scale each plot's `IntervalSummary` by `1 / width` with
`scale_extents`, merge them, and fit at width 1. Band, point, and nested band scales keep their
categories' layout and fit `clip_padding_lower` and `clip_padding_upper` instead, so that marks at
the outer categories stay in the range. A configured clip padding is a minimum.

To make an explicit domain nice, fit it as a summary with no extents:
```rust
use avenger_scales::scales::domain_summary::{IntervalSummary, NiceStep};

let summary = IntervalSummary::new(&[1.23, 8.97], &[0.0; 2], &[0.0; 2])?;
let nice = summary.fit_nice(1.0, NiceStep::Auto(10.0))?;
// (1.0, 9.0)
```

### Serialization
Scales serialize through [typetag](https://docs.rs/typetag), so a `Box<dyn ScaleImpl>`
deserializes without registration, including a scale from another crate. Such a scale derives
`Serialize` and `Deserialize`, names its `impl ScaleImpl` block with its `scale_type`, and depends
on `serde` and `typetag`:
```rust
#[derive(Debug, Serialize, Deserialize)]
struct LogitScale;

#[typetag::serde(name = "logit")]
impl ScaleImpl for LogitScale {
    fn scale_type(&self) -> &'static str {
        "logit"
    }
    // ...
}
```

### DataFusion
The `datafusion` feature adds `avenger_scales::datafusion`, whose functions summarize data, fit
domains, and apply scales in [DataFusion](https://datafusion.apache.org) queries:

- `summary_expr(scale, values, extents)` is an aggregate that summarizes values, with optional
  lower and upper pixel extents of their marks. DataFusion merges the partial summaries of each
  partition, and `GROUP BY` gives one summary for each group.
- `fit_expr(scale, summary, domain_options)` fits the scale's domain to a summary and returns the
  fitted configuration, a struct of its domain, range, and options.
- `scale_expr(scale, values, config)` applies a scale with a configuration from an expression: a
  fit in the same plan, or `config_expr(&config)` for a configuration fitted in Rust.

One plan can go from rows to pixels. Here circles take their radii from a size scale fitted in the
plan, and their x positions from a scale fitted to the circles and their radii.
`examples/datafusion_plan.rs` is the complete program.
```rust
use avenger_scales::datafusion::{fit_expr, scale_expr, summary_expr};
use datafusion::logical_expr::expr_fn::scalar_subquery;

// The configuration that `fit` returns over all of `df`'s rows, as a one-row subquery.
let fitted_config = |df: DataFrame, fit: Expr| -> Result<Expr> {
    let plan = df.aggregate(vec![], vec![fit.alias("config")])?;
    Ok(scalar_subquery(Arc::new(plan.into_unoptimized_plan())))
};
let options = DomainOptions::Interval(IntervalOptions::default());

let size_scale = LinearScale::unfitted((2.0, 12.0));
let size_summary = summary_expr(&size_scale, col("size"), None)?;
let size_config = fitted_config(points.clone(), fit_expr(&size_scale, size_summary, &options)?)?;
let radius = scale_expr(&LinearScale, col("size"), size_config)?;
let circles = points.select(vec![col("x"), radius.alias("r")])?;

let x_scale = LinearScale::unfitted((0.0, 400.0));
let x_summary = summary_expr(&x_scale, col("x"), Some((col("r"), col("r"))))?;
let x_config = fitted_config(circles.clone(), fit_expr(&x_scale, x_summary, &options)?)?;
let marks = circles.select(vec![
    scale_expr(&LinearScale, col("x"), x_config)?.alias("px"),
    col("r"),
])?;
```
To fit one domain to marks from several tables, summarize the union of their rows, or decode their
summaries with `DomainSummary::from_arrow` and `merge` them in Rust. For a separate scale for each
facet, fit with `GROUP BY facet` and join the fitted configurations to the rows by facet.
`scale_expr` scales the rows of each configuration together. `config_from_arrow` decodes a fitted
configuration into a `ScaleConfig`, for example to draw the scale's axis.

To order categories by an aggregate, query the order first and fit with `CategoryOrder::Explicit`:
```rust
let order = df
    .aggregate(vec![col("category")], vec![sum(col("total")).alias("sum")])?
    .sort(vec![col("sum").sort(false, true)])?
    .select(vec![col("category")])?
    .collect()
    .await?;
let order = concat(&order.iter().map(|batch| batch.column(0).as_ref()).collect::<Vec<_>>())?;
let options = DomainOptions::Categories(CategoryOptions {
    order: CategoryOrder::Explicit(order),
});
```
`ScaleExtensionCodec` is a datafusion-proto codec for plans that hold these functions. Pass it to
functions such as `logical_plan_to_bytes_with_extension_codec`, and the plans decode without
registering their scales. It passes other extensions to the codec given to `with_fallback`.

DataFusion builds zstd from C, so the feature's `wasm32-unknown-unknown` build needs a clang that
targets wasm32. The `cc` crate uses `clang` by default, but Apple's clang has no wasm32 target, so
on macOS point the build at Homebrew LLVM, for example in `~/.cargo/config.toml`:
```toml
[env]
CC_wasm32_unknown_unknown = "/opt/homebrew/opt/llvm/bin/clang"
AR_wasm32_unknown_unknown = "/opt/homebrew/opt/llvm/bin/llvm-ar"
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

## Labels

Scales don't format numbers or dates. Format tick values with a provider from `avenger-format`: a number provider, such as `avenger-format-number-d3` or `avenger-format-number-icu`, whose `format_ticks` labels a set of ticks together, or for time scales a datetime provider, whose `CalendarPatterns` label each tick by its calendar boundary. `scale_to_string` returns a scale's string outputs, such as an ordinal scale's string range, and converts other outputs to text.

## Examples

The `examples/` directory contains comprehensive examples:

- `linear_scale.rs` - Basic linear scale usage
- `color_scales.rs` - Color interpolation and gradients
- `categorical_scales.rs` - Band, point, and ordinal scales
- `logarithmic_scales.rs` - Logarithmic, power, and symlog transformations
- `quantile_threshold.rs` - Quantile, quantize, and threshold scales
- `ticks_and_options.rs` - Tick generation and scale options
- `datafusion_domains.rs` - Domains fitted in Rust to query summaries (`--features datafusion`)
- `datafusion_plan.rs` - Domains fitted and scales applied in one plan (`--features datafusion`)

Run examples with:
```bash
cargo run --release --example linear_scale
cargo run --release --example color_scales
```

## Contributing

Contributions are welcome! Please see the main Avenger repository for contribution guidelines.