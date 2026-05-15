# Facet Retarget Actions Refactor Plan

## Goal

Replace the current retarget customization model, where sizing strategies mutate fields on a generic canvas-oriented apply plan, with a clearer two-step model:

1. Build a mode-neutral retarget requirements plan from coordinated facet measurements.
2. Ask the active sizing strategy to convert those requirements into explicit retarget actions.

The executor should apply those actions directly. It should not infer sizing-mode policy from low-level booleans such as `has_coordinated_layout`, `cell_retarget_required`, `adjusted_main_size`, or `legend_main_axis_shrink`.

## Current Problem

The current `FacetBandCoordinationApplyPlan` mixes neutral facts with canvas-fit policy:

- Neutral facts:
  - facet axis
  - legend slabs
  - coordinated layout changed
  - coordinated domains exist
  - empty-cell ownership policy
  - child count
- Canvas-fit policy:
  - shrink the main-axis child plot area by legend overflow
  - apply coordinated band layout during retarget
  - retarget every child when legend overflow or coordinated domains exist

This makes plot-area-sized facets customize behavior by editing canvas-fit fields:

- `PlotAreaSizedCoordinationStrategy::prepare_retarget_apply_plan` resets `adjusted_main_size` and `legend_main_axis_shrink`.
- `PlotAreaSizedCoordinationStrategy::execution_retarget_apply_plan` clears `has_coordinated_layout`.

That works, but it is not a good extension point. Future sizing modes would need to know which apply-plan fields are facts and which fields are implementation policy.

## Design Principles

- Requirements are facts, not decisions.
- Strategies decide actions from requirements.
- Executors apply actions and validate plan coverage.
- Canvas-fit and plot-area-sized should share the traversal, coverage checks, trace plumbing, and child-count invariants.
- Sizing strategies should never mutate a plan that was already interpreted for another strategy.
- Action names should describe what will happen to measurements: preserve, apply coordinated layout, retarget plot area, rebuild domains, retarget scale ranges.
- Domain retarget and plot-area retarget should be representable independently.

## Proposed Concepts

### Retarget Requirements

`RetargetNodeRequirements` replaces the strategy-facing parts of `FacetBandCoordinationApplyPlan`.

```rust
pub(crate) struct RetargetNodeRequirements {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) child_count: usize,
    pub(crate) coordinated_overflow: CoordinatedOverflow,
    pub(crate) coordinated_layout: Option<CoordinatedLayout>,
    pub(crate) layout_changed: bool,
    pub(crate) legend_main_axis_slab: AxisSlab,
    pub(crate) has_legend_overflow: bool,
    pub(crate) has_coordinated_extents: bool,
    pub(crate) ownership: FacetOwnershipRequirement,
    pub(crate) child_plot_areas: Vec<PlotAreaSize>,
}
```

Suggested supporting structs:

```rust
pub(crate) struct AxisSlab {
    pub(crate) start: f32,
    pub(crate) end: f32,
}

pub(crate) struct PlotAreaSize {
    pub(crate) width: f32,
    pub(crate) height: f32,
}

pub(crate) struct FacetOwnershipRequirement {
    pub(crate) has_holes: bool,
    pub(crate) axis_owner_ignore_empty_cells: bool,
}
```

Notes:

- `legend_main_axis_slab` is a fact derived from coordinated overflow.
- `child_plot_areas` records the current child sizes at planning time.
- `layout_changed` is still a fact, but it does not imply that the executor should apply coordinated layout.
- `has_coordinated_extents` is a node-level summary; per-cell domain data remains on `FacetBandCoordMeasurement::cells`.

### Retarget Actions

`RetargetNodeActions` is produced by the sizing strategy.

```rust
pub(crate) struct RetargetNodeActions {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) band_action: BandRetargetAction,
    pub(crate) child_actions: Vec<CellRetargetAction>,
}

pub(crate) enum BandRetargetAction {
    Preserve,
    ApplyCoordinatedLayout,
}

pub(crate) enum CellRetargetAction {
    Preserve,
    RebuildDomains,
    RetargetPlotArea { target: PlotAreaSize },
    RetargetPlotAreaAndDomains { target: PlotAreaSize },
}
```

Open option:

- If scale-range-only updates become distinct from domain rebuilds, add `RetargetScaleRanges { target: PlotAreaSize }` rather than overloading `RebuildDomains`.

### Retarget Plan

`RetargetPlan` should store both requirements and actions, or store actions with embedded requirement summary fields for trace/debugging.

Recommended first version:

```rust
pub(crate) struct RetargetNodePlan {
    pub(crate) requirements: RetargetNodeRequirements,
    pub(crate) actions: RetargetNodeActions,
}
```

This is verbose but good during refactor because trace tests can compare facts and actions directly. We can flatten later if it feels heavy.

## Strategy Behavior

### Canvas-Fit

Canvas-fit realizes coordinated requirements inside a fixed outer canvas.

For each node:

- If `layout_changed`, use `BandRetargetAction::ApplyCoordinatedLayout`.
- If any child has coordinated domain extents, rebuild that child's domains.
- If `has_legend_overflow`, shrink the child plot area on the facet main axis by `legend_main_axis_slab.start + legend_main_axis_slab.end`.
- If either domain rebuild or legend shrink is needed for a child:
  - use `RetargetPlotAreaAndDomains` when both are needed,
  - use `RetargetPlotArea` when only plot area changes,
  - use `RebuildDomains` when only domains change.
- If no child action is needed, use `Preserve`.

The target plot area for legend shrink should be:

- Column facet: `(subplot_cross_size, max(1, child_height - legend_slab_total))`
- Row facet: `(max(1, child_width - legend_slab_total), subplot_cross_size)`

This preserves the existing canvas-fit behavior, but it is explicit.

### Plot-Area-Sized

Plot-area-sized realizes coordinated requirements by preserving leaf plot areas and expanding explicit placement.

For each node:

- Use `BandRetargetAction::Preserve` during retarget.
- Ignore legend main-axis slab for child plot-area sizing.
- Rebuild domains where coordinated domain extents are present, using the existing child plot area as the target size.
- Use `CellRetargetAction::Preserve` for children without coordinated domain extents.
- Keep placement refresh after requirements and retarget phases.

This removes the need to zero `adjusted_main_size`, zero `legend_main_axis_shrink`, or clear `has_coordinated_layout`.

## Executor Design

Replace or wrap `FacetBandCoordMeasurement::apply_coordinated_overflow_with_plan` with an action executor.

Suggested API:

```rust
pub(crate) async fn apply_retarget_actions(
    &mut self,
    eval_ctx: &EvaluationContext,
    actions: &RetargetNodeActions,
) -> Result<RetargetNodeOutcome, AvengerChartError>
```

Executor responsibilities:

- Assert `actions.axis == self.axis`.
- Assert `actions.child_actions.len() == self.cells.len()`.
- Record `subplot_cross_size_before`.
- Apply `BandRetargetAction::ApplyCoordinatedLayout` with the existing `apply_coordinated_layout_cross_size`.
- For each child action:
  - `Preserve`: do nothing.
  - `RebuildDomains`: rebuild scales/domains at the current child plot area.
  - `RetargetPlotArea`: retarget plot area without remeasurement.
  - `RetargetPlotAreaAndDomains`: rebuild domains and retarget plot area without remeasurement.
- Apply coordinated alignment slabs to child layouts.
- Return action-level counts.

Suggested outcome:

```rust
pub(crate) struct RetargetNodeOutcome {
    pub(crate) subplot_cross_size_before: f32,
    pub(crate) subplot_cross_size_after: f32,
    pub(crate) band_layout_applied: bool,
    pub(crate) plot_area_retarget_count: usize,
    pub(crate) domain_rebuild_count: usize,
}
```

## Helper Function Cleanup

The current helper name `retarget_measurement_plot_area_and_domains_no_remeasure` makes domain-only retarget awkward.

Suggested split:

```rust
retarget_measurement_plot_area_no_remeasure(...)
rebuild_measurement_domains_no_remeasure(...)
retarget_measurement_plot_area_and_domains_no_remeasure(...)
```

`rebuild_measurement_domains_no_remeasure` should take the existing or requested plot-area size explicitly so both strategies can use it without pretending a plot-area resize occurred.

## Trace Changes

Current `RetargetNodeTrace` still mirrors canvas-fit apply-plan fields. Replace those with requirement and action summaries.

Suggested trace fields:

```rust
pub(crate) struct RetargetNodeTrace {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) planned_child_count: usize,
    pub(crate) planned_has_legend_overflow: bool,
    pub(crate) planned_has_coordinated_extents: bool,
    pub(crate) planned_layout_changed: bool,
    pub(crate) planned_band_action: BandRetargetActionKind,
    pub(crate) planned_child_action_counts: CellRetargetActionCounts,
    pub(crate) parent_cross_size_propagated: bool,
    pub(crate) subplot_cross_size_before: f32,
    pub(crate) subplot_cross_size_after: f32,
    pub(crate) band_layout_applied: bool,
    pub(crate) plot_area_retarget_count: usize,
    pub(crate) domain_rebuild_count: usize,
}
```

Use small `Copy`/`Eq` summary enums for traces if the action structs themselves carry `f32` values.

## Migration Plan

- [x] Add neutral requirement/action structs to `coordination_plans.rs`.
- [x] Add unit tests for requirement construction from a fixture before changing behavior.
- [x] Implement `build_retarget_requirement_plan_with_strategy`.
- [x] Add a `build_retarget_actions` hook to `FacetSizingCoordinationStrategy`.
- [x] Implement canvas-fit action planning from requirements.
- [x] Implement plot-area-sized action planning from requirements.
- [x] Keep the old `FacetBandCoordinationApplyPlan` temporarily while action planning is being validated.
- [x] Add tests comparing old canvas-fit apply-plan outcomes to new canvas-fit actions for representative fixtures.
- [x] Add tests proving plot-area-sized legend slabs do not create plot-area retarget actions.
- [x] Add tests proving plot-area-sized coordinated domains create domain rebuild actions while preserving leaf plot-area sizes.
- [x] Implement `apply_retarget_actions` and `RetargetNodeOutcome`.
- [x] Switch `run_retarget_recursive` to execute action plans.
- [x] Add release-mode invariant errors for missing retarget action plans.
- [x] Add release-mode invariant errors for child action count mismatches.
- [x] Update `RetargetTrace` to action-level terminology.
- [x] Update trace alignment assertions to compare requirements/actions instead of old apply-plan fields.
- [x] Remove `prepare_retarget_apply_plan` from `FacetSizingCoordinationStrategy`.
- [x] Remove `execution_retarget_apply_plan` from `FacetSizingCoordinationStrategy`.
- [x] Remove `FacetBandCoordinationApplyPlan` if no longer used outside tests.
- [x] Rename or delete `derive_coordinated_apply_plan`.
- [x] Split domain-only and plot-area retarget helper functions.
- [x] Run focused canvas-fit coordination tests.
- [x] Run focused plot-area-sized coordination tests.
- [x] Run representative visual baseline tests for facet, facet_legend_sharing, nested_grid, plot-size variants.
- [x] Run `cargo fmt --all`.
- [x] Run `cargo check -p avenger-chart`.
- [x] Run `git diff --check`.

## Test Plan

Focused unit tests:

- [x] Requirement plan contains every facet node in deterministic postorder.
- [x] Requirement plan records legend slabs as facts without implying plot-area shrink.
- [x] Canvas-fit action planning shrinks main-axis child plot areas when legend slabs exist.
- [x] Canvas-fit action planning rebuilds domains when coordinated extents exist.
- [x] Plot-area-sized action planning preserves child plot areas when only legend slabs exist.
- [x] Plot-area-sized action planning rebuilds domains at current leaf size when coordinated extents exist.
- [x] Retarget executor errors on missing node action plan.
- [x] Retarget executor errors on child action count mismatch.
- [x] Retarget traces match planned action counts.

Focused behavior tests:

- [x] Existing canvas-fit `apply_coordinated_overflow_with_plan_retargets_cells_when_required` equivalent still passes under action executor.
- [x] Existing plot-area-sized leaf-size preservation tests still pass.
- [x] Existing plot-area-sized no-main-axis-overlap test still passes.

Visual tests:

- [x] Facet legend sharing cases with right/top/bottom legends.
- [x] Nested grid plot-size-from-canvas cases.
- [x] Plot-area-sized nested row/column mixed sharing.
- [x] Debug layout snapshots for at least one canvas-fit and one plot-area-sized nested case.

## Expected Diff Shape

Likely files touched:

- `avenger-chart/src/facet/coordination_plans.rs`
- `avenger-chart/src/facet/coordination_apply.rs`
- `avenger-chart/src/facet/coordination_strategy.rs`
- `avenger-chart/src/facet/coord.rs`
- `avenger-chart/src/facet/coordination.rs`
- `avenger-chart/src/plot/compiled/rendering.rs` for tests only, if needed

Likely removed concepts:

- `FacetSizingCoordinationStrategy::prepare_retarget_apply_plan`
- `FacetSizingCoordinationStrategy::execution_retarget_apply_plan`
- `FacetBandCoordinationApplyPlan::adjusted_main_size`
- `FacetBandCoordinationApplyPlan::legend_main_axis_shrink`
- `FacetBandCoordinationApplyPlan::cell_retarget_required`
- Possibly the entire `FacetBandCoordinationApplyPlan`

## Risks

- The old apply plan combines domain rebuild and plot-area retarget. Splitting these could accidentally miss scale-range updates. Mitigation: keep helper tests around scale ranges and add action-specific tests.
- Plot-area-sized parent facet scale ranges may depend on retarget-side effects that are currently indirect. Mitigation: verify final propagation tests and plot-area-sized leaf-size preservation tests.
- Trace ordering is postorder. The new plan must preserve this so existing debug snapshot logic remains stable.
- If action planning needs per-cell domain information, the action builder may need access to `FacetBandRef`, not only requirements. That is acceptable as long as requirements remain neutral and actions are strategy-owned.

## Open Questions

- Should `RetargetNodePlan` store both requirements and actions permanently, or should requirements be dropped after action construction?
- Should domain rebuild be represented as a child action, or should it be its own `ScaleRetargetAction` field?
- Should `BandRetargetAction::ApplyCoordinatedLayout` include the target layout explicitly, rather than reading `self.coordinated_layout` during execution?
- Should final propagation eventually use the same action vocabulary for plot-area and scale-range updates?

## Preferred Implementation Order

1. Add new structs and requirement builder without changing behavior.
2. Add action builders for both strategies and test them against current behavior.
3. Add the action executor alongside the old executor.
4. Switch retarget execution to action executor.
5. Remove the old field-mutation hooks and old apply plan.
6. Clean trace naming and tests.
