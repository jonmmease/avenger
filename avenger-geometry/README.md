# avenger-geometry

Geometry processing and spatial indexing for Avenger scene graphs.

## Purpose

This crate provides geometric operations and spatial data structures needed for visualization rendering and interaction. It converts scene graph marks into geometric representations and enables efficient spatial queries for features like hit testing and collision detection.

## Core Architecture

### GeometryInstance
Associates scene graph marks with their geometric representation:
- `mark_instance`: Reference to the original scene mark
- `z_index`: Rendering order
- `geometry`: Standard geo-types geometry (Point, LineString, Polygon, etc.)
- `half_stroke_width`: Stroke expansion for accurate bounds

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
- Point-in-geometry testing for hit detection
- Bounding box intersection queries
- Nearest neighbor searches
- Distance-based lookups

## Integration

- **Input**: Takes `avenger-scenegraph` scene graphs and converts marks to geometries
- **Dependencies**: Uses `avenger-text` for text measurement in geometry calculations
- **Output**: Provides geometry instances for spatial analysis and `geo-types` compatibility