# Avenger: A visualization engine and renderer
Avenger is an early stage prototype of a new foundational rendering library for information visualization (InfoVis) systems. Avenger defines a 2D scenegraph representation tailored to the needs of InfoVis systems. To start with, the initial application of Avenger is to serve as an alternative, GPU accelerated, rendering backend for Vega visualizations.

# Try it out in Python with Vega-Altair
The `avenger` Python package provides a [custom Altair renderer](https://altair-viz.github.io/user_guide/custom_renderers.html) named `avenger-png`. This renderer relies on vl-convert to extract the vega scenegraph corresponding to a chart and then uses Avenger to render the chart to a static PNG image.

First, install altair, vega-datasets, avenger, and vl-convert-python
```
pip install -U altair vega_datasets avenger "vl-convert-python>=1.2.3"
```

Then import Altair and activate the `avenger-png` renderer

```python
import altair as alt
alt.renderers.enable('avenger-png', scale=1)
```

Then create and display an Altair chart as usual:

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
## Comparison to vl-convert
There aren't currently many advantages to using Avenger to render Altar charts to PNG as compared with vl-convert, which performs rendering using [resvg](https://github.com/RazrFalcon/resvg). Performance is generally comparable, though Avenger can be a bit faster for charts with a large number of symbol instances.

One advantage is that Avenger's text rendering support is based on [COSMIC Text](https://github.com/pop-os/cosmic-text), which supports emoji (unlike resvg's text handling). For example, here is the result of rendering the emoji example from https://altair-viz.github.io/gallery/isotype_emoji.html using Avenger:

![isotype_emoji](https://github.com/jonmmease/avenger/assets/15064365/91a1db89-9bdd-46f3-b540-c7d7bcaac3c2)

# Try it out from Rust
Avenger is written in Rust, and may be used directly from Rust.

## Run native
For example, to launch a wgpu rendered visualization in a native window, run the following:
```
cd examples/wgpu-winit
cargo run --release
```

## Build wasm
Avenger may be compiled to WASM with rendering performed in WebGL2. Note that image and text marks aren't yet supported under WASM.

To build the example above to WASM + WebGL2, run the following:
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

# Roadmap / Ambitions
This is a hobby project with large ambitions. Where it goes will largely depend on whether other people get involved. But here are a few potential directions.
 - Alternative PNG export engine for vl-convert: Avenger could slot in next to resvg as an alternative png rendering engine in vl-convert. One advantage is that is supports emoji. The current Avenger performance isn't better than the current resvg approach across the board, but with some optimization this could likely be made a fair bit faster.
 - Alternative Vega renderer: Avenger can already be compiled to wasm (with some limitations), so it should be possible to write a little JavaScript glue code to register Avenger as a third Vega renderer (in addition to `svg` and `canvas`). Whether there would be a performance benefit to doing this is still TBD.
 - Vega native: Combining Avenger and [VegaFusion](https://vegafusion.io/) gets us close to being able to render Vega visualizations without JavaScript. More thought is needed, but it may make sense to add support for scales and guides (axes, legends, colorbars, etc.) to the Avenger SceneGraph. Then VegaFusion could have a mode that produces an Avenger scenegraph for rendering.  To support interactive charts, Avenger could adopt Vega's Event Stream system (https://vega.github.io/vega/docs/event-streams/).
 - Matplotlib backend: Avenger could potentially serve as a rendering backend for Matplotlib (as an alternative to Agg) that provides GPU acceleration. See https://matplotlib.org/stable/users/explain/figure/backends.html#the-builtin-backends.
 - CPU rendering: The wgpu backend requires GPU support, so it would be useful to have a CPU rendering option as well. This could be based on [tinyskia](https://github.com/RazrFalcon/tiny-skia), which is what resvg uses.
 - SVG/PDF rendering: Renderers that produce SVG and PDF documents from the Avenger SceneGraph could be written.

# Call for help
Do any of the ambitions above sound interesting? Are you interested in learning Rust? Please [start a discussion](https://github.com/jonmmease/avenger/discussions) and get involved. 