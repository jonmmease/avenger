# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Avenger is a Rust-based visualization engine and renderer designed for information visualization (InfoVis) systems. It provides GPU-accelerated rendering via WebGPU/WebGL2, supports WebAssembly for browser deployment, and aims to become a foundational rendering library for data visualization.

## Key Architecture

### Layered Design
1. **Scene Graph Layer** (`avenger-scenegraph`): Abstract representation of visual elements
2. **Rendering Layer** (`avenger-wgpu`): GPU-accelerated rendering implementation
3. **Event Handling** (`avenger-eventstream`): Reactive event processing
4. **Application Framework** (`avenger-app`): High-level application orchestration

### Core Concepts
- **Scene Graph**: Hierarchical representation with 11 mark types (Arc, Area, Path, Symbol, etc.)
- **Trait-Based Extensibility**: `SceneGraphBuilder<State>`, `EventStreamHandler<State>`, `Canvas`
- **Scales System**: Data transformations with pan/zoom support
- **Event Streams**: Sophisticated event handling with filtering, throttling, and consumption

## Build Commands

```bash
# Build entire workspace
cargo build
cargo build --release

# Build specific crate
cd avenger-scenegraph && cargo build

# Build WASM example
cd examples/iris-pan-zoom
wasm-pack build --target web --release
```

## Development Commands

```bash
# Format code
cargo fmt --all

# Lint with clippy
cargo clippy --all-targets

# Check code (strict mode)
RUSTFLAGS="-D warnings" cargo check --tests

# Run tests
cargo test
cargo test -- --nocapture  # with output

# Run native example
cd examples/iris-pan-zoom && cargo run --release

# Python development (via Pixi)
pixi run dev-py       # Develop Python bindings
pixi run build-py     # Build Python package

# Version management
pixi run bump-version # Bump version numbers
```

## Testing

Tests include unit tests and image baseline comparisons. Note: CI tests are currently disabled due to `MakeWgpuAdapterError` on Linux.

```bash
# Run all tests
cargo test

# Run specific crate tests
cd avenger-wgpu && cargo test
```

## Project Structure

The repository is organized as a Rust workspace with these key crates:
- `avenger-scenegraph`: Core scene graph representation
- `avenger-wgpu`: GPU rendering implementation
- `avenger-scales`: Visualization scales (linear, log, ordinal, etc.)
- `avenger-vega-scenegraph`: Vega compatibility layer
- `avenger-eventstream`: Event handling system
- `avenger-app`: Application framework

## Important Patterns

1. **State Management**: Use `SceneGraphBuilder<State>` pattern for reactive visualizations
2. **Event Handling**: Implement `EventStreamHandler<State>` for interactivity
3. **Resource Management**: Canvas holds GPU resources; proper initialization required
4. **Coordinate Systems**: Scene coordinates with configurable viewports
5. **Performance**: Use instanced rendering for marks with many instances

## Dependencies

- **GPU Rendering**: wgpu (cross-platform WebGPU implementation)
- **Geometry**: Lyon (tessellation), rstar (spatial indexing)
- **Text**: cosmic-text (cross-platform text rendering)
- **Images**: image crate with PNG/JPEG support
- **Data**: Apache Arrow for efficient data handling

## Common Development Tasks

When implementing new features:
1. For new mark types: Add to `SceneMark` enum and implement rendering in `avenger-wgpu`
2. For new scales: Implement `ScaleImpl` trait in `avenger-scales`
3. For interactivity: Use `EventStreamHandler` pattern in `avenger-app`
4. For GPU optimizations: Modify shaders in `avenger-wgpu/src/shaders/`