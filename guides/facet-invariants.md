# Facet System Invariants

This document catalogs the critical invariants in the facet system that aren't enforced by the type system. They must be maintained through careful coding practices and testing.

For the overall architecture, see `avenger-chart/docs/architecture/facet-system.md`. For the conceptual overflow model, see `guides/facet-layout-overflow-model.md`.

## Pipeline invariants

### 1. Phase Ordering: Tree → Coord → Coordination → Render

| Property | Value |
|----------|-------|
| **Invariant** | The four phases must execute in strict order: build `EvaluatedFacetTree`, run `FacetBandMeasurePipeline` per band, run `coordinate_facet_measurement_tree`, then render. |
| **Consequence if Violated** | Undefined behavior — incorrect sizes, positions, or missing data. |
| **Current Enforcement** | Implicit through function call structure in `coords.rs` and `plot/compiled/rendering.rs`. |

### 2. Coordination Must Complete Before Render Reads `coord_measurement`

| Property | Value |
|----------|-------|
| **Invariant** | `FacetBandCoordMeasurement` is mutable across coordination stages. Renderers must read it only after `FacetCoordinationStage::FinalPropagation` completes. |
| **Consequence if Violated** | Renderers see Phase-2 (local) values instead of Phase-3 (coordinated) values — cell sizes and padding will be wrong. |
| **Current Enforcement** | `CoordinationCheckpoint` gating in `plot/compiled/rendering.rs`; the coordinated measurement is the one boxed onto `ComponentsMeasurement.coord_measurement`. |
| **Location** | `avenger-chart/src/facet/coordination.rs`, `avenger-chart/src/plot/compiled/rendering.rs` |

### 3. `CoordinationGroupKey` Identifies Bands That Must Agree

| Property | Value |
|----------|-------|
| **Invariant** | Bands sharing the same `CoordinationGroupKey { depth, facet_group_identity }` must end coordination with identical `subplot_cross_size`, identical `padding_inner_px`, and consistent edge overflow. |
| **Consequence if Violated** | Misaligned grids — adjacent facets at the same depth will visibly drift apart. |
| **Current Enforcement** | `coordinate_facet_measurement_tree` distributes max-reduced values back to every band in each group. |
| **Location** | `avenger-chart/src/facet/coordination.rs:68` (key), `coord.rs:1100` (key construction) |

### 4. Visibility Logic Must Match in Measure and Render

| Property | Value |
|----------|-------|
| **Invariant** | The visibility decision for guides (axes, labels, titles) must be identical in measurement and render passes. |
| **Consequence if Violated** | Missing or extra guides — space allocated but nothing rendered, or rendered outside allocated space. |
| **Current Enforcement** | Shared sharing-policy functions (`show_axis_labels`, `show_axis_title`, `legend_owner_for_position`) consulted in both passes. |
| **Location** | `avenger-chart/src/facet/sharing_policy.rs`, `avenger-chart/src/facet/guide/band_guide_engine.rs` |

## Data invariants

### 5. Inner Facet Subplots Must Not Have Their Own Data

| Property | Value |
|----------|-------|
| **Invariant** | When a `Plot` is used as the `subplot` of a `Facet`, it must not have `.data()` attached. Data flows from the outer facet to inner plots via per-cell `data_override`. |
| **Consequence if Violated** | Inner plot uses its own data instead of the filtered slice, producing incorrect cell contents. |
| **Current Enforcement** | Explicit check in `Facet::compile` returns `AvengerChartError::InvalidArgument`. |
| **Location** | `avenger-chart/src/facet/marks/facet.rs:459-466` (FacetRow), `marks/facet.rs:705-712` (FacetCol) |

### 6. `measure_overflow()` Must Handle Empty DataFrames

| Property | Value |
|----------|-------|
| **Invariant** | `measure_overflow()` and per-cell measurement must produce a valid measurement for empty cells. |
| **Consequence if Violated** | Panic or incorrect layout for empty cells, common with `FacetEmptyCellPolicy::EmptySubplot`. |
| **Current Enforcement** | `FacetBandSemantics` classifies cells as `DomainPlaceholder` / `DataEmpty` / `Populated`; fallback scale builders are used for empty cells when needed. |
| **Location** | `avenger-chart/src/facet/band_attributes.rs:96`, `coord.rs:1797` (`empty_facet_band_measurement`) |

### 7. Scale Storage Uses Base Channel Names

| Property | Value |
|----------|-------|
| **Invariant** | Scales must be stored and retrieved using base channel names (e.g. `y` not `y2`). |
| **Consequence if Violated** | Runtime lookup failure — channels like `y2` share the same scale as `y` and depend on the base-name convention. |
| **Current Enforcement** | None — convention only. |
| **Location** | Scale lookup in `avenger-chart/src/plot/compiled/scales.rs` |

## Type/state invariants

### 8. `FacetContext.position < FacetContext.grid_dimensions`

| Property | Value |
|----------|-------|
| **Invariant** | A subplot's `position` (zero-indexed) must always be less than `grid_dimensions` on the same axis. |
| **Consequence if Violated** | Index out of bounds during coordination or rendering. |
| **Current Enforcement** | `FacetBandMeasurePipeline` constructs positions from `cells.len()`; consumers iterate `cells` rather than indexing externally. |

### 9. `SharingLevel` Normalization

| Property | Value |
|----------|-------|
| **Invariant** | User-facing `ScaleSharing` is normalized to `SharingLevel(u8)` at the entry point: `Free → SharingLevel(0)`, `Shared → SharingLevel(255)`, `Level(n) → SharingLevel(n)`. Comparisons must use the normalized form. |
| **Consequence if Violated** | Sharing groups computed from raw `ScaleSharing` won't match those computed from normalized `SharingLevel`; ancestor truncation paths diverge. |
| **Current Enforcement** | `FacetOptions::with_slot_sharing` calls `to_normalized()`. Downstream code uses `SharingLevel` exclusively. |
| **Location** | `avenger-chart/src/facet/marks/facet_config.rs`, `avenger-chart/src/facet/sharing_level.rs` |

### 10. Concurrency Limits in Scale Precompute

| Property | Value |
|----------|-------|
| **Invariant** | The `FacetScalePrecomputeStore` Mutex must not be held across an `await`. |
| **Consequence if Violated** | Async deadlock or starvation when multiple facet subtrees are precomputed concurrently. |
| **Current Enforcement** | Lock-and-clone pattern; the store is only locked for the duration of map insertion / lookup. |
| **Location** | `avenger-chart/src/facet/scale_precompute.rs:82` |

## Known scaffolding hazards

These aren't invariants in the strict sense — they're places where the *absence* of an invariant has produced a latent bug. Watch for them when modifying related code.

### A. `FacetBandProbeLayout` is `#[cfg(test)]`-gated but used in production

| Property | Value |
|----------|-------|
| **Hazard** | `FacetBandProbeLayout` (`probe_summary.rs:11`) is `#[cfg(test)]`-gated, but production code in `coord.rs:3131,3134,3146,3321` and `band_attributes.rs:62` references it. |
| **Why it works today** | Production builds happen to include the type because of how Cargo's cfg-test propagation interacts with the test target. |
| **What can break** | Any change to feature flags, conditional compilation, or workspace build settings that suppresses the test cfg will break the production build. |
| **Fix** | Remove the `#[cfg(test)]` attribute; it's wrong. |

### B. `FacetSizingCoordinationStrategy` Has Only One Implementor

| Property | Value |
|----------|-------|
| **Hazard** | The trait at `coordination_strategy.rs:82` has one impl (`FacetPolicyCoordinationStrategy`). The trait hooks `before_run`, `after_retarget_trace`, `after_final_propagation_trace` have empty defaults that are never overridden. |
| **Why it's there** | Anticipated future second strategy (e.g. for grid facets or alternative sizing modes). |
| **Risk** | Trait machinery obscures the single real algorithm. New contributors may add code paths to satisfy the trait that have no concrete consumer. |

### C. Disabled Outer-Edge Computation in `derive_layout_plan`

| Property | Value |
|----------|-------|
| **Hazard** | `coord.rs:3497-3567` computes `raw_outer_start`/`raw_outer_end` via `derive_outer_edges` (`coord.rs:2932`), then overrides with `outer_start = 0.0; outer_end = 0.0`. The computed values feed only a debug log. |
| **Why it's there** | Behavior was disabled during refactor; the supporting code wasn't removed. |
| **Risk** | A reader may believe the outer-edge mechanism is active and waste time tracing call sites. |

## See Also

- `avenger-chart/docs/architecture/facet-system.md` — Module layout and four-phase pipeline.
- `guides/facet-layout-overflow-model.md` — Conceptual overflow stacking model and aggregation rules.
- `avenger-chart/book/src/docs/coordinate-systems/faceting/` — User-facing facet usage.
