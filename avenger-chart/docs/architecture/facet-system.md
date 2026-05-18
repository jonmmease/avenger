# Facet System Architecture

This document is the authoritative reference for the avenger-chart facet system. It covers the module layout, the four-phase pipeline, key data structures, and the slot-sharing model.

For user-facing facet usage, see `avenger-chart/book/src/docs/coordinate-systems/faceting/`. For invariants that aren't enforced by the type system, see `guides/facet-invariants.md`. For the conceptual overflow-stacking model, see `guides/facet-layout-overflow-model.md`.

## Module layout (`avenger-chart/src/facet/`)

```
facet/
├── mod.rs                       — phase-1..4 module doc
├── marks/                       — public mark API + render entry
│   ├── facet.rs                 — Facet<C>, CompiledFacetRow/Col,
│   │                              render_facet_band_common
│   └── facet_config.rs          — FacetOptions builder DSL
├── evaluated_facet_tree.rs      — partition tree + path/predicate/enumeration caches
├── coord.rs                     — FacetColumn + FacetBandMeasurePipeline
│                                  + FacetBandCoordMeasurement
├── coord_row.rs                 — FacetRow (+ shared compute_band_layout)
├── coordination.rs              — orchestrator: 4-stage pipeline driver
├── coordination_plans.rs        — pure DTOs and plan builders
├── coordination_apply.rs        — side-effectful tree walkers
├── coordination_strategy.rs     — sizing-mode trait (single impl today)
├── domain_coordination.rs       — domain aggregation called from coord (not pipeline)
├── placement.rs                 — FacetBandPlacement (geometry)
├── band_positions.rs            — BandPosition iterator over a ConfiguredScale
├── band_attributes.rs           — staged pipeline structs
├── layout_plan.rs               — FacetCellPlan, FacetBandPlan, padding math
├── guide/
│   ├── row_guide.rs             — FacetRowGuide (thin wrapper)
│   ├── col_guide.rs             — FacetColGuide (thin wrapper)
│   └── band_guide_engine.rs     — shared axis-ops engine
├── guide_utils.rs               — label/title slab measurement & rendering
├── overflow_projection.rs       — overflow aggregation + edge-vs-max rules
├── scale_precompute.rs          — memoized scale builders per facet node
├── sharing_kernel.rs            — primitive math (group boundaries)
├── sharing_level.rs             — newtype SharingLevel(u8)
├── sharing_policy.rs            — high-level policy combinators
├── empty_cell_policy.rs         — FacetEmptyCellPolicy enum
├── ownership_policy.rs          — derives axis/legend ownership
├── padding_policy.rs            — padding constants and helpers
├── path_math.rs                 — path-truncation arithmetic
├── subtree_plot_area.rs         — pre-coordination plot-area seed
├── debug.rs                     — log helpers
├── dimension_config.rs          — Row/Col channel-name traits
├── keys.rs                      — FacetKeyExtractor::extract_keys
├── probe_summary.rs             — small probe summary structs
└── scalar_cmp.rs                — total-ordering on ScalarValue
```

## The four-phase pipeline

A faceted chart goes through one compile step plus four runtime phases. Each phase has a single responsibility and consumes the output of the previous phase.

### Phase 0 — Compile

Entry: `Mark<FacetRow|FacetColumn>::compile` (`marks/facet.rs:448`, `marks/facet.rs:694`).

Validates that the inner subplot has no `.data()` attachment (data flows from the outer facet via filtered per-cell `data_override`), compiles the inner plot, and packages the result into `CompiledFacetRow` or `CompiledFacetCol`. The compiled mark carries: `compiled_subplot: Arc<CompiledPlot>`, optional title, slot-sharing mode, position string, and empty-cell policy.

### Phase 1 — `evaluated_facet_tree` (data hierarchy)

Entry: `EvaluatedFacetTree::from_compiled_plot` (`evaluated_facet_tree.rs:303`).

Walks the compiled mark tree, runs distinct-value queries per facet level using a `SharedSlotCache`, and constructs a `PartitionNode` hierarchy. Each node is either:

- `PartitionContent::Leaf { values }` — terminal partition with observed values, or
- `PartitionContent::Branch { children: IndexMap<ScalarValue, Box<PartitionNode>> }` — a partition that itself partitions further.

After construction, `rebuild_precalculated_caches` (line 413) fills:

- `path_info_cache` — resolved branch-local indices/level-counts/directions per cell path
- `path_predicate_cache` — DataFusion filter expression per cell path
- `slot_membership_cache` — which slots a path belongs to for each `SharingLevel`
- `enumeration_cache` — values enumerated per facet variable, per sharing level
- `jagged_axis_cache` — which axes are jagged (non-rectangular) under each path

The tree is the read-only input to every later phase. It answers: which cells exist, which paths are jagged, which axis labels are visible, and how `SharingLevel(n)` groups partition the tree.

### Phase 2 — `coord` (per-band measurement)

Entry: `FacetBandMeasurePipeline::run()` (`coord.rs:3000`). Both `FacetColumn` (`coord.rs:2890`) and `FacetRow` (`coord_row.rs:39`) delegate here.

Seven steps:

1. **Resolve** (`resolve_node_or_empty`, `coord.rs:3330`) — locate the facet mark and scale; short-circuit for empty trees.
2. **Precompute** (`scale_precompute::ensure_subtree_precomputed`, `scale_precompute.rs:550`) — memoize shared / group / per-cell scale builders per node, keyed by `FacetScaleNodeKey`. The cache lives in a `Mutex<FacetScalePrecomputeStore>` on `EvaluationContext`.
3. **Cell semantics** (`FacetBandSemantics::from_tree_and_values`, `band_attributes.rs:96`) — classify each cell as `DomainPlaceholder` (cell exists in the partition but has no rows) vs `DataEmpty` vs `Populated`.
4. **Prepare** (`prepare_band_inputs_and_runtime`, `coord.rs:2329`) — build `FacetBandPreparedInputs` carrying per-cell scale builders and filtered data references.
5. **Probe** — `build_overflow_probe` → `coordinate_cell_domains_before_measurement` (`coord.rs:2497`, calls `domain_coordination::coordinated_extents_for_cell`) → `measure_cells_overflow_probe` (`coord.rs:2590`). Each cell's `ComponentsMeasurement` is measured; nested facets recurse through their own `FacetBandMeasurePipeline`.
6. **Local layout** — `build_local_layout` → `retarget_cells_to_final_plot_area`. Uses `overflow_projection::aggregate_facet_band_overflow_with_policy` for cross-cell overflow aggregation and `padding_policy::derive_parent_padding` for gap computation.
7. **Assemble** (`assemble_coord_measurement`, `coord.rs:3793`) — package results into `FacetBandCoordMeasurement`, the single 17-field type that carries everything downstream phases need.

### Phase 3 — `coordination` (cross-band reconciliation)

Entry: `coordinate_facet_measurement_tree` (`coordination.rs:136`), called from `coords.rs:213`.

Phase 2 produced a per-band measurement, but facet bands at different positions in the tree often need to *agree* on size, gap, and padding to render as a coherent grid. Coordination matches bands by `CoordinationGroupKey { depth, facet_group_identity }` (`coordination.rs:68`, key built at `coord.rs:1100`) and runs four ordered stages (`FacetCoordinationStage`, `coordination.rs:45`):

1. **InitialRequirements** — collect a snapshot per band, max-reduce across the group, and distribute back as `CoordinatedOverflow`, `CoordinatedBoundaryOverflow` (with chart-edge slabs stripped — see `strip_global_edge_overflow_for_boundary_coordination` at `coordination_plans.rs:491`), and `CoordinatedLayout` with `padding_inner_px` propagated through same-axis chains.
2. **Retarget** — `build_retarget_plan_with_strategy` calls `FacetBandCoordMeasurement::derive_retarget_requirements` (`coord.rs:1136`) per band; `apply_retarget_actions` (`coord.rs:1237`) mutates the tree, recomputing `subplot_cross_size` and `PlotAreaTarget`. Parent cross-size propagates into same-axis children via `set_child_parent_bandwidth_if_same_axis` (`coordination_strategy.rs:270`).
3. **RetargetedRequirements** — identical algorithm to stage 1 but re-collects after retarget mutations, so any downstream propagation effects converge.
4. **FinalPropagation** — `build_final_propagation_plan_with_strategy_for_eval` + `run_final_propagation_with_trace_with_strategy` (`coordination_apply.rs:535`) apply scale adjustments, may call `retarget_measurement_plot_area_policy_no_remeasure` (`coord.rs:1661`), then `realize_coordinated_child_frame_allocations`.

All stages run through the single strategy implementation `FacetPolicyCoordinationStrategy` (`coordination_strategy.rs:189`). The trait exists for future pluggability but currently has one implementor.

### Phase 4 — `placement` + `marks` (render)

The coordinated `FacetBandCoordMeasurement` is boxed onto `ComponentsMeasurement.coord_measurement` and threaded into `RenderContext::new` at `plot/compiled/rendering.rs:1431`.

At render time, `CompiledFacetCol::render_from_data` → `render_facet_band_common` (`marks/facet.rs:255`):

1. Downcast `context.coord_measurement()` to `FacetBandCoordMeasurement`.
2. Resolve placement: `resolved_placement_from_scale_specs` (`coord.rs:396`) dispatches on `placement_model`. The `ScaleBacked` arm calls `placement::resolve_scale_backed_facet_band_placement` (`placement.rs:167`) which reads the configured band scale via `BandPositionIterator::from_configured_scale`. The `Explicit` arm calls `FacetBandPlacement::from_explicit` (`placement.rs:88`) which reads positions stored on the measurement (used when the facet's main dimension is leaf-plot-area-sized).
3. `render_facet_band_with_placement` (`marks/facet.rs:110`) iterates `facet_measurement.cells`, applies the ownership policy (`resolve_facet_ownership_policy` — controls whether empty cells contribute axes / legends), and calls `compiled_subplot.build_plot_components` per cell.

## Key data structures

| Type | Location | Role |
|------|----------|------|
| `EvaluatedFacetTree` | `evaluated_facet_tree.rs:59` | Immutable partition hierarchy; query layer for the whole module |
| `PartitionNode` | `evaluated_facet_tree.rs:89` | A node in the partition tree (Leaf or Branch) |
| `FacetBandCoordMeasurement` | `coord.rs:231` | Output of Phase 2; mutated by Phase 3; input to Phase 4. 17 fields. The central type. |
| `FacetCellRuntime` | `coord.rs:113` | Per-cell: `FacetCellPlan` + `data_override: DataFrame` + `ComponentsMeasurement` |
| `FacetBandSemantics` | `band_attributes.rs:96` | Per-cell classification produced during Phase 2 step 3 |
| `FacetCellPlan` | `layout_plan.rs:14` | The canonical "plan" projection of `FacetBandCellSemantic` |
| `CoordinationGroupKey` | `coordination.rs:68` | `(depth, axis_prefix + facet_field_identity)` — matches bands across siblings |
| `CoordinatedOverflow` / `CoordinatedLayout` | `coordination_plans.rs` | Per-band reconciliation outputs |
| `FacetBandPlacement` | `placement.rs:29` | Final positions consumed by the renderer |
| `FacetScalePrecomputeStore` | `scale_precompute.rs:82` | Mutex-guarded memo of scale builders, lives on `EvaluationContext` |
| `SharingLevel(u8)` | `sharing_level.rs` | `FREE=0` / `GLOBAL=255` / intermediate ancestor levels |
| `FacetEmptyCellPolicy` | `empty_cell_policy.rs` | `Hole` / `EmptySubplot` / `Auto` |

### `FacetBandCoordMeasurement` (the central type)

A single struct carries the result of measurement, the working state of coordination, and the inputs to rendering. Its fields cover:

- **Identity**: `axis`, depth, `coordination_field_identity`, original band scale
- **Per-cell state**: `cells: Vec<FacetCellRuntime>`
- **Measured outputs**: `measured_overflow`, `local_layout`, `guide_padding_inner_px`
- **Coordinated outputs** (set during Phase 3): `coordinated_overflow`, `coordinated_boundary_overflow`, `coordinated_layout`, `coordinated_subplot_cross_size`, `parent_bandwidth`
- **Policy**: `allocation_ownership`, `placement_model`
- **Subplot**: `Arc<CompiledPlot>`

The three impl blocks at `coord.rs:284`, `coord.rs:477`, and `coord.rs:1099` reflect a natural three-way split: static output, accessor/setter, coordination-cluster API.

## Slot-sharing model

The user-facing enum is `ScaleSharing` (in `channel/config_traits.rs`):

- `Shared` — all facets share one domain (built from the full dataset)
- `Free` — each facet has an independent domain (built from filtered data)
- `Level(n)` — hierarchical: share with the ancestor `n` levels up the partition path

`ScaleSharing` is normalized to `SharingLevel(u8)` on entry via `to_normalized()`:

- `Free` → `SharingLevel(0)`
- `Shared` → `SharingLevel(255)`
- `Level(n)` → `SharingLevel(n)`

The layered sharing implementation:

1. `sharing_kernel.rs` — primitive math: `SharingGroupEdge`, `is_group_start`, `is_group_end`, `group_boundary`, `domain_group_key`. Pure functions over `(path, level)`.
2. `path_math.rs` — path-truncation arithmetic: `ancestor_key`, `enumeration_ancestor_path`, `sharing_group_boundary`. "Which slice of the path do we keep for `Level(n)` sharing?"
3. `sharing_policy.rs` — high-level combinators atop the kernel: `show_axis_labels`, `show_axis_title`, `domain_group_key`, `legend_owner_for_position`. Consumed by `evaluated_facet_tree`, `legends`, `domain_coordination`.

## Cross-references

- **Conceptual overflow model** (how facet overflows stack beyond subplot overflows, edge ownership): `guides/facet-layout-overflow-model.md`.
- **Invariants** (rules not enforced by types — visibility match, base-channel names, etc.): `guides/facet-invariants.md`.
- **User-facing usage** (`Facet<C>`, `FacetOptions`, `share_slots`, empty-cell policies): `avenger-chart/book/src/docs/coordinate-systems/faceting/`.
- **Rustdoc**: the top-level docstring on `FacetBandMeasurePipeline`, `FacetBandCoordMeasurement`, and `FacetCoordinationStage` are the most current single-source descriptions of their respective concerns.
