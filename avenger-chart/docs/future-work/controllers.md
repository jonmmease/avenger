# Controllers And Interactivity

## Goal Review

The goal is valid: Avenger should support reusable chart interactions such as
pan/zoom, brushing, selection, hover highlighting, linked views, and reset
actions.

Part of the foundation already exists outside `avenger-chart`:

- `avenger-eventstream` has `EventStreamConfig`, `EventStreamFilter`,
  `EventStreamHandler`, and `EventStreamManager`.
- `avenger-app` wires event streams into application state updates.
- `avenger-chart-core::Param` and `Plot::add_param` allow runtime parameter
  values to flow into expressions, themes, layout, and scale building.
- `EvaluatedPlot` can contain a `SceneGraphRTree` for hit testing.
- The `examples/iris-pan-zoom` demo implements pan/zoom behavior manually
  through event streams and parameter-like state.

That is not yet a chart controller API. Today, interactions are application
code that knows chart details.

## Current System Fit

Controllers should not be mark types or coordinate systems. They should be
chart/application adapters that:

- declare which scenegraph events they listen to,
- maintain interaction state,
- emit updated `Param` values or evaluation options,
- optionally use `SceneGraphRTree` hit-test results,
- optionally coordinate state across facet paths or child-frame paths.

The natural boundary is above `CompiledPlot::evaluate_with_options` and below
the app event loop.

## Recommended Direction

Create a small controller layer that composes existing event streams with chart
params. A first built-in controller should be pan/zoom for Cartesian plots
because the manual example already proves most of the event math.

Possible shape:

```rust
pub trait ChartController: Send + Sync {
    fn event_streams(&self) -> Vec<EventStreamConfig>;
    fn update(&self, event: &SceneGraphEvent, state: &mut ControllerState);
    fn params(&self, state: &ControllerState) -> IndexMap<String, ScalarValue>;
}
```

The trait should be treated as experimental until linked brushing and faceted
state sharing are designed.

## Alternate Paradigms

- **Application-only interactions**: current demos prove this works. It keeps
  `avenger-chart` simpler but makes common chart behaviors hard to reuse.
- **Reactive params only**: expose params and let users build event streams.
  This is flexible but still too low-level for pan/zoom and brush selection.
- **Vega-like signal graph**: powerful, but likely too large a semantic system
  for the current Rust-first API.

## Readiness

Ready for a design spike.

Do not design the full selection grammar first. Port the existing pan/zoom demo
into a reusable experimental controller and use that to settle state ownership,
parameter naming, and facet sharing.

## Decisions Needed

- Whether controllers live in `avenger-chart`, `avenger-app`, or a new
  `avenger-chart-interaction` crate.
- How controller state is scoped across facets, concat children, positioned
  subplots, and multiple rendered charts.
- How controllers discover scale names and coordinate channels.
- How selections map scenegraph hits back to data rows or mark identities.
- Whether interaction state is serialized with chart specs.
