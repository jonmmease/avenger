# avenger-app

Application framework for building interactive Avenger-based visualizations.

## Purpose

This crate provides a high-level framework that manages the components needed for interactive visualizations. It manages the app lifecycle, handles event processing, maintains scene graph state, and coordinates updates between user interactions and visual output.

## Integration

- **Input**: Receives `WindowEvent`s from windowing systems
- **Event Processing**: Uses `avenger-eventstream` for sophisticated event handling
- **Scene Management**: Produces an `avenger-scenegraph::SceneGraph` for each scene
- **Spatial Queries**: Maintains an `avenger-geometry::rtree` for each scene

## Scenes and tools

An app draws named scenes from shared app state. `AppBuild::scenes` declares each scene's name,
z-order, and event streams. `AppBuild::build` draws the scenes. Reusable tools can attach streams to
existing scenes and declare their own scenes, such as a tooltip overlay.

`AvengerApp::builder` takes the state, an `AppBuild` implementation, and a text engine. Add tools
and set the initial window size before calling `build`:

```rust
struct Chart;

#[async_trait]
impl AppBuild<State> for Chart {
    fn scenes(&self) -> Vec<Scene<State>> {
        vec![Scene::new("plot", 0).stream(config, handler)]
    }

    async fn build(&self, state: &mut State, scenes: &mut Scenes<'_>) -> Result<(), AvengerAppError> {
        if scenes.requested("plot") {
            scenes.set("plot", draw(state, scenes.size()))?;
        }
        Ok(())
    }
}

let app = AvengerApp::builder(state, Chart, engine)
    .tool(Tooltips::new("tooltip", 100).over(targets))
    .build()
    .await?;
```

- Each scene has a unique name and z-order, plus its own graph, hit-testing tree, and event streams.
  Scenes draw from bottom to top and receive events from top to bottom. The builder rejects
  duplicate names or z-orders and streams attached to undeclared scenes. Within a scene, app streams
  receive events first, followed by tool streams in registration order.
- Handlers return named `Rebuild` requests. `Level::Scene` rebuilds a graph while leaving its tree
  stale. `Level::Tree` also rebuilds the tree. The initial build, a resize with both dimensions at
  least one logical pixel, and `Refresh::Rebuild` request every scene at `Tree`.
- A build calls `AppBuild::build`, then every `Tool::update`. Tools are ordered by their lowest
  scene z-order. Tools without scenes use `i32::MIN`, and ties keep registration order. This orders
  whole tools. If two tools have interleaved scene ranges, it does not guarantee that every lower
  scene updates before a higher one.
- The app and each tool decide which of their scenes to set or clear. `requested` and
  `requested_tree` report the current requests. `changed` reports scenes already set or cleared in
  this build, so a tool can react to a dependency. The app logs requested scenes that no build or
  update sets or clears.
- `Scenes::set` replaces a graph and rebuilds its tree immediately, except for an explicit
  `Level::Scene` request. Setting a scene without its own request also rebuilds its tree, so
  dependent interactive scenes have current hit geometry. `clear` removes the marks and tree. `tree`
  returns the stored tree, and `is_current` reports whether it matches the graph. `below` visits
  lower scenes from top to bottom.
- Unchanged scenes retain their graphs, trees, and prepared GPU resources. A change in a lower scene
  does not automatically rebuild scenes above it.
- Each tree retains the data rows used to build it. Handlers and tools can read an instance's row
  through `datum`. If rows do not match the marks, the app logs the error and builds the tree
  without rows, preserving hit testing.
- `Tool::install` declares scenes and attaches streams to declared app or tool scenes. The app owns
  each private `Tool::State` and passes it only to that tool's handlers and update method. Selectors
  let reusable tools access app-owned data, such as the scales they modify.
- A handler's `disposition` decides, from the state and the scene trees, whether its stream
  ignores, handles, or handles and consumes each event that its configuration matches. A tool can
  consume presses on text with one stream and leave other events to lower scenes.
- `Tool::cursor` and `Tool::copy_text` provide cursor and copy-text preferences. Tools are queried
  in reverse update order, and the first `Some` value wins for each output. If no tool requests a
  cursor, the app uses `AppBuild::cursor`, then `CursorStyle::Default`. If no tool provides copy
  text, the result is empty. `AvengerApp::cursor` and `AvengerApp::copy_text` return the current
  values.
- `Scenes::size` and `AvengerApp::size` return the window size. A zero-size resize, such as
  minimization, does not trigger a full rebuild.
- Build errors are logged. Scenes set before an error retain their changes. An error during the
  initial build fails app construction.
- `update` and `refresh` return `Changes`, containing changed scene indices, a redraw flag, and
  the cursor and copy text when they change. `AvengerApp::scenes` returns the current graphs in
  ascending z-order.

## Wakeups

Background work wakes the host through `AppWaker`, returned by `AvengerApp::waker`. Requests have
three priorities: `Wakes` dispatches due events, `Redraw` draws the existing scenes, and `Rebuild`
rebuilds every scene and tree.

- The waker notifies the host only when a request raises the pending priority. `Rebuild` outranks
  `Redraw`, which outranks `Wakes`.
- `AvengerApp::refresh` consumes the pending request and dispatches due events. Their handlers can
  request named scene rebuilds. `Redraw` sets `Changes::redraw`, while `Rebuild` requests every
  scene at `Level::Tree`.
- A full rebuild clears the pending refresh before it starts. Requests made during the build remain
  pending.
- `AvengerApp::with_images` supplies an `ImageCache` to the host's canvas. An image requested by the
  canvas triggers a redraw when it loads or fails. Prefetch completion does not wake the host.

Debounced streams use a timer thread on native targets and `setTimeout` in the browser. After each
update or refresh, the app schedules the earliest wakeup. If more input delays a burst, the earlier
timer can still fire. The resulting refresh schedules the new deadline.

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

The app requests background work during a build. The examples use `Scenes::requested_tree` to
request resampling after gesture settling, on initial build, and on resize. During movement, they
draw the latest result through the current scales. Request keys identify the view used for
computation, such as its domains and pixel size. This lets the app display an older result while a
new one computes. A request for the pending key or the latest result's key starts no new job, so a
completion refresh can safely request the same key.
