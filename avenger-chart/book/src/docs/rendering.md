# Rendering and Output

Avenger Chart provides a flexible rendering architecture that separates visualization specification from output generation. This allows the same chart to be rendered to different formats.

## How Rendering Works

The rendering process follows a clear pipeline:

```
Plot → compile() → CompiledPlot → evaluate() → EvaluatedPlot → Renderer → Output
```

The key insight is that `EvaluatedPlot` contains a **backend-independent scene graph** — a complete description of all visual elements (shapes, text, colors, positions) without any renderer-specific code. Different renderers can consume this scene graph to produce different output formats.

## Current Renderer: WgpuRenderer

✅ **Available Now**

The `WgpuRenderer` provides GPU-accelerated rendering via WebGPU/wgpu, producing high-quality PNG images.

### Basic Usage

```rust,no_run
use avenger_chart::prelude::*;
use avenger_chart::render::WgpuRenderer;
use datafusion::prelude::*;

# async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(Symbol::new().x(col("x")).y(col("y")))
    .canvas_size(800.0, 600.0);

let compiled = plot.compile(&ctx).await?;

// Create renderer and export to PNG
let renderer = WgpuRenderer::new();
renderer.write_png(&compiled, &ctx, None, "output.png").await?;
# Ok(())
# }
```

### Configuration Options

#### Scale Factor (High-DPI Rendering)

Control the resolution of the output image:

```rust,no_run
use avenger_chart::render::WgpuRenderer;

# fn example() {
// Default scale = 1.0 (canvas size = image size)
let renderer = WgpuRenderer::new();

// 2x scale for high-DPI displays (Retina)
let renderer_2x = WgpuRenderer::new().with_scale(2.0);

// 3x scale for very high resolution
let renderer_3x = WgpuRenderer::new().with_scale(3.0);
# }
```

With `.canvas_size(800.0, 600.0)`:
- `scale = 1.0` → 800×600 pixel PNG
- `scale = 2.0` → 1600×1200 pixel PNG (sharper on high-DPI displays)
- `scale = 3.0` → 2400×1800 pixel PNG

**When to use higher scale factors:**
- Retina/high-DPI displays (2.0)
- Print quality (2.0-3.0)
- Large format displays (3.0+)

#### Canvas Configuration

Canvas size is set on the plot itself:

```rust,no_run
# use avenger_chart::prelude::*;
# fn example() {
let plot = Plot::<Cartesian>::new()
    .canvas_size(1200.0, 800.0);  // Width × Height in logical pixels
# }
```

The canvas size represents "logical pixels" — the actual output resolution is `canvas_size × scale`.

### Rendering Without Writing to Disk

You can render to an in-memory image:

```rust,no_run
use avenger_chart::render::WgpuRenderer;
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;

# async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
# let plot = Plot::<Cartesian>::new().data(df);
# let compiled = plot.compile(&ctx).await?;
let renderer = WgpuRenderer::new();
let image = renderer.render(&compiled, &ctx, None).await?;
// `image` is an image::RgbaImage - can be saved, processed, etc.
# Ok(())
# }
```

### Requirements

`WgpuRenderer` requires:
- GPU access (or software rendering via wgpu)
- Vulkan, Metal, DX12, or WebGPU backend support

For environments without GPU access, see the planned CPU-based renderer below.

## Planned Renderers

The following renderers are planned for future releases. See the [Roadmap](../roadmap.md) for status updates.

### SVG Renderer

🔜 **Planned**

An SVG renderer will produce scalable vector graphics suitable for:
- Web applications (inline SVG, smaller file sizes)
- Vector editing in tools like Inkscape or Illustrator
- Lossless scaling to any resolution
- Text remains searchable and selectable

**Planned API:**
```rust,no_run
use avenger_chart::render::SvgRenderer;

# async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
# let plot = Plot::<Cartesian>::new().data(df);
# let compiled = plot.compile(&ctx).await?;
let renderer = SvgRenderer::new();
renderer.write_svg(&compiled, &ctx, None, "output.svg").await?;
# Ok(())
# }
```

### PDF Renderer

🔜 **Planned**

A PDF renderer for publication-quality documents:
- Multi-page documents
- Embedded fonts
- Print-ready output
- Archival quality

**Planned API:**
```rust,no_run
use avenger_chart::render::PdfRenderer;

# async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
# let plot = Plot::<Cartesian>::new().data(df);
# let compiled = plot.compile(&ctx).await?;
let renderer = PdfRenderer::new();
renderer.write_pdf(&compiled, &ctx, None, "output.pdf").await?;
# Ok(())
# }
```

### CPU-Based PNG Renderer

🔜 **Planned**

A CPU-based renderer using [tinyskia](https://github.com/RazrFalcon/tinyskia) for environments without GPU support:
- Server-side rendering without GPU
- Docker containers
- Headless environments
- Embedded systems

**Planned API:**
```rust,no_run
use avenger_chart::render::CpuRenderer;

# async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
# let plot = Plot::<Cartesian>::new().data(df);
# let compiled = plot.compile(&ctx).await?;
let renderer = CpuRenderer::new().with_scale(2.0);
renderer.write_png(&compiled, &ctx, None, "output.png").await?;
# Ok(())
# }
```

Performance will be slower than GPU rendering but requires no special hardware.

## Interactive Rendering

🔜 **Planned**

Avenger Chart is built on the [Avenger](https://github.com/jonmmease/avenger) rendering engine, which includes support for interactive visualizations with pan, zoom, and event handling.

> **Heads up:** The chart crate itself does not ship built-in controllers yet. You can still preserve metadata for future interactions by calling `.details([...])` on marks, but interactivity today is provided by embedding a compiled plot in an Avenger application (see `avenger-app`/`avenger-eventstream`). Documentation here will expand once first-party controllers land.

Interactive features are planned for Avenger Chart, including:
- Pan and zoom interactions
- Tooltip system
- Selection interactions
- Interactive parameter updates

See the [Roadmap](../roadmap.md) for details on interactive feature development.

## Batch Rendering

You can render multiple charts efficiently by reusing the renderer:

```rust,no_run
use avenger_chart::render::WgpuRenderer;
# use avenger_chart::prelude::*;
# use datafusion::prelude::*;

# async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
# let plots = vec![Plot::<Cartesian>::new().data(df.clone())];
let renderer = WgpuRenderer::new();

for (i, plot) in plots.iter().enumerate() {
    let compiled = plot.compile(&ctx).await?;
    renderer.write_png(&compiled, &ctx, None, &format!("chart_{}.png", i)).await?;
}
# Ok(())
# }
```

The renderer instance maintains GPU resources across renders, improving performance for multiple charts.

## Understanding the Scene Graph

The `EvaluatedPlot` contains a scene graph — a tree of visual elements with these properties:

- **Shapes**: Rectangles, circles, paths, text, images
- **Styling**: Fill colors, strokes, opacity
- **Layout**: Positions, sizes, transforms
- **Z-order**: Drawing order

This scene graph is **renderer-independent**. Different renderers interpret it differently:
- WgpuRenderer → GPU tessellation and rasterization
- SvgRenderer (planned) → SVG path commands and elements
- PdfRenderer (planned) → PDF drawing operators

This separation allows you to:
1. Create visualization once (`compile`)
2. Evaluate with data (`evaluate`)
3. Render to multiple formats without recompiling

## Common Patterns

### Pattern: Export at Multiple Resolutions

```rust,no_run
# use avenger_chart::prelude::*;
# use avenger_chart::render::WgpuRenderer;
# use datafusion::prelude::*;
# async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
# let plot = Plot::<Cartesian>::new().data(df);
let compiled = plot.compile(&ctx).await?;

// Standard resolution
WgpuRenderer::new().with_scale(1.0)
    .write_png(&compiled, &ctx, None, "chart_1x.png").await?;

// High-DPI
WgpuRenderer::new().with_scale(2.0)
    .write_png(&compiled, &ctx, None, "chart_2x.png").await?;

// Print quality
WgpuRenderer::new().with_scale(3.0)
    .write_png(&compiled, &ctx, None, "chart_3x.png").await?;
# Ok(())
# }
```

### Pattern: Conditional Rendering Backend

```rust,no_run
# use avenger_chart::prelude::*;
# use avenger_chart::render::WgpuRenderer;
# use datafusion::prelude::*;
# async fn example(has_gpu: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
# let ctx = SessionContext::new();
# let df = ctx.read_csv("data.csv", CsvReadOptions::new()).await?;
# let plot = Plot::<Cartesian>::new().data(df);
# let compiled = plot.compile(&ctx).await?;
if has_gpu {
    // Fast GPU rendering
    WgpuRenderer::new()
        .write_png(&compiled, &ctx, None, "output.png").await?;
} else {
    // CPU fallback (when available)
    // CpuRenderer::new().write_png(&compiled, &ctx, None, "output.png").await?;
    todo!("CPU renderer not yet available");
}
# Ok(())
# }
```

## See Also

- [Your First Plot](../getting-started/first-plot.md) - Basic rendering workflow
- [Roadmap](../roadmap.md) - Planned rendering features
- [The Compilation Pipeline](../compilation.md) - Understanding compile vs evaluate
