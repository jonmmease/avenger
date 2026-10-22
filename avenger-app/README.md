# avenger-app

Application framework for building interactive Avenger-based visualizations.

## Purpose

This crate provides a high-level framework that manages the components needed for interactive visualizations. It manages the app lifecycle, handles event processing, maintains scene graph state, and coordinates updates between user interactions and visual output.

## Integration

- **Input**: Receives `WindowEvent`s from windowing systems
- **Event Processing**: Uses `avenger-eventstream` for sophisticated event handling
- **Scene Management**: Produces an `avenger-scenegraph::SceneGraph` for each layer
- **Spatial Queries**: Maintains an `avenger-geometry::rtree` for each layer

## Layers

An app contains layers ordered from bottom to top. Each layer implements `Layer` and has its own
scene graph, z-index ordering, hit-testing tree, and event streams. `AvengerApp::try_new` takes the
initial state, the layers, and the text engine.

- `Layer::build` creates the scene from app state, as a `BuiltScene` with the rows behind the
  scene's marks. Its `BuildContext` provides lower layers' scenes and hit-testing trees, plus the
  text engine. A tooltip can query the chart's tree for its target. Reusable layers can measure
  text without accessing app-specific state.
- Each tree keeps the rows of the build that made it, so a handler or a layer above reads an
  instance's row with the tree's `datum`. Rows that don't match their scene are logged, and the
  tree is built without them, so hits still work.
- A layer whose scene rebuilds without its tree, as during a pan, has a stale tree until the tree
  builds again, and `BuildContext::tree_is_current` tells a layer above. A tooltip or a crosshair
  shows nothing that a stale tree places. A change that moves nothing, such as a readout that
  follows the pointer, belongs in a layer of its own, since rebuilding its layer's scene without
  the tree marks the tree stale.
- `Layer::streams` returns the layer's event streams in dispatch order. Events visit layers from
  top to bottom. The [avenger-eventstream README](../avenger-eventstream/README.md#layers)
  describes the routing rules.
- When a handler requests a rebuild, the app rebuilds its layer and every layer above it. Lower
  layers remain unchanged, so a crosshair can follow the pointer without rebuilding the chart.
- A rebuilt layer builds its tree again when its own status asks for `rebuild_geometry`, when
  the waker asks for a `Rebuild`, or when a layer below it rebuilt. A layer whose build fails
  logs its error and keeps its previous scene, and the layers above it still build.
- `AvengerApp::update` and `AvengerApp::refresh` return `Changes`: the layers whose scenes changed,
  and whether to draw again. `AvengerApp::scenes` gives each layer's latest scene.

## Wakeups

Background work, such as an image fetch, wakes the host through the app's `AppWaker`, which
`AvengerApp::waker` hands out for the app's state to hold. A request asks for a `Refresh`:
`Wakes` runs the app's wakes that are due, `Redraw` draws the scene again, as when an image that
it shows arrives, and `Rebuild` builds the scene again, as when background work changes the
state.

- A request raises the pending refresh, and the host hears only when it rises, so a burst of
  requests becomes one host event. `Rebuild` outranks `Redraw`, which outranks `Wakes`.
- `AvengerApp::refresh` takes the pending refresh, runs the wakes that are due, and does what they
  ask. A `Rebuild` builds every layer and its tree again, a wake's handler rebuilds its own layer
  and those above it, and a `Redraw` sets `Changes::redraw`.
- Every build of all the layers clears the pending refresh first, since the new scenes show the
  state as it is. A request during the build raises it again.
- `AvengerApp::with_images` draws the scenes' image resources through an avenger-image
  `ImageCache`. The host gives the cache to its canvas, and each image that the canvas asked for
  requests a `Redraw` when it arrives or fails. A prefetch's arrival wakes nothing.

The app times the event streams' wakes, when debounced streams' bursts settle, with a timer of
its own: a thread on native targets, and the browser's `setTimeout`. After each update and
refresh, it sets the timer for the earliest wake, which requests `Wakes` when it's due. A burst
that goes on moves its wake later, so the timer rings at the earlier time, and the refresh then
sets it again.


## Background tasks

A `Task<K, T>` computes values off the event loop for the scene to draw, such as a query's
aggregates over the view that an event handler asks for. Its clones share one slot, so the app's
state holds a clone, and `AvengerApp::with_task` attaches it to the app's runtime and waker. The
app keeps no clone once the task is attached, so when the state's clones drop, the task's pending
request is canceled.

- On native targets, jobs run on the multi-thread Tokio runtime that `AvengerApp::with_runtime`
  sets, and a task's requests wait until the app has one. The app rejects a current-thread
  runtime, which runs its tasks only while a thread blocks on it. In the browser, jobs run on one
  current-thread runtime, which runs in slices of about 8 ms between browser tasks while any of
  its tasks is alive, such as a job or a query's partition that a canceled job left to clean up.
  A slice ends after the poll that runs past its time. A DataFusion task's poll runs until its
  partition ends or it spends Tokio's budget of 128 batches, so a browser job that runs
  DataFusion keeps its partitions or batches small. Browser jobs are CPU work: the runtime runs
  while any of its tasks is alive, so a task that waits on something else, such as a fetch,
  keeps the runtime polling, and the runtime has no timer for `tokio::time`.
- `Task::request(key, job)` runs `job` in place of the request that waits or runs, which is
  canceled: its `Cancel` is set, and its future drops at its next `.await`. A request for the
  pending key does nothing, and a request for the latest result's key cancels the pending request
  and runs nothing. A replaced job that finishes anyway never becomes the latest result.
- `Cancel::yield_now` yields to the runtime, which in the browser lets the runtime's slice end on
  time, and returns `Err(Cancelled)` once the job is canceled, so a long job calls it between
  chunks of work.
- `Task::latest` is the latest successful request with its key, and stays until a newer request
  succeeds, so the scene can draw it while the next request computes. `Task::error` is the last
  request's failure until the next request. On native targets a panic is a failure, and in the
  browser a panic aborts the module, as panics do on wasm32.
- Each request that finishes or fails requests a `Rebuild` on the app's waker.

Event handlers make the requests, often from a debounced stream, and the chart's layer only
places the latest result through the current scales, since it runs on each frame of a pan. Keys
record what a result was computed for, such as a view's domains and pixel size, so the layer
can place a result for an older view while a newer one computes.
