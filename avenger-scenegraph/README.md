# avenger-scenegraph

Scene graph data structure for 2D visualization rendering.

## Purpose

This crate defines the core intermediate representation used by the Avenger rendering system. It provides a hierarchical scene graph that bridges the gap between high-level visualization specifications (like Vega) and low-level rendering backends.

## Core Architecture

### SceneGraph
The root container holding:
- A collection of top-level marks
- Canvas dimensions (width, height)
- Origin coordinates for positioning

### SceneMark Types
Represents different visual primitives:
- **Geometric**: Arc, Area, Path, Line, Trail, Rect, Rule, Symbol
- **Content**: Text, Image  
- **Structural**: Group (hierarchical containers)

### SceneGroup
Hierarchical grouping with support for:
- Coordinate transformations and origin offsets
- Clipping regions (rectangular or path-based)
- Nested mark hierarchies
- Gradient definitions
- Fill and stroke styling

## Data Representation

Mark properties use `ScalarOrArray<T>` for efficient storage:
- Single values for uniform properties across instances
- Arrays for per-instance variation
- Optional indices for sparse data access

## Integration

- **Input**: `avenger-vega` converts Vega scenegraphs to this representation
- **Output**: `avenger-wgpu` renders scene graphs using GPU acceleration
- **Dependencies**: Uses `avenger-text` for typography and `avenger-image` for image data
