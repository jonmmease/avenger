# Codebase Structure

## Workspace Organization
Avenger is organized as a Cargo workspace with multiple crates:

### Core Crates (in dependency order)
1. **avenger-common**: Shared types and utilities
2. **avenger-geometry**: Geometry processing and spatial indexing
3. **avenger-text**: Text measurement and rasterization
4. **avenger-image**: Image loading and processing
5. **avenger-scales**: Data visualization scales (linear, log, ordinal, etc.)
6. **avenger-scenegraph**: Core SceneGraph representation (backend-independent)
7. **avenger-eventstream**: Interactive event handling system
8. **avenger-wgpu**: wgpu-based rendering implementation
9. **avenger-guides**: Visualization guide generation (axes, legends, colorbars)
10. **avenger-vega-scenegraph**: Vega scenegraph compatibility layer
11. **avenger-app**: Application framework for interactive visualizations
12. **avenger-winit-wgpu**: Native window application runner
13. **avenger-chart**: High-level charting API

### Supporting Crates
- **avenger-vega-test-data**: Test data generation (excluded from workspace)
- **avenger-sample-data**: Sample datasets

### Examples
Located in `examples/`:
- `iris-pan-zoom`: Interactive pan/zoom example
- `wgpu-scales`: Scale demonstration
- `wgpu-winit`: Window management example

### Documentation
- Main docs in `docs/`
- Crate-specific docs in `*/README.md`
- Debugging guide at `avenger-chart/docs/DEBUGGING.md`

### Configuration Files
- `Cargo.toml`: Workspace configuration and dependencies
- `pixi.toml`: Python environment and task definitions
- `CLAUDE.md`: AI assistant instructions
- `.github/workflows/rust.yml`: CI/CD configuration