# Installation

## Requirements

- Rust 1.75 or later (matches DataFusion 48's MSRV)
- A GPU backend supported by `wgpu` (Vulkan, Metal, DX12, or WebGPU)

## Adding Avenger Chart to Your Project

Add the crates that Avenger Chart expects in your `Cargo.toml`:

```toml
[dependencies]
avenger-chart = "0.1.0"
datafusion = "48.0.1"
tokio = { version = "1.37", features = ["macros", "rt-multi-thread"] }
palette = "0.7.6"
```

`tokio` powers the async APIs, and `palette` is handy for color palettes in the examples.

## Platform Notes

- macOS: Metal support is available out of the box.
- Linux: Install the Vulkan SDK or appropriate drivers (`libvulkan1` on Debian/Ubuntu).
- Windows: DirectX 12 is used automatically when available; otherwise Vulkan is used.

`wgpu` automatically picks the best available backend at runtime; you can override it via the `WGPU_BACKEND` environment variable if needed.

## Verifying Installation

Create a quick sanity check that compiles and renders an empty scatter plot:

```rust,no_run
use avenger_chart::prelude::*;
use avenger_chart::render::WgpuRenderer;
use datafusion::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = SessionContext::new();

    let plot = Chart::<Cartesian>::new().mark(Symbol::new().x(lit(0.0)).y(lit(0.0)));

    let compiled = plot.compile(&ctx).await?;
    let renderer = WgpuRenderer::new();
    renderer
        .write_png(&compiled, &ctx, None, "avenger-chart-smoke-test.png")
        .await?;
    println!("Rendered avenger-chart-smoke-test.png");
    Ok(())
}
```

Then run:

```bash
cargo run --release
```

You should see `avenger-chart-smoke-test.png` appear in your project directory with no runtime errors, confirming that the toolchain is configured correctly.

## Next Steps

Continue to [Your First Plot](./first-plot.md) to create your first visualization.
