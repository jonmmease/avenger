# Frame Layout Unification Plan

## Goal

Unify regular chart layout and facet chart layout around one local frame
layout model. The target is that a non-faceted chart, a facet root, and each
facet leaf subplot all use the same frame solver for margins, titles, guide
overflow bands, legends, and content bounds.

Frame layout and content layout should be separate concepts. The frame solver
handles chart chrome around one content rect. A content solver handles what
happens inside that content rect. Regular charts use the degenerate
single-plot content solver; faceted charts use the richer facet-band content
solver.

The facet-band content solver should remain specialized. It coordinates child
plot areas, facet guides, scale ranges, shared overflows, empty cells, and
legend sharing. The unification is not one giant solver. The unification is
shared allocation, demand, ownership, and realized frame vocabulary.

The first implementation should keep Taffy behind a `FrameLayoutSolver`
boundary. Once all consumers depend on neutral frame types instead of Taffy
shaped results, we can decide whether to replace Taffy with a small native
arithmetic frame solver.

## Current Research Summary

### Regular Chart Frame Layout

- [ ] `layout/chart_layout.rs` owns the current regular chart frame solver.
- [ ] `ChartLayout::new` builds a `GridLayout`, then builds a Taffy tree.
- [x] `ChartLayout::compute` sets root canvas sizing and plot-area sizing,
  runs `compute_layout`, then extracts a `LayoutResult`.
- [ ] The frame consists of:
  - root canvas,
  - margins,
  - optional title and subtitle,
  - optional guide overflow rows/columns,
  - one central plot area,
  - optional legend containers on each side.
- [ ] The current Taffy use is mostly deterministic frame arithmetic:
  - fixed rows/columns,
  - one flexible plot-area row/column,
  - expandable margins when both canvas and plot area are constrained,
  - simple flex stacking inside legend containers,
  - min-content canvas growth for plot-area-sized charts.

### Taffy Touchpoints

- [ ] `avenger-chart/Cargo.toml` depends on `taffy = "0.8.3"`.
- [ ] `layout/chart_layout.rs` imports `TaffyTree`, `NodeId`, `Style`,
  `TrackSizingFunction`, `AvailableSpace`, and grid/flex helpers.
- [ ] `layout/grid.rs` uses Taffy track sizing functions to encode chart
  frame tracks.
- [x] `render/types.rs` exposed `LayoutSolution { taffy_layout: LayoutResult }`;
  this is now neutralized to `frame_layout`.
- [ ] `plot/compiled/rendering.rs` passes `taffy::Size<f32>` to legend
  measurement and uses `layout.taffy_layout` for guide, legend, title,
  subtitle, and debug rendering.
- [ ] `facet/coord.rs` mutates `layout.taffy_layout` directly when applying
  coordinated side slabs or resized plot areas.
- [ ] `facet/overflow_projection.rs` inspects `measurement.layout.taffy_layout`
  to include legend bounds in rendered overflow.
- [ ] `error.rs` has a direct `From<taffy::TaffyError>` conversion.

### Facet Layout

- [ ] `facet/mod.rs` documents the current phases:
  - evaluated facet tree construction,
  - recursive local measurement,
  - global coordination,
  - placement/rendering.
- [ ] `coordination_apply.rs` now has a common coordination driver with a
  sizing strategy hook.
- [ ] `coordination_plans.rs` already names retarget plans, final propagation
  plans, plot-area targets, and per-node traces.
- [ ] `coordination_strategy.rs` reads the per-dimension
  `FacetRuntimeSizingPolicy`.
- [ ] `placement.rs` resolves facet band placement from either scale-backed
  band scales or explicit placement.
- [ ] `overflow_projection.rs` centralizes facet overflow projections, but it
  still works with guide/total overflow rather than a broader allocation
  ownership model.
- [ ] `subtree_plot_area.rs` estimates initial plot-area-sized subtree sizes
  before coordinated overflow and explicit placement are available.

### Main Design Pressure

- [ ] Regular charts and facet subtrees both need the same local frame question:
  "Given an allocation and measured side demands, where is my content rect and
  rendered envelope?"
- [ ] Facet bands need a different question:
  "Given child demands and facet policy, how should child allocations, facet
  guide positions, scale ranges, and residual overflows be coordinated?"
- [ ] Recent outer-margin bugs came from implicit ownership of side slabs. The
  system had similar fields for guide anchoring, sibling spacing, rendered
  legend/colorbar envelope, slot-absorbed slabs, and root chart overflow.
- [ ] The planned model should make ownership explicit instead of special
  casing "top level" vs "nested".

## Target Vocabulary

### Frame Allocation

The parent grants a subtree an allocation:

```rust
struct FrameAllocation {
    rect: LayoutRect,
    sizing: FrameSizingPolicy,
    owned_slabs: OwnedEdgeSlabs,
}
```

- [ ] `rect` is the parent-owned coordinate space for this subtree.
- [ ] `sizing` describes width/height ownership independently.
- [ ] `owned_slabs` declares which side slabs are already paid for by the
  parent allocation.

### Frame Demand

A measured subtree reports demand:

```rust
struct FrameDemand {
    guide_slabs: EdgeSlabs,
    legend_slabs: EdgeSlabs,
    title_slabs: EdgeSlabs,
    rendered_envelope: EdgeSlabs,
    sibling_boundary_demand: EdgeSlabs,
}
```

- [ ] `guide_slabs` are axis and coordinate guide bands adjacent to content.
- [ ] `legend_slabs` are rendered legend/colorbar extents outside guide slabs.
- [ ] `title_slabs` are frame-level title/subtitle bands.
- [ ] `rendered_envelope` is the full visual envelope around content.
- [ ] `sibling_boundary_demand` is the portion that can affect adjacent facet
  cells.

### Frame Layout

The local frame solver returns realized component bounds:

```rust
struct FrameLayout {
    canvas_size: Size2D,
    frame_rect: LayoutRect,
    content_rect: LayoutRect,
    guide_bounds: EdgeMap<LayoutRect>,
    legend_bounds: IndexMap<String, LayoutRect>,
    title_bounds: Option<LayoutRect>,
    subtitle_bounds: Option<LayoutRect>,
    demand: FrameDemand,
    residual_overflow: EdgeSlabs,
}
```

- [ ] `content_rect` replaces the current `plot_area` field conceptually.
- [ ] `demand` replaces split guide/total overflow semantics at the frame
  boundary.
- [ ] `residual_overflow` is what this subtree asks its parent to reserve.

### Residual Overflow Rule

The central invariant:

```text
residual_overflow = rendered_envelope - owned_slabs
```

- [ ] No chart should ask its parent for a slab the parent already owns.
- [ ] No chart should hide rendered content by subtracting a slab the parent
  does not own.
- [ ] "Top level" is not a layout behavior. It is simply an allocation whose
  parent is the root canvas.
- [ ] Nested facet behavior follows from the same ownership rule.

## Proposed Solver Split

### FrameLayoutSolver

```rust
trait FrameLayoutSolver {
    fn solve(&self, input: FrameLayoutInput) -> Result<FrameLayout, AvengerChartError>;
}
```

Responsibilities:

- [ ] Lay out margins, title, subtitle, guide overflow bands, legends, and one
  content rect.
- [ ] Resolve canvas-constrained, plot-area-sized, and mixed dimensions.
- [ ] Stack multiple legends on the same side.
- [ ] Produce component bounds for rendering/debug overlays.
- [ ] Produce frame demand and residual overflow.

Non-responsibilities:

- [ ] Do not coordinate facet siblings.
- [ ] Do not decide shared scale domains.
- [ ] Do not recursively measure child charts.
- [ ] Do not decide facet guide ownership.

### TaffyFrameLayoutSolver

First implementation:

- [ ] Move the existing `ChartLayout` and `GridBuilder` behavior behind
  `TaffyFrameLayoutSolver`.
- [ ] Keep output as neutral `FrameLayout`.
- [ ] Keep Taffy types private to the solver module.
- [ ] Preserve existing visual behavior before changing layout math.

### NativeFrameLayoutSolver

Later implementation:

- [ ] Implement the chart frame with explicit arithmetic.
- [ ] Match the Taffy solver output on a geometry test suite.
- [ ] Run full visual baselines and classify intentional drift.
- [ ] Remove Taffy only after the native solver is proven.

### ContentLayoutSolver

```rust
trait ContentLayoutSolver {
    fn measure_content_demand(
        &self,
        allocation: ContentAllocation,
    ) -> Result<ContentDemand, AvengerChartError>;

    fn coordinate_content(
        &self,
        demand: ContentDemand,
    ) -> Result<ContentCoordinationPlan, AvengerChartError>;

    fn realize_content(
        &self,
        plan: ContentCoordinationPlan,
    ) -> Result<ContentLayout, AvengerChartError>;
}
```

Responsibilities:

- [ ] Solve what happens inside a frame's `content_rect`.
- [ ] Produce child frame allocations when the content contains child plots.
- [ ] Produce content-specific demand that the parent frame can include in its
  rendered envelope.
- [ ] Keep regular charts and faceted charts under the same allocation and
  realization vocabulary.

Non-responsibilities:

- [ ] Do not place frame chrome such as root margins, frame titles, frame
  legends, or coordinate guide slabs.
- [ ] Do not hide sizing ownership behind chart-level special cases.

### SinglePlotContentSolver

The regular chart content solver is the degenerate case:

- [ ] It has no child plot-frame allocations.
- [ ] It measures and realizes the coordinate content inside the given content
  rect.
- [ ] Its coordination pass is a no-op unless future regular-chart content
  types need coordination.
- [ ] It lets a regular chart participate in the same measure, coordinate,
  realize, and refine pipeline as facets.

### FacetBandContentSolver

The facet-band content solver remains chart-specific:

- [ ] Divide a parent content allocation into facet child allocations.
- [ ] Coordinate sibling guide and rendered overflow.
- [ ] Retarget plot areas and scale ranges according to the sizing policy.
- [ ] Resolve scale-backed vs explicit placement.
- [ ] Apply facet guide/title/brace placement.
- [ ] Propagate residual overflow upward.

## End-State Layout Tree

Regular chart:

```text
FrameLayoutSolver
  content: SinglePlotContentSolver
    Cartesian/Polar coordinate content
```

Faceted chart:

```text
FrameLayoutSolver
  content: FacetBandContentSolver
    child allocation 0:
      FrameLayoutSolver
        content: SinglePlotContentSolver
          Cartesian/Polar coordinate content
    child allocation 1:
      FrameLayoutSolver
        content: SinglePlotContentSolver
          Cartesian/Polar coordinate content
```

Nested facet chart:

```text
FrameLayoutSolver
  content: FacetBandContentSolver
    child allocation:
      FrameLayoutSolver
        content: FacetBandContentSolver
          child allocation:
            FrameLayoutSolver
              content: SinglePlotContentSolver
                Cartesian/Polar coordinate content
```

## Pipeline Target

The same phase names should apply to regular and faceted charts:

1. Estimated measurement.
2. Coordination.
3. Realization.
4. Optional refinement.

### Estimated Measurement

- [ ] Build scales at provisional content size.
- [ ] Measure coordinate guide overflow.
- [ ] Measure legends with provisional available space.
- [ ] Run `FrameLayoutSolver` to get local frame demand.
- [ ] Run the active `ContentLayoutSolver`.
- [ ] For `FacetBandContentSolver`, recursively measure children and aggregate
  child demands.
- [ ] For `SinglePlotContentSolver`, measure the coordinate content directly.

### Coordination

- [ ] For `SinglePlotContentSolver`, coordination is a no-op or a one-node
  pass.
- [ ] For `FacetBandContentSolver`, use the existing coordination driver:
  - initial requirement pass,
  - retarget pass,
  - retargeted requirement pass,
  - final propagation.
- [ ] Coordination consumes `FrameDemand` and produces child
  `FrameAllocation`s.
- [ ] Coordination owns scale range and plot-area retarget decisions.

### Realization

- [ ] Run `FrameLayoutSolver` with coordinated allocation and measured demand.
- [ ] Produce final component bounds.
- [ ] Render marks, guides, legends, titles, subtitles, and debug overlays from
  the same `FrameLayout`.

### Refinement

- [ ] If realized content sizes changed enough that overflow may have grown,
  remeasure guide/legend overflow at the realized content size.
- [ ] Re-run coordination.
- [ ] Re-run realization.
- [ ] Preserve the current max-refinement-pass and convergence controls.

## Phase 0: Baseline and Guardrail Inventory

- [ ] Record the current commit hash and test status before starting.
- [ ] Run focused layout tests:
  - `cargo test -p avenger-chart --test visual_regression facet --release -- --nocapture`
  - `cargo test -p avenger-chart --test visual_regression nested_grid --release -- --nocapture`
  - `cargo test -p avenger-chart --test visual_regression facet_plot_size --release -- --nocapture`
  - `cargo test -p avenger-chart --test visual_regression facet_dimension_mixed --release -- --nocapture`
- [ ] Run unit tests for coordination and sizing modules.
- [ ] Capture representative debug layout baselines for:
  - root regular chart with legends,
  - one-cell facet,
  - nested column/row facet,
  - dimension-mixed facet.
- [ ] Add small geometry tests if current coverage cannot detect:
  - side slab double allocation,
  - side slab under allocation,
  - regular vs one-cell facet equivalence.

## Phase 1: Neutralize Taffy Names at Public Internal Boundaries

Goal: stop spreading Taffy terminology before changing behavior.

- [x] Rename `LayoutSolution.taffy_layout` to `frame_layout` or
  `component_layout`.
- [x] Rename docstrings that say "Result of layout computation from Taffy".
- [ ] Keep `LayoutResult` temporarily if a full rename is too large, but make
  comments neutral.
- [x] Update consumers in:
  - `plot/compiled/rendering.rs`,
  - `facet/coord.rs`,
  - `facet/overflow_projection.rs`,
  - `render/debug.rs`,
  - legend rendering helpers.
- [x] Keep this phase behavior-preserving.
- [x] Run focused compile/tests after the rename.
  - `cargo check -p avenger-chart`

## Phase 2: Introduce Frame Layout Types

Goal: add neutral types while keeping existing layout fields available through
compatibility accessors.

- [x] Add frame primitives to the layout module. The first pass keeps them in
  `layout/types.rs` next to existing bounds types instead of adding a separate
  `layout/frame.rs`.
- [ ] Define:
  - `LayoutRect` or reuse/rename `LayoutBounds`,
  - [x] `Size2D`,
  - [x] `EdgeSlabs`,
  - [x] `OwnedEdgeSlabs`,
  - [x] `FrameSizingPolicy`,
  - [x] `FrameAllocation`,
  - [x] `FrameDemand`,
  - [x] `FrameLayout`.
- [x] Implement conversions from current `OverflowSpaceRequirement`.
- [ ] Implement edge helpers:
  - [x] side get/set,
  - [x] max/union,
  - [x] subtract owned slabs with clamp to zero,
  - [x] axis main/cross projection.
- [ ] Replace ad hoc side math where it is low risk.
- [x] Add unit tests for edge arithmetic and residual overflow.
  - `cargo test -p avenger-chart --lib layout::types -- --nocapture`

## Phase 3: Put Taffy Behind FrameLayoutSolver

Goal: make Taffy an implementation detail, not an architectural dependency.

- [x] Add `FrameLayoutInput`.
- [x] Add `FrameLayoutSolver` trait or a small solver function boundary.
- [x] Move `ChartLayout` construction into `TaffyFrameLayoutSolver`.
- [x] Make `ChartLayout` private to the layout module.
- [x] Keep `GridBuilder` private to the Taffy implementation.
- [x] Convert Taffy output into `FrameLayout`.
- [x] Make `compute_layout_from_legend_plan` call the solver boundary rather
  than `ChartLayout::new` directly.
- [x] Keep visual output unchanged.
- [x] Taffy types should no longer appear in `render/types.rs`.
- [x] Taffy types should no longer appear in `plot/compiled/rendering.rs`
  except while constructing solver input if temporarily unavoidable.

## Phase 4: Remove Taffy Types From Legend Measurement APIs

Goal: remove accidental dependency on Taffy as a generic size struct.

- [x] Introduce an Avenger-owned `Size2D` for available legend measurement
  space.
- [x] Update `LegendMeasurement.size`.
- [x] Update `layout/legend.rs`.
- [x] Update `legend/renderer/mod.rs` and concrete renderers.
- [x] Update `plot/compiled/legends.rs`.
- [x] Keep conversion helpers near the Taffy solver only while Taffy remains.
- [x] Remove direct `use taffy::Size` outside the Taffy frame solver.

## Phase 5: Replace Post-Hoc Frame Mutation With Re-Solve/Retarget Inputs

Goal: stop mutating realized component bounds as a substitute for frame layout.

Current mutation sites:

- [x] `facet/coord.rs::apply_layout_side_slab`.
- [x] `facet/coord.rs::retarget_layout_for_resized_plot_area`.
- [x] Helpers that translate the whole `LayoutResult`.
- [x] Helpers that manually anchor legends after slab changes.

These operations now live in the frame layer as
`apply_frame_side_slab` and `retarget_frame_layout_for_plot_area`. They still
operate without remeasuring; the important first cleanup is that facet
coordination no longer owns frame component placement internals.

Plan:

- [ ] Model side-slab changes as updated `FrameAllocation` or `FrameDemand`.
- [ ] Re-run `FrameLayoutSolver` for affected measurements when component
  bounds must change.
- [x] For no-remeasure retarget paths, reuse measured demand but re-solve
  placement.
- [ ] Keep guide/legend measurements stable unless refinement explicitly
  requests remeasurement.
- [ ] Preserve current performance: no recursive remeasurement merely to move
  known boxes.
- [x] Add tests showing side slab application does not double count legends at
  root edges.
  - `cargo test -p avenger-chart --lib apply_frame_side_slab -- --nocapture`
  - `cargo test -p avenger-chart --lib retarget_layout -- --nocapture`

## Phase 6: Introduce Allocation Ownership in Facet Coordination

Goal: replace top-level/nested special behavior with explicit ownership.

- [x] Add `FrameAllocation` to `ComponentsMeasurement` or to a sibling
  realization structure.
- [x] Teach facet coordination to produce child allocations with
  `owned_slabs`.
- [x] Express root chart behavior as the root allocation's owned slabs.
- [x] Express nested facet slot absorption as child allocation owned slabs.
- [x] Move top-level/nested ownership decisions out of overflow projection and
  onto `FacetBandAllocationOwnership`.
- [x] Update `overflow_projection.rs` to work from `FrameDemand` and
  `OwnedEdgeSlabs`.
- [x] Replace "rendered subtree residual for absorbed slot" style helpers with
  a direct residual-overflow calculation.
- [x] Add targeted tests:
  - root right legend residual is retained,
  - nested right legend residual is absorbed only when the parent owns it,
  - facet guide slabs are not subtracted from unrelated rendered legend slabs.

## Phase 7: Introduce ContentLayoutSolver

Goal: make top-level regular content and facet content use the same content
allocation interface.

- [x] Add `ContentAllocation`, `ContentDemand`, `ContentCoordinationPlan`, and
  `ContentLayout` types or aliases that compose with the frame types.
- [x] Add `ContentLayoutSolver` as the interface below `FrameLayoutSolver`.
- [x] Implement `SinglePlotContentSolver` for non-faceted coordinate content.
- [x] Wrap the existing facet coordination path as `FacetBandContentSolver`.
- [x] Keep `FacetBandContentSolver` richer than the single-plot solver without
  forcing facet-only concepts onto regular charts.
- [x] Ensure both content solvers can participate in:
  - estimated measurement,
  - coordination,
  - realization,
  - optional refinement.

## Phase 8: Make Regular Charts Use the Same Allocation Pipeline

Goal: make non-faceted charts look like a one-node layout tree.

Design direction: remove the "non-facet as a special facet-sizing branch"
concept. A regular chart should select the `SinglePlotContentSolver`; a faceted
chart should select the `FacetBandContentSolver`. The facet runtime sizing
policy should stay facet-specific for now because its plot-size semantics are
per-leaf-subplot semantics, not generic single-plot semantics.

### Phase 8A: Resolve Content Kind Explicitly

- [x] Add a small `ResolvedContentKind` or `ContentSolverKind` enum with:
  - `SinglePlot`,
  - `FacetBand`.
- [x] Compute this once near the evaluated facet-tree/layout-spec setup instead
  of repeatedly asking whether marks contain a facet.
- [x] Replace `ResolvedFacetSizing::NonFacet` with a content-oriented wrapper
  such as `ResolvedChartSizing` or `ResolvedContentSizing`.
- [x] Keep `FacetRuntimeSizingPolicy` inside the facet branch rather than
  generalizing it prematurely.
- [x] Preserve one clear conversion from public layout spec to:
  - regular `ResolvedLayoutDimensions`,
  - facet `FacetRuntimeSizingPolicy`.

### Phase 8B: Move Dimension Resolution Toward Layout Vocabulary

- [x] Move or prepare to move `ResolvedLayoutDimension`,
  `ResolvedLayoutDimensions`, and `LayoutDimensionSource` out of
  `plot/compiled/rendering.rs` into the layout module.
- [x] Rename `LayoutDimensionSource::PlotArea` only if needed for clarity; for
  regular charts it means the current plot/content area, while for faceted
  charts the facet branch still interprets configured plot size as leaf plot
  size.
- [x] Keep media-query `width`/`height` params sourced from the public sizing
  contract:
  - canvas-constrained dimensions use canvas size,
  - plot-area-sized regular dimensions use plot-area size,
  - plot-area-sized facet dimensions use the configured leaf plot size where
    facet measurement already expects that value.
- [ ] Add assertions/tests that final retargeted inner sizes do not silently
  change media-query input dimensions.

### Phase 8C: Dispatch Content Coordination Through the Content Solver

- [x] Add a content-level coordination/realization helper that dispatches by
  `ResolvedContentKind`.
- [x] For `SinglePlot`, run `SinglePlotContentSolver` and perform no facet
  coordination.
- [x] For `FacetBand`, run `FacetBandContentSolver` and delegate to the existing
  facet coordination pipeline.
- [x] Stop calling `coordinate_overflow_for_guides_with_mode` for regular
  charts. A regular chart should not enter facet coordination just to no-op.
- [x] Keep layout snapshot behavior explicit:
  - local measured snapshots are available to both content kinds,
  - coordination snapshots are a single-plot no-op,
  - existing regular canvas refinement snapshots are preserved,
  - final layout works for both.

### Phase 8D: Represent the Regular Chart as One Node

- [x] Treat a regular chart measurement as:
  - a root `FrameAllocation`,
  - one local `FrameLayout`,
  - a `ContentAllocation` whose `content_rect` is the plot area,
  - a `SinglePlotContentSolver` demand/plan/layout with no child allocations.
- [x] Make `ComponentsMeasurement::content_layout()` the checked access point
  for this one-node layout result.
- [x] Add invariants for the regular case:
  - child allocation count is zero,
  - content rect equals frame plot area,
  - frame demand equals measured guide/rendered overflow,
  - residual overflow follows `FrameDemand::residual_overflow`.
- [x] Avoid changing `build_plot_components` behavior in this phase unless a
  small call-site cleanup is required; the first goal is a shared model, not a
  rendering rewrite.

### Phase 8E: Preserve Existing Refinement Semantics

- [x] Keep existing regular canvas refinement behavior, but call it from the
  single-plot final realization branch rather than from a `NonFacet` sizing
  branch.
- [x] Keep regular plot-area-sized measurement's coord-aware overflow rebuild
  where it is, unless the implementation naturally folds it into
  `SinglePlotContentSolver`.
- [x] Keep facet refinement under the facet branch and continue using the
  current max-iteration/convergence policy.
- [x] No new TODO needed; the existing regular canvas refinement snapshot
  behavior remains explicit.

### Phase 8F: Tests and Acceptance Criteria

- [x] Add unit tests for the new content-kind/sizing dispatch:
  - regular plot selects `SinglePlot`,
  - faceted plot selects `FacetBand`,
  - regular plot never calls facet coordination,
  - facet plot still calls the facet coordination path.
- [ ] Add regular chart equivalence tests for the new one-node model:
  - fixed canvas,
  - fixed plot area,
  - mixed canvas-width / plot-height,
  - mixed plot-width / canvas-height,
  - legend on each side if practical.
- [ ] Add regular vs one-cell facet equivalence tests where facet guides are
  hidden or zero-sized:
  - same final canvas size when canvas-sized,
  - same final plot/content rect when plot-sized,
  - same guide and legend bounds within tolerance.
- [ ] Add media-query regression tests that cover regular charts in canvas,
  plot-area, and mixed sizing modes.
- [x] Run focused tests first:
  - `cargo test -p avenger-chart --lib layout::content_solver -- --nocapture`,
  - targeted regular sizing/measurement tests,
  - targeted facet sizing/measurement tests.
- [x] Run final validation:
  - `cargo fmt --all`,
  - `cargo clippy -p avenger-chart --all-targets`,
  - `cargo test --release -p avenger-chart --test visual_regression -- --nocapture`.

### Phase 8 Acceptance Checklist

- [x] There is no `ResolvedFacetSizing::NonFacet` concept left.
- [x] The top-level evaluation path dispatches by content kind, not by treating
  regular charts as a facet special case.
- [x] Regular charts produce and validate a `SinglePlotContentSolver` layout.
- [x] Faceted charts still produce the same accepted visual output.
- [x] Media-query dimensions are unchanged for existing regular and facet
  charts.
- [x] The working tree has no baseline drift except intentional accepted
  equivalence/debug additions.

## Phase 9: Unify Debug Layout Snapshots

Goal: make debug images explain the new model.

Design direction: introduce a small debug overlay model for allocation and
demand geometry, then make it selectable separately from the existing
frame-component overlay. Do not keep adding ad hoc arguments to
`create_debug_layout_rects`.

### Phase 9A: Define the Debug Overlay Model

- [x] Add a `LayoutDebugOverlay` or `FrameDebugOverlay` data structure in
  `render/debug.rs` or `layout/debug.rs`.
- [x] Build the model from `ComponentsMeasurement::content_layout()` plus the
  measured `FrameDemand`.
- [x] Include explicit optional layers:
  - [x] frame allocation rect,
  - [x] content rect,
  - [x] guide/frame component bounds from the existing `FrameLayout`,
  - [x] owned side slabs,
  - [x] residual overflow slabs,
  - [x] child frame allocations.
- [x] Keep the model coordinate-system neutral: the builder should accept a
  translation or origin so top-level and subplot overlays use the same path.
- [x] Keep labels data-driven so the renderer does not hard-code every label
  branch in multiple places.

### Phase 9B: Render Allocation/Demand Layers

- [x] Keep `create_debug_layout_rects` as the frame-component overlay for plot
  area, guide overflow, legends, title, and subtitle.
- [x] Add `LayoutDebugOverlayMode` so callers can choose `Components`,
  `AllocationDemand`, `All`, or `Off`.
- [x] Add a renderer for the new overlay model:
  - [x] draw `FrameAllocation.rect`,
  - [x] draw `content_rect`,
  - [x] draw owned side slabs,
  - [x] draw residual overflow slabs,
  - [x] draw child frame allocations.
- [x] Use an explicit color vocabulary:
  - [x] existing magenta/light-blue/Okabe-Ito facet colors for frame components,
  - [x] a separate restrained color for frame allocation,
  - [x] a separate restrained color for content allocation,
  - [x] distinguish owned slabs from residual overflow.
- [x] Keep labels legible with the existing depth-based alignment flip for
  nested facets.
- [x] Avoid filled overlays that obscure chart content; prefer strokes and very
  light transparent fills only where slab area needs to be visible.

### Phase 9C: Centralize Top-Level vs Subplot Coordinates

- [x] Replace the current duplicated "top-level debug overlay" and "translated
  subplot debug overlay" logic with one helper.
- [x] The helper should take:
  - [x] the `ComponentsMeasurement`,
  - [x] the `FrameLayout`,
  - [x] the desired local origin/translation,
  - [x] facet depth/path for color and label alignment.
- [x] Ensure subplot debug overlays show their actual local coordinates and, if
  useful, the frame allocation granted by the parent.
- [x] Keep existing facet color cycling behavior for nested facet frame
  component overlays.

### Phase 9D: Snapshot Semantics

- [x] Keep existing snapshot names stable where possible.
- [x] Make each snapshot visibly answer a distinct question:
  - [x] local measured: measured frame demand before coordination,
  - [x] coordinated: coordinated child allocations before final realization,
  - [x] final: realized frame/content allocation after refinement policy,
  - [x] refinement: iteration-specific remeasured demand and allocation.
- [x] Decide whether whole-chart snapshots should show child allocations at
  every facet level by default, or only the active/top-level content allocation.
- [x] Ensure facet-subtree snapshots use the same overlay renderer as whole
  chart snapshots.

### Phase 9E: Tests and Baselines

- [x] Add unit tests for overlay-model construction:
  - [x] single plot has no child allocations,
  - [x] facet band exposes child frame allocation rectangles,
  - [x] owned slab rectangles match `FrameAllocation.owned_slabs`,
  - [x] residual overflow rectangles match `FrameDemand::residual_overflow`.
- [x] Add or update debug baselines that show:
  - [x] initial measured demand,
  - [x] coordinated allocations,
  - [x] final realized layout,
  - [x] one refinement pass result.
- [x] Keep existing `facet_debug` baselines component-only and add separate
  `facet_debug_allocation` baselines for allocation/demand-only views.
- [x] Prefer one compact nested-facet fixture over many broad baseline changes
  while the vocabulary settles.
- [x] Run focused debug layout tests first, then the full release visual suite.

### Phase 9 Acceptance Checklist

- [x] Debug overlays render the shared frame/content vocabulary, not only
  legacy plot-area/overflow boxes.
- [x] Component and allocation/demand overlays can be rendered independently.
- [x] Top-level and subplot overlays use the same rendering helper.
- [x] Regular charts and facet charts both get meaningful overlays.
- [x] Existing non-debug baselines are unchanged.
- [x] Debug baseline drift is intentional and reviewed.

## Phase 10: Enforce Coordinated Overflow Contracts

Goal: make local overflow an input to coordination only; final geometry should
come from coordinated overflow contracts and explicit projections.

### Phase 10A: Define The Contract

- [x] Document the phase invariant:
  - local measured overflow is valid during measurement and requirement
    collection,
  - coordinated overflow is the source of truth after coordination,
  - final rendering, final placement, final debug overlays, and final frame
    allocations must not pick local overflow when a coordinated contract exists.
- [x] Define the supported projection purposes explicitly:
  - rendered subtree envelope for ancestors,
  - guide anchor overflow for facet-guide placement,
  - sibling boundary overflow for facet-cell gaps,
  - residual rendered overflow after parent-owned slabs are subtracted,
  - explicit boundary demand for leaf-plot-area-sized placement.
- [x] Clarify that "global" means scoped by coordination key, not one max value
  across unrelated facet levels:
  - facet depth,
  - facet group identity,
  - facet axis,
  - slot-sharing/ownership policy,
  - sizing strategy.

### Phase 10B: Centralize Overflow Resolution

- [x] Add a small resolver API near `overflow_projection.rs` that answers:
  "given this measurement, phase, and projection purpose, which overflow should
  be used?"
- [x] Represent phase/purpose with explicit enums rather than booleans:
  - measurement/probe requirement collection,
  - coordinated/final realization,
  - refinement remeasurement.
- [x] Avoid using all-zero overflow as a proxy for "not coordinated"; use an
  explicit optional coordination state or resolver fallback policy.
- [x] Make fallback behavior visible:
  - final coordinated facet bands should normally require coordinated overflow,
  - uncoordinated regular charts and pre-coordination probes can use local
    overflow,
  - any final local fallback should be deliberate and logged/tested.

### Phase 10C: Move Guides Onto The Contract

- [x] Replace row-guide local-first final anchoring with coordinated guide-anchor
  projection.
- [x] Revisit column-guide special cases such as
  `local_first_hidden_top_title_if_nonzero`; keep only behavior that follows
  from a named projection or ownership rule.
- [x] Ensure guide measurement and guide evaluation use the same resolver with
  different phases:
  - measurement can use local/probe overflow,
  - final evaluation uses coordinated overflow,
  - refinement uses the current iteration's coordinated result.
- [x] Update row/column guide unit tests so they assert coordinated-final
  behavior and local-only fallback behavior separately.

### Phase 10D: Audit Final Geometry Call Sites

- [x] Search final render/realization paths for direct local overflow usage:
  - `measured_overflow_value`,
  - `FacetOverflowSource::MeasuredLocal`,
  - `facet_guide_anchor_local_overflow`,
  - `facet_rendered_subtree_local_overflow`,
  - local-first helper names.
- [x] Classify each remaining local usage as one of:
  - requirement collection,
  - refinement measurement,
  - final fallback for genuinely uncoordinated measurements.
- [x] Update debug allocation/component overlays so final snapshots display
  coordinated contracts consistently.
- [x] Add assertions where possible that final facet rendering does not use local
  overflow when coordinated overflow is present.

### Phase 10E: Tests and Baselines

- [x] Add or update focused coverage for the mixed tree:
  `row(division) -> column(department) -> row(team)`.
- [x] Add assertions that same-level `team` row facet guide anchors match across
  outer `division` rows after coordination.
- [x] Cover both canvas-constrained and leaf-plot-area-sized/mixed sizing where
  practical.
- [x] Refresh and review:
  - `facet_debug/facet_nested_mixed_debug_final.png`,
  - matching `facet_debug_allocation` image if it changes,
  - any guide/legend-sharing baselines affected by the column-guide policy.
- [x] Run focused guide and coordination tests first, then the release visual
  suite.

### Phase 10 Acceptance Checklist

- [x] Final rendering has one coordinated overflow source of truth per
  projection purpose.
- [x] Local overflow no longer changes final guide anchors once coordination has
  completed.
- [x] Mixed-axis nested facets keep same-level guides aligned across orthogonal
  ancestors.
- [x] Any remaining local-overflow fallback is explicitly named, tested, and not
  used for coordinated facet bands.

## Phase 10F: Feed Realized Boundary Demand Back Into Refinement

Goal: make refinement correct the sibling-gap cases where a child only reveals a
larger boundary demand after coordinated realization.

- [x] Add an explicit `FacetBandPaddingFeedback` map keyed by facet coordinate
  node path.
- [x] Collect realized sibling-boundary demand from each facet band after
  realization, including guide demand and total rendered boundary demand.
- [x] Convert realized demand back through the same
  `compute_padding_from_overflows` path used during measurement.
- [x] Feed the realized inner-padding lower bounds into the next refinement
  measurement instead of permanently mutating the layout plan.
- [x] Treat padding-feedback growth as refinement growth, alongside recursive
  overflow growth.
- [x] Apply the feedback path to canvas-sized, plot-area-sized, and mixed
  dimension policies through the shared refinement loop.
- [x] Increase the default refinement budget to two optional passes so the first
  pass can discover realized sibling gaps and the second can apply them.
- [x] Keep `max_refinement_passes = 0` as the fastest measure-once path.
- [x] Add unit coverage for monotonic padding-feedback growth detection.
- [x] Refresh affected visual baselines and rerun the release visual suite.

### Phase 10F Acceptance Checklist

- [x] Sibling gaps are at least the max of the realized right/left or
  bottom/top boundary demands of adjacent renderable children.
- [x] Globally outer overflows are still excluded from same-level sibling-gap
  coordination, except where they affect the subtree's rendered envelope.
- [x] Refinement can fix gaps that were not visible during the first measured
  layout.
- [x] No new sizing owner is introduced; realized feedback only supplies lower
  bounds for the existing facet-band padding calculation.

## Phase 11: Prototype NativeFrameLayoutSolver

Goal: decide whether Taffy still earns its dependency.

- [ ] Implement native solver next to the Taffy solver.
- [ ] Support current frame features:
  - fixed canvas,
  - fixed plot area,
  - width-only canvas,
  - height-only canvas,
  - width-only plot area,
  - height-only plot area,
  - expandable margins,
  - title/subtitle span modes,
  - guide overflow bands,
  - left/right/top/bottom legend containers,
  - flexible colorbar main-axis sizing.
- [ ] Add geometry comparison tests that run both solvers on the same input.
- [ ] Define acceptable tolerance and rounding policy.
- [ ] Use visual baselines to inspect intentional differences.
- [ ] Switch default solver only after comparison tests are stable.
- [ ] Remove Taffy dependency after:
  - no public/internal API imports Taffy,
  - native visual output is accepted,
  - `From<taffy::TaffyError>` is removed,
  - `Cargo.toml` no longer references Taffy.

## Phase 12: Cleanup and Documentation

- [ ] Update `layout/mod.rs` module docs to describe the frame solver model
  rather than Taffy.
- [ ] Update `facet/mod.rs` docs to refer to allocation, demand, coordination,
  realization, and refinement.
- [ ] Remove compatibility aliases after downstream code uses canonical names.
- [ ] Delete dead helper functions from old mutation-based frame updates.
- [ ] Add a concise architecture comment near the evaluation pipeline in
  `plot/compiled/rendering.rs`.
- [ ] Run:
  - [x] `cargo fmt --all`
  - [x] `cargo clippy -p avenger-chart --all-targets`
  - [x] `cargo test --release -p avenger-chart --test visual_regression -- --nocapture`
  - [x] `cargo check -p avenger-chart`
  - [x] `git diff --check`
  - [x] `cargo test -p avenger-chart --lib layout::types -- --nocapture`
  - [x] `cargo test -p avenger-chart --lib apply_frame_side_slab -- --nocapture`
  - [x] `cargo test -p avenger-chart --lib retarget_layout -- --nocapture`

## Key Invariants

- [ ] Every physical dimension has one sizing owner.
- [ ] Every side slab has one owner at a given parent/child boundary.
- [ ] `FrameLayoutSolver` never recursively measures children.
- [x] `ContentLayoutSolver` owns child allocation inside a frame content rect.
- [x] `SinglePlotContentSolver` is the regular-chart degenerate content case.
- [x] `FacetBandContentSolver` is the multi-child coordinated content case.
- [ ] Facet coordination never hand-edits realized frame component bounds.
- [ ] Refinement is the only phase that can remeasure guide/legend overflow
  after realization.
- [ ] Local overflow feeds requirement collection; coordinated overflow
  projections drive final geometry.
- [ ] Realization can re-solve frame placement from existing measurements
  without remeasuring text, legends, or guides.
- [ ] Regular charts and one-cell facets share the same frame semantics.
- [ ] Taffy is private to `TaffyFrameLayoutSolver` until it is removed.

## Expected Review Benefits

- [ ] Reviewers can ask "who owns this slab?" and find an explicit answer.
- [ ] Root vs nested behavior becomes data in `FrameAllocation`, not scattered
  conditionals.
- [ ] Regular and faceted chart layout share the same frame vocabulary.
- [ ] Taffy removal becomes an implementation swap, not an architectural
  rewrite.
- [ ] Debug layout snapshots expose allocation, demand, and residual overflow
  directly.

## Open Design Questions

- [ ] Should the canonical name be `FrameLayoutSolver`, `PlotFrameSolver`, or
  `ChartFrameSolver`? Current preference: `FrameLayoutSolver`, because it is
  local and applies to root charts and facet subplots.
- [ ] Should `FrameDemand` live on `ComponentsMeasurement`, or should
  `ComponentsMeasurement` be split into measurement vs realized layout structs?
- [ ] Should title/subtitle be treated as frame slabs or separate frame
  components with their own demand type?
- [ ] Should legend sharing/hoisting be represented as demand mutation before
  frame solving, or as a facet coordination decision that changes child
  allocations?
- [ ] How strict should regular vs one-cell facet equivalence be when facet
  guide defaults are present?
- [ ] Do we keep both Taffy and native solvers behind a feature/test flag during
  migration, or switch in one commit after comparison tests pass?
