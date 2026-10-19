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

Background work, such as an image fetch or an app's timer, wakes the host through the app's
`AppWaker`, which `AvengerApp::waker` hands out for the app's state to hold. A request asks for a
`Refresh`: `Redraw` draws the scene again, as when an image that it shows arrives, and `Rebuild`
builds the scene again, as when background work changes the state.

- A request raises the pending refresh, and the host hears only when it rises, so a burst of
  requests becomes one host event. `Rebuild` outranks `Redraw`.
- `AvengerApp::refresh` takes the pending refresh and does what it asks, and returns `Refreshed`:
  `Nothing`, `Redraw`, or `Scene` with the scene that it built again, along with its hit-testing
  tree.
- Every scene build clears the pending refresh first, since the new scene shows the state as it
  is. A request during the build raises it again.
- `AvengerApp::with_images` draws the scenes' image resources through an avenger-image
  `ImageCache`. The host gives the cache to its canvas, and each image that the canvas asked for
  requests a `Redraw` when it arrives or fails. A prefetch's arrival wakes nothing.

The app has no timers of its own. An app that waits, such as one that acts when the cursor rests,
runs its own timer and requests a refresh from it.

