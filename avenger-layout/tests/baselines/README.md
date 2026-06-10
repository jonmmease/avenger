# avenger-layout SVG baselines

Each SVG here is the rendered output of a like-named example test in
[`../svg_baselines.rs`](../svg_baselines.rs). The tests build layouts
through the crate's public API and snapshot them via `DebugScene::to_svg`,
so this directory doubles as a visual gallery of what every solver does —
open any file in a browser.

Run the suite:

```sh
cargo test -p avenger-layout --features svg --test svg_baselines
```

On mismatch the actual SVG is written to `../failures/<name>.svg`
(gitignored). After an intentional change, regenerate everything and
review the diffs:

```sh
AVENGER_LAYOUT_BLESS=1 cargo test -p avenger-layout --features svg --test svg_baselines
```

Blessing also writes a PNG next to each SVG (rendered with `resvg`, 2x
scale, white background) so the gallery is viewable anywhere SVGs are
inconvenient. The PNGs are a viewing convenience only: rasterization
goes through system fonts, so they are not byte-stable across machines
and the tests never compare them — the SVG string is the snapshot.

## Reading the SVGs

- black frame: the arrangement's content bounds (for frames, the solved
  envelope extent)
- filled blue: content rectangles
- overflow strips beside content rects encode three things: **hue** =
  layer (red = inner/guide-like, green = remainder up to the total),
  **shade** = nesting depth (dark at the base layer, lighter as nesting
  deepens; a frame's own chrome strips are the darkest step of the same
  ramps; key labels gain `d0`/`d1` when a scene has multiple depths), **solid vs hatched** =
  requested vs coordinated (the key swatches show solid|hatched side by
  side). Where a hatched band extends past the solid strip inside it,
  coordination grew that region's allocation. Frame margin/outer/inner
  strips are unlabeled — the key identifies them
- frame chrome strips: gray margins, amber bands (titles), green outer
  (legend-like), red inner (guide-like). Bands/outer/inner span the
  content on their cross axis (as realized chart chrome does); margins
  span the full envelope. A color key row below the scene identifies the
  kinds; strips too thin to hold a label rely on it
- tree regions are labeled with their structural index path (`c0`,
  `c12` = second item's third child, …); the prefix shows ancestry
- crosses: placement origins

## Gallery

| Baseline | Shows |
|---|---|
| `band_horizontal_gaps_and_min_gap_floor` | boundary chrome becomes gaps, floored by `min_gap`; outer offsets |
| `band_vertical_cross_align_center` | vertical band, ragged children centered on the cross axis |
| `band_placement_handoff_markers` | `to_placement_solution` origins as markers over the band |
| `grid_spans_holes_and_base_cell_size` | column span, empty slot, per-track base size floor |
| `grid_edge_demand_layers_and_gap_law` | layered envelopes and the gap rule `max(min_gap, after + before)` |
| `uniform_tracks_merged_policy` | two `UniformTracks` policies merged by max, then solved |
| `frame_envelope_fixed_chart_chrome` | canvas-style frame: chrome subtracted from a fixed envelope |
| `frame_content_fixed_envelope_derived` | content-first frame: envelope is the sum of all layers |
| `frame_envelope_and_content_fixed_margin_slack` | both fixed: margins absorb the slack |
| `frame_content_min_floor_overflows_envelope` | the content floor wins over a too-small envelope |
| `tree_nested_with_stacked_chrome` | nested band with stacked inner/outer chrome, framed with margins; layered vs geometric envelopes asserted in the test |
| `tree_allocation_stretches_tracks_evenly` | the same framed tree given a larger allocation: every track stretches |
| `alignment_merges_grids_across_instances` | two instances before alignment, then both re-solved on the merged grid |
| `alignment_coordinates_framed_charts` | the full loop on whole charts: measure → align across instances → re-solve; canvases end up identical, granted chrome shows hatched |
| `frame_wrapping_facet_tree` | a faceted chart in miniature: the tree's envelope becomes the frame's reservations, the frame's content allocates the tree |
