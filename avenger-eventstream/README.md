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
- `MouseCancel`: A press that ends without its release, as when the window loses focus first
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

### Layers

An app's scene graphs stack in layers, and each stream hears events in one of them, counted from
the bottom. `dispatch_event` takes each layer's hit-testing tree and walks the layers from the
top, and each event that a layer's streams hear carries the mark under the pointer in that
layer's own tree:

- Events visit layers from top to bottom and each layer's streams in registration order.
  Consumption stops further delivery, except that a press's owner still hears the press's
  pointer events.
- Without a press, an interactive hit stops pointer events at that layer. Key, window, and file
  events continue until consumed.
- A press belongs to the lowest layer it reaches, its owner. Subsequent pointer events are
  offered to that layer and those above it. Consumption in an upper layer cannot block delivery
  to the owner. Clicks still require matching press and release targets.
- Focus loss during a press sends `MouseCancel` to the owner and the layers above it. A stream
  that receives it closes an active `between` interval when its start configuration includes
  `MouseDown`.
- Hover tracking uses each layer's hit-testing tree. Without a press, layers below the topmost
  interactive hit have no hovered mark. During a press, the owner and the layers above it keep
  theirs, and the layers below the owner have none.

A handler gets a `LayerTrees`, which dereferences to its own layer's tree and gives the trees of
the layers below it. `dispatch_event` and `dispatch_due_wakes` return each layer's update status.

### Debouncing

A stream with `EventStreamConfig::debounce` waits for its input to settle. Matching events that
come less than `wait` milliseconds apart are a burst, and the handler sees the burst's last event
once `wait` milliseconds pass after it. With `leading`, it also sees the burst's first event at
once, and `max_wait` limits how long a burst can delay its last event. The stream takes each event
as it comes, so `consume` stops it from reaching later streams.

The manager's caller times the bursts: `next_wake` says when the earliest burst settles, and
`dispatch_due_wakes` hands each settled burst's last event to its handler. avenger-app's
`AvengerApp` does both, with a timer of its own.

## Integration

- **Input**: Receives `WindowEvent`s from windowing systems
- **Spatial Queries**: Uses `avenger-geometry::rtree::SceneGraphRTree` for hit testing
- **Context**: Provides `MarkInstance` targeting information for handlers
- **Output**: Calls user-defined handlers with `UpdateStatus` for render control
