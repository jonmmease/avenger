## Generate Test Data
This is a binary crate responsible for generating image scenegraph and image test baselines for testing avenger renderers.

From the project root
```sh
cd avenger-vega-test-data
cargo run --release
```

## How it works
A collection of Vega specs are located under `avenger-vega-test-data/vega-specs` inside category directories.
For example: `avenger-vega-test-data/vega-specs/rect/stacked_bar.vg.json`.

The main binary entry point scans this directory and for each Vega spec it uses 
[`vl-convert-rs`](https://github.com/vega/vl-convert) to
output the following two files in matching category directory under `avenger-vega-test-data/vega-scenegraphs`
 1. `{spec_name}.sg.json`: This is the JSON representation of the scenegraph that Vega generates for this Vega spec.
   This corresponds to the Vega scenegraph schema defined in the `avenger-vega-scenegraph` crate.
 2. `{spec_name}.png`:  PNG rendering of the Vega spec as created by vl-convert. To generate this, vl-convert exports the
   chart to SVG and then renders the SVG to PNG using [resvg](https://github.com/RazrFalcon/resvg). This PNG image serves
   as the baseline that avenger rendered PNGs are compared to.

Image baselines are tested in `avenger-wgpu/tests/test_image_baselines.rs`.

## Fixed map tiles

The `vl-convert/geoScale` and `vl-convert/maptile_background` specs embed their
OpenStreetMap tiles as PNG data URLs in a `tile_images` dataset. A lookup joins
the calculated tile URL to these fixed bytes. The generated scenegraphs contain
the same data URLs, so baseline tests do not download map tiles.

These fixtures cover the specs' initial view. If the tile calculation, viewport,
or zoom changes, add any newly required tiles to `tile_images` before regenerating
the references. Keep the original URL as the lookup key and in the accessible
image description. Refresh both the PNG and scenegraph with vl-convert, using
Vega/resvg to generate the reference PNG.

The eight tiles were captured on 2026-09-11. Their source URLs are recorded in
`tile_images`. Map images © OpenStreetMap contributors. See
[OpenStreetMap copyright and attribution](https://www.openstreetmap.org/copyright).
The specs retain their visible map attribution.

The two references were regenerated with `vl-convert-python==1.8.0`, using
`register_font_directory` for this crate's `fonts` directory, `vega_to_png` at
scale 2, and `vega_to_scenegraph`. This uses the same vl-convert version as the
Rust generator's lockfile. All remote URLs were disallowed during regeneration
except the separately versioned `vega-datasets@v1.29.0` world topology used by
`maptile_background`. The generated scenegraphs include the vector paths too,
so rendering either map test needs no network access.
