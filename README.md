# Vega wgpu renderer
This repo holds an early stage prototype of what it could look like to build an alternative [Vega](https://vega.github.io/vega/) 
visualization renderer in Rust using [wgpu](https://github.com/gfx-rs/wgpu). This is not useful yet (and may never be).

We're exploring how general the various components are, and so the names and structures of
crates are still changing. If you're interested in collaborating on a visualization renderer for
a non-Vega application, please open an issue.

# Try it out

## Run native
To launch a wgpu rendered visualization in a native window, run the following:
```
cd sg2d-wgpu
cargo run
```

## Build wasm
To build the example above to WASM + WebGL2, run the following:
```
cd sg2d-wgpu
wasm-pack build --target web
```

Then open `vega-wgpu-renderer/index.html` in a web browser.

## Export PNG
The `PngCanvas` struct can be used to convert a scene graph to a PNG image headlessly. See the tests in
vega-wgpu-renderer/tests/test_image_baselines.rs for usage.

# Motivation
Vega currently ships with two renderers: `svg` (which outputs SVG) and `canvas` (which renders to HTML Canvas).
The hypothesis is that Canvas rendering is expensive enough for charts with large marks that there will be
substantial benefit to rendering on the GPU. Admittedly, more experimentation and benchmarking is still needed to justify
this hypothesis. For this use case, the wgpu renderer would be compiled to WASM and registered with Vega as a third
renderer type.

Separately, this is an important step in the direction of developing a fully native Vega renderer. Combined with VegaFusion,
most of the logic for fully native rendering (starting with the Vega spec) will be available in Rust. There will be more
work to do to update VegaFusion to generate scene graphs, but this is a definite possibility.

Short of supporting full native rendering, this wgpu renderer may be useful to vl-convert to accelerate PNG static 
image export, and to VegaFusion to support serverside rendering of large marks. 

# Testing
To start with, the most valuable contribution of this project is probably the testing infrastructure. By relying on
vl-convert, a collection of input Vega specs are rendered to PNG and converted to scene graphs. The GPU rendered
PNG images are then compared for similarity to the baselines using structural similarity. See the `gen-test-data`
crate for more information.

Note: These tests aren't running on GitHub Actions yet due to a `MakeWgpuAdapterError` error that
needs to be diagnosed.
