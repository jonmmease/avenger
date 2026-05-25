# Avenger Chart - Future Work

This directory contains speculative designs for features that are not part of
the current architecture reference. Use `avenger-chart/docs/architecture/` for
the current system design.

## Feature Documents

**[Adjust API](adjust-api.md)** - Post-scale position adjustments
- Modify mark positions after scales have been applied
- Enables dodge positioning, jitter for overplotting, and smart label placement
- Operating in visual/pixel space

**[Derive API](derive-api.md)** - Child mark generation
- Generate child marks from parent marks' scaled data
- Enables automatic label placement, error bars, connectors, and annotations

**[Transform System](transform-system.md)** - Data transformations
- Pre-scale data transformations that are coordinate-system aware
- Includes binning, grouping, and stacking operations

**[Controllers](controllers.md)** - Interactive behaviors
- Manage interactive behaviors like pan/zoom, selection, and brushing
- Controller abstraction with state management

**[Text Mark](text-mark.md)** - Text rendering
- Render text labels on visualizations
- Needed for annotations, labels, and titles
- Smart label placement algorithms

**[Faceting System](faceting.md)** - Small multiples
- Data-driven subplot replication
- Group data by column values and render filtered subplots
- Manual (Facet) and automatic (FacetRow, FacetColumn, FacetWrap, FacetGrid) layouts
- Scale sharing and axis display options

**[Repeat System](repeat.md)** - Schema-driven replication
- Iterate over variable names, not data groups
- Show same visualization for different columns
- Manual (Repeat) and automatic (RepeatRow, RepeatColumn, RepeatWrap, RepeatGrid) layouts
- SPLOM (scatter plot matrix) support

**[Layout System](layout.md)** - Dashboard composition
- Compose independent visualizations
- GridLayout for CSS Grid-like positioning
- FlexH/FlexVLayout for flexbox arrangements
- Recursive composition support

**[Multi-Dimensional Coordinates](multi-dim-coords.md)** - Parallel and radar charts
- Repeated position channels for variable dimensions
- Parallel coordinates and advanced radar charts
- One-to-many mapping: data record → polyline/polygon

**[Sankey Coordinate System](sankey-coords.md)** - Flow diagrams
- Topological coordinate space with node guides
- Flow ribbons between nodes
- Alluvial diagrams and circular sankey

**[Hierarchical Coordinates](hierarchical-coords.md)** - Nested visualizations
- Treemap and sunburst using `.level()` pattern
- Parent containers as guides, leaves as marks
- Dynamic hierarchy depth and interactive drilling

## Use

The files in this directory are design sketches. They are not canonical
descriptions of the current runtime, and they should not be cited as
architecture references.
