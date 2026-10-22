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

`WindowCanvas` and `PngCanvas` store each layer in a separate renderer. `set_layer` installs one
scene without reinstalling other layers. Layers are indexed from zero at the bottom and drawn in
order from their retained GPU buffers. Updating an overlay preserves the chart's installed
buffers, but each frame still draws the chart. `set_scene` replaces the stack with one scene.
`WindowCanvas::set_resize_frame` sets an outline and resize handles drawn above the layers.

Scene marks are rendered using specialized GPU shaders:
- **Instanced rendering**: Efficient batching for marks with many instances
- **Multi-mark rendering**: Handles complex marks requiring tessellation

The renderer rasterizes each label with `avenger-typst-label`, the Typst-based label crate, into texture atlases. The `rich_text` example draws rich labels, and plain text and markup cut to a width with an ellipsis:

![Rich labels, and labels cut to a width](../docs/images/rich-text.png)

## Images that load by URL

A canvas or renderer resolves a scene's image resources, the images that name their pixels by URL,
through the `ImageResolver` that `CanvasConfig::images` or `AvengerRendererConfig::images` sets,
such as an avenger-image `ImageCache`.

- **Each frame**: the renderer asks the resolver for the installed scene's resources, in the
  scene's order, so a scene that lists the middle of the view first loads it first. A resource
  that isn't ready draws its placeholder, and without a resolver, every resource draws its
  placeholder. An image draws on the frame after it arrives, without a new scene. A mesh image
  instance that waited for its image draws it in place: the renderer uploads the image to a
  texture array layer, writes the layer into the instance's vertices, and builds nothing else.
  An image mark's image that changes installs the scene again, as a resize does, since image
  marks draw from atlases that the renderer builds when it installs a scene.
- **Leases**: the renderer leases the installed scene's images, so the resolver keeps them in
  memory. It takes a new scene's lease before it releases the last scene's, so images that both
  draw stay.
- **Status**: `image_status` reports the images that are pending and the ones that failed, as of
  the last frame. `frame_stats` reports whether the last frame installed its scene, whether it
  built its buffers and atlases again, which it does only after an install or a resize, the
  bytes of mesh image textures that it uploaded, and the bytes that their arrays hold.
- **Mesh image textures**: mesh images draw from texture arrays that stay on the GPU from scene
  to scene, one set for each image size, with a layer for each image. An image keeps its layer,
  found by the identity of its pixels' `Arc`, while it's alive and no scene needs the layer, so a
  map tile uploads once while the resolver holds it, and a pan uploads only the tiles that come
  into view. A scene's new image takes a layer whose image is gone, or else the least recently
  used layer that the scene doesn't draw, and a size gets another array when a scene fills its
  arrays. An array frees on the GPU once its images are gone, as when the scene stops drawing
  them and the image cache evicts them, and `frame_stats` reports the bytes that the arrays
  hold. Each layer clamps to its own edges. Image marks draw from atlases that the renderer
  builds when it installs a scene.

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
