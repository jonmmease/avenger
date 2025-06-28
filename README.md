# Avenger: A visualization engine and renderer
Avenger is an early stage prototype of a new foundational rendering library and visualization primitives for information visualization (InfoVis) systems. Avenger defines a 2D scenegraph representation tailored to the needs of InfoVis systems, along with visualization primitives like scales and guides. And an event stream system for interactive visualizations inspired by Vega.

# Try it out from Rust
Avenger is written in Rust, and may be used directly from Rust.

## Run native
For example, to launch a wgpu rendered visualization in a native window, run the following example:
```
cd examples/iris-pan-zoom
cargo run --release
```

## Build wasm
Avenger may be compiled to WASM with rendering performed in WebGPU or WebGL2 (If WebGPU is not supported by the browser)

To build the example above to WASM, run the following:
```
cd examples/iris-pan-zoom
wasm-pack build --target web --release
```

Then open `examples/iris-pan-zoom/index.html` in a web browser.

## Export PNG
The `PngCanvas` struct can be used to convert a scene graph to a PNG image headlessly. See the tests in
avenger-wgpu/tests/test_image_baselines.rs for usage.

# How it works
Avenger's core is written in Rust and is composed of the following crates:
 - `avenger-app`: Application framework for building interactive Avenger-based visualizations
 - `avenger-common`: Shared types and utilities for the Avenger visualization system
 - `avenger-eventstream`: Interactive event handling system for Avenger visualizations
 - `avenger-geometry`: Geometry processing and spatial indexing for Avenger scene graphs
 - `avenger-guides`: Visualization guide generation for Avenger (axes, legends, colorbars)
 - `avenger-image`: Image loading and processing for the Avenger rendering system
 - `avenger-scales`: High-performance data visualization scales with mappings between data domains and visual ranges
 - `avenger-scenegraph`: The core `SceneGraph` representation that is independent of rendering backend
 - `avenger-text`: Text measurement and rasterization for the Avenger rendering system
 - `avenger-vega-scenegraph`: Logic to construct an Avenger `SceneGraph` from a Vega scenegraph
 - `avenger-vega-test-data`: Crate that uses vl-convert to generate test data. For each baseline vega spec, `avenger-vega-test-data` will write out a vega scenegraph in JSON format along with a PNG rendering of the chart (which uses resvg). The tests in avenger-wgpu/tests/test_image_baselines.rs then input the scenegraph, render to PNG with `avenger-wgpu`, and compare the results to the baselines using structural similarity
 - `avenger-wgpu`: Logic to render an Avenger `SceneGraph` using [wgpu](https://github.com/gfx-rs/wgpu)
 - `avenger-winit-wgpu`: Native window application runner for interactive Avenger visualizations
 

# Roadmap / Ambitions
This is a hobby project with large ambitions. Where it goes will largely depend on whether other people get involved. But here are a few potential directions.
 - Serverside rendering of select marks with VegaFusion:  VegaFusion performs deep analysis of Vega specifications to optimize then and pre-evaluate data transformations in the Python kernel. This logic could be extended to include pre-rendering select marks using Avenger. This provides the benefit of sending only a png image to the browser rather than the full input dataset. Work would be needed to figure out how to support interactivity in this scenario.
 - Vega native: Combining Avenger and [VegaFusion](https://vegafusion.io/) gets us close to being able to render entire Vega visualizations without JavaScript. More thought is needed, but it may make sense to add support for scales and guides (axes, legends, colorbars, etc.) to the Avenger SceneGraph. Then VegaFusion could have a mode that produces an Avenger scenegraph for rendering.  To support interactive charts, Avenger could adopt Vega's Event Stream system (https://vega.github.io/vega/docs/event-streams/).
 - Matplotlib backend: Avenger could potentially serve as a rendering backend for Matplotlib (as an alternative to Agg) that provides GPU acceleration. See https://matplotlib.org/stable/users/explain/figure/backends.html#the-builtin-backends.
 - CPU rendering: The wgpu backend requires GPU support, so it would be useful to have a CPU rendering option as well. This could be based on [tinyskia](https://github.com/RazrFalcon/tiny-skia), which is what resvg uses.
 - SVG/PDF rendering: Renderers that produce SVG and PDF documents from the Avenger SceneGraph could be written.

# Call for help
Do any of the ambitions above sound interesting? Are you interested in learning Rust? Please [start a discussion](https://github.com/jonmmease/avenger/discussions) and get involved.
