# Architecture & Design Patterns

## Layered Architecture

### 1. Scene Graph Layer (avenger-scenegraph)
- Abstract representation of visual elements
- 11 mark types: Arc, Area, Path, Symbol, Line, Rect, Rule, Text, Trail, Image, Group
- Backend-independent representation
- Coordinate systems with configurable viewports

### 2. Rendering Layer (avenger-wgpu)
- GPU-accelerated rendering via wgpu
- Instanced rendering for performance
- Shader implementations in `avenger-wgpu/src/shaders/`
- PngCanvas for headless PNG export
- Supports WebGPU and WebGL2 backends

### 3. Event Handling Layer (avenger-eventstream)
- Reactive event processing
- Sophisticated filtering, throttling, and consumption
- Inspired by Vega's event stream system

### 4. Application Framework (avenger-app)
- High-level orchestration
- State management patterns

## Key Design Patterns

### Trait-Based Extensibility
- `SceneGraphBuilder<State>`: Build scene graphs from application state
- `EventStreamHandler<State>`: Handle events and update state
- `Canvas`: Abstract rendering interface
- `ScaleImpl`: Custom scale implementations

### State Management
Use the `SceneGraphBuilder<State>` pattern for reactive visualizations:
```rust
impl SceneGraphBuilder<MyState> for MyBuilder {
    fn build(&self, state: &MyState) -> SceneGraph {
        // Transform state into scene graph
    }
}
```

### Resource Management
- Canvas holds GPU resources; proper initialization required
- Use Arc/Rc for shared ownership where needed
- Recent refactoring eliminated interior mutability (Arc<Mutex>)

### Performance Patterns
- Instanced rendering for marks with many instances
- Spatial indexing with rstar for hit testing
- Lazy evaluation where possible
- Pre-tessellation of geometry

## Important Implementation Notes

### Coordinate Systems
- Scene coordinates with configurable viewports
- Transform pipeline: data → scale → scene coordinates

### Scale System
- Data transformations with pan/zoom support
- Types: linear, log, ordinal, band, point, time, etc.
- Immutable design for predictable behavior

### Layout System (avenger-chart)
- Frame chrome (margins, titles, legends, guide overflow) solves via the neutral `avenger_layout::Frame` model
- Debug visualization available via AVENGER_CHART_DEBUG_LAYOUT
- Handles plot areas, guides, legends, titles

### Recent Refactoring
- Eliminated interior mutability (Arc<Mutex> removed)
- LayoutInfo replaced with LayoutUpdates
- Facet rendering made deterministic via sorted domain values
