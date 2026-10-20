# avenger-app

Application framework for building interactive Avenger-based visualizations.

## Purpose

This crate provides a high-level framework that manages the components needed for interactive visualizations. It manages the app lifecycle, handles event processing, maintains scene graph state, and coordinates updates between user interactions and visual output.

## Integration

- **Input**: Receives `WindowEvent`s from windowing systems
- **Event Processing**: Uses `avenger-eventstream` for sophisticated event handling
- **Scene Management**: Produces `avenger-scenegraph::SceneGraph` for rendering
- **Spatial Queries**: Maintains `avenger-geometry::rtree` for interaction targeting

## Wakeups

Background work, such as an image fetch, wakes the host through the app's `AppWaker`, which
`AvengerApp::waker` hands out for the app's state to hold. A request asks for a `Refresh`:
`Wakes` runs the app's wakes that are due, `Redraw` draws the scene again, as when an image that
it shows arrives, and `Rebuild` builds the scene again, as when background work changes the
state.

- A request raises the pending refresh, and the host hears only when it rises, so a burst of
  requests becomes one host event. `Rebuild` outranks `Redraw`, which outranks `Wakes`.
- `AvengerApp::refresh` takes the pending refresh, runs the wakes that are due, and does what they
  ask. It returns `Changes`, as `AvengerApp::update` does: the scene that it built again, along
  with its hit-testing tree when the refresh or a wake's handler asks for it, the new overlay, and
  whether to draw again.
- Every scene build clears the pending refresh first, since the new scene shows the state as it
  is. A request during the build raises it again.
- `AvengerApp::with_images` draws the scenes' image resources through an avenger-image
  `ImageCache`. The host gives the cache to its canvas, and each image that the canvas asked for
  requests a `Redraw` when it arrives or fails. A prefetch's arrival wakes nothing.

The app times the event streams' wakes, when debounced streams' bursts settle, with a timer of
its own: a thread on native targets, and the browser's `setTimeout`. After each update and
refresh, it sets the timer for the earliest wake, which requests `Wakes` when it's due. A burst
that goes on moves its wake later, so the timer rings at the earlier time, and the refresh then
sets it again.

## Scenes, rows, and overlays

A `SceneGraphBuilder` returns a `BuiltScene`: the scene and its `SceneData`, the rows behind its
marks by mark name, one row for each of a mark's instances. The app pairs the rows with the
instances in the hit-testing tree, so a handler or an overlay can read the data under the pointer.
A handler's `UpdateStatus` asks for what changed: `SCENE` builds the scene again, `SCENE_AND_TREE`
builds its tree too, and `OVERLAY` builds only the overlays.

An overlay draws over the scene without building it, such as a tooltip that follows every pointer
move. `AvengerApp::with_overlay` adds an `OverlayBuilder`, whose `build` returns marks from the
state, the scene, and the scene's rows, and whose `streams` are the event streams that drive it.
The app builds its overlays after each scene build and whenever a handler asks for them, draws
them in the order that it took them, and returns the new overlay in `Changes`. After it builds
the tree again, it sends `TreeRebuilt` to the streams, so an overlay can find its data where the
tree now holds it. An overlay that fails to build logs its error and draws nothing, and the scene
and the other overlays still draw.

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

Event handlers make the requests, often from a debounced stream, and the scene builder only
places the latest result through the current scales, since it runs on each frame of a pan. Keys
record what a result was computed for, such as a view's domains and pixel size, so the builder
can place a result for an older view while a newer one computes.
