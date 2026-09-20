# avenger-eventstream

Interactive event handling system for Avenger visualizations.

## Purpose

This crate provides the event handling infrastructure that enables interactive visualizations built with Avenger. It transforms low-level window events into high-level, visualization-aware interactions by using spatial indexing to determine which scene graph elements are being targeted by user input.

## Core Architecture

### EventStreamManager<State>
The central coordinator that:
- Receives raw window events from windowing systems (winit, etc.)
- Uses R-tree spatial indexing to determine which scene marks are under the cursor
- Converts window events into scene-aware events with mark context
- Dispatches events to registered handlers based on their configurations
- Manages interaction state (modifiers, double-click detection, mouse enter/leave)

### Event Types

#### Window Events
Low-level input from the windowing system:
- Mouse input (clicks, movement, scrolling)
- Keyboard input (key press/release)
- Window events (resize, focus, close)
- File system changes (for live reloading)

#### Scene Graph Events
High-level events enriched with visualization context:
- `Click`/`DoubleClick`: Mouse clicks with target mark information
- `MouseEnter`/`MouseLeave`: Hover state changes for specific marks
- `KeyPress`/`KeyRelease`: Keyboard input with cursor position context
- `MouseWheel`: Scroll events with spatial targeting
- `FileChanged`: File system monitoring for development workflows

### Event Stream Configuration

Event streams support sophisticated filtering and behavior control:

#### Targeting
- **Event Types**: Filter by specific interaction types
- **Mark Paths**: Target specific visualization elements
- **Source Groups**: Limit to events within scene graph groups
- **Custom Filters**: Arbitrary event filtering logic

#### Behavior Control
- **Consumption**: Prevent event propagation to other handlers
- **Throttling**: Limit event frequency for performance
- **Between States**: Conditional activation based on start/end triggers
- **Spatial Bounds**: Geographic or coordinate-based filtering

### Finishing and canceling a stream

Set `EventStreamConfig::between_lifecycle` to opt an existing `between` stream into terminal notifications. The handler receives `EventStreamContext::phase`:

- `Update`: an ordinary matching event, possibly delayed by debounce.
- `Finish`: the end trigger. Any pending update is delivered first, followed by one immediate finish notification with the original start context.
- `Cancel`: a matching cancellation trigger. Pending updates are discarded before the cancellation notification.

```rust
use avenger_eventstream::{
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{BetweenLifecycle, DebounceConfig, EventStreamConfig, EventStreamFilter},
};

let drag = EventStreamConfig {
    types: vec![SceneGraphEventType::CursorMoved],
    between: Some((
        Box::new(EventStreamConfig {
            types: vec![SceneGraphEventType::MouseDown],
            ..Default::default()
        }),
        Box::new(EventStreamConfig {
            types: vec![SceneGraphEventType::MouseUp],
            ..Default::default()
        }),
    )),
    between_lifecycle: Some(BetweenLifecycle {
        cancel: Some(EventStreamFilter::event(|event| {
            matches!(event, SceneGraphEvent::WindowFocused(false))
        })),
    }),
    debounce: Some(DebounceConfig::new(5)),
    ..Default::default()
};
```

Cancellation filters can inspect the event, active start context, and scene geometry. Cancellation takes precedence if the same event also matches the end trigger. Only active streams receive terminal notifications. Terminal triggers use their own conditions and bypass the ordinary update types, filters, throttle, and debounce. An earlier handler consuming the event does not prevent active streams from receiving these notifications.

Finish and cancel clear the start snapshot, accepted-event history, throttle state, and pending timer. Stale timer events cannot deliver an update from the closed session. If the handler rejects or fails a flushed update, the finish notification still arrives. Rejection or failure of either terminal notification does not reopen the stream. Ordinary updates retain their existing admission behavior. Cancellation does not undo application state or cancel application-owned timers such as a separate `DebouncedCommit`.

With `between_lifecycle: None`, existing `between`, `emit_between_end_event`, and debounce behavior is preserved. The lifecycle option overrides `emit_between_end_event` when enabled and has no effect without `between`. It does not acquire pointer capture or select a mouse button. Use the existing host commands and event filters for those policies.

### Position context

`EventStreamContext` provides `start_position()`, `current_position()`, and `previous_position()` in logical window coordinates. The current position comes from the delivered event snapshot, including delayed delivery. The previous position comes from the last accepted event. At finish, it includes a flushed update only if that update was accepted.

`delta_from_start()` and `delta_from_previous()` subtract those positions from the current position. Each accessor returns `None` when a required snapshot is absent or its event has no position. A blur cancellation, for example, has no current position. Callers provide plot-local coordinate conversion, clamping, and scale inversion.

## Integration

- **Input**: Receives `WindowEvent`s from windowing systems
- **Spatial Queries**: Uses `avenger-geometry::rtree::SceneGraphRTree` for hit testing
- **Context**: Provides `MarkInstance` targeting information for handlers
- **Output**: Calls user-defined handlers with `UpdateStatus` for render control
