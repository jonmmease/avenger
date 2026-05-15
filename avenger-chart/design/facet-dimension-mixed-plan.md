# Facet Dimension-Mixed Sizing Plan

## Goal

Support faceted charts where width and height can use different sizing anchors.
The motivating example is a website or book layout with a known available width
where the chart should grow vertically:

```rust
plot
    .canvas_constraint(CanvasConstraint::width(720.0))
    .plot_constraint(PlotConstraint::height(120.0))
```

For a faceted chart this should mean:

- final canvas width is constrained to 720px,
- each leaf subplot plot area has height 120px,
- final canvas height grows to contain all subplot rows, guide overflows, facet
  guides, legends, title, subtitle, and margins.

The implementation should generalize current canvas-fit and plot-area-sized
facets rather than adding a narrow third special case.

## Implementation Status

- [x] Added a per-physical-dimension runtime sizing policy with
  canvas-constrained and leaf-plot-area-sized dimensions.
- [x] Resolved faceted chart sizing from `canvas_constraint` and
  `plot_constraint` per dimension, including conflict errors for width/width
  and height/height ownership.
- [x] Made initial plot/canvas dimension resolution dimension-aware.
- [x] Synthesized mixed measured layout specs such as canvas-width plus
  plot-area-height and plot-area-width plus canvas-height.
- [x] Updated facet measurement so each band chooses scale-backed or explicit
  placement from the policy for that band's physical main dimension.
- [x] Added a policy-driven coordination path for mixed trees.
- [x] Added dimension-aware realization and refinement for mixed sizing.
- [x] Added dimension-aware no-remeasure retargeting so final canvas-constrained
  plot-area changes propagate through nested facet children while leaf-sized
  dimensions remain fixed.
- [x] Added targeted unit coverage for valid partial plot sizing, invalid
  same-dimension conflicts, and mixed canvas-width/leaf-height preservation.
- [x] Added and accepted initial `facet_dimension_mixed` visual baselines.
- [ ] Future cleanup: collapse the wrapper-based explicit-placement model into
  a single placement field on `FacetBandCoordMeasurement`.
- [ ] Future cleanup: convert retarget actions from main-axis sizes to full
  physical plot-area targets.
- [ ] Future cleanup: replace the coarse final propagation resize policy with
  per-dimension resize/range-retarget permissions.
- [ ] Future docs: update public sizing docs and add media-query-specific
  coverage for mixed dimensions.

## Non-Goals

- Do not preserve the current internal `CanvasFit` vs `PlotAreaSized` split if a
  per-dimension model is clearer.
- Do not maintain the current invalid partial-constraint behavior for faceted
  charts.
- Do not add baseline variants for every existing facet chart in the first pass.
  Add targeted visual coverage for the new behavior, then expand if needed.
- Do not hand-roll separate nested facet algorithms for mixed sizing. The shared
  coordination, retarget, refinement, placement, and rendering pipeline should
  remain the foundation.

## Design Principle

Sizing is a physical-dimension policy:

```rust
struct FacetSizingPolicy {
    width: FacetDimensionPolicy,
    height: FacetDimensionPolicy,
}

enum FacetDimensionPolicy {
    CanvasConstrained { canvas_size: f32 },
    LeafPlotAreaSized { leaf_plot_size: f32 },
}
```

Existing modes become special cases:

- `canvas_size(w, h)`:
  - width: `CanvasConstrained { canvas_size: w }`
  - height: `CanvasConstrained { canvas_size: h }`
- `plot_size(w, h)`:
  - width: `LeafPlotAreaSized { leaf_plot_size: w }`
  - height: `LeafPlotAreaSized { leaf_plot_size: h }`
- `canvas_constraint(CanvasConstraint::width(w))`
  plus `plot_constraint(PlotConstraint::height(h))`:
  - width: `CanvasConstrained { canvas_size: w }`
  - height: `LeafPlotAreaSized { leaf_plot_size: h }`
- `canvas_constraint(CanvasConstraint::height(h))`
  plus `plot_constraint(PlotConstraint::width(w))`:
  - width: `LeafPlotAreaSized { leaf_plot_size: w }`
  - height: `CanvasConstrained { canvas_size: h }`

## Terminology

- **Physical dimension**: chart width or height.
- **Facet main dimension**: width for column facets, height for row facets.
- **Facet cross dimension**: height for column facets, width for row facets.
- **Canvas-constrained dimension**: final canvas size is fixed or constrained;
  subplot plot areas must fit inside it.
- **Leaf-plot-sized dimension**: leaf subplot plot-area size is fixed; the
  containing subtree and final canvas may grow.
- **Scale-backed placement**: facet cell positions are resolved from the active
  band scale.
- **Explicit placement**: facet cell positions are stored in the facet
  measurement after computing rendered gaps and subtree extents.

## Current State

- [ ] Current public APIs already expose `canvas_constraint` and
  `plot_constraint`.
- [ ] `SizeMode::Width` and `SizeMode::Height` already exist in
  `layout/sizing.rs`.
- [ ] Faceted charts currently reject partial size modes in
  `CompiledPlot::resolve_facet_sizing_strategy`.
- [ ] `FacetRuntimeSizingMode` currently has only:
  - `CanvasFit`
  - `PlotAreaSized { leaf_plot_width, leaf_plot_height }`
- [ ] Measurement dispatch currently branches globally on
  `FacetRuntimeSizingMode`.
- [ ] Plot-area-sized mode uses a wrapper measurement type,
  `FacetBandCoordMeasurementPlotAreaSized`, to store explicit placement.
- [ ] Coordination strategy is already mostly shared through
  `FacetSizingCoordinationStrategy`.
- [ ] Retarget planning is now split into neutral requirements and
  strategy-produced actions.
- [ ] Final propagation already stores optional width and height targets, which
  is a useful foundation for per-dimension policy.

## Public API Plan

### Supported Facet Sizing Inputs

- [ ] Keep `plot.canvas_size(width, height)` as fixed final canvas sizing.
- [ ] Keep `plot.plot_size(width, height)` as fixed per-leaf subplot plot area
  for top-level faceted charts.
- [ ] Enable `plot.canvas_constraint(CanvasConstraint::width(width))` for
  faceted charts.
- [ ] Enable `plot.canvas_constraint(CanvasConstraint::height(height))` for
  faceted charts.
- [ ] Enable `plot.plot_constraint(PlotConstraint::width(width))` for faceted
  charts as fixed per-leaf subplot plot width.
- [ ] Enable `plot.plot_constraint(PlotConstraint::height(height))` for faceted
  charts as fixed per-leaf subplot plot height.

### Validation Rules

- [ ] For each physical dimension, allow exactly one sizing owner:
  - canvas owns the dimension, or
  - leaf plot area owns the dimension.
- [ ] Reject `canvas width` plus `plot width`.
- [ ] Reject `canvas height` plus `plot height`.
- [ ] Reject missing ownership only if there is no sensible default.
  Recommended first version:
  - missing width defaults to current canvas default width,
  - missing height defaults to current canvas default height.
- [ ] Preserve the existing top-level-only rule for facet subplot plot sizing:
  nested subplots under a facet should not set their own `plot_size` or
  `plot_constraint`.
- [ ] Update user-facing errors to describe per-dimension conflicts, for example:
  `Faceted charts cannot constrain both canvas width and leaf plot width`.
- [ ] Remove the current generic error that says faceted charts do not support
  partial `canvas_constraint` or `plot_constraint`.

### Documentation and Prelude

- [ ] Update `layout/sizing.rs` module docs with a facet mixed-sizing example.
- [ ] Update `Plot::canvas_constraint` docs to mention faceted chart behavior.
- [ ] Update `Plot::plot_constraint` docs to mention per-leaf subplot behavior
  for faceted charts.
- [ ] Confirm no new public type needs to be added to `prelude.rs`.
  The preferred plan reuses existing public constraint types.

## Internal Data Model Plan

### Replace Global Runtime Mode

- [ ] Replace `FacetRuntimeSizingMode` with a policy object:

  ```rust
  pub(crate) struct FacetRuntimeSizingPolicy {
      pub(crate) width: FacetDimensionPolicy,
      pub(crate) height: FacetDimensionPolicy,
  }
  ```

- [ ] Add helpers:
  - `policy.width_policy()`
  - `policy.height_policy()`
  - `policy.policy_for_physical_dimension(PhysicalDimension)`
  - `policy.policy_for_facet_main_axis(FacetAxis)`
  - `policy.policy_for_facet_cross_axis(FacetAxis)`
  - `policy.is_fully_canvas_constrained()`
  - `policy.is_fully_leaf_plot_sized()`
  - `policy.has_leaf_plot_sized_dimension()`

- [ ] Add a small internal enum:

  ```rust
  enum PhysicalDimension {
      Width,
      Height,
  }
  ```

- [ ] Add helpers:
  - `FacetAxis::main_physical_dimension()`
  - `FacetAxis::cross_physical_dimension()`
  - `PhysicalDimension::scale_name()` if useful for x/y scale retargeting.

### Unify Facet Band Measurement Type

- [ ] Replace `FacetBandCoordMeasurementPlotAreaSized` with a placement field on
  `FacetBandCoordMeasurement`.

  ```rust
  enum FacetBandPlacementModel {
      ScaleBacked,
      Explicit(FacetBandExplicitPlacement),
  }
  ```

- [ ] Store `placement_model` on `FacetBandCoordMeasurement`.
- [ ] Remove the `FacetBandCoordMeasurementCanvasFit` type alias if it stops
  adding clarity.
- [ ] Remove `facet_band_plot_area_sized_ref` and
  `facet_band_plot_area_sized_mut` downcast helpers.
- [ ] Replace placement downcasts with measurement methods:
  - `facet_band.uses_explicit_placement()`
  - `facet_band.resolved_placement(scales)`
  - `facet_band.recompute_explicit_placement_if_needed()`
  - `facet_band.plot_area_sized_extent_for_leaf_sized_dimensions()` or a better
    final name after implementation.

### Placement Model Selection

- [ ] A facet band uses explicit placement when its facet main physical
  dimension is `LeafPlotAreaSized`.
  - Column facet: explicit placement when width is leaf-plot-sized.
  - Row facet: explicit placement when height is leaf-plot-sized.
- [ ] A facet band uses scale-backed placement when its facet main physical
  dimension is `CanvasConstrained`.
  - Column facet: scale-backed placement when width is canvas-constrained.
  - Row facet: scale-backed placement when height is canvas-constrained.
- [ ] Mixed trees can therefore contain both placement models at different
  levels.
- [ ] Rendering must resolve placement through the measurement, not through a
  global sizing mode.

## Facet Sizing Policy Resolution

- [ ] Replace `FacetSizingStrategy` with either:
  - `FacetSizingPolicy`, or
  - `FacetSizingStrategy { policy: FacetSizingPolicy }`.
- [ ] Convert evaluated layout spec into a `FacetSizingPolicy` for faceted
  charts.
- [ ] Preserve non-facet sizing behavior as much as possible.
- [ ] Add helpers to read width and height ownership independently from:
  - `evaluated_layout_spec.canvas`
  - `evaluated_layout_spec.plot_area`
- [ ] Make `plot_size(width, height)` produce leaf plot width and height policy
  for faceted charts.
- [ ] Make `canvas_size(width, height)` produce canvas width and height policy
  for faceted charts.
- [ ] Make partial constraints produce mixed policy.
- [ ] Add unit tests for every valid and invalid policy combination.

## Measured Layout Spec Synthesis

Current plot-area-sized mode rewrites the evaluated layout spec to a synthetic
root plot-area size estimated from leaf size. Mixed sizing needs a
dimension-aware version.

- [ ] Replace `layout_spec_for_facet_sizing_strategy` with
  `layout_spec_for_facet_sizing_policy`.
- [ ] For canvas-constrained dimensions, preserve the canvas constraint in the
  measured layout spec.
- [ ] For leaf-plot-sized dimensions, estimate the root plot-area dimension from
  the facet tree and leaf size.
- [ ] Examples:
  - fully canvas-constrained:
    - preserve `canvas: Fixed { width, height }`
    - `plot_area: Auto`
  - fully leaf-plot-sized:
    - `canvas: Auto`
    - `plot_area: Fixed { estimated_root_width, estimated_root_height }`
  - canvas-width plus leaf-plot-height:
    - `canvas: Width(width)`
    - `plot_area: Height(estimated_root_height)`
  - leaf-plot-width plus canvas-height:
    - `canvas: Height(height)`
    - `plot_area: Width(estimated_root_width)`
- [ ] Make sure `layout/grid.rs` expands margins only in dimensions where both
  canvas and plot area are constrained.
- [ ] Add tests for the synthesized measured layout spec.

## Initial Dimension Resolution

Current `resolve_dimensions_from_spec` returns `(width, height, is_plot_area_mode)`.
This is too coarse for mixed sizing.

- [ ] Replace with a dimension-aware result:

  ```rust
  struct ResolvedPlotDimensions {
      width: f32,
      height: f32,
      width_is_plot_area: bool,
      height_is_plot_area: bool,
  }
  ```

- [ ] Or use a more explicit version:

  ```rust
  struct DimensionResolution {
      width: DimensionValue,
      height: DimensionValue,
  }

  struct DimensionValue {
      value: f32,
      source: DimensionSource,
  }

  enum DimensionSource {
      Canvas,
      PlotArea,
  }
  ```

- [ ] Update `compute_layout_and_dimensions` so width and height can be sourced
  independently.
- [ ] For canvas-sourced dimensions, use grid layout to determine plot-area size
  in that dimension.
- [ ] For plot-area-sourced dimensions, use the requested plot-area size and let
  grid layout determine canvas size in that dimension.
- [ ] Add focused tests for:
  - canvas width plus plot-area height,
  - plot-area width plus canvas height,
  - defaults in the unspecified dimension.

## Scale Building and Media Query Semantics

Mixed sizing raises an important question: what dimensions should be visible to
user expressions and media queries?

Recommended first rule:

- [ ] Dimension params should represent the currently known top-level external
  dimensions where constrained, and the current plot-area estimate where
  plot-area-sized.
- [ ] For canvas-width plus plot-height:
  - width param should be the constrained canvas width,
  - height param should initially be the estimated root plot-area height during
    measurement, then final canvas height after realization if re-evaluated.
- [ ] Document this internal behavior in code comments near dimension
  resolution.
- [ ] Add media query baseline tests once the behavior is stable.

Open design decision:

- [ ] Decide whether final top-level render params should expose final canvas
  size or plot-area size in a mixed dimension. Prefer consistency with existing
  non-facet behavior, then document the choice.

## Measurement Pipeline

- [ ] Update `FacetBandMeasurePipeline::resolve_subplot_band_size` to use
  `policy_for_facet_main_axis`.
- [ ] When facet main dimension is canvas-constrained:
  - use the current canvas-fit bandwidth behavior.
- [ ] When facet main dimension is leaf-plot-sized:
  - estimate child subtree main size from leaf plot size.
- [ ] Update `derive_local_layout_from_probe`.
  - If facet main dimension is canvas-constrained, build pass-2 scale and use
    resulting bandwidth.
  - If facet main dimension is leaf-plot-sized, preserve the estimated explicit
    main size.
- [ ] Update `assemble_coord_measurement` to set `placement_model` based on
  policy instead of wrapping in `FacetBandCoordMeasurementPlotAreaSized`.
- [ ] Delete or collapse `coord_canvas_fit.rs` and `coord_plot_area_sized.rs`
  if they become identical wrappers.
- [ ] Add unit tests for mixed measurement shape:
  - column facet in width-constrained mode is scale-backed,
  - row facet in height-leaf-sized mode is explicit,
  - nested col-row mixed tree has both models.

## Retarget Actions

Current cell retarget actions carry `main_axis_size`. Mixed sizing needs full
physical target sizes.

- [ ] Replace:

  ```rust
  RetargetPlotArea { main_axis_size: f32 }
  RetargetPlotAreaAndDomains { main_axis_size: f32 }
  ```

  with:

  ```rust
  RetargetPlotArea { target: PlotAreaSize }
  RetargetPlotAreaAndDomains { target: PlotAreaSize }
  ```

- [ ] Remove `CellRetargetAction::main_axis_size`.
- [ ] Add `CellRetargetAction::target_plot_area`.
- [ ] Update `RetargetNodeRequirements` to include child plot-area width and
  height, which it already mostly does.
- [ ] Make action construction dimension-aware:
  - shrink only canvas-constrained dimensions that need to absorb legend slabs,
  - preserve leaf-plot-sized dimensions,
  - rebuild domains independently.
- [ ] Add tests where:
  - target width changes while height is preserved,
  - target height changes while width is preserved,
  - domains rebuild without plot-area resize.

## Coordination Strategy

- [ ] Replace `CanvasFitCoordinationStrategy` and
  `PlotAreaSizedCoordinationStrategy` with a policy-driven strategy, or reduce
  them to wrappers around a common policy strategy.
- [ ] Strategy reads `FacetRuntimeSizingPolicy` from `EvaluationContext`.
- [ ] `build_retarget_actions` uses the policy for the facet band's main
  physical dimension.
- [ ] `BandRetargetAction::ApplyCoordinatedLayout` is used only when the facet
  main dimension is canvas-constrained.
- [ ] For leaf-plot-sized main dimensions, preserve band scale layout during
  retarget and refresh explicit placement.
- [ ] `refresh_placement_after_requirement_patch`, `refresh_placement_after_retarget_node`,
  and `refresh_placement_after_final_propagation_node` should refresh explicit
  placement only for bands that use explicit placement.
- [ ] Update trace labels to use policy terminology rather than old mode names.
- [ ] Remove assertions that assume every leaf width and height are fixed in
  plot-area-sized mode. Replace with per-dimension assertions.

## Final Propagation

Current `FinalChildResizePolicy` is too coarse:

```rust
struct FinalChildResizePolicy {
    allow_plot_area_resize: bool,
    allow_scale_range_retarget: bool,
}
```

Replace it with:

```rust
struct FinalChildResizePolicy {
    allow_width_resize: bool,
    allow_height_resize: bool,
    allow_x_range_retarget: bool,
    allow_y_range_retarget: bool,
}
```

- [ ] Update final propagation planning to compute target width and target
  height independently.
- [ ] Allow child plot-area width resize only when physical width is
  canvas-constrained for the relevant subtree.
- [ ] Allow child plot-area height resize only when physical height is
  canvas-constrained for the relevant subtree.
- [ ] Preserve leaf plot width in leaf-plot-width dimensions.
- [ ] Preserve leaf plot height in leaf-plot-height dimensions.
- [ ] Retarget x scale ranges when width changes.
- [ ] Retarget y scale ranges when height changes.
- [ ] Add final propagation tests for both mixed orientations.

## Explicit Placement and Subtree Extents

Current explicit placement is tied to fully plot-area-sized facets. Mixed mode
requires explicit placement per facet band.

- [ ] Update `compute_explicit_facet_band_placement` so it works for any facet
  band whose main dimension is leaf-plot-sized.
- [ ] Update `cell_main_plot_size` and `cell_cross_plot_size` to use resolved
  subtree extents from child measurements, not downcasts to a plot-area-sized
  wrapper.
- [ ] Track realized subtree width and height on `FacetBandCoordMeasurement` if
  the placement model alone is not enough.
- [ ] Ensure inter-cell gap logic uses rendered boundary demand only in the
  explicit-placement main dimension.
- [ ] Keep scale-backed placement for canvas-constrained dimensions.
- [ ] Add tests for explicit placement gaps in mixed mode:
  - gap accounts for tick label overflow,
  - gap accounts for facet guide overflow,
  - no extra gap is left for unrendered legends or titles.

## Realization and Refinement

The current code has separate canvas refinement and plot-area-sized realization.
Mixed sizing should use one dimension-aware loop.

- [ ] Introduce `realize_facet_extents_after_coordination` that works from
  `FacetSizingPolicy`.
- [ ] Replace `realize_plot_area_sized_extents_no_remeasure` with a generalized
  version:
  - realizes width from explicit placement if width is leaf-plot-sized,
  - realizes height from explicit placement if height is leaf-plot-sized,
  - preserves canvas-constrained dimensions.
- [ ] Replace `remeasure_plot_area_sized_coord_at_current_plot_area` with
  `remeasure_facet_coord_at_current_plot_area_for_policy`.
- [ ] Replace `run_plot_area_sized_refinement_iteration` with
  `run_policy_refinement_iteration`.
- [ ] Keep existing convergence behavior:
  - refinement 0 performs mandatory realization,
  - refinement passes remeasure and recoordinate,
  - stop when recursive overflow does not grow,
  - stop at max refinement passes.
- [ ] For fully canvas-constrained policy, preserve current canvas refinement
  behavior.
- [ ] For fully leaf-plot-sized policy, preserve current plot-area-sized
  behavior.
- [ ] For mixed policy, realize only leaf-plot-sized dimensions and preserve
  canvas-constrained dimensions.
- [ ] Update refinement snapshots to work with policy naming.

## Layout Rebuild After Realization

- [ ] Add helper to construct a realized top-level layout spec from policy and
  realized root plot-area dimensions.
- [ ] For canvas-width plus leaf-plot-height:
  - keep `canvas: Width(width)`,
  - set `plot_area: Height(realized_plot_height)`.
- [ ] For leaf-plot-width plus canvas-height:
  - set `plot_area: Width(realized_plot_width)`,
  - keep `canvas: Height(height)`.
- [ ] For fully leaf-plot-sized:
  - keep `canvas: Auto`,
  - set `plot_area: Fixed { realized_width, realized_height }`.
- [ ] For fully canvas-constrained:
  - keep fixed canvas constraints,
  - leave plot area auto unless explicitly specified by a valid non-facet case.
- [ ] Rebuild layout with coordinated overflow and the realized layout spec.
- [ ] Retarget legends after scales are retargeted.
- [ ] Add assertions:
  - final canvas-constrained dimensions match requested size,
  - realized leaf-plot-sized dimensions match requested leaf sizes at leaves,
  - legends are within final canvas.

## Rendering

- [ ] Remove global render dispatch based on old runtime mode.
- [ ] Resolve facet placement through the measurement:

  ```rust
  let placement = resolve_facet_band_placement(context.coord_measurement(), context.scales())?;
  render_facet_band_with_placement(...)
  ```

- [ ] Delete or simplify `facet_canvas_fit.rs` and `facet_plot_area_sized.rs`.
- [ ] Make debug layout overlays work with mixed placement models.
- [ ] Use global facet debug color cycle as today.
- [ ] Confirm `FacetSubtreeSnapshot` works for:
  - scale-backed bands,
  - explicit-placement bands,
  - mixed nested trees.

## Clipping

Recommended first implementation:

- [ ] If top-level faceted chart has any leaf-plot-sized dimension, use
  `Clip::None` at the root facet plot.
- [ ] Keep existing clip behavior for fully canvas-constrained facets.
- [ ] Revisit dimension-specific clipping only after mixed baselines are stable.

## Metrics and Debugging

- [ ] Update `EvaluationMetrics` names if they mention old global modes.
- [ ] Add optional trace fields for:
  - sizing policy width,
  - sizing policy height,
  - number of scale-backed facet bands,
  - number of explicit-placement facet bands,
  - realized root plot width,
  - realized root plot height.
- [ ] Update debug layout snapshots to make mixed sizing understandable.
- [ ] Add one debug visual baseline for mixed sizing.

## Unit Test Plan

### Policy Resolution

- [ ] `canvas_size(width, height)` resolves to fully canvas-constrained policy.
- [ ] `plot_size(width, height)` resolves to fully leaf-plot-sized policy.
- [ ] `canvas_constraint(width) + plot_constraint(height)` resolves to mixed
  width-canvas, height-leaf policy.
- [ ] `canvas_constraint(height) + plot_constraint(width)` resolves to mixed
  width-leaf, height-canvas policy.
- [ ] `canvas_constraint(width) + plot_constraint(width)` errors.
- [ ] `canvas_constraint(height) + plot_constraint(height)` errors.
- [ ] `plot_constraint(width)` alone uses default canvas height policy.
- [ ] `plot_constraint(height)` alone uses default canvas width policy.
- [ ] `canvas_constraint(width)` alone uses default canvas height policy.
- [ ] `canvas_constraint(height)` alone uses default canvas width policy.

### Layout Spec Synthesis

- [ ] Fully canvas policy preserves canvas fixed layout.
- [ ] Fully leaf policy synthesizes root plot-area fixed layout.
- [ ] Width-canvas plus height-leaf synthesizes canvas width plus plot-area
  height.
- [ ] Width-leaf plus height-canvas synthesizes plot-area width plus canvas
  height.
- [ ] Estimated root plot-area dimension is derived from facet tree shape.

### Measurement and Placement

- [ ] Column facet uses scale-backed placement when width is canvas-constrained.
- [ ] Column facet uses explicit placement when width is leaf-plot-sized.
- [ ] Row facet uses scale-backed placement when height is canvas-constrained.
- [ ] Row facet uses explicit placement when height is leaf-plot-sized.
- [ ] Nested col-row mixed tree contains both placement models.
- [ ] Empty cells preserve expected extent in explicit-placement dimensions.

### Retarget and Coordination

- [ ] Retarget actions can resize width only.
- [ ] Retarget actions can resize height only.
- [ ] Retarget actions can rebuild domains without resizing.
- [ ] Coordinated layout is applied only for canvas-constrained facet main
  dimensions.
- [ ] Explicit placement is refreshed only for explicit-placement bands.
- [ ] Final propagation preserves leaf plot sizes per dimension.

### Realization and Refinement

- [ ] Realization grows height while preserving canvas width.
- [ ] Realization grows width while preserving canvas height.
- [ ] Refinement 0 performs mandatory realization.
- [ ] Refinement 1 can remeasure and update overflow in a mixed tree.
- [ ] Convergence is recorded when overflow does not grow.
- [ ] Max-pass metric is recorded if overflow keeps growing.

## Visual Baseline Plan

Add a new baseline directory:

```text
avenger-chart/tests/baselines/facet_dimension_mixed/
```

Add visual test module:

```text
avenger-chart/tests/visual_tests/test_facet_dimension_mixed.rs
```

Initial targeted baselines:

- [x] `mixed_width_canvas_height_plot_row_iris_scatter`
  - Single-level row facet.
  - Fixed canvas width, fixed leaf plot height.
  - Assert visual height grows.
- [ ] `mixed_width_canvas_height_plot_col_iris_scatter`
  - Single-level column facet.
  - Fixed canvas width, fixed leaf plot height.
  - Confirms column facets remain width-constrained.
- [x] `mixed_width_canvas_height_plot_nested_col_row_shared`
  - Nested column then row facets.
  - Exercises scale-backed width and explicit height in the same chart.
- [ ] `mixed_width_canvas_height_plot_nested_row_col_shared`
  - Nested row then column facets.
  - Exercises explicit height at outer level and scale-backed width at inner
    level.
- [ ] `mixed_width_canvas_height_plot_right_legend`
  - Right legend with fixed canvas width.
  - Confirms legends fit within width and height grows as needed.
- [ ] `mixed_width_canvas_height_plot_top_legend`
  - Top legend with grown height.
  - Confirms top legend contributes to final height.
- [ ] `mixed_width_canvas_height_plot_continuous_colorbar`
  - Continuous colorbar beside nested facets.
  - Confirms colorbar length and placement update.
- [ ] `mixed_width_canvas_height_plot_free_scales`
  - Free scales with adaptive ticks.
  - Confirms tick count uses final subplot dimensions.
- [ ] `mixed_width_canvas_height_plot_empty_subplot_policy`
  - Empty-cell or hole policy case.
  - Confirms explicit vertical placement handles holes.
- [x] `mixed_height_canvas_width_plot_col_iris_scatter`
  - Reverse mixed orientation.
  - Fixed canvas height, fixed leaf plot width.
- [ ] `mixed_height_canvas_width_plot_nested`
  - Reverse mixed orientation with nested facets.
  - Fixed canvas height, fixed leaf plot width.
- [ ] `mixed_width_canvas_height_plot_debug_final`
  - Debug layout snapshot.
  - Confirms debug overlays are understandable for mixed placement.

## Baseline Acceptance Criteria

- [ ] Final rendered scene width equals the requested canvas width for
  width-canvas mixed tests.
- [ ] Final rendered scene height is greater than the default canvas height when
  needed.
- [ ] Leaf subplot plot heights equal the requested leaf plot height within a
  small tolerance.
- [ ] Right and left legends do not extend outside the final canvas.
- [ ] Top and bottom legends do not overlap facet guides.
- [ ] Facet guide labels sit on overflow regions when tick labels require space.
- [ ] No large empty gaps appear where no rendered content exists.
- [ ] Adaptive tick density is based on final subplot dimensions, not estimated
  dimensions.
- [ ] Debug layout clearly shows both explicit and scale-backed placement
  regions.

## Implementation Order

### Phase 1: Policy Types and Public Resolution

- [ ] Add `PhysicalDimension`, `FacetDimensionPolicy`, and
  `FacetRuntimeSizingPolicy`.
- [ ] Replace `FacetRuntimeSizingMode` usage in `EvaluationContext` with the new
  policy.
- [ ] Implement policy resolution from `EvaluatedLayoutSpec`.
- [ ] Replace partial facet constraint rejection with per-dimension validation.
- [ ] Add policy resolution unit tests.
- [ ] Keep old strategy behavior temporarily by mapping:
  - fully canvas policy to old canvas path,
  - fully leaf policy to old plot-area path,
  - mixed policy should initially return a clear internal todo error until
    later phases land.

### Phase 2: Full Plot-Area Targets in Retarget Actions

- [ ] Change retarget actions from `main_axis_size` to full `PlotAreaSize`.
- [ ] Update canvas-fit and plot-area-sized action construction.
- [ ] Update retarget executor.
- [ ] Update retarget trace tests.
- [ ] Run focused retarget tests.

### Phase 3: Unified Placement Model

- [ ] Add `FacetBandPlacementModel` to `FacetBandCoordMeasurement`.
- [ ] Migrate explicit placement out of `FacetBandCoordMeasurementPlotAreaSized`.
- [ ] Update placement helpers to read from the unified measurement.
- [ ] Update render path to resolve placement without global mode dispatch.
- [ ] Remove obsolete plot-area-sized downcast helpers.
- [ ] Run existing canvas and plot-size visual tests.

### Phase 4: Dimension-Aware Measurement

- [ ] Update `resolve_subplot_band_size`.
- [ ] Update local layout derivation to use policy for facet main dimension.
- [ ] Set placement model during measurement.
- [ ] Support mixed policy measurement.
- [ ] Add mixed measurement unit tests.

### Phase 5: Policy-Driven Coordination

- [ ] Replace old strategy pair with policy-driven strategy.
- [ ] Make retarget action construction per-dimension.
- [ ] Make final propagation policy per-dimension.
- [ ] Replace old fixed leaf width/height assertions with per-dimension
  assertions.
- [ ] Run coordination and retarget tests.

### Phase 6: Dimension-Aware Realization and Refinement

- [ ] Generalize plot-area-sized realization to leaf-sized dimensions.
- [ ] Generalize remeasure and refinement loops.
- [ ] Rebuild final layout spec from policy and realized dimensions.
- [ ] Preserve canvas-constrained dimensions exactly.
- [ ] Add realization/refinement tests.

### Phase 7: Visual Tests and Baselines

- [ ] Add `test_facet_dimension_mixed.rs`.
- [ ] Add targeted baseline scenarios.
- [ ] Add one debug layout snapshot baseline.
- [ ] Run focused visual tests in release mode.
- [ ] Review generated failures manually.
- [ ] Accept intentional baselines.

### Phase 8: Cleanup

- [ ] Remove stale `CanvasFit`/`PlotAreaSized` terminology where it no longer
  names a public concept or true special case.
- [ ] Delete obsolete wrappers and helper functions.
- [ ] Update architecture comments for the facet pipeline.
- [ ] Update docs and public API comments.
- [ ] Run:
  - `cargo fmt --all --check`
  - `cargo check -p avenger-chart`
  - focused unit tests
  - focused visual tests
  - full `avenger-chart` visual regression tests in release mode

## Risk Register

- [ ] Mixed media-query semantics may expose ambiguous dimension params. Decide
  and document behavior before accepting baselines.
- [ ] Full unification of facet measurement types is a broad refactor. Keep
  phase boundaries small and preserve passing tests between phases.
- [ ] Explicit-placement extents for only one dimension may reveal assumptions
  in `plot_area_sized_extent`.
- [ ] Adaptive tick counts could change because final dimensions become more
  accurate. Baseline review should focus on whether the result is reasonable.
- [ ] Root clipping with mixed dimensions may need a second pass after visual
  review.
- [ ] The reverse mode, fixed canvas height plus leaf plot width, may reveal less
  obvious horizontal overflow cases. Include at least one visual test early.

## Done Criteria

- [ ] Public mixed sizing example evaluates without error.
- [ ] Existing canvas-fit facet baselines remain reasonable.
- [ ] Existing plot-area-sized facet baselines remain reasonable.
- [ ] Mixed width-canvas height-plot baselines show fixed width and grown height.
- [ ] Mixed reverse baselines show fixed height and grown width.
- [ ] Unit tests cover policy resolution, invalid conflicts, measurement
  placement model selection, retarget actions, final propagation, and
  realization.
- [ ] Debug layout snapshots make mixed placement understandable.
- [ ] No obsolete code paths require global canvas-vs-plot-area dispatch for
  facet placement or rendering.
