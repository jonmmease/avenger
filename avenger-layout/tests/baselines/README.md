# avenger-layout SVG baseline gallery

Every test in `tests/svg_baselines.rs` builds a [`Layout`] through the public
API, solves it in one step with `Layout::solve`, and snapshots the solution's
SVG. The committed SVGs are the snapshots; the PNGs beside them are a viewing
convenience (regenerated at bless time, never compared).

Regenerate after an intentional change:

```sh
AVENGER_LAYOUT_BLESS=1 cargo test --release -p avenger-layout --test svg_baselines
```

## Visual language

- **Filled blue**: honest content rectangles (a leaf's measured size, a
  grid's solved track extent, a contained node's derived content).
- **Dashed gray outline**: the **slot** (allotment) where it differs from
  the content — granted space the content does not fill (stretch slack,
  ragged cousins, fixed-track overflow).
- **Chrome slabs** (drawn behind): gray margins, amber strips, dark green
  legend, dark red guide — declared chrome positioned by the solver, carved
  outside-in with vertical sides before horizontal (corners belong to the
  outer-more / vertical-first slab).
- **Demand strips** beside each content rectangle encode three things:
  **hue** is the stratum (red = guide, green = legend), **shade** is nesting
  depth (darker shallower), and **solid vs hatched** is requested vs granted
  — where a hatched strip extends past the solid one inside it, coordination
  granted that region more than it asked for.
- **Black frame**: the solved canvas (envelope). A color key row identifies
  every kind present in the scene.

## Gallery

| Baseline | What it demonstrates |
|---|---|
| `row_gaps_and_min_gap_floor` | boundary chrome becomes gaps via `max(min_gap, after + before)`; outer offsets |
| `column_cell_align_ragged_children` | ragged children positioned by per-child `CellAlign`; slack as dashed slots |
| `solution_query_by_id_and_path` | regions queryable by caller id and structural path; slot vs content distinct |
| `grid_spans_holes_and_base_cell_size` | column span, empty slot, per-track base size floor |
| `grid_edge_demand_layers_and_gap_law` | layered guide/legend demands and the gap law |
| `chromed_leaf_solve_for_content` | canvas-style chart: canvas given, chrome carved, content gets the remainder |
| `chromed_leaf_solve_for_envelope` | plot-area-sized chart: content given, envelope derived (the default mode) |
| `chromed_leaf_solve_for_margins` | both given: flexible margins absorb the slack |
| `chromed_leaf_content_min_overflows_envelope` | the content floor wins over a too-small envelope |
| `per_axis_allocation_plot_sized_height` | width figure-sized + height plot-area-sized in one solve |
| `strips_all_four_sides_corner_rule` | repeatable strips on every side; corner-ownership carving |
| `nested_grid_with_chrome` | chrome on a grid node (header on the guide stratum, legend on the legend stratum) replacing stacked edges |
| `allocation_stretches_tracks_evenly` | default `StretchTracks`: slots grow, leaf content stays honest |
| `track_size_fixed_and_flex` | CSS-style tracks: rigid `Fixed`, weighted `Flex` leftover split, content `Auto` |
| `distribute_space_between` | free space into gaps when no `Flex` track exists |
| `fixed_track_content_overflow` | `Fixed` never grows: oversized content overflows honestly |
| `uniform_share_tolerates_ragged_counts` | uniform policy merge across cousins with different track counts |
| `nested_facet_columns_coordinated` | the nested facet lowering: measured vs coordinated panels, granted chrome hatched, gaps absorbing it |
| `shared_charts_coordinate_in_one_solve` | two chart-like groups under one root made congruent by a share key |
| `edge_reservation_layered_vs_strip` | ways to reserve the same 18px against a layered cousin (guide 14 + legend 8): a `legend` stratum coexists (gaps lift to 32), a `guide` stratum is contained in the same stratum (26), a `strip` reserves the contained extent with a solver-positioned slab (22) |
| `min_slack_asymmetric_share` | the min-slack rule: asymmetric offers, congruent cousins, honest slack |
| `share_group_shape_mismatch_diagnostics` | mismatched non-uniform group skipped and reported |
| `aspect_contain_fit_slack` | the aspect-ratio recipe's terminal state: standing slot-vs-content slack |
