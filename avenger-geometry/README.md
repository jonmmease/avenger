# avenger-geometry

Geometry processing and spatial indexing for Avenger scene graphs.

## Purpose

This crate provides geometric operations and spatial data structures needed for visualization rendering and interaction. It converts scene graph marks into geometric representations and enables efficient spatial queries for features like hit testing and collision detection.

## Core Architecture

### GeometryInstance
Associates scene graph marks with their geometric representation:
- `mark_instance`: Reference to the original scene mark
- `interactive`: Whether picking and queries can find the instance
- `geometry`: Standard geo-types geometry (Point, LineString, Polygon, etc.)
- `reach`: How far the drawn instance extends beyond `geometry`: half its stroke width, plus a
  circle symbol's radius, so a circle symbol is exact at any size

### MarkGeometryUtils Trait
Converts scene marks into geometry instances:
- Handles all mark types (Arc, Area, Line, Rect, Symbol, Text, etc.)
- Applies coordinate transformations and origin offsets
- Accounts for stroke widths in bounding calculations
- Generates per-instance geometries for marks with multiple elements

### Lyon Path Conversion
The `IntoGeoType` trait converts Lyon paths to geo-types:
- Flattens curves into line segments with configurable tolerance
- Supports both filled (polygon) and unfilled (linestring) modes
- Handles trail marks with variable stroke widths via tessellation
- Manages multi-path geometries and path closure

### Spatial Indexing
Uses R-tree data structures for efficient spatial queries:
- Point-in-geometry testing for hit detection, where clips leave marks visible
- Top-mark picking by draw order, which the tree records for each instance
- Bounding box intersection queries
- Nearest neighbor searches
- Distance-based lookups

### Region Queries
`SceneGraphRTree::query_shape` selects the interactive instances in a rectangle, a circle, or a
polygon such as a lasso, in scene coordinates:
- A hit policy picks what must lie in the region: the centroid, any part of the drawn
  instance, or all of it
- A polygon's fill rule, nonzero or even-odd, decides which areas a self-crossing lasso encloses
- An instance in a clipped group matches only where the clip leaves it visible

### Scene data and focus
`SceneGraphRTree::from_scene_graph_with_data` pairs each mark's instances with its rows in a
`SceneData`, by mark name. Rows for a mark that the scene doesn't draw, such as a layer that a
view leaves out whole, pair with nothing, and it fails when a name names several marks, or the
rows don't number the mark's instances. A line's hit policy, `LineHit::Vertices`, makes each vertex an
instance, so a line's rows are its vertices'.

`SceneGraphRTree::focus` finds the data under the pointer, the `Focus` that a tooltip describes,
with a `FocusQuery` of a pointer and a `FocusStrategy`:
- `Exact`, the topmost instance under the pointer, and `Nearest`, `NearestX`, and `NearestY`,
  the instance nearest the pointer within a radius
- `GroupX`, each mark's instance nearest the pointer's x, such as each series' vertex, with an
  optional limit of the marks nearest the pointer's y
- `among(marks)` limits the search to some marks, and `showing(focus)` keeps a shown member's
  mark in place until another is nearer by more than a pixel

A focus holds only instances whose marks have rows. `Focus::in_scene` finds a focus in a newer
scene, `SceneGraphRTree::find_again` in a newer tree, both by mark name and instance index, and
`Focus::in_data` first moves each member onto its row in newer rows: the same row in the same
rows, or else the row that holds its key.

## Integration

- **Input**: Takes `avenger-scenegraph` scene graphs and converts marks to geometries
- **Dependencies**: Uses `avenger-typst-label` for text measurement in geometry calculations
- **Output**: Provides geometry instances for spatial analysis and `geo-types` compatibility