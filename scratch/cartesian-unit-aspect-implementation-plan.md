# Cartesian `unit_aspect` Implementation Plan

## Goal

Add opt-in support for Cartesian coordinate systems where x and y data units have
a fixed screen-length ratio.

The motivating API is:

```rust
Cartesian::new().unit_aspect(1.0)
```

This means one x-domain unit and one y-domain unit occupy the same number of
screen pixels. A line where `x == y` should render at 45 degrees.

This plan is intentionally independent of Web Mercator, map tiles, and
coordinate-owned viewport params. `unit_aspect` is a Cartesian domain/range
constraint, not navigation state.

## Core Semantics

Let:

```text
px_per_x = abs(x_range_span) / x_domain_span
px_per_y = abs(y_range_span) / y_domain_span
```

Define:

```text
unit_aspect = px_per_y / px_per_x
```

So:

- `unit_aspect(1.0)` means equal x/y data-unit screen length.
- `unit_aspect(2.0)` means one y-domain unit is twice as long on screen as one
  x-domain unit.

For a plot area of width `W` and height `H`:

```text
px_per_y = unit_aspect * px_per_x
H / y_span = unit_aspect * W / x_span
y_span = H * x_span / (unit_aspect * W)
x_span = unit_aspect * W * y_span / H
```

This is not a "square domain" feature. In an `800 x 400` plot with
`unit_aspect(1.0)`, `x_domain_span` must be twice `y_domain_span` for `x == y`
to render at 45 degrees.

## Public API

Recommended Cartesian API:

```rust
let plot = Plot::<Cartesian>::with_coord(
    Cartesian::new().unit_aspect(1.0),
)
.mark(Line::new().x(col("x")).y(col("y")));
```

For the common equal-units case, add sugar only if it feels worth the extra API:

```rust
Cartesian::new().equal_units()
```

The ordinary Cartesian API should not require users to name `"x"` and `"y"`.
Cartesian owns those metric dimensions.

Lower-level future API, not necessary for v1:

```rust
Cartesian::new().unit_aspect_between("x", "y", 1.0)
```

## V1 Scope Decisions

V1 should be deliberately conservative:

- Support `UnitAspectPolicy::ExpandDomain` only.
- Support continuous linear numeric x/y scales only.
- Require exactly one x scale and one y scale for the constrained coordinate
  channels.
- Support shared, level-N, and global domain coordination only when the sharing
  cohort is owned by facet/repeat layout, where cells are generated from one
  subplot spec and the implementation can validate consistent plot-area aspect
  equations for the affected domain groups.
- Forbid `unit_aspect` domain sharing across ordinary concat/grid/wrap concat
  child frames in v1. Those containers can allocate independently sized
  subplots through `TrackSizing`, spans, responsive wrapping, and layout
  retargeting.
- Support fixed plot-area layouts first.
- Support canvas/mixed sizing by extending the existing refinement machinery,
  not by adding a second independent refinement loop.
- Make box selection and box zoom aspect constraints explicit tool options.
- Defer standalone tool ratios until there is a separate, well-named box-aspect
  API. In v1, `.unit_aspect()` on a tool means "read the active Cartesian
  coordinate's unit-aspect constraint."

The key boundary is not "sharing" by itself; it is **sharing across independently
sized plot areas**. Local post-build expansion is unsafe for shared domains
because one child could expand a shared x domain without propagating that
expansion to siblings. But facet/repeat sharing can be supported by solving the
unit-aspect expansion at the same coordination layer that already carries
radius-aware shared extents.

Ordinary concat/grid sharing should remain an explicit v1 error because those
containers let authors specify different subplot sizes. A later layout-aware
policy could support them by either proving all shared children have a common
plot-area aspect or by using `ShrinkPlotArea`/letterboxing.

## Implementation Readiness

This plan is ready to start implementation. Phases 1-3 are intended to be
mostly mechanical and can proceed directly from the instructions below:

- Cartesian API/type changes and re-exports;
- resolved unit-aspect constraint metadata;
- authored concat/grid/wrap rejection metadata;
- local post-build `ExpandDomain` behavior;
- the standalone span-graph solver and unit tests.

The architecture-sensitive work starts in Phase 4. The implementing agent
should treat Phase 4 as a short implementation spike plus commit, not as a
purely mechanical patch. Before landing Phase 4, record the concrete choices in
this plan:

- the exact facet insertion point for building and solving the unit-aspect span
  graph;
- the exact generated-repeat concat insertion point;
- the representation used to feed solved domains back into the subsequent scale
  build;
- whether the radius-aware base-domain input comes from a cached-builder scale
  prepass or from enriched `DomainExtent` metadata;
- whether shared facet/repeat preview is implemented immediately or falls back
  to full rebuild until a later phase.

## Progress Tracking Rules

This document is the progress ledger for implementation. Every concrete task
that changes code, tests, docs, or visual baselines should have a checkbox in
the phase where it is expected to land.

Rules for future agents:

- Work one phase at a time unless a later task is required to keep an earlier
  phase compiling.
- Check off a task only after the code is implemented, formatted, tested, and
  included in the phase commit.
- Add a visual-baseline catalog item before generating any new PNG baseline.
  The catalog item must include the target file, scenario name, minimum data and
  layout shape, and visual acceptance criteria.
- Check off a baseline only after the visual scenario and PNG baseline have
  landed, the image has been reviewed, and the matching phase task is checked.
- If a baseline cannot be accepted in its planned phase, leave the checkbox
  unchecked and add a short deferral note naming the phase that will accept it.
- Each phase commit must include the checklist updates for that phase, so the
  document can be used as an implementation resume point.

## Cartesian Type Shape

`avenger-chart-cartesian/src/coord.rs` currently defines:

```rust
pub struct Cartesian;
```

To store options, change this to a struct with fields:

```rust
#[derive(Clone, Serialize, Deserialize)]
pub struct Cartesian {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unit_aspect: Option<CartesianUnitAspect>,
}
```

Add:

```rust
impl Cartesian {
    pub const fn new() -> Self;
    pub fn unit_aspect(mut self, ratio: f64) -> Self;
    pub fn equal_units(self) -> Self;
    pub fn without_unit_aspect(mut self) -> Self;
    pub fn unit_aspect_constraint(&self) -> Option<CartesianUnitAspect>;
}

impl Default for Cartesian {
    fn default() -> Self {
        Self::new()
    }
}
```

Do not add a same-name `pub const Cartesian: Cartesian = Cartesian::new()` in
v1. That shadowing pattern is unidiomatic and not used elsewhere in the repo.
Instead, grep for bare value-style uses such as `Plot::with_coord(Cartesian)` or
`Cartesian.interaction...` and migrate them to `Cartesian::new()`.

## Core Contract

Add a small, coordinate-neutral constraint hook to `avenger-chart-core`.

Sketch:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CartesianUnitAspect {
    pub ratio: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnitAspectConstraint {
    pub x_channel: String,
    pub y_channel: String,
    pub ratio: f64,
    pub policy: UnitAspectPolicy,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum UnitAspectPolicy {
    ExpandDomain,
}

#[derive(Clone, Debug)]
pub struct UnitAspectAdjustment {
    pub x_scale: String,
    pub y_scale: String,
    pub adjusted_axis: UnitAspectAdjustedAxis,
    pub original_x_domain: (f64, f64),
    pub original_y_domain: (f64, f64),
    pub adjusted_x_domain: (f64, f64),
    pub adjusted_y_domain: (f64, f64),
}
```

Add a small runtime-side description of the resolved scale pair:

```rust
#[derive(Clone, Debug)]
pub struct ResolvedUnitAspectConstraint {
    pub x_channel: String,
    pub y_channel: String,
    pub x_scale: String,
    pub y_scale: String,
    pub ratio: f64,
    pub policy: UnitAspectPolicy,
}
```

`CompiledPlot` should expose helpers such as:

```rust
pub(crate) fn has_unit_aspect_constraints(&self) -> bool;
pub(crate) fn resolved_unit_aspect_constraints(
    &self,
) -> Result<Vec<ResolvedUnitAspectConstraint>, AvengerChartError>;
```

The resolver should use `scale_to_coord_channel` and the same channel/scale
target discovery used by range bindings and tools. This keeps container code
from re-implementing coordinate-channel lookup.

Extend `CoordinateSystemTransformCore`:

```rust
fn unit_aspect_constraints(&self) -> Vec<UnitAspectConstraint> {
    Vec::new()
}
```

`Cartesian` returns one constraint when configured:

```rust
UnitAspectConstraint {
    x_channel: "x".to_string(),
    y_channel: "y".to_string(),
    ratio,
    policy: UnitAspectPolicy::ExpandDomain,
}
```

This keeps the public API Cartesian-specific while making scale application
generic enough for future Cartesian-like coordinates.

Compound marks and child frames should not inherit a parent's unit-aspect
constraint implicitly. Each `CompiledPlot` should use the constraints reported
by its own coordinate transform. A compound mark that owns a child Cartesian
coordinate can choose to configure `Cartesian::new().unit_aspect(...)` on that
child explicitly.

## Domain Coordination Validation

Validation has two layers.

Compile-time/local validation:

1. Resolve constrained coordinate channels to scale names using the same scale
   target information used for coordinate range bindings and tools.
2. Require exactly one x scale and one y scale for the constrained pair.
3. Reject if x and y resolve to the same scale.
4. Reject unsupported scale types early when scale specs make that practical.
5. Preserve existing raw-domain coordination validation.

Container/facet validation:

1. If a unit-aspect child plot participates in non-free child-frame domain
   sharing inside ordinary `HConcat`, `VConcat`, `GridConcat`, or `WrapConcat`,
   return a targeted error before calling
   `coordinated_child_frame_domain_extents(...)`.
2. Allow non-free domain sharing inside true facet bands and generated repeat
   containers, then run the sharing-aware unit-aspect domain solver described
   below.
3. If the solver finds inconsistent aspect equations, return an error naming
   the involved channels/domain groups and the plot-area aspects that conflict.

Existing detection points:

- Facet domain sharing is represented by `CellDomainInfo` and
  `ChannelDomainExtent` in `avenger-chart/src/facet/coord.rs` plus
  `avenger-chart/src/facet/domain_coordination.rs`.
- Child-frame/concat domain sharing is represented by
  `ChildFrameDomainSharingInput`, `ChildFrameChannelDomainExtent`, and
  `coordinated_child_frame_domain_extents(...)` in
  `avenger-chart/src/plot/compiled/container_domain_sharing.rs`.
- Concat measurement calls `coordinated_domain_extents_for_concat_children(...)`
  before measuring children in `avenger-chart/src/concat/mod.rs`. That is the
  best v1 place to reject ordinary concat/grid unit-aspect sharing.

Repeat detection:

Repeat currently lowers to concat-family containers. Do not infer repeat origin
from child keys like `"repeat_cell:..."`; that would be brittle. Add internal
origin metadata to concat coordinate structs:

```rust
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConcatOrigin {
    #[default]
    Authored,
    RepeatColumns,
    RepeatRows,
    RepeatGrid,
    RepeatWrap,
}
```

Add `origin: ConcatOrigin` to `HConcat`, `VConcat`, `GridConcat`, and
`WrapConcat` with `#[serde(default)]`, plus pub(crate) setters used only by
`lower_repeat_*_plot(...)`. Generated repeat lowering should set the matching
origin. Ordinary authored concat/grid/wrap keeps `Authored`.

V1 rule:

- `ConcatOrigin::Authored` + non-free domain sharing on a unit-aspect child
  errors.
- `ConcatOrigin::Repeat*` may use the sharing-aware solver, with a runtime
  validation that every connected unit-aspect sharing component has consistent
  plot-area aspect equations.

### Code Detection Cookbook

Detect active unit-aspect constraints on a compiled child plot:

```rust
// Add a PreparedChildFramePlot accessor, or expose the underlying CompiledPlot
// through a narrow helper, rather than reaching through private fields.
let constraints = prepared.child_plot.plot().resolved_unit_aspect_constraints()?;
let has_unit_aspect = !constraints.is_empty();
```

Detect whether a constrained scale participates in child-frame sharing:

```rust
let sharing = prepared.child_plot.channel_domain_sharing_levels();
let x_shared = sharing
    .get(&constraint.x_scale)
    .map(|coordination| !SharingLevel::from(coordination.scope).is_free())
    .unwrap_or(false);
let y_shared = sharing
    .get(&constraint.y_scale)
    .map(|coordination| !SharingLevel::from(coordination.scope).is_free())
    .unwrap_or(false);
```

Authored concat/grid/wrap rejection:

```rust
if concat.origin() == ConcatOrigin::Authored
    && constraints.iter().any(|constraint| x_shared(constraint) || y_shared(constraint))
{
    return Err(AvengerChartError::InvalidArgument(
        "unit_aspect with shared domains across authored concat/grid children is not supported"
            .to_string(),
    ));
}
```

Where to call it:

- `measure_band_concat_coord_system(...)` after `prepare_concat_child(...)` and
  before `coordinated_domain_extents_for_concat_children(...)`.
- `measure_grid_concat_coord_system(...)` after preparing children and before
  `coordinated_domain_extents_for_concat_children(...)`.
- `measure_wrap_concat_coord_system(...)` at the same point.

Facet support detection:

- `CompiledPlot::resolved_unit_aspect_constraints()` tells the facet
  measurement code which scale names are constrained.
- `nested_ctx.facet_tree.channel_domain_sharing_level_typed(scale_name)` tells
  whether each constrained scale is free or non-free in the current facet tree.
- Existing owner-path helpers produce the graph node keys for non-free domains.

Repeat support detection:

- Repeat lowering sets `ConcatOrigin::Repeat*` on the generated concat
  coordinate.
- The same prepared-child sharing lookup above identifies the constrained
  child-frame domain groups.
- The generated origin switches the concat path from "reject authored
  cross-child sharing" to "run the span graph and validate consistency."

## Sharing-Aware Domain Expansion

For facet/repeat sharing, solve unit-aspect expansion over domain-sharing groups
rather than independently per child.

The right abstraction is a small span graph:

- A node is one domain span that must be shared exactly.
  - Non-free domains use the existing `CoordinationScopeKey` produced by
    `domain_coordination_scope_key_with_owner(...)` for facets or
    `child_frame_domain_scope_key(...)` for child frames.
  - Free domains use a synthetic per-cell/per-channel key.
- Each node stores a base numeric domain interval and center.
- Each unit-aspect cell adds an equation between its x node and y node:

```text
Sx = k * Sy
k = ratio * plot_area_width / plot_area_height
```

This equation is equivalent to:

```text
ratio = (plot_area_height / Sy) / (plot_area_width / Sx)
```

Solve each connected component in log space:

```text
log(Sx) - log(Sy) = log(k)
```

Algorithm sketch:

1. Initialize every node with its base span.
2. Traverse each connected component, assigning a relative factor to every node.
   If an already-visited node is reached with a different factor beyond epsilon,
   the component is inconsistent and must error.
3. Pick the minimal component scale `T` such that every node's final span is at
   least its base span:

```text
T = max(base_span[node] / factor[node])
final_span[node] = T * factor[node]
```

4. Expand each node's domain around its stored center to `final_span`.
5. Preserve `DomainExtent` metadata such as radius padding and ordered-discrete
   flags where applicable. Unit aspect only applies to numeric bounds.

This graph solver naturally covers the useful cases:

- free x / free y: one independent two-node component per cell, equivalent to
  local `ExpandDomain`;
- shared x / free y: a large y in one cell increases the shared x span for the
  whole cohort; other cells then expand y if needed;
- free x / shared y: symmetric;
- shared x / shared y with a common plot-area aspect: one shared pair expands
  globally;
- repeat matrix domains: variable-level x/y domain groups form a graph, and
  consistent components solve globally.

It also naturally rejects impossible cases:

- the same domain group used for x and y when `k != 1`;
- two shared groups connected by cells with different required `k` values;
- ordinary concat/grid sharing where independently sized children would create
  inconsistent equations, which v1 should reject earlier with a clearer message.

### Radius-Aware Interaction

Unit-aspect sharing should use base domains **after** ordinary domain
normalization, raw-domain overrides, `zero`, `nice`, padding, and radius-aware
symbol padding. If the graph runs on raw `DomainExtent` bounds and radius-aware
padding runs afterward, symbol padding can change one axis span and break the
unit-aspect invariant.

Implementation options:

1. Preferred for shared facet/repeat support: perform a base scale prepass for
   the cells in a sharing cohort at their current plot-area sizes with
   unit-aspect disabled. Extract the configured numeric domains/ranges for the
   constrained scales, run the span graph, then apply the resulting domain
   overrides for the measurement/render scale build.
2. Simpler non-shared path: keep `apply_unit_aspect_constraints(...)` after
   `ScaleBuilder::build_scales(...)`; this already sees radius-aware padded
   domains because `build_scales` has completed normalization.

The prepass can reuse cached `ScaleBuilder` instances, so it should not
re-query data. It is extra scale math and guide measurement work, not extra
DataFusion domain inference.

### Where To Apply The Solver

Facet:

- Extend the facet domain coordination path after `aggregate_domain_extents(...)`
  has produced coordinated base extents, but before each cell's builder is
  extended/measured.
- Use `CellDomainInfo`, `ChannelDomainExtent`, and the facet tree's owner-path
  helpers to build graph nodes.
- Use the current operating-point child plot-area width/height for each cell.
- Re-run during existing refinement passes when plot-area sizes change.

Repeat:

- Because repeat lowers to concat-family containers, use the generated
  `ConcatOrigin::Repeat*` metadata in concat measurement.
- In `coordinated_domain_extents_for_concat_children(...)`, build graph nodes
  from `ChildFrameDomainSharingInput` and the child plot-area sizes used for the
  measurement pass.
- Run the solver before calling `measure_prepared_concat_child(...)`.
- Re-run during retarget/refinement when child plot-area sizes change.

Ordinary concat/grid/wrap:

- If any prepared child plot has active unit-aspect constraints and any
  constrained scale has non-free child-frame domain sharing, error before
  `coordinated_child_frame_domain_extents(...)`.
- Still allow each child plot to use `unit_aspect` independently when its
  constrained x/y domains are local to that child.

## Scale Application Point

For plots whose constrained x/y domains are local to one plot, apply unit-aspect
constraints in `CompiledPlot::build_scales_from_builder(...)` after
`ScaleBuilder::build_scales(...)` returns configured scales.

Why here:

- inferred, explicit, `zero`, `nice`, padding, local-domain, and raw-domain
  inputs have resolved into configured domains;
- coordinate-owned range bindings have produced x/y ranges for the current plot
  area;
- `DynamicScaleProvider`, child-frame providers, dataframe scale builds, and
  final render scale builds flow through the same method;
- the helper can be deterministic from `(fresh configured scales, plot area,
  coordinate constraints)`.

For facet/repeat shared domains, the graph solver above should produce domain
overrides before the final measurement/render scale build. The post-build helper
should then be a no-op or only expand local/free companion axes. It must not
independently expand a non-free shared axis in one child.

Important idempotence rule:

`apply_unit_aspect_constraints` must run on freshly built scales or on explicit
pre-unit-aspect base domains. It must not repeatedly expand already-adjusted
domains after a range retarget, because resize/preview interactions could
ratchet domains outward or fail to shrink back when the plot size changes.

Add helper:

```rust
pub fn apply_unit_aspect_constraints(
    scales: &mut HashMap<String, ConfiguredScaleWithSpec>,
    constraints: &[UnitAspectConstraint],
    scale_to_coord_channel: &HashMap<String, String>,
    sharing_policy: UnitAspectSharingPolicy,
) -> Result<Vec<UnitAspectAdjustment>, AvengerChartError>
```

Sketch:

```rust
pub enum UnitAspectSharingPolicy {
    LocalOnly,
    ForbidSharedAxisExpansion {
        shared_x: bool,
        shared_y: bool,
    },
}
```

The helper should:

- find the actual scale names for coordinate channels `x` and `y`;
- require exactly one matching scale for each constrained coordinate channel in
  v1;
- require finite positive `ratio`;
- require continuous linear numeric domains and ranges;
- require positive finite domain spans and range spans;
- read numeric interval domains/ranges;
- expand the necessary domain around its center;
- respect sharing policy:
  - `LocalOnly` allows either axis to expand;
  - `ForbidSharedAxisExpansion` errors if the local helper would expand a
    non-free shared axis, because the coordination graph should have handled it;
- patch the configured scale with `with_domain_interval(...)`;
- return adjustment diagnostics for tests/logging.

Zero or degenerate spans should be rejected with a clear error in v1. If scale
normalization already padded a single-valued domain, the helper will see a
positive span and proceed. A future pass can add an epsilon expansion policy for
truly collapsed domains.

## Constraint Policy

V1 implements:

```rust
UnitAspectPolicy::ExpandDomain
```

The constraint preserves the center of the adjusted domain and expands the
minimum required axis so neither original domain is clipped.

Algorithm:

```text
input:
  x_domain = [x0, x1]
  y_domain = [y0, y1]
  x_range_span = abs(x_range_1 - x_range_0)
  y_range_span = abs(y_range_1 - y_range_0)
  ratio = unit_aspect

x_span = abs(x1 - x0)
y_span = abs(y1 - y0)

target_y_span_for_x = y_range_span * x_span / (ratio * x_range_span)

if y_span < target_y_span_for_x:
    expand y around y_center to target_y_span_for_x
else:
    target_x_span_for_y = ratio * x_range_span * y_span / y_range_span
    expand x around x_center to target_x_span_for_y
```

This is a post-normalization constraint. Existing scale inference, explicit
domains, `zero`, `nice`, padding, and raw-domain values produce an initial
domain. Then `unit_aspect` expands one axis if needed.

Do not re-run `nice` after expansion in v1. Re-nicing after the constraint could
break the invariant or require an iterative solve.

Future policy:

```rust
UnitAspectPolicy::ShrinkPlotArea
```

This would preserve domains and reduce the drawable plot area, e.g. letterbox
or pillarbox. It is a layout allocation feature, so it should not be part of the
first pass.

## Layout Interaction

Fixed plot-area sizing is straightforward:

```text
known plot area -> fresh scale build -> unit-aspect expansion -> measure guides -> render
```

Canvas-sized and mixed sizing are trickier:

```text
estimated plot area -> fresh scale build -> unit-aspect expansion
-> guide overflow/layout -> realized plot area may change
```

Do not add a second refinement loop around `compute_layout_and_dimensions`.
Instead, extend the existing refinement machinery controlled by
`FacetLayoutRefinement::max_refinement_passes`.

Implementation guidance:

- The first measurement may build constrained scales at an estimated plot-area
  size.
- When layout realizes a different plot-area size, the next refinement pass must
  rebuild scales from the cached `ScaleBuilder` at that plot-area size, then
  remeasure guides against those freshly constrained scales.
- The refinement loop should treat a unit-aspect scale signature change as a
  reason to continue, alongside existing overflow-growth criteria.
- The loop must not re-infer data extents from marks on each pass. It should
  reuse cached `ScaleBuilder` data and only rebuild configured scales for the
  current plot-area size.
- `max_refinement_passes = 0` should retain the existing fast behavior, but
  tests and docs should make clear that canvas/mixed unit-aspect correctness
  requires the normal refinement setting.
- Record diagnostics through the existing refinement metrics rather than adding
  a parallel counter.

`retarget_scale_ranges_for_plot_area(...)` should not be the final mechanism for
unit-aspect layouts unless the scales also carry pre-unit-aspect base domains.
Range-only retargeting is safe for ordinary scales; with `unit_aspect`, domains
depend on ranges, so a fresh scale build from cached extents is the safer v1
path.

## Preview And Retargeting

Preview currently has fast paths that clone a cached measurement, patch raw
domains or retarget ranges, and reuse prepared/rendered marks through affine
scale adjustments.

With `unit_aspect`, domains depend on plot-area ranges. Therefore, v1 should
prefer scale refresh from a cached `ScaleBuilder` over in-place domain patching.

Rules:

1. Add a predicate such as `CompiledPlot::has_unit_aspect_constraints()`.
2. If active unit-aspect constraints are present, bypass
   `apply_domain_overrides_to_scales(...)` as a raw-domain fast path.
3. Build refreshed scales from the cached scale builder using the target plot
   area and current params.
4. Install those refreshed scales before computing affine mark retarget
   adjustments.
5. If a cached builder is unavailable, fall back to the existing full
   measurement/render rebuild.
6. For faceted/free-cell unit-aspect preview, initially fall back to full
   profile rebuild unless per-cell cached-builder refresh is implemented in the
   same phase.

Preview patch order when scale refresh is available:

```text
1. resolve current params, including raw-domain params
2. build fresh configured scales from cached builder at target plot area
3. apply unit-aspect constraints during that scale build
4. retarget layout geometry as needed
5. compute scale adjustments between old constrained scales and new constrained scales
6. reuse rendered data marks only if affine adjustments are valid
```

Future optimization:

- Store pre-unit-aspect base domains in scale metadata or in a measurement-level
  `UnitAspectAdjustment` table.
- Then range retargeting could reapply the constraint idempotently from base
  domains without rebuilding scales.

## Pan/Zoom Tool Behavior

`PanScrollZoom::cartesian()` updates raw x/y domain params. With
`unit_aspect`, both domains remain low-level raw-domain inputs, and the
coordinate constraint expands after those raw domains are read.

Important behavior:

- ordinary pan of both axes preserves aspect naturally;
- wheel zoom of both axes with the same factor preserves aspect naturally;
- x-only/y-only pan can still be valid, but the final constrained domain may
  expand the other axis around its old center;
- x-only/y-only zoom should be documented as surprising under `unit_aspect`.

Potential enhancement:

```rust
PanScrollZoom::cartesian().respect_unit_aspect(true)
```

When enabled, the tool expansion context can detect the coordinate unit-aspect
constraint and ensure wheel zoom updates both axes with compatible factors even
if the user only configured one axis. This is useful but not required for the
first rendering feature.

## Box Selection And Box Zoom

The existing tools compute box endpoints from:

```rust
ev::start_coord(channel)
ev::event_at_start_clipped_coord(channel)
```

`BoxSelection` stores those endpoints in a mutable store and uses the same
intervals for selection clauses. The overlay reads the store, so changing the
endpoint expressions once affects both the selection predicate and the visible
rectangle.

`BoxZoom` stores overlay endpoint params during drag, then writes raw-domain
params on release. It needs constrained endpoint expressions in both places so
the preview box matches the zoom result.

### Tool API

Add explicit unit-aspect options:

```rust
BoxSelection::cartesian("brush")
    .unit_aspect()

BoxZoom::cartesian()
    .unit_aspect()
```

The no-argument form means "use the active Cartesian coordinate's unit-aspect
constraint." If the coordinate has no compatible constraint for the tool's x/y
channels, return a compile-time tool expansion error.

Recommended option enum:

```rust
pub enum UnitAspectBox {
    CoordinateMetric,
    Viewport,
}

BoxSelection::cartesian("brush")
    .unit_aspect_box(UnitAspectBox::CoordinateMetric)

BoxZoom::cartesian()
    .unit_aspect_box(UnitAspectBox::Viewport)
```

Defaults:

- `BoxSelection::unit_aspect()` -> `CoordinateMetric`
- `BoxZoom::unit_aspect()` -> `Viewport`

Defer `unit_aspect_ratio(...)` in v1. A standalone ratio is easy to misuse
because it does not make the coordinate itself obey that ratio. If users need
general fixed-aspect brushes later, design that as a separate box-aspect feature
with names that describe screen/data/metric behavior directly.

### Box Modes

There are two useful constraints; do not confuse them.

1. **CoordinateMetric**: constrains the dragged data rectangle so its edges have
   equal screen length under the start scale geometry. With
   `unit_aspect(1.0)`, this is both a data square and a screen square. With
   `unit_aspect(2.0)`, the data-space `dy/dx` is `1/2`, because y units are
   twice as long on screen.
2. **Viewport**: constrains the dragged box so it can become the new plot view
   without immediate extra unit-aspect expansion. The screen box has the same
   aspect as the plot area.

### Endpoint Math

Use a fit-inside-drag policy for v1. The constrained box should remain inside
the raw dragged rectangle and preserve drag direction.

Use live start-scale geometry for the endpoint math, not just the configured
ratio. This keeps the tool correct after raw-domain overrides and avoids
assuming that a standalone ratio implies coordinate-level enforcement.

Given:

```text
sx, sy = start data coordinate
cx, cy = clipped current data coordinate
dx = cx - sx
dy = cy - sy
abs_dx = abs(dx)
abs_dy = abs(dy)
sign_x = sign(dx)
sign_y = sign(dy)

start_x_span = abs(end(start_domain(x)) - start(start_domain(x)))
start_y_span = abs(end(start_domain(y)) - start(start_domain(y)))
W = start_plot_width
H = start_plot_height

actual_unit_aspect = (H / start_y_span) / (W / start_x_span)
```

For a CoordinateMetric box:

```text
target_dy_per_dx = 1 / actual_unit_aspect
                 = W * start_y_span / (H * start_x_span)
```

For a Viewport box:

```text
target_dy_per_dx = start_y_span / start_x_span
```

When the coordinate invariant holds, the Viewport formula is equivalent to:

```text
target_dy_per_dx = H / (W * configured_unit_aspect)
```

Then:

```text
if abs_dy > abs_dx * target_dy_per_dx:
    constrained_abs_dx = abs_dx
    constrained_abs_dy = abs_dx * target_dy_per_dx
else:
    constrained_abs_dx = abs_dy / target_dy_per_dx
    constrained_abs_dy = abs_dy

x1 = sx + sign_x * constrained_abs_dx
y1 = sy + sign_y * constrained_abs_dy
```

Build constrained endpoints before calling `ev::interval_ordered(...)` so drag
direction is preserved and the existing interval helpers can sort endpoints
afterward.

Add small DataFusion expression helpers in `avenger-chart-tools` first:

```rust
fn abs_expr(expr: Expr) -> Expr
fn sign_expr(expr: Expr) -> Expr
fn start_domain_span(channel: &str) -> Expr
fn constrained_box_endpoints(...)
fn constrained_drag_interval(...)
fn constrained_drag_distance_squared_px(...)
```

If these prove useful outside tools, move the public pieces to
`avenger-chart-core::event`.

For `BoxZoom::min_size_px`, compare pixel lengths of the constrained box, not
the raw pointer drag:

```text
constrained_dx_px = constrained_abs_dx * W / start_x_span
constrained_dy_px = constrained_abs_dy * H / start_y_span
distance_squared = constrained_dx_px^2 + constrained_dy_px^2
```

Filter or cancel when start domains, plot dimensions, or target ratios are
degenerate or non-finite.

## Event Context Requirements

The event system already exposes:

- `event_at_start_clipped_coord(channel)`
- `start_coord(channel)`
- `start_domain(channel)`
- `start_plot_width()`
- `start_plot_height()`

No event runtime fields are required for the endpoint math.

The needed tool-context addition is coordinate unit-aspect metadata during
expansion:

```rust
ToolExpansionContext::unit_aspect_constraints()
```

`ToolExpansionContext` already carries `scale_targets`, and already has
`single_domain_coordination_for_channel(...)`. Extend the same discovery pass
that builds tool scale targets to include the coordinate transform's
unit-aspect constraints. Tool expansion should verify that the tool channels
map to the constrained coordinate channels. If the constrained domains are
shared, the tool should require that the plot passed unit-aspect sharing
validation; ordinary authored concat/grid cross-child sharing remains an error.

## Tests

Core/unit tests:

- `Cartesian::unit_aspect(...)` rejects non-positive, NaN, and infinite ratios.
- `Cartesian` serde round-trip preserves the optional unit-aspect field.
- Applying equal units to x/y linear scales expands y for a tall required span.
- Applying equal units expands x for a wide required span.
- The adjusted domains preserve the adjusted axis center.
- Reversed y range does not reverse or corrupt y domain.
- Degenerate zero-span domains/ranges return clear errors.
- Nonlinear/log/categorical constrained scales return clear errors.
- Same-scale constraints return clear errors.
- The span-graph solver expands the least required nodes for free/free,
  shared/free, free/shared, and shared/shared components.
- The span-graph solver rejects inconsistent cycles.
- The span-graph solver rejects same-domain x/y nodes unless
  `ratio * width / height == 1` within epsilon.

Domain-coordination tests:

- A simple uncoordinated Cartesian plot with `unit_aspect(1.0)` compiles.
- A facet with free domains and unit-aspect coordinates is accepted.
- A facet with shared x and free y expands the shared x cohort only when local y
  spans require it, and all cells keep the same final shared x domain.
- A facet with free x and shared y behaves symmetrically.
- A facet/repeat with both x and y shared and common plot-area aspect gets one
  shared expanded pair.
- A repeat matrix domain case with consistent variable-domain equations solves
  globally.
- A repeat/facet same-domain x/y group errors when the cell plot area is
  incompatible with the configured ratio.
- An authored `HConcat`/`VConcat`/`GridConcat`/`WrapConcat` with unit-aspect
  child plots and non-free child-frame domain sharing errors before child
  domain coordination.
- An authored concat/grid child with local unit-aspect domains remains allowed.
- A generated repeat concat records `ConcatOrigin::Repeat*` and does not take
  the authored-concat rejection path.
- A positioned child-frame domain-sharing case inside an ordinary concat/grid
  container errors when constrained unit-aspect domains are non-free.
- Radius-aware symbol padding plus unit-aspect sharing uses padded base domains
  and preserves the final ratio.

Scale/runtime tests:

- `Plot::<Cartesian>::with_coord(Cartesian::new().unit_aspect(1.0))` compiles
  and final scales satisfy `px_per_y / px_per_x == 1.0`.
- Explicit x/y domains are accepted as initial domains and one axis expands.
- Raw-domain params are accepted as initial domains and one axis expands.
- Applying the helper to two identical fresh scale builds produces identical
  adjusted domains.
- Preview/retargeting never applies the helper repeatedly to already-expanded
  domains.
- The post-build helper errors if it would expand a shared axis that should have
  been handled by the sharing graph.

Layout tests:

- Fixed plot-area layout measures guides against post-expanded domains.
- Axis ticks/labels are generated from the expanded domain, not the original
  pre-unit-aspect domain.
- Canvas/mixed layout with normal refinement converges and final scales satisfy
  the invariant.
- Facet/repeat shared unit-aspect cohorts re-run the span graph during
  refinement when plot-area sizes change.
- Canvas/mixed layout with `max_refinement_passes = 0` keeps existing fast-path
  behavior and is documented/test-named as an approximate mode if needed.
- Refinement metrics use the existing refinement counters.

Preview tests:

- Raw-domain preview refreshes scales from cached builder when
  unit-aspect constraints are active.
- Resize preview with unit-aspect refreshes constrained scales instead of only
  retargeting ranges.
- Repeated resize preview does not ratchet domains outward.
- Rendered mark reuse only occurs when affine adjustments are valid.
- Faceted/free-cell unit-aspect preview falls back to full rebuild unless
  per-cell scale refresh is implemented.

Tool expansion tests:

- `BoxSelection::unit_aspect()` errors if the coordinate has no unit-aspect
  constraint.
- `BoxZoom::unit_aspect()` errors if the coordinate has no unit-aspect
  constraint.
- `BoxSelection::unit_aspect()` uses constrained intervals in both store rows
  and selection clauses.
- `BoxZoom::unit_aspect()` uses constrained endpoints for drag overlay params
  and release raw-domain params.
- Endpoint construction happens before interval ordering and preserves drag
  direction.
- `CoordinateMetric` and `Viewport` produce different endpoint formulas in a
  non-square plot.
- A `unit_aspect(2.0)` test proves `CoordinateMetric` uses `dy/dx = 1/2` under
  the enforced coordinate.
- `BoxZoom` min-size cancellation uses constrained pixel dimensions.
- Degenerate start-domain spans cancel or no-op cleanly.

Visual baseline catalog:

Each baseline should be added in the phase listed below. When a baseline lands,
check off the catalog item and the matching phase task in the same commit.
Prefer examples where the unconstrained and constrained behavior are visually
obvious at thumbnail size.

If implementation work reveals another behavior that needs visual coverage, add
the new baseline here first with the same file/scenario/spec/acceptance shape,
then add a phase checkbox that names when it will land.

Visual scenarios should live in a new visual-test module such as
`avenger-chart/tests/visual_tests/test_cartesian_unit_aspect.rs`, registered
from `visual_tests/mod.rs`. The baseline category should be
`cartesian_unit_aspect`.

- [ ] Phase 5 baseline:
      `avenger-chart/tests/baselines/cartesian_unit_aspect/diagonal_default_distorted.png`
      Scenario name: `cartesian_unit_aspect_diagonal_default_distorted`.
      Spec: fixed non-square plot area, approximately `600 x 300`, line data
      `y = x` over a symmetric numeric domain, ordinary Cartesian coordinate
      without `unit_aspect`. Acceptance: the diagonal is visibly not 45 degrees,
      establishing the control image for the constrained baseline.
- [ ] Phase 5 baseline:
      `avenger-chart/tests/baselines/cartesian_unit_aspect/diagonal_equal_units.png`
      Scenario name: `cartesian_unit_aspect_diagonal_equal_units`.
      Spec: same data, marks, style, and fixed non-square plot area as the
      distorted control, but with `Cartesian::new().unit_aspect(1.0)`.
      Acceptance: the `y = x` line is visibly 45 degrees and one domain is
      expanded symmetrically around its original center.
- [ ] Phase 5 baseline:
      `avenger-chart/tests/baselines/cartesian_unit_aspect/circle_equal_units.png`
      Scenario name: `cartesian_unit_aspect_circle_equal_units`.
      Spec: dense parametric circle polyline, e.g. `(cos(t), sin(t))`, in the
      same non-square plot area with `unit_aspect(1.0)`. Acceptance: the circle
      renders as a circle, not an ellipse, and remains inside the expanded axes.
- [ ] Phase 5 baseline:
      `avenger-chart/tests/baselines/cartesian_unit_aspect/guide_expanded_domain.png`
      Scenario name: `cartesian_unit_aspect_guide_expanded_domain`.
      Spec: explicit symmetric starting domains, visible axes and grid, and a
      non-square plot area that forces x or y expansion. Acceptance: ticks and
      labels are generated from the expanded domain rather than the original
      domain, and marks near the original edge are not clipped.
- [ ] Phase 4 or Phase 6 baseline:
      `avenger-chart/tests/baselines/cartesian_unit_aspect/facet_shared_equal_units.png`
      Scenario name: `cartesian_unit_aspect_facet_shared_equal_units`.
      Spec: facet or generated repeat with shared constrained domains and
      common cell plot-area aspect, using line or circle marks that make aspect
      distortion obvious in every cell. Acceptance: all cells preserve equal
      units and coordinated domains are identical where sharing requires it.
- [ ] Phase 4 baseline:
      `avenger-chart/tests/baselines/cartesian_unit_aspect/radius_aware_symbols_shared.png`
      Scenario name: `cartesian_unit_aspect_radius_aware_symbols_shared`.
      Spec: sized symbols near numeric domain edges in a shared facet/repeat
      unit-aspect cohort. Acceptance: radius-aware padded base domains feed the
      span graph, symbols are not clipped, and the final unit ratio is still
      preserved.
- [ ] Phase 6 baseline:
      `avenger-chart/tests/baselines/cartesian_unit_aspect/canvas_refined_equal_units.png`
      Scenario name: `cartesian_unit_aspect_canvas_refined_equal_units`.
      Spec: canvas or mixed sizing with visible guides where guide overflow
      changes the realized plot area during refinement. Acceptance: the final
      rendered marks preserve equal units after refinement, not just in the
      first estimated layout pass.
- [ ] Phase 8 or Phase 9 baseline:
      `avenger-chart/tests/baselines/cartesian_unit_aspect/box_zoom_viewport_drag.png`
      Scenario name: `cartesian_unit_aspect_box_zoom_viewport_drag`.
      Spec: interaction visual test that captures the active drag overlay for
      `BoxZoom::unit_aspect()` in a non-square plot. Acceptance: the overlay is
      constrained to the viewport aspect, and the eventual raw-domain update
      would preserve the coordinate unit ratio.
- [ ] Phase 8 or Phase 9 baseline:
      `avenger-chart/tests/baselines/cartesian_unit_aspect/box_selection_metric_drag.png`
      Scenario name: `cartesian_unit_aspect_box_selection_metric_drag`.
      Spec: interaction visual test for
      `BoxSelection::unit_aspect_box(CoordinateMetric)` on a
      `unit_aspect(2.0)` coordinate. Acceptance: the metric-constrained box is
      visibly different from viewport mode and demonstrates the non-1.0 ratio
      semantics.

## Implementation Phases

Agents should work one phase at a time. At the end of each phase:

- update this checklist so completed tasks are checked;
- run the focused tests listed for the phase;
- run `cargo fmt --all`;
- commit with a Conventional Commit message;
- include the checklist update in the same phase commit.

If a task is intentionally deferred, add a short note under that phase instead
of leaving an ambiguous unchecked item.

### Phase 1: Coordinate API And Validation Boundaries

- [x] Add `CartesianUnitAspect`, `UnitAspectConstraint`, `UnitAspectPolicy`, and
      diagnostic adjustment types.
- [x] Add `ResolvedUnitAspectConstraint` and `CompiledPlot` helpers for resolving
      coordinate-channel constraints to scale names.
- [x] Change `Cartesian` from unit struct to option-bearing struct.
- [x] Add `Cartesian::new()`, `.unit_aspect(...)`, optional `.equal_units()`,
      and `.without_unit_aspect()`.
- [x] Migrate bare value-style `Cartesian` call sites to `Cartesian::new()`.
- [x] Add `CoordinateSystemTransformCore::unit_aspect_constraints()`.
- [x] Implement constraint return in Cartesian transform.
- [x] Add compile validation for exact one x/y scale target and different
      scales.
- [x] Add `ConcatOrigin` metadata to concat coordinate structs with repeat
      lowering setting `RepeatColumns`, `RepeatRows`, `RepeatGrid`, or
      `RepeatWrap`.
- [x] Add authored-concat/grid validation that rejects non-free child-frame
      sharing for unit-aspect constrained scales.
- [x] Add serde and validation unit tests.
- [x] Re-export new user-facing types in `avenger-chart-cartesian`,
      `avenger-chart`, and `prelude.rs`.
- [x] Commit Phase 1.

### Phase 2: Scale Constraint Helper

- [x] Implement `apply_unit_aspect_constraints(...)`.
- [x] Add `UnitAspectSharingPolicy` so local builds can forbid expanding a
      shared axis that should have been handled by coordination.
- [x] Require finite positive ratios and positive finite domain/range spans.
- [x] Require linear continuous numeric interval domains/ranges.
- [x] Expand the smaller required domain around its center.
- [x] Return `UnitAspectAdjustment` diagnostics.
- [x] Call the helper from `CompiledPlot::build_scales_from_builder(...)`.
- [x] Confirm the helper only runs on freshly built scales in normal build
      paths.
- [x] Add core scale-application tests.
- [x] Commit Phase 2.

### Phase 3: Sharing-Aware Domain Solver

- [x] Implement the unit-aspect span graph over domain-sharing nodes.
- [x] Represent free domains as per-cell/per-channel nodes and non-free domains
      as existing facet or child-frame `CoordinationScopeKey`s.
- [x] Solve connected components in log space and expand domains around their
      base centers.
- [x] Detect inconsistent cycles and same-domain x/y incompatibilities.
- [x] Preserve numeric `DomainExtent` metadata, including radius padding.
- [x] Add unit tests for free/free, shared/free, free/shared, shared/shared,
      repeat-matrix graph, and inconsistent-cycle cases.
- [x] Commit Phase 3.

### Phase 4: Facet/Repeat Shared Domain Integration

Phase 4 progress note:

- Facet insertion point: `coordinate_cell_domains_before_measurement(...)`
  after `aggregate_domain_extents(...)` has produced ordinary coordinated
  extents and before `coordinated_extents_for_cell_with_owner_paths(...)`
  fills each `FacetCellDraft`.
- Generated-repeat insertion point: concat measurement builds seeded
  operating-point child plot areas, runs
  `coordinated_child_frame_domain_extents_with_unit_aspect(...)` for
  `ConcatOrigin::Repeat*`, then passes ordinary coordinated extents plus
  `unit_aspect_domain_overrides` into `measure_prepared_concat_child(...)`.
- Generated-repeat solved domains feed back through a separate
  `unit_aspect_domain_overrides` map keyed by scale name. The child-frame scale
  provider applies those overrides after the normal scale build, which keeps
  child-local scale normalization from requiring a second shared-axis
  adjustment.
- Facet solved domains use the same explicit `unit_aspect_domain_overrides`
  representation. `coordinate_cell_domains_before_measurement(...)` now takes
  the estimated subplot plot-area size, builds facet-owner scope keys, solves
  the span graph, and installs overrides before measuring cell builders.
- Facet guide overflow estimates may need to build subplot scales before the
  facet solve has a concrete cell measurement. For unit-aspect subplots, that
  estimate uses `UnitAspectSharingPolicy::AllowSharedExpansion`; actual cell
  measurements still use the sharing solver and override map.
- Shared graph inputs now come from a cached-builder scale prepass with
  unit-aspect disabled. The prepass runs after ordinary shared-domain
  coordination at the current operating-point plot-area size, extracts the
  configured numeric domains after `zero`/`nice`/padding/radius-aware symbol
  padding, and feeds those base domains into the span graph. This avoids
  solving from raw `DomainExtent` bounds and then having radius padding change
  the final ratio afterward.

- [x] Spike and document the exact facet insertion point for graph solve.
- [x] Spike and document the exact generated-repeat concat insertion point for
      graph solve.
- [x] Decide and document how solved domains are fed back into scale builds:
      use a separate `unit_aspect_domain_overrides` map, keyed by scale name,
      and apply it after ordinary child scale construction.
- [x] Decide and document whether shared graph inputs come from a cached-builder
      base-domain prepass or enriched `DomainExtent` metadata.
- [x] Integrate the graph solver into facet domain coordination after ordinary
      extents are coordinated and before cell builders are measured.
- [x] Integrate the graph solver into generated repeat concat measurement using
      `ConcatOrigin::Repeat*`.
- [x] Feed generated-repeat unit-aspect solved domains through
      `unit_aspect_domain_overrides` so the final child scale domains match the
      graph solution after scale normalization.
- [x] Use base configured domains after radius-aware padding for graph inputs,
      via a cached-builder scale prepass when needed.
- [x] Ensure ordinary authored concat/grid/wrap cross-child sharing errors before
      graph solving.
- [x] Add facet/repeat sharing tests, authored-concat rejection tests, and
      radius-aware shared unit-aspect tests.
- [x] Add child-frame unit-aspect graph tests for repeat-style shared/free and
      inconsistent shared equations.
- [x] Add a generated-repeat runtime smoke test proving repeat-origin children
      can render with unit-aspect shared domains.
- [x] Add a facet shared-domain runtime smoke test proving facet-origin children
      can render with unit-aspect shared domains.
- [x] Allow facet-guide overflow estimates for unit-aspect subplots to use the
      sharing-tolerant scale-build policy before concrete cell overrides exist.
- [x] Add radius-aware shared unit-aspect runtime tests.
- [ ] Add or prepare
      `cartesian_unit_aspect/facet_shared_equal_units.png`
      (`cartesian_unit_aspect_facet_shared_equal_units`; see catalog spec) when
      the facet/repeat graph integration is renderable in this phase.
- [ ] Add or prepare
      `cartesian_unit_aspect/radius_aware_symbols_shared.png`
      (`cartesian_unit_aspect_radius_aware_symbols_shared`; see catalog spec)
      using sized symbols near coordinated domain edges.
- [x] Commit Phase 4 generated-repeat domain override slice.
- [x] Commit Phase 4 facet-domain slice.
- [ ] Commit Phase 4.

### Phase 5: Fixed Plot-Area Layout And Guides

- [ ] Add fixed plot-area runtime tests proving final scales satisfy
      `px_per_y / px_per_x == ratio`.
- [ ] Add guide measurement tests proving ticks/labels use expanded domains.
- [ ] Add and review
      `cartesian_unit_aspect/diagonal_default_distorted.png`
      (`cartesian_unit_aspect_diagonal_default_distorted`; see catalog spec).
- [ ] Add and review
      `cartesian_unit_aspect/diagonal_equal_units.png`
      (`cartesian_unit_aspect_diagonal_equal_units`; see catalog spec).
- [ ] Add and review
      `cartesian_unit_aspect/circle_equal_units.png`
      (`cartesian_unit_aspect_circle_equal_units`; see catalog spec).
- [ ] Add and review
      `cartesian_unit_aspect/guide_expanded_domain.png`
      (`cartesian_unit_aspect_guide_expanded_domain`; see catalog spec).
- [ ] Review the Phase 5 generated baseline images before accepting them.
- [ ] Commit Phase 5.

### Phase 6: Canvas/Mixed Layout Refinement

- [ ] Extend existing `max_refinement_passes` refinement flow to account for
      unit-aspect scale signature changes.
- [ ] Rebuild scales from cached `ScaleBuilder` at realized plot-area sizes.
- [ ] Avoid final range-only retargeting for unit-aspect scales unless base
      domains are available.
- [ ] Re-run facet/repeat sharing graph solves when plot-area sizes change.
- [ ] Add convergence diagnostics through existing refinement metrics.
- [ ] Add canvas/mixed sizing tests.
- [ ] Add no-ratcheting tests for repeated plot-area changes.
- [ ] Add and review
      `cartesian_unit_aspect/canvas_refined_equal_units.png`
      (`cartesian_unit_aspect_canvas_refined_equal_units`; see catalog spec).
- [ ] Add and review
      `cartesian_unit_aspect/facet_shared_equal_units.png`
      (`cartesian_unit_aspect_facet_shared_equal_units`; see catalog spec) here
      if it was only prepared, not accepted, in Phase 4 because final plot-area
      refinement was required.
- [ ] Commit Phase 6.

### Phase 7: Preview And Interaction Retargeting

- [ ] Add `CompiledPlot::has_unit_aspect_constraints()` or equivalent.
- [ ] Bypass raw-domain in-place scale patching when unit-aspect constraints
      are active.
- [ ] Refresh scales from cached `ScaleBuilder` for raw-domain preview and
      resize preview.
- [ ] Ensure mark reuse sees post-constraint domains.
- [ ] Decide and document whether shared facet/repeat preview refreshes solved
      domains in place or falls back to full rebuild.
- [ ] Fall back to full rebuild when cached-builder refresh is unavailable or
      per-cell facet refresh is not implemented.
- [ ] Add preview, resize, and no-ratcheting tests.
- [ ] Commit Phase 7.

### Phase 8: Box Tool Support

- [ ] Extend `ToolExpansionContext` with unit-aspect constraint metadata.
- [ ] Add `UnitAspectBox` with `CoordinateMetric` and `Viewport`.
- [ ] Add `BoxSelection::unit_aspect()` and
      `BoxSelection::unit_aspect_box(...)`.
- [ ] Add `BoxZoom::unit_aspect()` and `BoxZoom::unit_aspect_box(...)`.
- [ ] Add constrained endpoint expression helpers.
- [ ] Use constrained intervals in `BoxSelection` store rows and selection
      clauses.
- [ ] Use constrained endpoints in `BoxZoom` drag overlay and release raw-domain
      params.
- [ ] Update `BoxZoom` min-size checks to use constrained pixel dimensions.
- [ ] Add tool expansion and expression tests, including `unit_aspect(2.0)`.
- [ ] Add or prepare
      `cartesian_unit_aspect/box_zoom_viewport_drag.png`
      (`cartesian_unit_aspect_box_zoom_viewport_drag`; see catalog spec) once
      the drag overlay is available to the visual test harness.
- [ ] Add or prepare
      `cartesian_unit_aspect/box_selection_metric_drag.png`
      (`cartesian_unit_aspect_box_selection_metric_drag`; see catalog spec)
      once selection-store assertions and visual capture are available.
- [ ] Commit Phase 8.

### Phase 9: Remaining Visual Baselines And Docs

- [ ] Accept
      `cartesian_unit_aspect/box_zoom_viewport_drag.png` if it was only prepared
      in Phase 8.
- [ ] Accept
      `cartesian_unit_aspect/box_selection_metric_drag.png` if it was only
      prepared in Phase 8.
- [ ] Verify every baseline in the visual baseline catalog is either checked off
      or has a written deferral note in this plan.
- [ ] Review all generated baseline images as a set before accepting the final
      visual updates.
- [ ] Update user-facing docs/examples for Cartesian `unit_aspect`.
- [ ] Document v1 restrictions: linear scales, expand-domain policy, authored
      concat/grid sharing rejection, and explicit tool opt-in.
- [ ] Commit Phase 9.

## Recommended V1 Scope

Include:

- Cartesian public API: `unit_aspect(1.0)`.
- Expand-domain policy only.
- Linear continuous x/y position scales only.
- Local free-domain expansion.
- Sharing-aware expansion for facet/repeat cohorts with consistent plot-area
  aspect equations.
- Authored concat/grid/wrap validation that rejects cross-child shared
  unit-aspect domains.
- Fixed plot-area correctness.
- Canvas/mixed layout through existing refinement, if it can be integrated
  cleanly.
- Explicit constrained box selection/zoom options.

Defer:

- Authored concat/grid/wrap shared-domain support.
- Shrink/letterbox plot-area policy.
- Generic user-authored channel pairs.
- Nonlinear scale semantics.
- Automatic aspect behavior for every tool.
- Standalone fixed-aspect box ratios.
- Data/image viewport state.

## Open Questions

Should `BoxZoom` automatically respect coordinate `unit_aspect` by default?

For compatibility, the first pass should require an explicit `.unit_aspect()`
call. But once the coordinate promises equal units, an unconstrained box zoom can
feel broken because the released domain will be expanded. We may want to flip
the default later.

Should `unit_aspect(1.0)` disable Cartesian's default y-zero behavior?

Probably not. `zero` should run before the aspect constraint, and the constraint
expands after that. The final domain may include more space, but it will still
contain zero if zero was requested.

Should the expanded domain be nice?

Not in v1. Exact geometry should win over pretty endpoints. A later iterative
nice-and-constrain solver could be explored if axis aesthetics suffer.

Should same-domain x/y coordination be allowed?

With `ExpandDomain`, yes only when the span graph is consistent. If x and y are
literally the same domain node, the cell requires
`ratio * plot_area_width / plot_area_height == 1`. Otherwise no amount of
domain expansion can satisfy both "same domain" and "fixed unit aspect." A
future `ShrinkPlotArea` policy could make the incompatible case work by reducing
the drawable plot area instead of changing one domain independently.

Should preview retargeting eventually avoid scale rebuilds?

Yes, but only after scales or measurements store pre-unit-aspect base domains.
Without that base state, in-place range retarget plus re-expansion is not
idempotent.
