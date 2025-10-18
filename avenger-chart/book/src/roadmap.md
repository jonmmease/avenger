# Roadmap

Avenger Chart is under active development. This page outlines planned features and improvements to help you understand the project's direction and how you might contribute.

## Feature Status Legend

- ✅ **Available Now** - Feature is implemented and documented
- 🔜 **Planned** - Feature is committed to the roadmap
- 💡 **Under Consideration** - Feature is being explored but not committed

## Rendering Backends

**Priority**: ⭐⭐⭐ High

Expanding rendering options to support different use cases and environments.

### Current

- [x] **GPU-Accelerated PNG** (✅ Available) - WgpuRenderer via WebGPU/wgpu
  - High-quality rasterization
  - Configurable resolution scaling
  - Fast batch rendering
  - See: [Rendering Guide](./guides/rendering.md)

### Planned

- [ ] **SVG Renderer** (🔜 Planned)
  - Vector graphics output
  - Web-friendly inline SVG
  - Scalable to any resolution
  - Editable in vector graphics tools
  - Text remains searchable and selectable

- [ ] **PDF Renderer** (🔜 Planned)
  - Publication-quality documents
  - Multi-page support
  - Embedded fonts
  - Print-ready output
  - Archival quality

- [ ] **CPU-Based PNG Renderer** (🔜 Planned)
  - Software rendering using [tinyskia](https://github.com/RazrFalcon/tiny-skia)
  - No GPU required
  - Server-side rendering in headless environments
  - Docker containers without GPU access
  - Embedded systems

## Interactive Features

**Priority**: ⭐⭐⭐ High

Avenger Chart is built on the [Avenger](https://github.com/jonmmease/avenger) visualization engine, which includes sophisticated event handling and interactive visualization support. These capabilities will be exposed in Avenger Chart.

### Planned

- [ ] **Pan and Zoom** (🔜 Planned)
  - Interactive navigation of visualizations
  - Smooth transitions
  - Constrained pan/zoom regions
  - Reset to default view

- [ ] **Tooltip System** (🔜 Planned)
  - Data-driven tooltips on hover
  - Customizable tooltip content
  - Multi-series tooltips
  - Formatted values

- [ ] **Selection Interactions** (🔜 Planned)
  - Click to select data points
  - Brush selection (rectangular region)
  - Multi-select with modifier keys
  - Selection highlighting
  - Linked selections across multiple views

- [ ] **Event Stream Integration** (🔜 Planned)
  - Reactive event handling system
  - Filter, throttle, and debounce events
  - Event consumption and propagation
  - Custom event handlers
  - Inspired by [Vega Event Streams](https://vega.github.io/vega/docs/event-streams/)

- [ ] **Interactive Parameters** (🔜 Planned)
  - Update parameters without recompilation
  - Smooth transitions between states
  - Integration with UI controls
  - Real-time data updates

### Implementation Notes

Interactive features will leverage:
- **Avenger Core**: Event stream system and canvas abstraction
- **WebAssembly**: Browser deployment with WebGPU/WebGL2 rendering
- **Native Windows**: winit + wgpu for desktop applications

Example from [Avenger](https://github.com/jonmmease/avenger):
```bash
cd examples/iris-pan-zoom
cargo run --release  # Native window with pan/zoom
```

## Additional Features

**Priority**: ⭐⭐ Medium

### Planned

- [ ] **Additional Mark Types** (💡 Under Consideration)
  - As needed for specific visualization types
  - Community-driven additions
  - Maintain consistency with existing marks

- [ ] **Advanced Layout Options** (💡 Under Consideration)
  - Faceted plots (small multiples)
  - Custom subplot arrangements
  - Responsive layouts
  - Grid systems

- [ ] **Animation Support** (💡 Under Consideration)
  - Animated transitions
  - Keyframe animations
  - Animated parameters
  - Frame export for video

- [ ] **WebAssembly Examples** (🔜 Planned)
  - Interactive browser examples
  - WASM deployment guides
  - Integration with JavaScript frameworks
  - Performance optimization tips

## Documentation

**Priority**: ⭐⭐ Medium (Ongoing)

Documentation is continuously improving. See [documentation-gaps-plan.md](https://github.com/jonmmease/avenger/blob/main/avenger-chart/tasks/documentation-gaps-plan.md) for detailed tracking.

### Completed

- [x] Threshold scales (Tier 1)
- [x] Compilation pipeline explanation (Tier 1)
- [x] Expression vs literal distinction (Tier 1)
- [x] DataFusion integration guide (Tier 2)
- [x] Rendering backends guide (Tier 2)

### In Progress

- [ ] The `:x` and `:y` special references (Tier 2)
- [ ] Advanced features documentation (Tier 3)
  - Rect legends, line legends
  - Multi-channel combined legends
  - Symbol/line visual padding
  - Data domain specification
  - Right axis positioning
  - Axis title expressions
  - Axis conditional configuration
  - CSS cardinality ranges
  - Plot serialization

### Quality Improvements

- [ ] Enhanced navigation and cross-references
- [ ] Troubleshooting guide
- [ ] FAQ section
- [ ] API quick reference
- [ ] More real-world examples

## Integration Opportunities

**Priority**: ⭐ Lower (Exploratory)

Potential integrations that could expand Avenger Chart's ecosystem:

### Under Consideration

- **VegaFusion Integration** (💡)
  - Pre-render marks server-side
  - Optimize large datasets
  - Hybrid client-server rendering
  - See: [VegaFusion](https://vegafusion.io/)

- **Vega Native** (💡)
  - Render Vega specifications without JavaScript
  - Combine Avenger + VegaFusion
  - Native Vega implementation in Rust
  - Interactive Vega charts via event streams

- **Matplotlib Backend** (💡)
  - GPU-accelerated rendering for Matplotlib
  - Alternative to Agg backend
  - Leverage existing Matplotlib API
  - See: [Matplotlib Backends](https://matplotlib.org/stable/users/explain/figure/backends.html)

## Contributing

Interested in helping with any of these features? We welcome contributions!

**How to Get Involved:**

1. **Discussions**: Share your ideas and use cases at [GitHub Discussions](https://github.com/jonmmease/avenger/discussions)
2. **Issues**: Report bugs or request features at [GitHub Issues](https://github.com/jonmmease/avenger/issues)
3. **Pull Requests**: Submit code contributions following the [contribution guidelines](https://github.com/jonmmease/avenger/blob/main/CONTRIBUTING.md)

**Areas Where Help is Needed:**

- **Rendering Backends**: Experience with SVG, PDF, or tinyskia
- **Interactive Features**: Event handling, animation systems
- **Documentation**: Examples, tutorials, API documentation
- **Testing**: Visual regression tests, integration tests
- **Performance**: Benchmarking, optimization

## Release Schedule

Avenger Chart follows a flexible release schedule driven by feature completeness rather than fixed timelines. Major features are released when they're thoroughly tested and documented.

**Current Development Focus:**
1. Rendering backends (SVG, PDF, CPU-based PNG)
2. Interactive features (pan, zoom, tooltips)
3. Documentation improvements (Tier 2 & 3 items)

Stay updated:
- Watch the [GitHub repository](https://github.com/jonmmease/avenger)
- Follow release notes
- Join discussions

## Version History

See [CHANGELOG.md](https://github.com/jonmmease/avenger/blob/main/CHANGELOG.md) for detailed version history and release notes.
