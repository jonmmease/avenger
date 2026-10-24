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

An app draws named scenes from one state. The app's `AppBuild` returns the app's scenes from
`scenes`, each a `Scene` with its name, its z-order, and the app's streams on it, and draws them
in `build`. Tools, reusable interactions such as the tooltips of avenger-app-tools, declare and
draw their own scenes. `AvengerApp::builder` takes the state, the app's build, and the text engine.
The builder then takes the window's size and the tools, and `build` runs the first build:

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

- Each scene has a unique name and a unique z-order, which sets both the order in which the
  scenes draw and the top-down order in which they hear events. Each scene has its own graph,
  hit-testing tree, and streams. The builder fails for a duplicate name or z-order, and for a
  tool's stream on a scene that no one declared. In a scene, the app's streams hear events before
  the tools' streams, which hear them in the order that the app added the tools.
- A handler returns a `Rebuild` that names the scenes that its change affects, each at a `Level`:
  `Scene` rebuilds the scene's graph and leaves its tree stale, as during a pan, and `Tree` also
  rebuilds its tree. The first build, a resize to a nonzero size, and a `Refresh::Rebuild`
  request every scene at `Tree`.
- When anything is requested, the app runs one build: the app's `AppBuild::build`, then each
  tool's `Tool::update`, from the tool whose scenes lie lowest in the z-order up, so each tool
  reads the latest graphs of the scenes below its own. Tools without scenes update first, and
  tools at the same height update in the order that the app added them. The build is responsible
  for the app's scenes, and each tool for its own, so the app calls every tool's update and the
  tool decides from `Scenes` whether it has anything to do. The app logs each requested scene that
  no one set or cleared.
- `Scenes::requested` and `requested_tree` report the requests. `set` replaces a scene's graph and
  rows, and rebuilds its tree at once unless the scene was requested at `Scene`, so a tool reads a
  tree that the app's build just made. A scene that a build sets without a request of its own, as
  a tool sets its scene when a scene below it changes, gets its tree too, so its marks take hits
  where they draw. `clear` removes a scene's marks and tree. `tree`,
  `is_current`, and `changed` report a scene's tree, whether it shows its latest graph, and
  whether this build set or cleared it, and `below` gives the scenes under a scene from the top
  down. A scene that no one sets keeps its graph, its tree, and the GPU resources that the host
  prepared for it, and the app never rebuilds a scene because a scene below it changed.
- Each tree keeps the rows of the build that made it, so a handler or a tool reads an instance's
  row with the tree's `datum`. Rows that don't match their scene are logged, and the tree is
  built without them, so hits still work.
- A tool declares its scenes and attaches its streams, to its own scenes or the app's, in
  `Tool::install`. Its private `Tool::State`, which the app keeps, reaches only its handlers and
  its update. A reusable tool works with any app's state, so it reaches the app's data through
  selectors that the app gives it, such as the scales that it moves.
- The app owns the window's size, which `Scenes::size` and `AvengerApp::size` return. A resize
  to a zero size, as of a minimized window, requests nothing.
- A build that fails logs its error, and the scenes that it set before the error keep their
  changes. A first build that fails fails the builder.
- `AvengerApp::update` and `AvengerApp::refresh` return `Changes`: the scenes that the build set
  or cleared, and whether to draw again. `AvengerApp::scenes` gives each scene's latest graph,
  from the bottom of the z-order up.

## Wakeups

Background work, such as an image fetch, wakes the host through the app's `AppWaker`, which
`AvengerApp::waker` hands out for the app's state to hold. A request asks for a `Refresh`:
`Wakes` runs the app's wakes that are due, `Redraw` draws the scene again, as when an image that
it shows arrives, and `Rebuild` builds the scene again, as when background work changes the
state.

- A request raises the pending refresh, and the host hears only when it rises, so a burst of
  requests becomes one host event. `Rebuild` outranks `Redraw`, which outranks `Wakes`.
- `AvengerApp::refresh` takes the pending refresh, runs the wakes that are due, and does what they
  ask. A `Rebuild` requests every scene with its tree, a wake's handler requests the scenes that
  it names, and a `Redraw` sets `Changes::redraw`.
- Every build that requests every scene with its tree clears the pending refresh first, since the
  new scenes show the state as it is. A request during the build raises it again.
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

The app's build makes the requests, often when its view comes to rest, which `Scenes::requested_tree`
reports, and otherwise only places the latest result through the current scales, since it runs on
each frame of a pan. Keys record what a result was computed for, such as a view's domains and pixel
size, so the build can place a result for an older view while a newer one computes. A request for
the pending key or the latest result's key starts nothing, so the rebuild that a finished request
causes requests the same key and runs no new job.
