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
- **Debouncing**: Handle a burst of events once it settles, as its last event
- **Between States**: Conditional activation based on start/end triggers
- **Spatial Bounds**: Geographic or coordinate-based filtering

### Debouncing

A stream with `EventStreamConfig::debounce` waits for its input to settle. Matching events that
come less than `wait` milliseconds apart are a burst, and the handler sees the burst's last event
once `wait` milliseconds pass after it. With `leading`, it also sees the burst's first event at
once, and `max_wait` limits how long a burst can delay its last event. The stream takes each event
as it comes, so `consume` stops it from reaching later streams.

The manager's caller times the bursts: `next_wake` says when the earliest burst settles, and
`dispatch_due_wakes` hands each settled burst's last event to its handler. avenger-app's
`AvengerApp` does both, with a timer of its own.

### Layers

`register_handler_in_layer` puts a stream in a layer, such as an overlay's above the scene's, and
`register_handler` puts it in the scene's, 0. Streams in higher layers hear each event first, and
within a layer, the streams added first hear first. A stream that consumes an event takes it from
the streams that hear after it, so a stream whose filter matches only presses on what its layer
draws can take those presses from the scene beneath.

## Integration

- **Input**: Receives `WindowEvent`s from windowing systems
- **Spatial Queries**: Uses `avenger-geometry::rtree::SceneGraphRTree` for hit testing
- **Context**: Provides `MarkInstance` targeting information for handlers
- **Output**: Calls user-defined handlers with `UpdateStatus` for render control