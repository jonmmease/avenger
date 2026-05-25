# Facet System Architecture

This document is the authoritative internal reference for the current
`avenger-chart` facet system. It covers the module layout, the compile/runtime
pipeline, important data structures, and the slot-sharing model.

For user-facing facet usage, see `avenger-chart/book/src/docs/coordinate-systems/faceting/`.
For invariants that are not enforced by the type system, see
`guides/facet-invariants.md`. For the conceptual overflow-stacking model, see
`guides/facet-layout-overflow-model.md`.

## Module Layout

The core facet runtime lives in `avenger-chart/src/facet/`.

```text
facet/
├── mod.rs                       - phase overview and public facet exports
├── marks/
│   ├── facet.rs                 - facet subplot authoring/compiled marks and render entry
│   └── facet_config.rs          - FacetOptions builder DSL
├── evaluated_facet_tree.rs      - facet adapter over partition nodes and query caches
├── coord.rs                     - FacetColumn measurement and shared band pipeline
├── coord_row.rs                 - FacetRow measurement entry
├── coordination.rs              - four-stage coordination driver
├── coordination_plans.rs        - immutable requirement, retarget, and propagation plans
├── coordination_apply.rs        - bounded mutation walkers over measurement trees
├── coordination_policy.rs       - facet sizing policy hooks used by coordination
├── domain_coordination.rs       - per-cell domain reconciliation before measurement
├── placement.rs                 - FacetBandPlacement geometry resolution
├── band_attributes.rs           - staged band/cell semantic data
├── layout_plan.rs               - FacetCellPlan, FacetBandPlan, and padding math
├── guide/
│   ├── row_guide.rs             - FacetRowGuide wrapper
│   ├── col_guide.rs             - FacetColGuide wrapper
│   └── band_guide_engine.rs     - shared facet guide engine
├── guide_utils.rs               - label/title slab measurement and rendering helpers
├── overflow_projection.rs       - overflow projection and edge-vs-boundary rules
├── scale_precompute.rs          - memoized scale builders per facet node
├── sharing_policy.rs            - high-level axis/legend/domain ownership policy
├── empty_cell_policy.rs         - facade re-export for FacetEmptyCellPolicy
├── ownership_policy.rs          - render-time ownership policy for empty cells
├── padding_policy.rs            - padding constants and helpers
├── path_math.rs                 - ancestor/path truncation helpers
├── subtree_plot_area.rs         - leaf-size-to-subtree plot-area estimates
├── debug.rs                     - facet debug overlay mode helpers
├── dimension_config.rs          - facade re-export for row/column configs
├── direction.rs                 - FacetDirection
└── probe_summary.rs             - small overflow probe summary structs
```

Two nearby support modules matter for facet architecture:

- `avenger-chart/src/partition.rs` owns reusable partition primitives such as
  `PartitionNode`, `PartitionContent`, `PartitionDimensionSpec`,
  `PartitionCellPlan`, and `PartitionKeyExtractor`.
- `avenger-chart/src/layout/band_position.rs` owns `BandPositionIterator`, the
  shared band-scale iterator used by scale-backed facet placement.

## Pipeline

A faceted chart has one compile step and four runtime phases. Each phase has a
single responsibility and consumes the output of preceding phases.

### Phase 0 - Compile

`SubplotContainerCoordinateSystem` implementations for `FacetRow` and
`FacetColumn` live in `facet/marks/facet.rs`. They validate the neutral
`Subplot` mark, compile the child plot into a `CompiledSubplotPayload`, and
package the result as `CompiledFacetRowSubplot` or
`CompiledFacetColumnSubplot`.

Facet subplots do not allow explicit child plot data. Data flows from the outer
facet through filtered per-cell `data_override` values. The compiled facet mark
carries the compiled child plot payload, optional title, slot-sharing mode,
position string, and empty-cell policy.

### Phase 1 - Data Hierarchy

`EvaluatedFacetTree::from_compiled_plot` walks the compiled mark tree, discovers
nested facet subplot marks, queries distinct values through
`PartitionKeyExtractor`, and builds a hierarchy of `PartitionNode` values.

Each `PartitionNode` stores the facet `FacetDirection`, sharing level, field
identity, observed values, and one of two `PartitionContent` variants:

- `PartitionContent::Leaf { values }` for the innermost partition slots.
- `PartitionContent::Branch { children }` for a partition that contains another
  facet level.

After construction, `EvaluatedFacetTree::rebuild_precalculated_caches` fills
query caches for resolved path metadata, filter predicates, slot membership,
facet-value enumeration, jagged-axis checks, and observed sharing levels. The
tree is the read-only input to later phases: it answers which cells exist,
which paths are jagged, which predicates filter a cell, which axis labels and
titles are visible, and how `SharingLevel` values group paths.

### Phase 2 - Per-Band Measurement

`FacetBandMeasurePipeline::run` is the shared per-band measurement pipeline.
`FacetColumn` in `coord.rs` and `FacetRow` in `coord_row.rs` both delegate to it
with axis-specific operations.

The pipeline steps are:

1. **Resolve** with `resolve_node_or_empty`: find the relevant compiled facet
   mark, band scale, current facet node, slot-sharing level, and empty-cell
   policy.
2. **Precompute** with `scale_precompute::ensure_subtree_precomputed`: memoize
   shared, group, and per-cell scale artifacts in `FacetScalePrecomputeStore`.
3. **Cell semantics** with `FacetBandSemantics::from_tree_and_values`: classify
   each enumerated cell as populated, data-empty, or a domain placeholder.
4. **Prepare** with `prepare_band_inputs_and_runtime`: build per-cell scale
   inputs, filtered data overrides, and child plot preparation state.
5. **Probe** with `build_overflow_probe`,
   `coordinate_cell_domains_before_measurement`, and
   `measure_cells_overflow_probe`: measure estimated child `ComponentsMeasurement`
   values and recurse into nested facets when needed.
6. **Local layout** with `build_local_layout`: aggregate overflow through
   `overflow_projection`, derive padding through `padding_policy`, and retarget
   cells to their final local plot-area size.
7. **Assemble** with `assemble_coord_measurement`: package the result into
   `FacetBandCoordMeasurement`.

`FacetBandCoordMeasurement` is the central runtime value for facets. It carries
the measured cells, the original band scale, the local layout, measured
overflow, coordinated state populated by phase 3, placement model, empty-cell
policy, child-frame path prefix, and the compiled child plot needed for later
retargeting and rendering.

### Phase 3 - Cross-Band Coordination

`coordinate_facet_measurement_tree` runs the four-stage coordination pipeline
over a tree of `ComponentsMeasurement` values. The coordination driver lives in
`coordination.rs`; immutable plans live in `coordination_plans.rs`; bounded
mutations live in `coordination_apply.rs`; sizing-mode decisions live in
`FacetCoordinationPolicy`.

Coordination uses two kinds of identity:

- `CoordinationNodeKey` is a traversal address for applying plans back to the
  measurement tree.
- `CoordinationScopeKey` is the semantic grouping key for the behavior being
  coordinated, such as child size, overflow, boundary overflow, or guide anchor
  alignment.

The ordered stages are:

1. **InitialRequirements**: collect a snapshot per facet band, aggregate by
   `CoordinationScopeKey`, and distribute `CoordinatedOverflow`,
   boundary-overflow, guide-anchor-overflow, and `CoordinatedLayout` patches.
2. **Retarget**: build and run a `RetargetPlan`. This derives per-band
   retarget requirements from `FacetBandCoordMeasurement`, asks
   `FacetCoordinationPolicy` for actions, mutates affected child measurements,
   and propagates parent cross-size into same-axis child facet bands.
3. **RetargetedRequirements**: repeat requirement collection and distribution
   after retargeting so downstream propagation effects converge.
4. **FinalPropagation**: build and run a final propagation plan, update scale
   ranges, retarget child plot areas when the sizing policy requires it, and
   realize coordinated child-frame allocations.

The pipeline is policy-driven rather than trait-pluggable today: there is a
single `FacetCoordinationPolicy` implementation that reads the current
`FacetRuntimeSizingMode` from the evaluation context.

### Phase 4 - Placement And Render

The coordinated `FacetBandCoordMeasurement` is stored in
`ComponentsMeasurement.coord_measurement` and passed through the top-level plot
rendering path.

At render time, `CompiledFacetRowSubplot` and `CompiledFacetColumnSubplot`
dispatch to `render_facet_band_common`:

1. Downcast the coordinate measurement to `FacetBandCoordMeasurement`.
2. Resolve placement through `FacetBandCoordMeasurement::resolved_placement_from_scale_specs`.
   Scale-backed placement uses `placement::resolve_scale_backed_facet_band_placement`
   and `BandPositionIterator::from_configured_scale`; explicit placement uses
   `FacetBandPlacement::from_explicit`.
3. `render_facet_band_with_placement` iterates `FacetCellRuntime` values,
   applies `resolve_facet_ownership_policy`, and calls
   `CompiledPlot::build_plot_components` for each renderable cell.

## Key Data Structures

| Type | Module | Role |
|------|--------|------|
| `EvaluatedFacetTree` | `facet/evaluated_facet_tree.rs` | Facet query adapter over a partition hierarchy and its caches |
| `PartitionNode` / `PartitionContent` | `partition.rs` | Nested data-partition tree used by facets |
| `PartitionDimensionSpec` | `partition.rs` | Internal build request for one facet dimension |
| `PartitionCellPlan` | `partition.rs` | Canonical plan for one enumerated partition cell |
| `FacetBandCoordMeasurement` | `facet/coord.rs` | Per-band measurement result, coordination working state, and render input |
| `FacetCellRuntime` | `facet/coord.rs` | Per-cell plan, filtered data override, measurement, and domain extents |
| `FacetBandSemantics` | `facet/band_attributes.rs` | Geometry-independent cell semantics for one band |
| `FacetCellPlan` | `facet/layout_plan.rs` | Facet alias over `PartitionCellPlan` used by layout/render code |
| `CoordinationNodeKey` | `facet/coordination_plans.rs` | Plan-local traversal key for applying coordination results |
| `CoordinationScopeKey` | `plot/compiled` | Semantic grouping key used to merge layout/overflow requirements |
| `FacetCoordinationPolicy` | `facet/coordination_policy.rs` | Current sizing-policy decisions used by coordination |
| `FacetBandPlacement` | `facet/placement.rs` | Final cell positions consumed by render |
| `FacetScalePrecomputeStore` | `facet/scale_precompute.rs` | Mutex-guarded memo of per-node scale artifacts on `EvaluationContext` |
| `SharingLevel` | `avenger-chart-core` | Normalized slot/domain/legend sharing level |
| `ScaleSharing` | `avenger-chart-core` | User-facing sharing mode: `Free`, `Level(n)`, or `Shared` |
| `FacetEmptyCellPolicy` | `avenger-chart-core` | Empty-cell rendering policy: `Hole`, `EmptySubplot`, or `Auto` |

## Slot-Sharing Model

The user-facing enum is `ScaleSharing`:

- `Free`: each facet cell has an independent domain or slot set.
- `Level(n)`: share with the ancestor `n` levels up the partition path.
- `Shared`: share globally.

`ScaleSharing` normalizes to `SharingLevel`:

- `Free` becomes `SharingLevel::FREE`.
- `Shared` becomes `SharingLevel::GLOBAL`.
- `Level(n)` becomes `SharingLevel::from_raw(n)`.

The implementation is layered:

1. `avenger-chart-core` owns the primitive sharing math, including
   `SharingLevel`, `SharingGroupEdge`, edge ownership helpers, and shared path
   helpers.
2. `facet/path_math.rs` adapts path-truncation arithmetic for facet-specific
   ancestor and enumeration queries.
3. `facet/sharing_policy.rs` combines those primitives into higher-level
   policies for axis label visibility, axis title ownership, domain-group keys,
   and legend ownership.
4. `EvaluatedFacetTree`, legend planning, domain coordination, and guide
   rendering consume those policies through facet-tree and guide-sharing
   queries.
