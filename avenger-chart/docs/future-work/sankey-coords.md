# Sankey And Alluvial Diagrams

## Goal Review

The goal is valid: Sankey and alluvial diagrams are important flow
visualizations, but they do not fit the current numeric Cartesian/Polar model
as simple x/y marks.

Framing Sankey as a coordinate system where nodes are guides and flow ribbons
are marks remains plausible, but it is not the only good paradigm. A Sankey
diagram combines data transformation, graph layout, guide rendering, and
custom ribbon geometry.

## Current System Fit

Current architecture supports the pieces in principle:

- Coordinate crates can define custom transforms and guides.
- `PlotGeometry` is object-safe and can be extended with mark-specific
  geometry types.
- Custom marks can render scenegraph paths or groups.
- Built-in legend contracts are independent of coordinate systems.

Missing pieces:

- A ribbon mark or reusable ribbon-path geometry helper for variable-width
  flows. `PathMark` exists, but it does not by itself solve Sankey ribbon
  layout.
- A graph layout stage that can run before render but after data and scale
  planning decisions.
- A clear ownership model for node guides, node labels, and flow anchors.

## Recommended Direction

Treat Sankey as a specialized coordinate/mark family only if the coordinate
system owns the node layout and exposes node rectangles as coordinate guide
geometry. In that model:

- `SankeyCoord` owns stage assignment, node ordering, node sizing, and layout
  algorithm options.
- `SankeyGuide` renders nodes, stage labels, and optional node labels.
- `FlowRibbon` is the main data mark. It consumes source, target, value, and
  visual channels.
- The coordinate measurement computes a reusable flow layout for both guide
  rendering and mark rendering.

This fits the guide/mark separation better than making nodes ordinary marks
and flows guess their anchors independently.

## Alternate Paradigms

- **Transform plus generic path marks**: a Sankey layout transform could output
  ribbon paths, node rectangles, and labels into ordinary Cartesian space. This
  is attractive now that general path, area, and Cartesian text marks exist,
  but it still needs a good story for variable-width ribbons and guide-owned
  node layout.
- **Single compiled Sankey mark under `ZeroDCoord`**: fastest to implement,
  but least integrated with guides, legends, hit testing, and future
  interactions.
- **Graph-layout coordinate system**: most integrated, but it requires new
  coordinate measurement and guide conventions for non-scale topology.

## Readiness

Discovery first.

Before an implementation plan, build or sketch the lowest-level path/ribbon
scenegraph needs and decide whether graph layout is a coordinate measurement
or a data transform. A small static alluvial diagram is the right spike; cyclic
Sankey, animation, and force layout should stay out of v1.

## Decisions Needed

- Whether node layout is a coordinate responsibility or a transform
  responsibility.
- Whether a generic `Ribbon` mark should be implemented before Sankey.
- How duplicate edges are aggregated and ordered.
- How cycles are handled: reject, break into back edges, or support a separate
  circular layout.
- How node labels and flow labels relate to richer [text-mark.md](text-mark.md)
  placement and leader-line behavior.
- How hit testing maps ribbon geometry back to source/target rows.
