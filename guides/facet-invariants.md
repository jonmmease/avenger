# Facet System Invariants

This document catalogs the critical invariants in the facet system that aren't enforced by the type system. They must be maintained through careful coding practices and testing.

For the overall architecture, see `avenger-chart/docs/architecture/facet-system.md`. For the conceptual overflow model, see `guides/facet-layout-overflow-model.md`.

## Pipeline invariants

### 1. Phase Ordering: Tree → Coord → Coordination → Render

| Property | Value |
|----------|-------|
| **Invariant** | The four phases must execute in strict order: build `EvaluatedFacetTree`, run `FacetBandMeasurePipeline` per band, run `coordinate_facet_measurement_tree` (fold → solve → install → adopt), then render. |
| **Consequence if Violated** | Undefined behavior — incorrect sizes, positions, or missing data. |
| **Current Enforcement** | Implicit through function call structure in `coords.rs` and `plot/compiled/rendering.rs`. |

### 2. Coordination Must Complete Before Render Reads `coord_measurement`

| Property | Value |
|----------|-------|
| **Invariant** | `FacetBandCoordMeasurement` is mutated by coordination (channel install, geometry adoption). Renderers must read it only after `coordinate_facet_measurement_tree` returns (`CoordinationCheckpoint::Adopted`). |
| **Consequence if Violated** | Renderers see local (pre-coordination) values instead of adopted ones — cell sizes and padding will be wrong. |
| **Current Enforcement** | `CoordinationCheckpoint` gating in `plot/compiled/rendering.rs`; the coordinated measurement is the one boxed onto `ComponentsMeasurement.coord_measurement`. |
| **Location** | `avenger-chart/src/facet/coordination.rs`, `avenger-chart/src/plot/compiled/rendering.rs` |

### 3. `CoordinationScopeKey` Identifies Bands That Must Agree

| Property | Value |
|----------|-------|
| **Invariant** | Bands sharing the same coordination scope (`CoordinationScopeKey::container_group(kind, depth, "{axis}:{field_identity}")`) must end coordination with identical `subplot_cross_size`, identical `padding_inner_px`, and consistent edge overflow. |
| **Consequence if Violated** | Misaligned grids — adjacent facets at the same depth will visibly drift apart. |
| **Current Enforcement** | The pre-solve fold (`compute_band_folds`) takes the group max slot count over the share key; the real-tree solve (`facet/tree_solve.rs::tree_solved_round`) lowers cousins sharing their scope key so the solver equalizes spacing and overflow asks (`Region.coordinated`); every band reads the merged values as views into the installed solution handle. |
| **Location** | `avenger-chart/src/plot/compiled/coordination_scope.rs` (key), `coord.rs` `coordination_scope_key_for_depth` (key construction) |

### 4. Visibility Logic Must Match in Measure and Render

| Property | Value |
|----------|-------|
| **Invariant** | The visibility decision for guides (axes, labels, titles) must be identical in measurement and render passes. |
| **Consequence if Violated** | Missing or extra guides — space allocated but nothing rendered, or rendered outside allocated space. |
| **Current Enforcement** | Shared sharing-policy functions (`show_axis_labels`, `show_axis_title`, `legend_owner_for_position`) consulted in both passes. |
| **Location** | `avenger-chart/src/facet/sharing_policy.rs`, `avenger-chart/src/facet/guide/band_guide_engine.rs` |

## Coordination laws (fold–solve–adopt)

### 5. The Staleness Law: Adoption Moves Geometry, Never Re-Measures

| Property | Value |
|----------|-------|
| **Invariant** | Within one coordination run, measured chrome and overflow stay frozen at their epoch measurements (`measured_overflow`, `overflow_cells` are never recomputed from live cells after coordination mutates them). Adoption moves plot areas and scale ranges only. Re-measurement belongs exclusively to the refinement loop (`max_refinement_passes`). |
| **Consequence if Violated** | Re-measuring mid-run rebuilds scales, which changes y-domains (a long-standing failure mode), and breaks the determinism of repeated coordination runs on one tree. |
| **Current Enforcement** | Adopt runs through the no-remeasure substrate (`retarget_*_no_remeasure`); the epoch fields are captured once at measurement. |
| **Location** | `avenger-chart/src/facet/coordination_apply.rs` (`run_adopt`), `coord.rs` (substrate) |

### 6. The Transition Law: Adopt Computes Targets from the Installed Artifact, Never from Retained Slot Geometry

| Property | Value |
|----------|-------|
| **Invariant** | The solve lowers cells at their LIVE sizes, so the retained solution's slot geometry describes the current state, not the next one. Adoption must apply the transition (bandwidth of the band scale at the active coordinated layout; legend shrink inside a fixed container), with law values read from the installed solution artifact. |
| **Consequence if Violated** | Copying raw slots double-counts when coordinated n exceeds live n (tracks never shrink below lowered content) and inherits the chrome-accounting boundary (the lowered tree keeps nested epoch chrome inside cell slots; the render model absorbs the same chrome into parent gaps). Three failed F2 experiments document this. |
| **Current Enforcement** | `run_adopt` derives every target through the bandwidth/division law; the census `adopt_delta` probe reports the per-run transition size, and the geometry probe asserts adoption lands on the solve's fixed point (settled slots ≡ live geometry). |
| **Location** | `avenger-chart/src/facet/coordination_apply.rs`, `avenger-chart/src/facet/tree_solve.rs` (census) |

### 7. The Ownership Realization Order

| Property | Value |
|----------|-------|
| **Invariant** | Realized legend-slab ownership clears at channel install (`clear_realized_owned_legend_slabs`) and re-realizes only after geometry adoption (`realize_coordinated_child_frame_allocations` at the end of each band's adopt). |
| **Consequence if Violated** | Owned slabs computed against stale geometry; double-counted or missing legend space at band boundaries. |
| **Current Enforcement** | `apply_requirement_pass` clears; `run_adopt` and the parent-retarget substrate realize after geometry writes. |
| **Location** | `avenger-chart/src/facet/coordination_apply.rs`, `coord.rs` |

### 8. Fold Determinism: Lowering Inputs Are Construction-Time Values

| Property | Value |
|----------|-------|
| **Invariant** | `compute_band_folds` (slot counts, lowered gaps) reads only snapshot-stable construction-time values — never values coordination itself writes — so repeated coordination runs on one tree lower identically. |
| **Consequence if Violated** | The pipeline's checkpoint re-runs (rendering invokes coordination several times per render) would produce drifting solutions; the census idempotence probe would show nonzero deltas. |
| **Current Enforcement** | The fold walk reads `local_layout` / `min_slot_count` / `slot_sharing` only; the census idempotence probe (≈ 0.0 suite-wide) pins it. |
| **Location** | `avenger-chart/src/facet/tree_solve.rs` |

### 9. Placement and the Solve Are One Law, Two Evaluators

| Property | Value |
|----------|-------|
| **Invariant** | Explicit placement (`explicit_placement()`, computed on read) and the coordination solve share every law value (folded slot counts, gaps, outer spacing). At the settled state the on-read strip solve must reproduce the retained solution's band tracks, up to the chrome-accounting boundary (slot-side vs gap-side accounting of nested epoch chrome). |
| **Consequence if Violated** | Render placement drifts from coordinated geometry — cells visually overlap or gap. |
| **Current Enforcement** | The census placement probe compares per-cell starts/sizes and band extents between the two evaluators (175/181 explicit-band runs exact; the residual is the documented chrome-accounting boundary in the two `facet_plot_size_variants` tests). |
| **Location** | `avenger-chart/src/facet/placement.rs`, `avenger-chart/src/facet/tree_solve.rs` (probe) |

## Data invariants

### 10. Inner Facet Subplots Must Not Have Their Own Data

| Property | Value |
|----------|-------|
| **Invariant** | When a `Plot` is used as the `subplot` of a `Facet`, it must not have `.data()` attached. Data flows from the outer facet to inner plots via per-cell `data_override`. |
| **Consequence if Violated** | Inner plot uses its own data instead of the filtered slice, producing incorrect cell contents. |
| **Current Enforcement** | Explicit check in `Facet::compile` returns `AvengerChartError::InvalidArgument`. |
| **Location** | `avenger-chart/src/facet/marks/facet.rs` (FacetRow and FacetCol compile hooks) |

### 11. `measure_overflow()` Must Handle Empty DataFrames

| Property | Value |
|----------|-------|
| **Invariant** | `measure_overflow()` and per-cell measurement must produce a valid measurement for empty cells. |
| **Consequence if Violated** | Panic or incorrect layout for empty cells, common with `FacetEmptyCellPolicy::EmptySubplot`. |
| **Current Enforcement** | `FacetBandSemantics` classifies cells as `DomainPlaceholder` / `DataEmpty` / `Populated`; fallback scale builders are used for empty cells when needed. |
| **Location** | `avenger-chart/src/facet/band_attributes.rs`, `coord.rs` (`empty_facet_band_measurement`) |

### 12. Scale Storage Uses Base Channel Names

| Property | Value |
|----------|-------|
| **Invariant** | Scales must be stored and retrieved using base channel names (e.g. `y` not `y2`). |
| **Consequence if Violated** | Runtime lookup failure — channels like `y2` share the same scale as `y` and depend on the base-name convention. |
| **Current Enforcement** | None — convention only. |
| **Location** | Scale lookup in `avenger-chart/src/plot/compiled/scales.rs` |

## Type/state invariants

### 13. `FacetContext.position < FacetContext.grid_dimensions`

| Property | Value |
|----------|-------|
| **Invariant** | A subplot's `position` (zero-indexed) must always be less than `grid_dimensions` on the same axis. |
| **Consequence if Violated** | Index out of bounds during coordination or rendering. |
| **Current Enforcement** | `FacetBandMeasurePipeline` constructs positions from `cells.len()`; consumers iterate `cells` rather than indexing externally. |

### 14. `SharingLevel` Normalization

| Property | Value |
|----------|-------|
| **Invariant** | User-facing `Sharing` is normalized to `SharingLevel(u8)` at the entry point: `Free → SharingLevel(0)`, `Shared → SharingLevel(255)`, `Level(n) → SharingLevel(n)`. Comparisons must use the normalized form. |
| **Consequence if Violated** | Sharing groups computed from raw `Sharing` won't match those computed from normalized `SharingLevel`; ancestor truncation paths diverge. |
| **Current Enforcement** | `FacetOptions::with_slot_sharing` calls `to_normalized()`. Downstream code uses `SharingLevel` exclusively. |
| **Location** | `avenger-chart/src/facet/marks/facet_config.rs`, `avenger-chart/src/facet/sharing_level.rs` |

### 15. Concurrency Limits in Scale Precompute

| Property | Value |
|----------|-------|
| **Invariant** | The `FacetScalePrecomputeStore` Mutex must not be held across an `await`. |
| **Consequence if Violated** | Async deadlock or starvation when multiple facet subtrees are precomputed concurrently. |
| **Current Enforcement** | Lock-and-clone pattern; the store is only locked for the duration of map insertion / lookup. |
| **Location** | `avenger-chart/src/facet/scale_precompute.rs` |

## Known scaffolding hazards

These aren't invariants in the strict sense — they're places where the *absence* of an invariant has produced a latent risk. Watch for them when modifying related code.

(Resolved since the original catalog: `FacetBandProbeLayout` is now consistently `#[cfg(test)]`-gated at both its definition and every use; `FacetSizingCoordinationStrategy` was deleted along with the legacy retarget/final-propagation machinery.)

### A. Disabled Outer-Edge Computation in `derive_layout_plan`

| Property | Value |
|----------|-------|
| **Hazard** | `coord.rs` computes `raw_outer_start`/`raw_outer_end` via `derive_outer_edges`, then overrides with `outer_start = 0.0; outer_end = 0.0`. The computed values feed only a debug log. |
| **Why it's there** | Edge legend slabs are routed upward as sibling-boundary or root residual demand instead, so leaf plot-area sizes stay uniform across sibling facet subtrees; the raw computation remains for diagnostics. |
| **Risk** | A reader may believe the outer-edge mechanism is active and waste time tracing call sites. |

## See Also

- `avenger-chart/docs/architecture/facet-system.md` — Module layout and the fold–solve–install–adopt pipeline.
- `guides/facet-layout-overflow-model.md` — Conceptual overflow stacking model and aggregation rules.
- `avenger-chart/book/src/docs/coordinate-systems/faceting/` — User-facing facet usage.
