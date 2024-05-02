# Avenger: A visualization engine and renderer
Avenger is an early stage prototype of a new foundational rendering library for information visualization (InfoVis) systems. Avenger defines a 2D scenegraph representation tailored to the needs of InfoVis systems. To start with, the initial application of Avenger is to serve as an alternative, GPU accelerated, rendering backend for the Vega ecosystem.

# Try it out in Python with Vega-Altair
The `avenger` Python package provides two [custom Altair renderers](https://altair-viz.github.io/user_guide/custom_renderers.html) named `avenger-png` and `avenger-html`. The `avenger-png` renderer relies on vl-convert to extract the Vega scenegraph corresponding to a chart and then uses Avenger in the Python kernel to render the chart to a static PNG image. The `avenger-html` renderer relies on the `avenger-vega-renderer` JavaScript + WebAssembly package to run Avenger in the browser, which enables rendering interactive charts.

First, install Vega-Altair, vega-datasets, avenger, and vl-convert-python
```
pip install -U altair vega_datasets avenger "vl-convert-python>=1.2.3"
```

Then import Vega-Altair and activate the `avenger-html` renderer

```python
import altair as alt
alt.renderers.enable('avenger-png', scale=1)
```

Then create and display an Vega-Altair chart as usual:

```python
import altair as alt
from vega_datasets import data

source = data.cars()

chart = alt.Chart(source).mark_circle(size=60).encode(
    x='Horsepower',
    y='Miles_per_Gallon',
    color='Origin',
)
chart
```
![cars_scatter](https://github.com/jonmmease/avenger/assets/15064365/d661e142-c7c5-4816-a375-49a73985bb6d)

Or, activate the `avenger-png` renderer

```python
alt.renderers.enable('avenger-png', scale=1)
```

Then create and display an Vega-Altair chart as usual:

```python
import altair as alt
from vega_datasets import data

source = data.cars()

chart = alt.Chart(source).mark_circle(size=60).encode(
    x='Horsepower',
    y='Miles_per_Gallon',
    color='Origin',
)
chart
```
![cars_scatter](https://github.com/jonmmease/avenger/assets/15064365/d661e142-c7c5-4816-a375-49a73985bb6d)

Or, convert the chart to a PNG rendered by Avenger:

```python
import avenger
png = avenger.altair_utils.chart_to_png(chart, scale=1)
with open("scatter.png", "wb") as f:
    f.write(png)
```
## Comparison to vl-convert for PNG export
There aren't currently many advantages to using Avenger to render Altar charts to PNG as compared with vl-convert, which performs rendering using [resvg](https://github.com/RazrFalcon/resvg). Performance is generally comparable, though Avenger can be a bit faster for charts with a large number of symbol instances.

One advantage is that Avenger's text rendering support is based on [COSMIC Text](https://github.com/pop-os/cosmic-text), which supports emoji (unlike resvg's text handling). For example, here is the result of rendering the emoji example from https://altair-viz.github.io/gallery/isotype_emoji.html using Avenger:

![isotype_emoji](https://github.com/jonmmease/avenger/assets/15064365/91a1db89-9bdd-46f3-b540-c7d7bcaac3c2)

We expect it will be possible to substantially improve PNG rendering performance in the future by performing the conversion from Vega scenegraph to Avenger scene graph in the Deno JavaScript runtime using the `avenger-vega-renderer` package.

## Comparison to standard Vega-Altair html renderer
The standard Vega-Altair `html` renderer relies on Vega's built-in `canvas` renderer, which performs rendering using an [HTML Canvas](https://developer.mozilla.org/en-US/docs/Web/API/Canvas_API).  Initial informal benchmarking indicates that Avenger is significantly faster than the canvas renderer for marks with many instances (e.g. large scatter plots), however the time Vega takes to generate the scenegraph is often much larger than the rendering time, so the perceived benefit of enabling the `avenger-html` renderer isn't always noticeable.

One motivation for integrating VegaFusion with Avenger in the future is to make it possible to render large marks without passing the full dataset through Vega's scene graph generation logic,

# Try it out from Rust
Avenger is written in Rust, and may be used directly from Rust.

## Run native
For example, to launch a wgpu rendered visualization in a native window, run the following example:
```
cd examples/wgpu-winit
cargo run --release
```

## Build wasm
Avenger may be compiled to WASM with rendering performed in WebGPU or WebGL2 (If WebGPU is not supported by the browser)

To build the example above to WASM, run the following:
```
cd examples/wgpu-winit
wasm-pack build --target web
```

Then open `examples/wgpu-winit/index.html` in a web browser.

## Export PNG
The `PngCanvas` struct can be used to convert a scene graph to a PNG image headlessly. See the tests in
avenger-wgpu/tests/test_image_baselines.rs for usage.

# How it works
Avenger's core is written in Rust and is composed of the following crates:
 - `avenger`: The core `SceneGraph` representation that is independent of rendering backend
 - `avenger-vega`: Logic to construct an Avenger `SceneGraph` from a Vega scenegraph.
 - `avenger-wgpu`: Logic to render an Avenger `SceneGraph` using [wgpu](https://github.com/gfx-rs/wgpu).
 - `avenger-vega-test-data`: Crate that uses vl-convert to generate test data. For each baseline vega spec, `avenger-vega-test-data` will write out a vega scenegraph is JSON format along with a PNG rendering of the chart (which uses resvg). The tests in avenger-wgpu/tests/test_image_baselines.rs then input the scenegraph, render to PNG with `avenger-wgpu`, and compare the results to the baselines using structural similarity.
 - `avenger-python`: Python bindings to `avenger`, `avenger-vega`, and `avenger-wgpu` which also provides a custom Altair renderer (See above). 
 - `avenger-vega-renderer`: JavaScript implementation of a Vega renderer that interfaces with the WASM build of `avenger` and `avenger-wpu`. 

**Note:** The initial plan was for `avenger-vega-renderer` to use the `avenger-vega` crate to convert the Vega scenegraph to Avenger scenegraph, but it turned out to be really slow to transfer the Vega scenegraph from JavaScript to WASM and deserialize it with serde. It's much faster to move this logic to JavaScript and only pass TypedArrays from JavaScript to WASM. We expect the same paradigm will be much faster for PNG export with vl-convert, so we may eventually remove `avenger-vega` unless we find other motivation for keeping this logic in Rust.

# Roadmap / Ambitions
This is a hobby project with large ambitions. Where it goes will largely depend on whether other people get involved. But here are a few potential directions.
 - Alternative PNG export engine for vl-convert: Avenger could slot in next to resvg as an alternative png rendering engine in vl-convert. One advantage is that is supports emoji. The current Avenger performance isn't better than the current resvg approach across the board, but with some optimization this could likely be made a fair bit faster.
 - Alternative Vega renderer: An initial implementation of a Vega renderer using a WASM build of Avenger is now available. More evaluation is needed to understand its performance characteristics. If the community feels it has significant benefits for the ecosystem, then this renderer could be integrated throughout the ecosystem (e.g. Altair and the Vega Editor are two candidates).
 - Serverside rendering of select marks with VegaFusion:  VegaFusion performs deep analysis of Vega specifications to optimize then and pre-evaluate data transformations in the Python kernel. This logic could be extended to include pre-rendering select marks using Avenger. This provides the benefit of sending only a png image to the browser rather than the full input dataset. Work would be needed to figure out how to support interactivity in this scenario.
 - Vega native: Combining Avenger and [VegaFusion](https://vegafusion.io/) gets us close to being able to render entire Vega visualizations without JavaScript. More thought is needed, but it may make sense to add support for scales and guides (axes, legends, colorbars, etc.) to the Avenger SceneGraph. Then VegaFusion could have a mode that produces an Avenger scenegraph for rendering.  To support interactive charts, Avenger could adopt Vega's Event Stream system (https://vega.github.io/vega/docs/event-streams/).
 - Matplotlib backend: Avenger could potentially serve as a rendering backend for Matplotlib (as an alternative to Agg) that provides GPU acceleration. See https://matplotlib.org/stable/users/explain/figure/backends.html#the-builtin-backends.
 - CPU rendering: The wgpu backend requires GPU support, so it would be useful to have a CPU rendering option as well. This could be based on [tinyskia](https://github.com/RazrFalcon/tiny-skia), which is what resvg uses.
 - SVG/PDF rendering: Renderers that produce SVG and PDF documents from the Avenger SceneGraph could be written.

# Call for help
Do any of the ambitions above sound interesting? Are you interested in learning Rust? Please [start a discussion](https://github.com/jonmmease/avenger/discussions) and get involved.
