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

## Integration

This crate serves as the "main" entry point for native Avenger applications. It orchestrates the entire stack from window events through GPU rendering, providing a complete solution for desktop interactive visualizations.

Typical usage involves creating an `AvengerApp` with custom state and scene graph builder, then running it through the winit event loop.
