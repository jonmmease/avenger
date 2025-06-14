# Avenger Architecture Overview

## Introduction

Avenger is a modular Rust visualization framework that provides a flexible architecture for building interactive data visualizations. The framework is built around a scene graph abstraction with multiple rendering backends and a sophisticated event handling system.

## Core Architecture Principles

### 1. **Layered Architecture**
The codebase follows a clean layered architecture with clear separation of concerns:

- **Scene Graph Layer** (`avenger-scenegraph`): Abstract representation of visual elements
- **Rendering Layer** (`avenger-wgpu`): GPU-accelerated rendering implementation
- **Event Handling Layer** (`avenger-eventstream`): Reactive event system
- **Application Layer** (`avenger-app`): High-level application framework
- **Integration Layer** (`avenger-vega-scenegraph`): Vega/Vega-Lite compatibility

### 2. **Trait-Based Design**
The architecture heavily leverages Rust's trait system for extensibility:

- `SceneGraphBuilder<State>`: Trait for building scene graphs from application state
- `EventStreamHandler<State>`: Trait for handling events and updating state
- `Canvas`: Core trait for rendering backends
- `ScaleImpl`: Trait for implementing different scale types
- `TextMeasurer` and `TextRasterizer`: Traits for text handling across platforms

### 3. **Separation of Concerns**
Each crate has a focused responsibility:

- **avenger-common**: Shared types and utilities
- **avenger-geometry**: Geometric operations and spatial indexing
- **avenger-scales**: Data transformation and scaling
- **avenger-text**: Text measurement and rasterization
- **avenger-image**: Image loading and processing
- **avenger-guides**: Axes and legends generation

## Key Components

### Scene Graph (`avenger-scenegraph`)

The scene graph is the core data structure representing the visual hierarchy:

```rust
pub struct SceneGraph {
    pub marks: Vec<SceneMark>,
    pub width: f32,
    pub height: f32,
    pub origin: [f32; 2],
}
```

**Mark Types**:
- Arc, Area, Path, Symbol, Line, Trail, Rect, Rule, Text, Image, Group

Each mark type has its own properties and can be nested within groups for hierarchical organization.

### Rendering System (`avenger-wgpu`)

The rendering system uses WebGPU for cross-platform GPU acceleration:

**Canvas Trait**:
- Provides abstraction over different rendering targets (Window, PNG, etc.)
- Manages mark renderers and handles scene updates
- Supports both instanced rendering (for performance) and multi-mark rendering

**Rendering Pipeline**:
1. Scene graph is converted to render commands
2. Marks are sorted by z-index
3. Instanced marks (>100 instances) use specialized shaders
4. Multi-mark renderer handles complex shapes and smaller collections

### Event System (`avenger-eventstream`)

Sophisticated event handling with stream-based architecture:

**Event Types**:
- Mouse events (click, double-click, move, enter, leave, wheel)
- Keyboard events (key press/release)
- Window events (resize, move, focus)
- File change events (for hot reloading)

**Event Stream Features**:
- Event filtering by type, mark path, or custom predicates
- Throttling and debouncing support
- Event consumption (stop propagation)
- Between-stream event handling (e.g., drag = between mousedown and mouseup)

### Application Framework (`avenger-app`)

High-level framework that ties everything together:

```rust
pub struct AvengerApp<State> {
    scene_graph_builder: Arc<dyn SceneGraphBuilder<State>>,
    event_stream_manager: EventStreamManager<State>,
    rtree: SceneGraphRTree,
    scene_graph: Arc<SceneGraph>,
}
```

**Application Flow**:
1. Initialize with state and event handlers
2. Build initial scene graph
3. Process events through registered handlers
4. Update state based on event handling
5. Rebuild scene graph if needed
6. Update spatial index (R-tree) for hit testing

### Scales System (`avenger-scales`)

Flexible data transformation system:

**Scale Types**:
- Linear, Log, Power, Symlog (continuous numeric)
- Band, Point (discrete positioning)
- Ordinal (discrete mapping)
- Quantile, Quantize, Threshold (binning)

**Scale Features**:
- Pan and zoom support with adjustments
- Color interpolation
- Custom formatters
- Invertibility for interaction

## Architectural Patterns

### 1. **Builder Pattern**
Used extensively for constructing complex objects:
- `ConfiguredScale` builder for scale configuration
- Scene graph builders for declarative visualization construction

### 2. **Strategy Pattern**
Different implementations can be plugged in:
- Text measurement strategies (Cosmic Text, HTML Canvas)
- Color coercion strategies
- Scale implementations

### 3. **Command Pattern**
Events are treated as commands that can be processed asynchronously:
- Event handlers return `UpdateStatus` indicating what needs updating
- Decouples event source from handling logic

### 4. **Spatial Indexing**
R-tree spatial index for efficient hit testing:
- Built from scene graph geometry
- Enables fast point-in-shape queries for interaction

### 5. **Resource Management**
Careful handling of GPU resources:
- Proper drop order for WGPU resources
- Shared ownership with `Arc` for expensive objects
- Lazy initialization of text atlases and textures

## Platform Considerations

### Native vs WebAssembly
The architecture supports both native and WASM targets:

**Native**:
- Uses Cosmic Text for text rendering
- File I/O for data loading
- Full WebGPU feature set

**WASM**:
- Uses HTML Canvas for text measurement
- Embedded resources
- WebGL-compatible subset of WebGPU

### Cross-Platform Abstractions
- `cfg_if!` macros for conditional compilation
- Trait-based abstractions for platform-specific functionality
- Consistent API across platforms

## Performance Optimizations

### 1. **Instanced Rendering**
Marks with many instances (>100) use instanced rendering:
- Single draw call for all instances
- GPU-based transformations
- Significant performance improvement for scatter plots

### 2. **Incremental Updates**
The system supports incremental updates:
- `LinearScaleAdjustment` for smooth pan/zoom
- Geometry caching when possible
- Selective scene graph rebuilding

### 3. **Lazy Evaluation**
Resources are created on-demand:
- Text atlases built as needed
- Render pipelines cached and reused
- Spatial index updated only when geometry changes

## Extension Points

### Adding New Mark Types
1. Define the mark structure in `avenger-scenegraph`
2. Implement geometry generation in `avenger-geometry`
3. Add rendering logic in `avenger-wgpu`
4. Update the Canvas trait with the new mark method

### Adding New Scale Types
1. Implement the `ScaleImpl` trait
2. Define domain inference method
3. Implement forward/inverse transformations
4. Add any special operations (pan/zoom)

### Custom Event Handlers
1. Implement `EventStreamHandler<State>` trait
2. Define event filtering configuration
3. Register with the event stream manager
4. Handle state updates in the handler

## Best Practices

### State Management
- Keep application state separate from rendering state
- Use immutable updates where possible
- Leverage Rust's ownership for safe concurrent access

### Error Handling
- Use `Result<T, E>` throughout the API
- Define specific error types per crate
- Propagate errors up to the application level

### Testing
- Unit tests for individual components
- Integration tests for mark rendering
- Visual regression tests using image comparison

### Documentation
- Document trait requirements clearly
- Provide examples for common use cases
- Keep architectural decisions documented

## Future Considerations

### Potential Improvements
1. **Declarative API**: Higher-level declarative API for visualization specification
2. **Animation System**: Built-in support for transitions and animations
3. **Layout Engine**: Automatic layout algorithms for complex visualizations
4. **Additional Backends**: Support for other rendering targets (SVG, Canvas 2D)
5. **Performance Monitoring**: Built-in profiling and performance tracking

### Scalability
The architecture is designed to scale:
- Modular design allows independent evolution of components
- Trait-based design enables easy extension
- Clear boundaries facilitate parallel development

This architecture provides a solid foundation for building high-performance, interactive data visualizations while maintaining flexibility and extensibility.