## Generate Test Data
This is a binary crate responsible for generating image baseline test data for the vega-wgpu-renderer.

From the project root
```
cargo run -p gen-test-data
```

## How it works
A collection of Vega specs are located under `gen-test-data/vega-specs` inside category directories.
For example: `gen-test-data/vega-specs/rect/stacked_bar.vg.json`.

The main binary entry point scans this directory and for each Vega spec it uses 
[`vl-convert-rs`](https://github.com/vega/vl-convert) to
output the following three files in matching category directory under `vega-wgpu-renderer/tests/specs`
 1. `{spec_name}.sg.json`: This is the JSON representation of the scenegraph that Vega generates for this Vega spec.
   This is used as the input for the wgpu renderer.
 2. `{spec_name}.dims.json`: This JSON file contains the final chart width, height, and origin values.
   These are values that are not contained in the scenegraph, but passed as arguments to the renderer when registered with Vega.
 3. `{spec_name}.png`:  PNG rendering of the Vega spec as created by vl-convert. To generate this, vl-convert exports the
   chart to SVG and then renders the SVG to PNG using [resvg](https://github.com/RazrFalcon/resvg). This PNG image serves
   as the baseline that wgpu rendered PNGs are compared to.

Image baselines are tested in `vega-wgpu-renderer/tests/test_image_baselines.rs`. Image similarity is measured
using [DSSIM](https://github.com/kornelski/dssim).
