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

Here's a simple scatter plot using the famous iris dataset:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
# let iris_path = format!("{}/../tests/data/iris.parquet", env!("CARGO_MANIFEST_DIR"));
let df = ctx
    .read_parquet(iris_path, ParquetReadOptions::default())
    .await
    ?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Iris Dataset")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .fill_with(col("species"), |c| {
                c.scale_with::<Ordinal>(|s| s)
                    .legend(|l| l.title("Species"))
            })
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;
Ok(evaluated)
```

This example demonstrates:
- Loading data from Parquet files with DataFusion
- Creating a scatter plot with `Symbol` marks
- Mapping columns to visual channels (x, y, fill)
- Customizing color scales and legends
- The compile → evaluate workflow for rendering

## Next Steps

- [Installation](./getting-started/installation.md) - Get started with Avenger Chart
- [Core Concepts](./concepts/coordinate-systems.md) - Understand the key abstractions
- [Guides](./guides/scatter-plots.md) - Learn by example
