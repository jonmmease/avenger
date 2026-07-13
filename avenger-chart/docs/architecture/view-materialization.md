# View Materialization And Preview Retargeting

`mark.view(...)` lets a mark define view-local channels and transforms that can
depend on resolved plot ranges, domains, and pixel sizes. The first use case is
Datashader-style rasterization:

1. An exact evaluation builds scales and layout, then emits a view-local
   materialization request such as `Rasterize2D`.
2. `PlotSession` schedules that request off the interaction path.
3. While the request is queued or running, evaluation renders the best
   available data:
   - the requested ready result if present;
   - the latest settled/exact ready result for preview requests when the view
     policy is `ViewStalePolicy::RetargetCached`;
   - the latest ready result for exact requests when stale fallback is allowed;
   - otherwise the transform-provided empty dataframe.
4. When materialization finishes, `MaterializationCache` emits an evaluation
   invalidation through `EvaluationInvalidationHub`.
5. `avenger-chart-app` maps that evaluation invalidation to a
   `RenderInvalidationHub` request so the host can rebuild and render the ready
   result.

Two view coordinate contracts are serialized. `View::cartesian()` resolves
its x/y domain and range parameters from configured positional scales.
`View::pixel_frame()` is scale-free: x/y domains and ranges are
`0..frame_width` and `0..frame_height` in logical pixels. The latter lets
`PixelFrame` content, including composed widget parts, use the same view-local
transform and materialization pipeline without synthesizing coordinate scales
or guides.

## View Policy

`ViewAsyncPolicy` stores one stale-result setting:
`ViewStalePolicy::HideUntilReady` or `ViewStalePolicy::RetargetCached`.
The public `.preview_cached(true)` builder method is only sugar for
`RetargetCached`; it is not stored as a separate runtime flag.

`debounce` and `throttle` apply only to preview-priority materializations.
Exact and force-remeasure evaluations bypass both so settled interaction states
can start immediately.

When debounce or throttle defers a queued request, the scheduler emits a delayed
evaluation invalidation. The next evaluation re-checks the same queue even if no
new pointer or wheel event arrives.

Preview consumption of a ready result is additionally gated on key stability:
a `RetargetCached` preview only rebuilds data marks to consume a ready
materialization once its desired key has been unchanged for
`PREVIEW_CONSUME_STABILITY`. While the view params are still moving
frame-to-frame (an active gesture), completions keep retargeting the cached
scene instead of interrupting the gesture with a rebuild, and the deferral
schedules a delayed re-evaluation so a hold or release still swaps the fresh
result in without another interaction event. Previews that rebuild for other
reasons (for example, no cached data marks yet) consume ready results
immediately.

## Cache Bounds

`MaterializationCache` stores queued, running, ready, and error entries. Queued
entries for the same identity are replaced by newer queued work, but running
work is allowed to settle.

Ready results are retained conservatively:

- each materialization identity keeps its latest ready result;
- each identity also keeps its latest settled/exact ready result when distinct;
- unscoped ready results are bounded by a small LRU;
- a defensive global ready cap evicts only unprotected ready entries.

This keeps long pan/zoom sessions from retaining every intermediate viewport
while preserving the stale fallback needed for smooth previews.

## Group View Scopes

`MarkGroup::view(...)` shares one view scope across every child mark of a
group, splitting the group's chains by closure position exactly the way
mark-level `.view(...)` splits a mark's own chain:

- group transforms **outside** the view closure are pre-view: they feed
  positional domain inference and are prepared once as shared group base
  data;
- group transforms **inside** the closure form the group's **view-local
  chain**: it runs once per (plot, group, facet path) per evaluation and its
  output dataframe and derived scalars are shared by all children;
- child marks inside the scope are view-scoped: their own transforms and
  channels are view-local, and their x/y render channels do not contribute
  to positional domain inference.

At plot compile time, `lower_group_views` lowers the group's view spec onto
each child mark's compiled state, so every existing view consumer
(domain-inference gating, view param resolution, tool target discovery,
preview rebuild forcing) applies to group children with no special casing.
The group's own view-local chain stays on the compiled group state.

At runtime the shared chain is memoized on first use in a per-evaluation
cache on `EvaluationContext` (`group_view_data_cache`): the first child whose
view preparation needs the group result computes it with its own view-param
context, and later children reuse it (children share the cell's scales, so
their resolved view params are identical — asserted in debug builds).
Children's view chains start from the shared output dataframe with its
derived scalars seeded in; the schedule-only preview path mirrors this so it
computes identical materialization keys.

This is the substrate for adaptive representation switching: a group
view-local `Filter` + eager `ScalarAggregate` count feeds gates on an async
rasterized child and a synchronous scatter child, switching at a point
budget (see `avenger-chart-app/examples/taxi_adaptive_points.rs` and the
`group_view` visual baselines).

## Transform Execution

Ordinary mark transforms, view-local transforms, group view-local chains, and
cached-preview scheduling all use the same scoped transform-chain executor in
`plot::compiled::mark_data_runtime`.

The executor owns:

- scoped transform ordering;
- facet dataframe narrowing as scopes get more specific;
- selection predicate expansion;
- derived-scalar resolution into later stage expressions (seeded with
  inherited scalars from prepared base / group / pre-view chains);
- ordinary `CompiledDataTransform::apply(...)`;
- optional `view_materialization_request(...)`;
- derived scalar duplicate detection.

The schedule-only preview path still advances through each materialization's
display dataframe. That keeps later transforms in the same view chain seeing the
same schema as full view evaluation.

View-local transforms placed **after** a materialized transform apply to the
materialization's display dataframe on every path — pending-empty, ready, and
stale-preview. This is how a scalar-gated raster drops its raster row: the
gate filter follows `Rasterize2D` in the chain, so the decision never enters
the materialization key and the fallback cache keeps real rasters.

## Host Wiring

Async materialization completion wakes a renderer only when the chart app has a
`ChartRuntimeResources` value and the host uses the same
`RenderInvalidationHub`. If materialization work is queued without that
subscription, `avenger-chart-app` logs a one-time warning because completed
results may not trigger a rerender.
