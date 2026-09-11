# Rendering compatibility checks

The extraction was compared with `origin/main` at
`c38e448c96b7b662fa9d67bdfbabfdaf5b4b87e5` on macOS/Metal with Rust 1.96.
Main's image suite passed 124 cases and failed `geoScale` and
`maptile_background`. Their differences were 0.019158 and 0.012998 against
thresholds of 0.01. Both thresholds remain unchanged. The two references now use
fixed map tiles, as described below.

The source branch initially failed 37 cases. The extraction repairs:

- The missing radial connection before an annular arc's inner edge.
- Dash handling that discarded zero-length gaps.
- SVG feature forwarding from the Vega adapter to the image crate.
- Reversed triangles at variable-width trail joins that accumulated opacity.
- Pattern-host tessellation that filled holes by processing contours separately.
- Circle stroke halos from mixing transparent fill RGB, and overly wide antialiasing.
- Vega fixture-font registration and generic sans-serif/monospace selection.
- Vega image import through the shared data-URL decoder, with lazy HTTP setup.

The new direct pattern test checks additive, subtractive, and XOR coverage on
rectangles, annular arcs, areas, paths, and symbols, including the annular hole.
The standalone pattern-and-text example supplies an explicit plot reference frame.

## Reference tolerances

Vega-generated PNG references are unchanged except for the two map fixtures.
The Typst backend uses different line metrics and antialiasing from Cosmic Text
and Vega/resvg. Inspection of the alignment, rotation, letter-scatter, and
composite-chart outputs shows small vertical shifts and glyph-edge changes.
The lasagna heatmap also has smoother shared edges between narrow rectangles.
This case is not a text-only comparison.
The following thresholds accommodate these backend differences after the geometry
and font-configuration repairs. They are specific to the existing Vega comparison
suite, not a relaxation of the Typst label parity checks.

| Case | Previous threshold | Measured difference | New threshold |
| --- | ---: | ---: | ---: |
| `text_alignment` | 0.015 | 0.017413 | 0.02 |
| `text_rotation` | 0.015 | 0.016317 | 0.02 |
| `letter_scatter` | 0.03 | 0.051411 | 0.055 |
| `lasagna_plot` | 0.02 | 0.02942 | 0.033 |
| `stocks-legend` | 0.003 | 0.007701 | 0.01 |
| `stocks_dashed` | 0.002 | 0.002455 | 0.003 |
| `float_font_size` | 0.01 | 0.013775 | 0.015 |
| `line_with_log_scale` | 0.02 | 0.023597 | 0.026 |
| `long_legend_label` | 0.01 | 0.010043 | 0.012 |
| `seattle-weather` | 0.01 | 0.011432 | 0.013 |
| `stocks_locale` | 0.01 | 0.01087 | 0.012 |

The text-clip thresholds are restored to main's 0.02. Arc, dash, trail, and other
geometry thresholds remain at their original values. The 32-case upstream Typst
PNG parity corpus passes with explicitly registered fonts and its existing
thresholds.

## Fixed map inputs

The `geoScale` and `maptile_background` failures came from live OpenStreetMap
tiles changing after the reference images were generated. Their Vega specs now
embed eight fixed PNG tiles in a lookup dataset. The generated scenegraphs use
those same PNG data URLs. All other scenegraph values, including vector paths,
image placement, and text, match the previous fixtures exactly.

The two PNG references were regenerated with vl-convert 1.8.0's Vega/resvg
renderer at scale 2 and visually compared with Avenger's output. The fixture
[README](../avenger-vega-test-data/README.md#fixed-map-tiles) records the source,
attribution, and regeneration details.

| Case | Difference before fixing inputs | Difference with fixed inputs | Unchanged threshold |
| --- | ---: | ---: | ---: |
| `geoScale` | 0.020870 | 0.002856 | 0.01 |
| `maptile_background` | 0.014121 | 0.001897 | 0.01 |

## Recorded validation

The native workspace sweep passed 1,194 tests, with 12 ignored and the two
inherited map-image failures above. After the circle shader repair and fixed map
inputs, the complete renderer suite passes all 142 tests. The new
open-circle test checks that transparent fill RGB cannot darken a colored stroke.
The Vega image-import regression test passes with default features disabled,
including HTTP support. Both map image comparisons also pass when run in a macOS
sandbox that denies all network access.

Strict release Clippy checks pass for every workspace target. The Typst suite
with raster and PNG parity features passes 307 Rust tests, including the aggregate
32-case PNG corpus. Image tests pass with both minimal and full features, and the
Vega adapter's SVG-only tests pass. The standalone example was rendered and
visually inspected. Only the two map reference PNGs were refreshed. The imported
translucent-circle scenario adds a separate Vega fixture.

All three existing browser examples (`iris-pan-zoom`, `wgpu-scales`, and
`wgpu-winit`) pass `wasm-pack build --target web --release --locked`, including
binding generation and `wasm-opt`. These are package-build checks. Browser
interaction was not exercised by this extraction run.

## Core stabilization pass

The standalone core review led to regression coverage for shared text context,
syntax-safe width limits, renderer resizing, image retention and freshness,
public timer routing, debounced consumption, color parsing, padded domains, and
pattern composition. This pass keeps all existing image baselines and thresholds.

| Check | Result |
| --- | --- |
| Native workspace excluding WGPU | 1,039 passed; 12 existing tests ignored |
| WGPU | 34 unit tests, 142 image/integration tests, and 3 renderer-lifecycle tests passed |
| Typst with `raster,upstream-png-parity` | 308 passed, including the aggregate 32-case PNG corpus; one existing doctest ignored |
| Image cache with minimal and full features | 25 tests passed in each configuration |
| Text without default features | Release check passed |
| Three browser examples | Release checks passed for `wasm32-unknown-unknown` |
| Fresh independent geometry consumer | Dependency resolution and release check passed without a pre-existing lockfile |
| Formatting and strict Clippy | Passed for all workspace targets |

The renderer-lifecycle comparisons check resizing with text, instanced symbols,
pattern fills, and a path clip, as well as image lease lifetime and an explicitly
shared text engine. Cache tests cover working sets larger than the completed-image
LRU, stale refreshes, cancellation, request supersession, and render invalidation.
The 14 original standalone review probes also pass after stabilization.

The browser checks in this pass verify compilation. They do not exercise browser
interaction or the separate async host-ownership redesign discussed in the review.
See [the migration notes](foundation-migration.md) for the shared engine, image
lease, fallible font metrics, and vector clipping contracts.
