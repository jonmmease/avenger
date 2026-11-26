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
- **Trait-Based Extensibility**: `AppBuild<State>`, `Tool<State>`, `EventStreamHandler<State>`, `Canvas`
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
- `avenger-selection`: Selections of rows: clauses and their conditions, pixel grids, matching over Arrow arrays
- `avenger-selection-datafusion`: Filters of DataFusion queries by selections: `selected` and its lowering, the row tests, and pre-aggregation
- `avenger-query-graph`: Kept results of the subplans of a document's DataFusion queries: cuts where an input is first read, keys of the inputs read, pre-aggregation for a focus, and the graph of a document's charts, live over its sources or baked with its rows, with the file of a baked graph
- `avenger-chart-datafusion`: The DataFusion side of avenger's charts, natively and in WebAssembly pages: the aggregates of the marks and of the compositions, and the codec of the logical plans that call them, with the functions and the nodes of a host
- `avenger-coords`: Plot projections between pixels and the positions of plots' dimensions, for Cartesian, polar, and map plots, and plot coordinates: a projection with the scales of its dimensions
- `avenger-vega-scenegraph`: Vega compatibility layer
- `avenger-eventstream`: Event handling system
- `avenger-app`: Application framework
- `avenger-app-tools`: Reusable app tools (tooltips, pan and zoom)
- `avenger-widgets`: Controls (buttons, check boxes, check box lists, radio groups, sliders, range sliders, text inputs, group boxes), drawn by a `Widgets` tool behind the `app` feature and placed by the app's layout, or drawn on their own
- `avenger-chart-eval`: The evaluation of avenger charts at any size: the style tree and its computed styles, scales and domains from the format, marks, guides, the layout, and the scene graph
- `avenger-chart-app`: An avenger app of a chart: the tools that its plots and layers declare, and one handle of updates of its chart, its styles, its mode, and its selections
- `avenger-query-web`: A WebAssembly library that runs a document's graph of queries in a page, over avenger-chart-web's renderer, so its charts follow the reader's selections without a server
- `avenger-results-egui`: The result of a chart script in egui, as the window of `avenger view` shows it: one live chart, or a table of the result whose columns and rows resize and whose chart cells open their charts over the table
- `avenger-datafusion-shared`: Shared inputs of DataFusion plans: an input that a plan reads several times and that runs once, and the rule that computes cheap inputs again in place of sharing them
- `avenger-sql`: DataFusion SQL that builds avenger charts: the mark functions and constructors of avenger-chart's `functions` feature as DataFusion functions, the transforms of avenger-datafusion-transforms, and the rewrites that DataFusion needs to plan their queries
- `avenger-sql-language`: Editor features of avenger-sql scripts from one analysis: diagnostics, semantic highlighting, completion, hover, go to definition, references, rename, signature help, and the outline, in the shape of the Language Server Protocol's requests
- `avenger-sql-project`: Projects of avenger-sql: the manifest `avenger.toml`, the file source of a directory or of embedded files in an object store, the roles of files, the entry stylesheet, what the charts of a project share (the label engine, the color scheme's preferences and background, and the cascade), the `{avenger-sql}` cells of Quarto documents, the runs of chart queries through a store of results that later runs reuse, a live project whose loader and analysis stay in step, the watch of its files, and adding data files
- `avenger-cli`: `avenger`, the command-line program of avenger-sql projects: `check`, `fmt`, `tables`, `schema`, `query`, and `charts`, which read a project's data and list its charts, `render`, which draws charts to PNG, SVG, or PDF, `view`, a window on a chart that reloads as its files change, `dap`, a debug adapter of a query's stages, `docs`, the searchable reference of the language, and `complete` and `signature` at a position, with text or JSON output and exit statuses for scripts, CI, and agents, and `lsp`, the Language Server Protocol for editors, of scripts, modules, and the `{avenger-sql}` cells of Quarto documents
- `avenger-style`: The CSS style language of charts: stylesheets, their checks against the vocabulary of the style tree, and property values

## Important Patterns

1. **State Management**: Build an app from one state, an `AppBuild` that draws the app's named scenes, and `Tool`s, such as tooltips, that draw their own; each scene has its own z-order, graph, hit-testing tree, and event streams, and handlers request the scenes to rebuild by name
2. **Event Handling**: Implement `EventStreamHandler<State>` for interactivity
3. **Resource Management**: Canvas holds GPU resources; proper initialization required
4. **Coordinate Systems**: Scene coordinates with configurable viewports
5. **Performance**: Use instanced rendering for marks with many instances

## Dependencies

- **GPU Rendering**: wgpu (cross-platform WebGPU implementation)
- **Geometry**: Lyon (tessellation), rstar (spatial indexing)
- **Text**: avenger-typst-label (Typst-based label typesetting, measuring and rasterizing)
- **Images**: image crate with PNG/JPEG support
- **Data**: Apache Arrow for efficient data handling

## Common Development Tasks

When implementing new features:
1. For new mark types: Add to `SceneMark` enum and implement rendering in `avenger-wgpu`
2. For new scales: Implement `ScaleImpl` trait in `avenger-scales`
3. For interactivity: Attach `EventStreamHandler`s to scenes, or package a reusable interaction as a `Tool`, in `avenger-app`
4. For GPU optimizations: Modify shaders in `avenger-wgpu/src/shaders/`

To write or read avenger-sql, start with `avenger-sql/SPEC.md`, a short guide to the language

To move `avenger-typst-label` to a new upstream Typst release, follow "Following upstream" in
`avenger-typst-label/UPSTREAM.md`
