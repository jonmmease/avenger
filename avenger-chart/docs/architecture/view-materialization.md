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

## Transform Execution

Ordinary mark transforms, view-local transforms, and cached-preview scheduling
all use the same scoped transform-chain executor in
`plot::compiled::mark_data_runtime`.

The executor owns:

- scoped transform ordering;
- facet dataframe narrowing as scopes get more specific;
- selection predicate expansion;
- ordinary `CompiledDataTransform::apply(...)`;
- optional `view_materialization_request(...)`;
- derived scalar duplicate detection.

The schedule-only preview path still advances through each materialization's
display dataframe. That keeps later transforms in the same view chain seeing the
same schema as full view evaluation.

## Host Wiring

Async materialization completion wakes a renderer only when the chart app has a
`ChartRuntimeResources` value and the host uses the same
`RenderInvalidationHub`. If materialization work is queued without that
subscription, `avenger-chart-app` logs a one-time warning because completed
results may not trigger a rerender.
