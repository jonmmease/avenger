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
- Pattern origins, where group-anchored pattern fills start

### Pattern Fills
Rect, path, symbol, arc, and area marks take a `fill_pattern` that draws over their fill: up to
eight layers of stripes, which can be dashed, and of built-in symbol shapes repeated on a lattice.
Each layer adds to the pattern's coverage, subtracts from it, or flips it with XOR, and the coverage
is painted once, in a solid ink or in black or white to contrast with the fill. A pattern's
`anchor` sets where its stripes and lattices start: the `pattern_origin` of the nearest enclosing
group that sets one, or the scene's corner when no group sets one, each mark item's top-left
corner, or the scene's corner. Marks that share an origin share their pattern's phase.

![Stripes shared across bars, a lattice of dots, and a crosshatch](../docs/images/patterns.png)

Stripes at any angle, wide, dashed, crossed with XOR, and dotted:

![Stripe patterns](../docs/images/pattern-stripes.png)

The built-in symbol shapes, filled and open:

![Symbol patterns](../docs/images/pattern-symbols.png)

A layer of circles added to stripes, subtracted from them, and combined with XOR:

![Layer operations](../docs/images/pattern-operations.png)

Stripes anchored at their group's origin line up across bars, and stripes anchored at each mark
restart in each bar. Contrasting ink is black on light fills and white on dark ones:

![Pattern anchors and contrasting ink](../docs/images/pattern-anchors.png)

`cargo run --release -p avenger-wgpu --example patterns docs/images` renders these images.

## Data Representation

Mark properties use `ScalarOrArray<T>` for efficient storage:
- Single values for uniform properties across instances
- Arrays for per-instance variation
- Optional indices for sparse data access

## Integration

- **Input**: `avenger-vega` converts Vega scenegraphs to this representation
- **Output**: `avenger-wgpu` renders scene graphs using GPU acceleration
- **Dependencies**: Uses `avenger-typst-label` for labels and `avenger-image` for image data
