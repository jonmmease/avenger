# Facet Sizing Unification Follow-Up Plan

> Historical note: this plan predates the removal of configured facet
> row/column scales as layout-geometry storage. References to
> "scale-backed" facet placement and facet band scales describe the old
> implementation, not the current architecture. The current pipeline stores
> render/readback geometry in `CurrentFacetGeometry`.

## Goal

Finish the parts of the facet sizing design that are still transitional after
adding dimension-mixed sizing. The target design is one policy-driven facet
pipeline where canvas-fit, plot-area-sized, and dimension-mixed charts are
special cases of the same physical-dimension sizing model.

This is primarily a maintainability refactor. The current behavior is working,
but several concepts are still split by old sizing modes:

- placement is encoded by wrapper type (`FacetBandCoordMeasurementPlotAreaSized`)
  instead of by an explicit placement field,
- rendering dispatch still checks global mode or measurement type,
- retarget actions still carry a facet-main-axis size instead of full physical
  plot-area targets,
- final propagation still uses a coarse resize permission,
- realization/refinement still has separate canvas, plot-area-sized, and mixed
  paths.

## Design Target

### Canonical Model

- [x] Treat `FacetRuntimeSizingPolicy` as the canonical internal sizing model
  for all faceted charts.
- [x] Keep `CanvasFit` and `PlotAreaSized` wording out of internal faceted
  dispatch; `CanvasFit` remains only as the non-faceted context fallback.
- [x] Prefer code shaped around physical dimensions:
  - width,
  - height,
  - facet band/main dimension,
  - facet orthogonal/cross dimension.
- [x] Avoid type checks that imply a sizing mode. Ask the measurement or policy
  directly what placement/retarget behavior applies.

### Core Invariants

- [x] A physical dimension has exactly one sizing owner:
  canvas-constrained or leaf-plot-area-sized.
- [x] A facet band uses scale-backed placement when its band physical dimension
  is canvas-constrained.
- [x] A facet band uses explicit placement when its band physical dimension is
  leaf-plot-area-sized.
- [x] Canvas-constrained dimensions may be retargeted after final layout.
- [x] Leaf-plot-area-sized dimensions are preserved at leaf subplots.
- [ ] Overflow measurement is independent of later coordinated alignment slabs.
- [x] Rendering receives a resolved placement object and does not care how it
  was produced.

## Phase 1: Put Placement on the Base Measurement

### Problem

`FacetBandCoordMeasurementPlotAreaSized` wraps `FacetBandCoordMeasurement` only
to carry `FacetBandExplicitPlacement`. This creates repeated downcasts across
coordination, placement, rendering, and layout realization. Mixed sizing now
makes the wrapper less accurate conceptually because explicit placement is a
per-band property, not a whole-chart mode.

### Plan

- [x] Add a placement model field to `FacetBandCoordMeasurement`:

  ```rust
  enum FacetBandPlacementModel {
      ScaleBacked,
      Explicit(FacetBandExplicitPlacement),
  }
  ```

- [x] Add methods on `FacetBandCoordMeasurement`:
  - `uses_explicit_placement()`
  - `recompute_explicit_placement_if_needed()`
  - `preserve_empty_slot_plot_area_if_explicit(width, height)`
  - `plot_area_extent()`
  - `resolved_placement(scales)`
- [x] Move `FacetBandCoordMeasurementPlotAreaSized::recompute_explicit_placement`
  logic into base measurement methods.
- [x] Move `plot_area_sized_extent` to a generic `plot_area_extent` method.
- [x] In measurement assembly, choose `FacetBandPlacementModel` from
  `facet_runtime_sizing_mode().policy().facet_band_dimension(axis)`.
- [x] Remove `FacetBandCoordMeasurementPlotAreaSized`.
- [x] Remove `FacetBandCoordMeasurementCanvasFit` alias unless it still adds
  clarity after the wrapper is gone.
- [x] Remove `facet_band_canvas_ref`, `facet_band_plot_area_sized_ref`, and
  related mut helpers in favor of one `facet_band_ref`.

### Tests

- [x] Existing `facet_plot_size` visual subset passes.
- [x] Existing `nested_grid_equivalent` visual subset passes.
- [ ] Add/adjust unit tests:
  - scale-backed placement resolves from band scale,
  - explicit placement resolves from stored placement,
  - mixed col-row tree has one scale-backed band and one explicit band.

## Phase 2: Make Rendering Placement-Driven

### Problem

`facet.rs` still dispatches to `facet_canvas_fit` or `facet_plot_area_sized`
based on runtime mode and, for mixed mode, downcasts to the wrapper type. The
two render paths already converge on `render_facet_band_with_placement`, so the
split is mostly accidental complexity.

### Plan

- [x] Replace mode-based render dispatch with:

  ```rust
  let facet_measurement = facet_band_ref(context.coord_measurement())?;
  let placement = facet_measurement.resolved_placement(context.scales())?;
  render_facet_band_with_placement(...)
  ```

- [x] Delete or collapse `facet_canvas_fit.rs` and `facet_plot_area_sized.rs`
  into one renderer.
- [x] Ensure empty-cell behavior remains unchanged for scale-backed and
  explicit placement.
- [x] Keep debug overlays using the same resolved placement path.

### Tests

- [x] Run all facet visual subsets that include debug overlays:
  - [x] `facet_layout_snapshot` (`facet_debug` baselines)
  - [x] `facet_plot_size`
  - [x] `facet_dimension_mixed`

## Phase 3: Replace Main-Axis Retarget Actions with Full Plot-Area Targets

### Problem

`CellRetargetAction::RetargetPlotArea { main_axis_size }` encodes the old idea
that a retarget changes only the facet orthogonal dimension. Mixed sizing proved
that retargeting is naturally physical: width and height should be independent.

### Plan

- [x] Replace `CellRetargetAction` with a struct-like action:

  ```rust
  struct CellRetargetAction {
      plot_area_target: Option<PlotAreaSize>,
      rebuild_domains: bool,
  }
  ```

  or equivalent enum variants that carry `PlotAreaSize`.

- [x] Compute action targets from `RetargetNodeRequirements.child_plot_areas`.
- [x] Shrink only dimensions whose policy is canvas-constrained and whose
  legend slab is on that dimension.
- [x] Preserve leaf-plot-area-sized dimensions in all retarget actions.
- [x] Make domain rebuild independent of plot-area resize.
- [x] Update traces/counts to report:
  - plot-area target count,
  - width-target count,
  - height-target count,
  - domain rebuild count.
- [x] Delete `CellRetargetAction::main_axis_size`.

### Tests

- [ ] Unit tests where only width changes.
- [ ] Unit tests where only height changes.
- [ ] Unit tests where domains rebuild without plot-area resize.
- [x] Existing retarget/apply tests updated to full target semantics.

## Phase 4: Make Final Propagation Per-Dimension

### Problem

`FinalChildResizePolicy` currently has:

```rust
allow_plot_area_resize: bool
allow_scale_range_retarget: bool
```

That is too broad for mixed sizing. The executor currently delegates to a
strategy hook to avoid breaking leaf-sized dimensions, but the plan itself does
not describe the exact dimension permissions.

### Plan

- [x] Replace `FinalChildResizePolicy` with:

  ```rust
  struct FinalChildResizePolicy {
      allow_width_resize: bool,
      allow_height_resize: bool,
      allow_x_range_retarget: bool,
      allow_y_range_retarget: bool,
  }
  ```

- [x] Build `FinalPropagationChildPlan` with independent width and height
  targets.
- [x] Compute resize permission from the sizing policy:
  - width resize if width is canvas-constrained,
  - height resize if height is canvas-constrained.
- [x] Retarget x scale range only when width changes and x range is plot-area
  backed.
- [x] Retarget y scale range only when height changes and y range is plot-area
  backed.
- [x] Remove dimension-specific fallback behavior from final propagation child
  updates.
- [x] Make the final propagation plan fully explain what execution will do.

### Tests

- [ ] Final propagation preserves leaf height in width-canvas/height-leaf mode.
- [ ] Final propagation preserves leaf width in height-canvas/width-leaf mode.
- [ ] Canvas-fit behavior is unchanged.
- [ ] Plot-area-sized behavior is unchanged.

## Phase 5: Replace Strategy Trio with One Policy Strategy

### Problem

The old `CanvasFitCoordinationStrategy`, `PlotAreaSizedCoordinationStrategy`,
and dimension-policy strategy encoded mode-specific behavior. After placement
and retarget actions are policy-shaped, the strategy layer can become one
policy-driven implementation.

### Plan

- [x] Introduce `FacetPolicyCoordinationStrategy`.
- [x] Make it traverse the unified `FacetBandCoordMeasurement` only.
- [x] Make policy retarget planning read `FacetRuntimeSizingPolicy` from
  `EvaluationContext`.
- [x] Remove the remaining test/back-compat `build_retarget_actions_for_eval`
  split.
- [x] Apply coordinated band layout only when the band dimension is
  canvas-constrained.
- [x] Refresh explicit placement only when the band placement model is explicit.
- [x] Replace the old mode-specific `FacetCoordinationMode` variants with one
  full-cycle mode.
- [x] Delete `coordination_canvas_fit.rs`, `coordination_plot_area_sized.rs`,
  and `coordination_dimension_policy.rs` if they become wrappers with no value.
- [x] Remove assertions that assume all leaf width and height are fixed; replace
  with per-dimension assertions.

### Tests

- [x] Existing coordination unit tests still pass in release mode.
- [ ] Add policy-specific tests for:
  - fully canvas policy,
  - fully leaf policy,
  - mixed width-canvas/height-leaf,
  - mixed width-leaf/height-canvas.

## Phase 6: Unify Realization and Refinement

### Problem

`rendering.rs` has separate code paths for canvas refinement,
plot-area-sized realization, and mixed-policy realization. They are now
variations of one loop:

1. coordinate requirements,
2. realize final plot-area dimensions from policy,
3. retarget without remeasure,
4. optionally remeasure and repeat until overflow converges.

### Plan

- [x] Introduce a single policy realization/refinement set of methods:
  - `realized_root_plot_area_for_policy`
  - `layout_spec_for_realized_policy`
  - `retarget_measurement_for_policy_no_remeasure`
  - `remeasure_coord_for_policy_at_current_plot_area`
- [x] For canvas-constrained dimensions, get the realized plot-area dimension
  from the rebuilt layout bounds.
- [x] For leaf-plot-area-sized dimensions, get the realized plot-area dimension
  from explicit placement subtree extents.
- [x] Preserve the existing convergence semantics:
  - iteration 0 always performs mandatory realization,
  - iteration N remeasures, recoordinates, realizes,
  - stop when recursive overflow does not grow.
- [x] Replace fully canvas faceted use of:
  - `refine_canvas_measurement_after_coordination`
  - `realize_plot_area_sized_layout_after_coordination`
  - `realize_dimension_policy_layout_after_coordination`
  with one policy-driven entry point. This required carrying realized subplot
  probe-size overrides into policy refinement and rebuilding component
  measurements at those realized sizes.
- [x] Update layout snapshot handling so refinement snapshots are policy-based,
  not mode-based.

### Tests

- [x] Canvas-fit snapshots still work.
- [x] Plot-area-sized snapshots still work.
- [x] Mixed snapshots work for both orientations.
- [ ] Existing performance tests for deeply nested canvas facets still pass.

## Phase 7: Consolidate Layout Spec and Dimension Resolution

### Problem

There are currently several helpers that synthesize measured or realized layout
specs from the same policy information. There is also a legacy boolean
`dimensions_are_plot_area`, which is less expressive than the per-dimension
model.

### Plan

- [x] Replace the old `FacetSizingStrategy` split with
  `ResolvedFacetSizing::{NonFacet, Policy(FacetRuntimeSizingPolicy)}`.
- [x] Keep convenience constructors for fully canvas and fully leaf policies.
- [x] Replace `ResolvedLayoutDimensions { width_is_plot_area, height_is_plot_area }`
  with:

  ```rust
  struct ResolvedDimension {
      value: f32,
      source: DimensionSource,
  }

  enum DimensionSource {
      Canvas,
      PlotArea,
  }
  ```

- [x] Add one helper for measured layout spec synthesis.
- [x] Add one helper for realized layout spec synthesis.
- [x] Document media-query/dimension-param semantics for mixed modes near this
  helper.
- [x] Remove remaining old comments that describe layout as a boolean
  canvas-vs-plot-area choice.

### Tests

- [ ] Unit tests for all valid layout spec combinations.
- [ ] Unit tests for conflict errors.
- [ ] Media query visual or unit coverage for mixed dimensions.

## Phase 8: Clean Module Boundaries and Naming

### Problem

After the above phases, several modules and names should become stale.

### Plan

- [x] Remove mode-specific module names if they no longer contain distinct
  behavior:
  - `coord_canvas_fit`
  - `coord_plot_area_sized`
  - `coordination_canvas_fit`
  - `coordination_plot_area_sized`
  - `coordination_dimension_policy`
  - `facet_canvas_fit`
  - `facet_plot_area_sized`
- [ ] Rename remaining concepts around the canonical terms:
  - sizing policy,
  - placement model,
  - scale-backed placement,
  - explicit placement,
  - realized plot area,
  - canvas-constrained dimension,
  - leaf-plot-area-sized dimension.
- [x] Update `facet/mod.rs` architecture docs to reflect the unified pipeline.
- [ ] Keep `canvas-fit` and `plot-area-sized` wording only where describing
  public user-facing behavior or compatibility aliases.

## Phase 9: Make Rendered Overflow Ownership Explicit

### Problem

Nested facet groups can render guides, legends, and colorbars outside their
leaf plot areas. The parent layout currently has to decide how much of that
subtree overflow should affect:

- guide anchoring against adjacent facet bands,
- sibling spacing,
- outer canvas overflow,
- root canvas expansion or plot-area shrinkage.

Those are not the same question. The current projection path can strip
legend/colorbar slabs when computing parent layout overflow, which prevents
double reservation in some cases but clips or crowds rendered edge content in
others. The failure mode is visible in nested legend/colorbar sharing cases:
the child subtree really renders a wider envelope, but the parent sees only the
facet-guide portion.

### Design Target

Make overflow ownership explicit rather than encoding it in projection variants.
The layout pipeline should carry separate values for:

- **Guide anchor overflow**: space used to position facet guide labels and
  braces against sibling plot/facet bands.
- **Sibling spacing demand**: space that must exist between adjacent facet
  siblings so rendered guide structures do not collide.
- **Rendered subtree envelope**: the actual local bounds of everything rendered
  by a facet subtree, including axes, facet guides, legends, colorbars, titles,
  and child content.
- **Canvas residual overflow**: the part of the rendered subtree envelope that
  still protrudes outside the space already assigned to the subtree by facet
  placement.

The parent should reserve enough space for the rendered subtree envelope while
guide placement should continue to use guide anchor overflow. This avoids both
clipping and accidental double allocation.

### Current Evidence

- [x] Reproduce and keep notes for the known reject baselines:
  - `facet_legend_sharing/facet_col_legend_sharing_level1_left.png`
  - `facet_legend_sharing/facet_col_legend_sharing_level1_right.png`
  - `facet_legend_sharing/facet_col_legend_sharing_merged_group_min_level.png`
  - `nested_grid/color_channel_with_level1.png`
- [x] Confirm the trace-level relationship for each reject:
  rendered side demand equals guide demand plus legend/colorbar demand, while
  parent layout currently receives only guide demand.
- [x] Add a debug-layout snapshot for at least one reject so guide anchor
  overflow and rendered envelope can be inspected visually.

### Plan

- [x] Audit `overflow_projection.rs` and rename projection methods around the
  semantic question they answer:
  - guide anchor projection,
  - sibling spacing projection,
  - rendered subtree envelope,
  - canvas residual projection.
- [x] Replace ambiguous `ParentLayout` projection usage with an explicit helper
  whose return type says whether it is an anchor overflow or rendered envelope.
- [x] Route rendered-subtree envelope through an explicit helper. The first
  implementation keeps the envelope represented as `CoordinatedOverflow` and
  exposes it through `rendered_subtree_overflow_from_coord_measurement`.
- [x] Keep facet guide placement driven by guide anchor overflow, not by the
  full rendered envelope.
- [x] Drive parent outer layout, root canvas bounds, and clipping invariants
  from the rendered subtree envelope or canvas residual overflow.
- [x] Centralize sibling-gap calculations so all sizing policies derive gaps
  from the same spacing demand helper.
- [x] Verify that this does not reintroduce the old double-gap behavior in:
  - nested legend sharing with level-1 legends,
  - nested plot-area-sized charts,
  - dimension-mixed charts,
  - canvas-sized charts with one refinement pass.

### Invariants

- [x] Every rendered subtree has one authoritative rendered envelope.
- [x] Ancestors never infer rendered bounds by reusing guide-anchor overflow.
- [x] Facet guide labels and braces are anchored from guide overflow, even when
  legends or colorbars widen the rendered subtree envelope.
- [x] Legend/colorbar slabs are reserved exactly once: by the subtree that
  renders them, then propagated upward as part of the rendered envelope.
- [ ] Empty or ragged facet slots do not own edge overflow unless they actually
  render content or are required by explicit placement.
- [x] Canvas-fit, plot-area-sized, and dimension-mixed sizing use the same
  envelope projection helpers.

### Implementation Note

Rendered-subtree projection now treats top-level and nested facet bands
differently because they have different ownership:

- top-level facet bands propagate the full measured rendered envelope because
  there is no parent facet slot that can absorb their main-axis outer slabs,
- nested facet bands propagate residual overflow after subtracting the
  main-axis outer slabs already represented by the parent slot placement.

An attempted component-layout overflow cache was rejected because it fed a
plot's own previous layout overflow back into its coord overflow measurement,
which double-counted root-owned legends.

### Tests

- [x] Add targeted visual coverage for level-shared legends on all sides:
  left, right, top, and bottom.
- [ ] Add targeted visual coverage for nested colorbars on all sides, including
  flexible colorbar height/width.
- [ ] Add a ragged/empty-cell case where the renderable edge cell is not the
  first or last domain slot.
- [x] Add a same-axis nested facet case and an orthogonal nested facet case with
  an intermediate-level legend.
- [x] Add at least one canvas-sized, one plot-area-sized, and one
  dimension-mixed variant for the new invariant.
- [ ] Strengthen rendered-bounds assertions so they check absolute placement in
  the root canvas, not only each child's local canvas.

### Success Criteria

- [x] The four known reject baselines no longer clip or double-allocate edge
  space.
- [x] Previously accepted spacing baselines remain visually equivalent except
  for intentional envelope-driven corrections.
- [x] Debug overlays make it possible to see the distinction between guide
  anchor overflow and rendered subtree envelope.
- [x] The code no longer needs a projection that silently strips rendered
  legend/colorbar slabs under a generic parent-layout name.

## Suggested Implementation Order

The safest order is:

1. [x] Phase 1: base measurement placement model.
2. [x] Phase 2: placement-driven renderer.
3. [x] Phase 3: full plot-area retarget targets.
4. [x] Phase 4: per-dimension final propagation.
5. [x] Phase 5: one policy coordination strategy.
6. [ ] Phase 6: one policy realization/refinement loop.
7. [ ] Phase 7: layout spec/dimension cleanup.
8. [ ] Phase 8: module and naming cleanup.
9. [x] Phase 9: explicit rendered overflow ownership.

This order removes type-based dispatch first, then makes the action/planning
objects expressive enough to support one strategy and one realization loop.

## Validation Cadence

After each phase:

- [x] `cargo fmt --all`
- [x] `cargo check -p avenger-chart`
- [x] focused unit tests for the changed planning/execution objects
- [x] `cargo test --release -p avenger-chart --test visual_regression facet_plot_size -- --nocapture`
- [x] `cargo test --release -p avenger-chart --test visual_regression facet_dimension_mixed -- --nocapture`

After phases 1, 4, and 6:

- [x] `cargo test --release -p avenger-chart --test visual_regression test_nested_grid_equivalent -- --nocapture`

Before committing the full cleanup:

- [x] full `avenger-chart` visual regression run in release mode
- [x] review image diffs, especially:
  - nested facet spacing,
  - legend sharing,
  - plot-size-from-canvas variants,
  - debug layout overlays.

## Expected Payoff

- Fewer downcasts and wrapper types.
- One rendering path for all facet sizing modes.
- Retarget/final propagation plans that fully describe physical width/height
  effects.
- Mixed sizing becomes a natural policy case instead of a third mode.
- Future strategies, such as fixed canvas width with content-driven height or
  responsive width bands, can plug into the same policy/realization layer.
