# Introduction

Avenger Chart is a high-performance, GPU-accelerated charting library for Rust. It provides a declarative API for creating data visualizations with support for interactive features like pan and zoom.

## Key Features

- **GPU-Accelerated**: Uses WebGPU/WebGL2 for fast rendering
- **DataFusion Integration**: Built on Apache Arrow and DataFusion for efficient data processing
- **Declarative API**: Grammar-of-graphics inspired design
- **Interactive**: Built-in support for pan, zoom, and other interactions
- **Flexible Theming**: CSS-based styling system

## Design Philosophy

Avenger Chart follows these core principles:

1. **Separation of Concerns**: Data, scales, and visual encoding are explicitly separate
2. **Coordinate System Abstraction**: Different coordinate systems (Cartesian, Polar, etc.) share the same mark types
3. **Type Safety**: Leverage Rust's type system to catch errors at compile time
4. **Performance**: GPU acceleration and Arrow-native data processing

## Quick Example

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::*;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
# let ctx = datafusion::execution::context::SessionContext::new();
# let df = ctx.read_csv("data.csv", datafusion::prelude::CsvReadOptions::default()).await?;
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            .x(col("gdp_per_capita"))
            .y(col("life_expectancy"))
            .fill_with(col("continent"), |c| c.legend(|l| l.title("Continent")))
    );
# Ok(())
# }
```

## Next Steps

- [Installation](./getting-started/installation.md) - Get started with Avenger Chart
- [Core Concepts](./concepts/coordinate-systems.md) - Understand the key abstractions
- [Guides](./guides/scatter-plots.md) - Learn by example
