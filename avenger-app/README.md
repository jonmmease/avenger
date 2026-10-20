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
  ask. It returns `Refreshed`: `Nothing`, `Redraw`, or `Scene` with the scene that it built again,
  along with its hit-testing tree when the refresh or a wake's handler asks for it.
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

