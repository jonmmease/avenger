# avenger-wgpu

GPU-accelerated rendering backend for Avenger scene graphs.

## Purpose

This crate implements Avenger's primary rendering engine using wgpu for cross-platform GPU acceleration. It converts scene graphs into optimized GPU resources and handles the rendering pipeline from geometry tessellation through final pixel output.

## Architecture

The crate provides a renderer and the canvases built on it:
- **AvengerWgpuRenderer**: Rendering into a texture that an application owns (see Embedding
  below)
- **WindowCanvas**: Native rendering to window surfaces via winit
- **PngCanvas**: Offscreen rendering to PNG images
- **HtmlCanvasCanvas**: WASM rendering to HTML canvas elements

Scene marks are rendered using specialized GPU shaders:
- **Instanced rendering**: Efficient batching for marks with many instances
- **Multi-mark rendering**: Handles complex marks requiring tessellation

The renderer rasterizes each label with `avenger-typst-label`, the Typst-based label crate, into texture atlases. The `rich_text` example draws rich labels, and plain text and markup cut to a width with an ellipsis:

![Rich labels, and labels cut to a width](../docs/images/rich-text.png)

## Integration

- **Input**: Takes `avenger-scenegraph::SceneGraph` structures
- **Output**: Renders to GPU surfaces or exports to PNG images
- **Text**: Uses `avenger-typst-label` for text measurement and rasterization
- **Geometry**: Leverages Lyon for path tessellation and GPU vertex generation

## Embedding in a wgpu application

An application that owns a wgpu device draws Avenger scenes with `AvengerWgpuRenderer`, without
an Avenger window or PNG canvas. Presentation and frame scheduling stay with the application.

- **Shared pipelines**: create one `AvengerGpuContext` for the device, and build each renderer
  from it. Renderers that draw into the same texture format with the same sample count share
  their pipelines, so a dashboard of charts compiles them once.
- **Targets**: `AvengerRenderTarget` describes the texture view to draw into, its format and
  sample count, which must match the renderer's, and the load operation. `OffscreenTarget`
  allocates an ordinary texture, with a multisampled texture that resolves into it when the
  sample count is above 1.
- **Recording**: `AvengerWgpuRenderer::encode` records a frame into the application's command
  encoder. `build_frame_commands` returns the frame as command buffers instead.
- **A rectangle of a larger texture**: `AvengerRenderTarget::with_viewport_origin` draws the
  scene into the rectangle the renderer's size at that origin. A clear would erase the whole
  texture, so such a target loads its contents, and the scene draws its own background.
- **sRGB targets**: a renderer that draws into an sRGB texture writes linear colors, which the
  texture encodes. Opaque colors match the colors that a texture without sRGB encoding stores.
  Translucent colors blend in linear space, so anti-aliased edges and overlaps differ slightly
  from a browser's. A view of the texture without sRGB encoding gives a browser's blending.
- **Compositing**: marks drawn onto `TRANSPARENT_CLEAR` leave premultiplied colors, which
  composite with `One, OneMinusSrcAlpha` blending.
- **The wgpu version**: the renderer takes the application's `wgpu` types, so the application
  uses the wgpu that `avenger_wgpu::wgpu` re-exports. Each wgpu upgrade in Avenger
  changes this interface.

The `egui-embed` example draws two charts into one texture that an egui application shows, with
tooltips from the charts' scenes:

```bash
cargo run --release -p egui-embed
```
