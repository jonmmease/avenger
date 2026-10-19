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
- **Content**: Text, Image, Mesh image (images drawn on triangle meshes, one per instance)
- **Structural**: Group (hierarchical containers)

### SceneGroup
Hierarchical grouping with support for:
- Coordinate transformations and origin offsets
- Clipping regions (rectangular or path-based)
- Nested mark hierarchies
- Gradient definitions
- Fill and stroke styling
- Pattern origins, where group-anchored pattern fills start

### Images
Each instance of an image mark or a mesh image mark draws a `SceneImageSource`: `Inline` pixels,
or a `Resource` that names its image by URL, with a declared width and height and an optional
placeholder image. The declared size places a resource and sets its aspect before its pixels
arrive, and the pixels that arrive draw across that place. A renderer loads resources through an
avenger-image `ImageResolver` as it draws, and draws a resource's placeholder until the resolver
has its pixels, or after they fail to load. A resource without a placeholder draws nothing until
then. `SceneGraph::image_urls` lists a scene's resource URLs, each once, in the scene's order, and
`SceneGraph::resolve_images` resolves them from the images that a resolver holds, without
requesting any, for an export.

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

## Rendering Semantics

The renderers share these rules for drawing a scene, and the `render` module implements them:
`render::order` orders a scene's items, `render::items::path_items` turns marks into the filled and
stroked paths that vector renderers draw, and `render::paint::resolve_paint` resolves their paint.
`render::stroke`, `render::fill`, `render::pattern`, and `render::text` hold the dashes and stroke
outlines, filled areas, pattern geometry, and label shapes that the renderers share.

- Fills and path clips use the nonzero rule, and a pattern stays inside its host's fill.
- An item with a non-finite coordinate draws nothing: an instance of a mark such as a symbol, rect,
  or label, or a whole line, area, trail, or mesh image.
- Strokes use a miter limit of 8, as SVG measures it. Isolated line samples and zero-length dashes
  keep round and square caps, a zero-length dash's square cap follows the path's direction, and
  translucent paint applies once where a stroke's dashes overlap.
- A trail is a variable-width stroke, drawn as its filled outline in the stroke's paint, so
  translucent paint applies once where it overlaps itself.
- Gradient coordinates are in the unit box over an item's bounds: its path's bounds, a symbol's
  nominal square with side `sqrt(size)` whatever its shape and rotation, a trail's centerline
  bounds, or a stroke's undashed centerline bounds. Radial gradients use the square that centers
  on and encloses those bounds, and interpolate between their starting and ending circles.
- A gradient without stops, a radial gradient between identical circles, and a gradient that
  varies over bounds with no area paint nothing. A gradient with one stop, and a linear gradient
  whose ends coincide, paint their last stop's color.
- Images filter in premultiplied alpha, so transparent pixels don't tint their neighbors.
- An image resource that isn't ready draws its placeholder, or nothing without one.

The `avenger-render-fixtures` crate's `parity` module holds the scenes that check these rules in
each renderer.

## Data Representation

Mark properties use `ScalarOrArray<T>` for efficient storage:
- Single values for uniform properties across instances
- Arrays for per-instance variation
- Optional indices for sparse data access

## Integration

- **Input**: `avenger-vega` converts Vega scenegraphs to this representation
- **Output**: `avenger-wgpu` renders scene graphs using GPU acceleration
- **Dependencies**: Uses `avenger-typst-label` for labels and `avenger-image` for image data
