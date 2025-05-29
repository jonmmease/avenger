## Generate Test Data
This is a binary crate responsible for generating image scenegraph and image test baselines for testing avenger renderers.

From the project root
```
cd avenger-vega-test-data
cargo run
```

## How it works
A collection of Vega specs are located under `avenger-vega-test-data/vega-specs` inside category directories.
For example: `avenger-vega-test-data/vega-specs/rect/stacked_bar.vg.json`.

The main binary entry point scans this directory and for each Vega spec it uses 
[`vl-convert-rs`](https://github.com/vega/vl-convert) to
output the following three files in matching category directory under `avenger-vega-test-data/vega-scenegraphs`
 1. `{spec_name}.sg.json`: This is the JSON representation of the scenegraph that Vega generates for this Vega spec.
   This corresponds to the vega scenegraph schema defined in the `avenger-vega` crate.
 2. `{spec_name}.png`:  PNG rendering of the Vega spec as created by vl-convert. To generate this, vl-convert exports the
   chart to SVG and then renders the SVG to PNG using [resvg](https://github.com/RazrFalcon/resvg). This PNG image serves
   as the baseline that avenger rendered PNGs are compared to.

Image baselines are tested in `avenger-wgpu/tests/test_image_baselines.rs`.
