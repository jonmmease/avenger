# avenger-app

Application framework for building interactive Avenger-based visualizations.

## Purpose

This crate provides a high-level framework that manages the components needed for interactive visualizations. It manages the app lifecycle, handles event processing, maintains scene graph state, and coordinates updates between user interactions and visual output.

## Integration

- **Input**: Receives `WindowEvent`s from windowing systems
- **Event Processing**: Uses `avenger-eventstream` for sophisticated event handling
- **Scene Management**: Produces `avenger-scenegraph::SceneGraph` for rendering
- **Spatial Queries**: Maintains `avenger-geometry::rtree` for interaction targeting

## Background tasks

`background::BackgroundTasks` owns an app's background work. Create one `BackgroundTask<T>` for each independently replaceable operation, such as a visible chart query and hover warm-up. Attach the group with `app.with_background_tasks(tasks)` before giving the app to its host.

```rust
use avenger_app::background::BackgroundTasks;

let tasks = BackgroundTasks::new();
let mut query = tasks.task::<Vec<u32>>();
query.submit(async {
    Ok::<_, std::io::Error>(vec![1, 2, 3])
})?;
```

The host starts queued work when it can receive events. A later `query.submit(...)` replaces the previous request and cancels its future. `query.cancel()` also invalidates a result whose wake is already queued. Cancellation is cooperative and does not interrupt synchronous code inside a future poll or an independently spawned blocking operation.

In a `SceneGraphEvent::RuntimeWake` handler, call `query.handle_wake(wake)`. It returns `Some(Ok(Arc<T>))` or `Some(Err(BackgroundTaskError))` for the current completion, and `None` for unrelated, stale, or already handled wakes. Keep the previous chart result visible until this handler installs its replacement. `query.is_pending()` stays true until the current completion is handled. Errors retain their original source, and results need not implement `Clone`.

Task handles can live in cloneable app state. Each clone retains its own expected request and delivery cursor, while the completion value is shared. Reading a completion does not drain it from another state copy. Submission and cancellation remain immediate external effects. A failed scene build does not undo those operations, and an older state copy cannot accept a newer candidate's result.

The host cancels the group when the app is replaced or the host shuts down. Each replacement app needs a fresh group. Dropping the last task handle also cancels its work. Futures should capture their input data and services, without capturing their own controlling task handle.

The helper has no query cache or dependency graph. Canceling a dataflow query drops that consumer future, leaving the dataflow runtime to decide whether shared calculations still have other consumers. Debouncing remains in `avenger-eventstream`.

Custom hosts implement `background::host::Executor` and retain an `Attachment`. Native winit hosts use their existing Tokio runtime. Browser hosts use `spawn_local`, which schedules on the browser thread and does not offload CPU-heavy work to a Web Worker. The initial API requires `Send` futures and `Send + Sync` results on both platforms.
