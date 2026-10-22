# avenger-geometry

Geometry processing and spatial indexing for Avenger scene graphs.

## Purpose

This crate provides geometric operations and spatial data structures needed for visualization rendering and interaction. It converts scene graph marks into geometric representations and enables efficient spatial queries for features like hit testing and collision detection.

## Core Architecture

### GeometryInstance
Associates scene graph marks with their geometric representation:
- `mark_instance`: Reference to the original scene mark
- `geometry`: Standard geo-types geometry (Point, LineString, Polygon, etc.)
- `radius`: How far the fill extends beyond `geometry`: a circle symbol's radius, so a circle
  symbol is exact at any size, and 0 otherwise
- `margin`: How far the drawn instance extends beyond its fill: half its stroke width, or a text
  label's 1 px hit margin
- `shape(extent, clip)`: The instance's shape at an extent that its clip leaves visible

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
`SceneGraphRTree` holds the interactive instances of a scene in an R-tree, in the order the scene
draws them, with each mark's clip. It finds an instance by its mark path and instance index with
`instance`, and gives a mark's clip with `clip`.

### Rows
`SceneData` holds the rows behind a scene's marks, as an Arrow `RecordBatch` for each mark name,
whose rows number the mark's instances, or a line's vertices.
`SceneGraphRTree::from_scene_graph_with_data` builds a tree that keeps its scene's rows, and
`datum` gives an instance's `Datum`: its mark's rows and its row in them. A tree's rows come from
the build that made the tree, so they always match its instances. Building the tree fails when
several marks share a name that has rows, or when a mark's rows don't number its instances. Rows
for a name that the scene draws no mark of pair with nothing, since a scene can leave a mark out.

### Queries
`SceneGraphRTree::query` finds the instances that stand in a DE-9IM relation to a query shape, in
scene coordinates, topmost first. A query shape is a point, a rect, a circle, closed contours with
a fill rule, nonzero or even-odd, such as a lasso that crosses itself, any `geo` geometry, or an
instance's own shape.

Each relation compares a candidate's shape A with the query shape Q, so `Within` finds the
instances inside the query shape and `Contains` the instances that hold it:

| Relation | DE-9IM pattern | A and Q |
|---|---|---|
| `Equals` | `T*F**FFF*` | are the same point set |
| `Disjoint` | `FF*FF****` | share no point |
| `Intersects` | not `Disjoint` | share a point, so shapes that only touch intersect |
| `Touches` | `FT*******`, `F**T*****`, or `F***T****` | share boundary points, and no interior point |
| `Crosses` | `T*T******`, `T*****T**`, or `0********` | meet inside in a lower dimension, such as a line through an area |
| `Within` | `T*F**F***` | A lies inside Q, and their interiors meet |
| `Contains` | `T*****FF*` | Q lies inside A, and their interiors meet |
| `Overlaps` | `T*T***T**` or `1*T***T**` | have one dimension, meet inside, and each has a part outside the other |
| `Covers` | `T*****FF*` and three more | no point of Q lies outside A |
| `CoveredBy` | `T*F**F***` and three more | no point of A lies outside Q |
| `ContainsProperly` | `T**FF*FF*` | Q lies in A's interior, clear of its boundary |
| `InteriorsIntersect` | `T********` | interiors meet, with one inside the other or not |
| `Pattern` | any | match a pattern of nine cells from `T`, `F`, `*`, `0`, `1`, and `2` |

An extent selects the shape of each instance that a relation sees:
- `Drawn`, the default: the geometry grown by its radius and margin, so a thin line or an outline
  takes hits
- `Fill`: the area that the geometry encloses, so neighbors whose strokes overlap only touch.
  Lines, rules, and line vertices fill nothing.
- `Geometry`: the point, line, or polygon as the tree stores it, for relations of points and
  lines, such as `Crosses`
- `Centroid`: the centroid of the geometry, to select pie slices by their centers

Relations are exact, with drawn circles and strokes within 0.01 px of their round edges. A query
that wants slop around the pointer uses a circle as its shape, or `nearest` with a largest
distance.

A query can filter its candidates by mark and instance before any exact geometry, and limit its
results to the topmost matches. A mark's clip hides the instances that it shows none of, so a lasso
drawn past a plot's edge doesn't select points clipped out of view, and a query at a point that the
clip hides, such as the pointer over an axis beside the plot, finds none of the mark's instances.
An instance that the clip shows in part counts by its whole shape.

`SceneGraphRTree::nearest` yields the instances within a distance of a point, which can be
infinite, nearest first, by the distance to each instance's shape at an extent, with each
`Neighbor`'s distance. Its metric measures in the plane, or along x or y alone, so a band query
finds every line that spans the pointer's x at distance 0. Equal distances go nearest in the plane
first, then topmost first.

## Integration

- **Input**: Takes `avenger-scenegraph` scene graphs and converts marks to geometries
- **Dependencies**: Uses `avenger-typst-label` for text measurement in geometry calculations
- **Output**: Provides geometry instances for spatial analysis and `geo-types` compatibility