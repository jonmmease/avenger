# avenger-winit-wgpu

Native window application runner for interactive Avenger visualizations.

## Purpose

This crate provides a complete native application framework that combines all Avenger components into a runnable desktop application. It handles window management, event processing, and the main application loop for interactive visualizations.

## Architecture

The main component is `WinitWgpuAvengerApp`, which integrates:
- **winit**: Cross-platform window creation and event handling
- **avenger-wgpu**: GPU rendering to window surfaces
- **avenger-app**: Application state management and scene graph coordination
- **avenger-eventstream**: Event processing and interaction handling

The crate also includes `FileWatcher` events for development workflows, supporting live reloading when specification files change.

The event loop's user event is `WinitWgpuEvent`: `App` carries an event for the app, such as a
changed file, and `Refresh` comes from the app's `AppWaker`, through the event loop's proxy, when
background work asks for a refresh or a wake is due. The host then calls `AvengerApp::refresh`,
and draws the scene again or installs the scene that the app built again. A pending render never skips a
refresh. The host gives the app's image resolver, from `AvengerApp::with_images`, to its window
canvas, and refreshes once after its first scene, for the requests that came before it listened.
The same path runs in the browser.

The host displays the cursor from `AvengerApp::cursor` and applies updates from `Changes::cursor`.
Keyboard events, including Escape, reach the app so tools can clear a selection. Only a
window-close request exits the host.

Ctrl+C or Super+C (Cmd+C on macOS) copies the app's nonempty copy text to the system clipboard,
using `arboard` natively or the browser Clipboard API. The host stores the last reported text,
so the shortcut can copy it without waiting for an app update.

## Integration

This crate serves as the "main" entry point for native Avenger applications. It orchestrates the entire stack from window events through GPU rendering, providing a complete solution for desktop interactive visualizations.

Typical usage involves creating an `AvengerApp` with custom state and scene graph builder, then running it through the winit event loop.
