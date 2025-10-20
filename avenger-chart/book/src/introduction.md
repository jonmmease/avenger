# Introduction

Avenger Chart is a high-performance, GPU-accelerated charting library for Rust. It provides a declarative API for creating data visualizations, and plugs into the broader Avenger runtime for interactive scenarios such as pan/zoom (chart-level interaction APIs are on the roadmap).

## Key Features

- **GPU-Accelerated**: Uses the `wgpu` stack (Vulkan, Metal, DX12, or WebGPU) for fast rendering
- **DataFusion Integration**: Built on Apache Arrow and DataFusion for efficient data processing
- **Declarative API**: Grammar-of-graphics inspired design
- **Flexible Theming**: CSS-based styling system

## Before You Start

These docs assume a couple of things about your environment and background:

- **Runtime & tooling**: You can run async Rust (typically with `tokio`) and have a `wgpu`-compatible backend available or configured for headless testing.
- **Data access**: Your data is reachable through Apache DataFusion (Arrow record batches, Parquet, CSV, SQL sources, etc.).
- **Concepts**: You’re comfortable with DataFusion expressions (`col()`, `lit()`, aggregates) since they drive the channel mappings shown throughout the guide.

If any of the above is new, the [Installation](./getting-started/installation.md) page covers the environment setup, and the [DataFusion Integration](./docs/datafusion-integration.md) page provides background on working with DataFusion expressions.

## Design Philosophy

Avenger Chart follows these core principles:

1. **Separation of Concerns**: Data, scales, and visual encoding are explicitly separate
2. **Coordinate System Abstraction**: Different coordinate systems (Cartesian, Polar, etc.) share the same mark types
3. **Type Safety**: Leverage Rust's type system to catch errors at compile time
4. **Performance**: GPU acceleration and Arrow-native data processing

## Quick Example

Here's a simple scatter plot using the famous iris dataset:

> **Note:** All code examples in this documentation are interactive and show rendered output. Click the <i class="fa fa-eye"></i> icon in code blocks to reveal hidden boilerplate. See [Your First Plot](./getting-started/first-plot.md#about-documentation-examples) for details on how examples work.

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let df = ctx
    .read_parquet(avenger_sample_data::iris_path(), ParquetReadOptions::default())
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
- [Coordinate Systems](./docs/coordinate-systems/index.md) - Understand the key abstractions
- [Your First Plot](./getting-started/first-plot.md) - Learn by example
