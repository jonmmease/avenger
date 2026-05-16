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

- [ ] Generalize `FacetRuntimeSizingPolicy` into a chart-level physical
  dimension policy if naming becomes misleading.
- [ ] Let `ResolvedFacetSizing::NonFacet` collapse into a general
  `ResolvedLayoutSizingPolicy` if practical.
- [ ] Represent a regular chart as:
  - root allocation,
  - one local frame,
  - coordinate content.
- [ ] Make non-faceted "coordination" either:
  - a no-op implementation of the same trait, or
  - a single-node realization pass with no child allocations.
- [ ] Ensure media query dimensions remain defined from the public sizing
  contract, not from incidental retargeted inner sizes.
- [ ] Add regular vs one-cell facet equivalence tests where facet guides are
  hidden or zero-sized.

## Phase 9: Unify Debug Layout Snapshots

Goal: make debug images explain the new model.

- [ ] Render `FrameAllocation.rect`.
- [ ] Render `content_rect`.
- [ ] Render owned side slabs.
- [ ] Render residual overflow.
- [ ] Render facet child allocations with depth-based colors.
- [ ] Keep existing snapshot names stable where possible.
- [ ] Add at least one debug baseline that shows:
  - initial measured demand,
  - coordinated allocations,
  - final realized layout,
  - one refinement pass result.

## Phase 10: Prototype NativeFrameLayoutSolver

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

## Phase 11: Cleanup and Documentation

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
