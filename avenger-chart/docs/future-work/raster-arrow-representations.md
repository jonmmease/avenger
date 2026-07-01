# Raster Arrow Representations

## Status

Historical design spike. The current implemented uniform raster representation
uses the DataArray-style `geometry.dimensions` plus `values.dims` schema from
the `UniformRaster2D` implementation and
[rasterize-uniform-2d-udaf.md](rasterize-uniform-2d-udaf.md). Older examples in
this note that mention fixed `geometry.columns` / `geometry.rows` fields are
superseded and should not be used as implementation guidance.

This note still captures useful future-work context for non-uniform and
curvilinear rasters, but it does not specify current renderer implementation,
legend behavior, or View integration.

## Goal

Define Arrow/DataFusion-compatible raster structs that can be produced by
transforms, stored as ordinary dataframe columns, inspected with normal
DataFusion expressions, and consumed by raster marks.

The first design target is a two-dimensional raster that Avenger can colorize
into an image or mesh. Numeric value planes use Avenger color scales;
direct-color value planes use `no_scale()`. Existing already-colorized image
payloads should keep using the existing image mark/resource path.

## Core Decisions

- A mark still receives a dataframe.
- Each dataframe row may contain one raster-valued struct column.
- Compact raster marks render one raster per input row.
- Non-raster columns remain ordinary dataframe columns and may drive facets,
  filtering, ordering, opacity, and other non-geometry mark behavior.
- A grouped or partitioned `Rasterize2D` transform returns one row per output
  raster, plus ordinary grouping columns.
- All raster structs have top-level `geometry` and `values` fields.
- `values` has the same schema contract across geometry kinds.
- `geometry` varies by kind.
- `geometry.kind` is a string discriminator for metadata and validation.
- `geometry.kind` is not an Arrow `UnionArray`; each raster column has one
  concrete geometry schema.
- `geometry.coordinate_space` is optional PROJ-compatible CRS metadata.
- Coordinate names are metadata. They do not automatically route to mark
  channels.
- Coordinate-system-specific raster marks may validate or interpret coordinate
  names, but that interpretation belongs to the mark/coordinate system, not to
  the generic raster struct.
- Public raster marks should be geometry-specific. Do not start with a generic
  `Raster2D` mark that only supports uniform rasters.
- Uniform rasters in an affine cartesian coordinate path can compile to the
  existing image scene mark after color scaling.
- Non-uniform rasters should compile to dedicated low-level scene marks instead
  of being forced through image rendering.
- Scene marks should receive already-colorized image/cell payloads. They should
  not own Avenger scale logic.

Conceptually:

```text
raster: Struct<
  geometry: Struct<kind: Utf8, ...>,
  values: Struct<data: List<TCell>, ...>,
>
```

The authoring API should know which concrete schema it expects from the mark
or transform type:

```rust
UniformRaster2D::new()
    .raster(col("raster"));
```

and then validate:

```text
raster.geometry.kind == "uniform"
```

## DataFusion Field Access

Struct fields remain accessible through normal DataFusion expressions. The
preferred Rust shape is the extension-trait API:

```rust
use datafusion::prelude::*;
use datafusion_functions::core::expr_ext::FieldAccessor;

let data = col("raster").field("values").field("data");
let column_count = col("raster")
    .field("geometry")
    .field("columns")
    .field("count");
```

Do not use `col("raster.values.data")` for nested field access. Dotted strings
are column qualification syntax, not struct traversal.

Public raster marks do not need typed input wrappers because dedicated mark
types provide the schema context. Internal helpers should hide most of these
expression paths:

```rust
let raster = UniformRaster2DFields::new(col("raster"));

raster.values_data()
raster.columns_count()
```

## Shared Values Struct

All raster geometry kinds use the same value-plane contract.

Arrow pseudo-schema:

```text
values: Struct<
  data: List<TCell>,
  null_count: UInt32?,    // optional summary
  non_finite_count: UInt32?, // optional summary for float value planes
>
```

`TCell` depends on the fill channel mode:

- For scaled raster values, `TCell` is any Arrow scalar type supported by the
  configured Avenger color channel. The v1 required support is numeric scalar
  values.
- For direct-color rasters, `TCell` is any Arrow color value type supported by
  Avenger color coercion, and users select this mode with
  `raster_with(..., |r| r.fill(|f| f.no_scale()))`.

Numeric `TCell` values should be any Arrow numeric value type that can be cast
through Arrow/DataFusion into Avenger's numeric scale working type. Do not
require `Float64` input.

Required scaled numeric v1 support:

```text
Int8, Int16, Int32, Int64
UInt8, UInt16, UInt32, UInt64
Float32, Float64
```

`Float16` and decimal value planes may be accepted if Arrow casting to the
scale working type succeeds, but v1 should not depend on them unless explicit
tests are added.

Required direct-color v1 support should match ordinary no-scale color channels:

```text
Utf8/LargeUtf8 CSS color strings
Dictionary-encoded CSS color strings
List<Float32 or castable numeric> with length 4, interpreted as normalized RGBA
FixedSizeList<Float32 or castable numeric, 4>, interpreted as normalized RGBA
```

Direct-color rasters are still raster structs, not existing image marks. They
carry grid geometry plus one color value per cell, and the mark still handles
placement, null masks, facets, opacity, and future mesh rendering.

Semantics:

- `data` stores one scalar per cell unless a future values association says
  otherwise.
- Cell values are row-major:

  ```text
  index = row * column_count + column
  ```

- Row/column order is a storage contract, not a permanent x/y binding. A raster
  mark may provide an explicit `transpose()` option that swaps how rows and
  columns bind to plot coordinates. That option must not mutate the raster
  struct or change `values.data`.
- For scaled fill channels, fill/color domain inference is over `values.data`
  using generic ListArray domain inference. This avoids duplicated summary
  fields that can become inconsistent with the value plane.
- For no-scale fill channels, `values.data` is interpreted as direct color data
  and does not contribute to scale-domain inference or colorbar legends.
- For floating value planes, `NaN`, `+inf`, and `-inf` are non-finite cells, not
  null cells.
- If no finite, non-null cells exist, inferred numeric fill/color domains should
  follow the same empty-domain behavior as other numeric channels.
- `null_count` is optional. Arrow validity is authoritative; the summary only
  avoids scanning when useful.
- `non_finite_count` is optional. It is a summary for diagnostics and fast
  validation; finite checks on `values.data` are authoritative.

For v1, `values.data` is cell-associated:

```text
len(values.data) = row_count * column_count
```

A future smooth-interpolated mesh could add:

```text
association: Utf8   // "cell" | "vertex"
```

but this is not part of the initial representation.

## Shared Coordinate Space Metadata

All raster geometry kinds may carry coordinate-space metadata that describes
the coordinate reference system in which the grid geometry is expressed. Align
this metadata with PROJ rather than inventing Avenger-specific CRS names.

Arrow pseudo-schema:

```text
coordinate_space: Struct<
  authority: Utf8?,  // e.g. "EPSG", "OGC", "ESRI"
  code: Utf8?,       // e.g. "3857", "4326", "CRS84"
  name: Utf8?,       // optional human-readable label
  wkt: Utf8?,        // optional CRS WKT, preferably WKT2
  projjson: Utf8?,   // optional PROJJSON document
  axis_order: List<Utf8>?, // e.g. ["x", "y"], ["lon", "lat"]
>
```

Axis sampling is stored on the geometry axis fields, not in
`coordinate_space`. Use a simple string vocabulary:

```text
sampling: Utf8?  // "linear", "log10", "ln", "sqrt", ...
```

Do not add a structured transform/base object unless Avenger adopts or depends
on a standard one. Strings like `log10` are clearer than `{ transform: "log",
base: 10 }` for this metadata layer.

Semantics:

- `authority` + `code` is the preferred compact identifier when available.
- `wkt` and `projjson` are optional full CRS definitions for custom or
  authority-less coordinate spaces.
- `projjson` should be treated as PROJJSON, not an arbitrary JSON blob.
- Do not add a legacy PROJ string field as the canonical CRS representation.
  PROJ strings may be useful for user input, but they can be lossy compared
  with authority identifiers, WKT, or PROJJSON.
- `axis_order` is Avenger-facing metadata that records how the raster geometry
  fields map onto the CRS axes. This is important for geographic CRSs where
  formal axis order and plotting convention may differ.
- Do not include a separate units field in v1. Avenger does not have a general
  unit system yet; CRS unit details should stay inside authority definitions,
  WKT, or PROJJSON until there is a concrete compatibility model.
- Missing `coordinate_space` means the consuming mark/coordinate system owns
  interpretation.

Recommended compact identifiers:

```text
authority = "EPSG", code = "3857"   // Web Mercator projected meters
authority = "EPSG", code = "4326"   // formal EPSG geographic CRS
authority = "OGC",  code = "CRS84"  // longitude, latitude order
```

For longitude/latitude rasters, prefer `OGC:CRS84` or provide explicit
`axis_order`. This avoids silently treating a lon/lat data convention as formal
EPSG:4326 axis order.

## Coordinate-System-Specific Axes

The generic raster struct should not hard-code cartesian `x`/`y` semantics.
Uniform and rectilinear rasters have `columns` and `rows`; those axes carry
`coord` strings that describe what coordinate each axis represents. A cartesian
mark can map columns to x and rows to y by mark semantics. A polar mark can map
columns to theta and rows to radius by its own semantics.

Do not add a separate geometry kind just because the axes are polar. A regular
theta/radius grid is still uniform or rectilinear in its own coordinate space:

```text
raster: Struct<
  geometry: Struct<
    kind: Utf8,                 // "uniform"
    coordinate_space: Struct<...>?,
    columns: Struct<
      coord: Utf8,              // "theta"
      sampling: Utf8?,          // usually "linear"
      start: Float64,           // e.g. 0.0
      stop: Float64,            // e.g. 2 * pi
      count: UInt32,
    >,
    rows: Struct<
      coord: Utf8,              // "r"
      sampling: Utf8?,          // "linear", "log10", ...
      start: Float64,
      stop: Float64,
      count: UInt32,
    >,
  >,
  values: Struct<data: List<TCell>, ...>,
>
```

Example polar uniform grid:

```text
geometry.kind = "uniform"
geometry.columns.coord = "theta"
geometry.columns.sampling = "linear"
geometry.columns.start = 0.0
geometry.columns.stop = 6.283185307179586
geometry.columns.count = 360

geometry.rows.coord = "r"
geometry.rows.sampling = "linear"
geometry.rows.start = 0.0
geometry.rows.stop = 1.0
geometry.rows.count = 100
```

Example log-sampled radial grid:

```text
geometry.kind = "uniform"
geometry.columns.coord = "theta"
geometry.columns.start = 0.0
geometry.columns.stop = 6.283185307179586
geometry.columns.count = 360

geometry.rows.coord = "r"
geometry.rows.sampling = "log10"
geometry.rows.start = 0.0     // sampled coordinate for source r = 1
geometry.rows.stop = 3.0      // sampled coordinate for source r = 1000
geometry.rows.count = 100
```

`coordinate_space` remains CRS metadata, not a general "cartesian vs polar"
flag. Most polar rasters should omit `coordinate_space`; the `PolarRaster2D`
mark or polar coordinate system owns the interpretation of `theta` and `r`.
If a polar raster is derived from a projected or geographic source, that source
CRS should be tracked by the transform that produced the polar coordinates, not
by pretending the polar axes themselves are a PROJ CRS.

Validation for a polar-specific mark should be mark-owned:

```text
PolarRaster2D requires one theta axis and one radial axis.
Default convention: columns.coord == "theta", rows.coord == "r".
Alternative axis order should be explicit in the mark API, not inferred by a
global coord-name router.
```

## Coordinate And Scale Compatibility

Direct raster rendering should be conservative. A raster mark may render a
raster directly only when the raster geometry is already expressed in the
coordinate space consumed by the mark and the coordinate system can preserve
the raster geometry kind. When metadata does not line up, v1 should return a
clear error instead of silently projecting, resampling, or changing geometry
kind.

Required direct-render checks:

- The mark supports the raster's concrete `geometry.kind`.
- The mark can assign `columns` and `rows` to its coordinate roles.
- `coordinate_space`, when present, is compatible with the coordinate system.
- `sampling` is compatible with the position scale transform for each raster
  axis.
- The coordinate system can render the geometry kind without changing its
  topology.
- Discrete position scales such as band, point, ordinal, threshold, quantize,
  and quantile are unsupported for compact raster geometry.

The future path for mismatches is an explicit transform:

```text
source raster -> project/resample/mesh transform -> compatible raster -> mark
```

Do not make the chart mark perform implicit CRS projection, nonlinear
resampling, antimeridian splitting, or polar-to-cartesian conversion.

### Cartesian Compatibility

For ordinary cartesian plots, `coordinate_space` is usually absent. The raster
is interpreted as ordinary x/y data-space geometry, and the mark owns the axis
role mapping:

```text
UniformRaster2D<Cartesian>
  defaults to columns -> x and rows -> y
  may explicitly transpose to rows -> x and columns -> y
```

When `coordinate_space` is present, a Cartesian raster mark should treat it as
descriptive metadata for the numeric x/y coordinates, not as an instruction to
project. For example, a longitude/latitude raster with `OGC:CRS84` can be drawn
on a Cartesian plot as degrees if the x/y scales are degree-like linear scales.
It is not automatically converted to Web Mercator.

For v1, CRS-backed rasters should use missing/linear axis sampling. Nonlinear
sampling such as `log10` is for non-geo Cartesian axes unless Avenger later
adds a standard way to describe transformed CRS axes.

Uniform raster image rendering is valid only when each axis is uniformly spaced
in the coordinate space that the position scale maps affinely to pixels:

```text
axis sampling    compatible position scale    inverse for scale input
-------------    -------------------------    -----------------------
missing/linear   Linear                       identity
log10            Log with base 10             pow10
ln               Log with base e              exp
sqrt             Sqrt / Pow with exponent 0.5 square
```

For nonlinear compatible pairs, `start`, `stop`, and rectilinear `edges` remain
stored in sampled coordinates. The mark must apply the known inverse sampling
function when contributing coordinate domains and when passing extent/edge
coordinates through the position scale. Example: a log10-sampled raster over
source x values `1..1000` stores `columns.start = 0` and `columns.stop = 3`;
with a base-10 log x scale, it contributes x domain `1..1000` and places the
image corners at `scale(1)` and `scale(1000)`.

If a uniform raster has `sampling = "linear"` and the plot uses a log x scale,
the cells are not uniformly spaced in screen space. The mark should error
rather than stretch a rectangular image across the log-scaled extent. A future
transform can convert the raster to `RectilinearRaster2D`, `QuadMesh2D`, or a
view-dependent resampled `UniformRaster2D`.

Other nonlinear scales, including arbitrary `Pow` exponents and `Symlog`,
should error until Avenger has an explicit sampling/scale compatibility table
and inverse mapping for those strings.

Rectilinear rasters are more permissive because edge coordinates are explicit.
For a Cartesian `RectilinearRaster2D`, a compact rectilinear scene mark can
transform every row and column edge through a monotone continuous x/y scale and
preserve axis-aligned cells. Non-monotone or discrete scales should error.

QuadMesh and CellMesh rasters are the general Cartesian fallback because every
cell boundary is explicit. They can be rendered by transforming vertices
through compatible continuous x/y scales. They are still not a substitute for
implicit CRS projection; if the coordinates are in the wrong CRS, use an
explicit projection transform first.

### WebMercator Compatibility

The current WebMercator coordinate system consumes projected Web Mercator x/y
meters internally. Its coordinate domains require linear numeric x and y
scales. Direct raster support should therefore require projected metadata:

```text
geometry.coordinate_space.authority = "EPSG"
geometry.coordinate_space.code = "3857"
geometry.coordinate_space.axis_order = ["x", "y"]   // optional but preferred

geometry.columns.coord = "x" or "webmercator_x"
geometry.rows.coord = "y" or "webmercator_y"
geometry.columns.sampling = "linear" or null
geometry.rows.sampling = "linear" or null
```

A uniform EPSG:3857 raster can render directly as an image in WebMercator
because projected x/y meters map linearly to screen for a fixed viewport. This
is the natural output shape for a WebMercator `View`/Datashader-style pipeline:
project lon/lat points to EPSG:3857, aggregate in the current projected view
domain, and emit a uniform projected raster.

Longitude/latitude rasters should not render directly in WebMercator:

```text
OGC:CRS84 uniform lon/lat raster -> error for direct WebMercator rendering
EPSG:4326 uniform lat/lon raster -> error for direct WebMercator rendering
```

Future transforms can support these cases by converting geometry first:

```text
uniform lon/lat raster
  -> project latitude/longitude edges
  -> rectilinear or quadmesh EPSG:3857 raster

uniform lon/lat raster
  -> resample in current WebMercator view
  -> uniform EPSG:3857 raster
```

Rasters that cross the antimeridian, exceed the Web Mercator latitude clamp, or
use a non-EPSG:3857 projected CRS should also error in direct rendering until a
projection/splitting transform exists.

### Polar Compatibility

Polar raster metadata can line up with a polar mark when the raster has one
theta axis and one radial axis. Even then, a `uniform` theta/r raster is not
image-renderable after the polar transform. `UniformRaster2D` on a polar
coordinate system should error unless it explicitly routes through a native
polar raster renderer or a generated mesh path.

Direct polar support should live on a coordinate-aware mark such as
`PolarRaster2D`, which can compile to `ScenePolarRasterMark` or
`SceneQuadMeshMark`. A future transform may also convert a theta/r raster to a
cartesian `QuadMesh2D` or resample it to a cartesian `UniformRaster2D`.

## Null And Non-Finite Color Handling

Raster marks should distinguish Arrow null cells from non-finite floating-point
cells. These cases often have different meanings:

```text
null      -> missing/no observation/masked cell
NaN       -> invalid computed value
+inf/-inf -> overflow or unbounded computed value
```

All typed raster marks should expose separate mark-level color configuration:

```rust
UniformRaster2D::new()
    .raster(col("raster"))
    .null_color(Color::TRANSPARENT)
    .non_finite_color(Color::TRANSPARENT)
```

The exact color type should match existing Avenger constant color APIs. These
settings are constant mark configuration, not data channels. They are applied
after extracting `values.data` and before building the RGBA image or cell color
buffer.

Suggested defaults:

```text
null_color = transparent
non_finite_color = transparent
```

The defaults may be the same, but the configuration knobs must remain separate.
Diagnostic styles can then make invalid computed values visible without also
showing genuinely missing cells:

```rust
UniformRaster2D::new()
    .raster(col("raster"))
    .null_color(Color::TRANSPARENT)
    .non_finite_color(Color::MAGENTA)
```

Color-scale and domain rules:

- For scaled fill channels, null cells do not participate in fill/color domain
  inference.
- For scaled fill channels, non-finite cells do not participate in fill/color
  domain inference.
- For scaled fill channels, fill/color domain inference scans `values.data`
  with generic ListArray domain inference and uses the same finite, non-null
  rule.
- For no-scale fill channels, `values.data` is direct color data and does not
  participate in fill/color domain inference.
- Non-finite color handling only applies to floating value planes. Integer and
  boolean planes cannot contain `NaN` or infinities.
- `null_color` and `non_finite_color` are outside the fill color scale. They
  should not affect colorbar scale limits.
- Future legend/colorbar work may add optional swatches for null and
  non-finite cells, but v1 should keep them out of the continuous colorbar.

Renderer rules:

- Compact raster marks colorize null cells with `null_color`.
- Compact raster marks colorize `NaN`, `+inf`, and `-inf` with
  `non_finite_color`.
- Typed explode transforms should preserve the original cell value semantics:
  Arrow null remains null, and non-finite floats remain non-finite floats. The
  downstream row-wise mark then owns its normal null/non-finite behavior.

## Uniform Raster Geometry

Use `uniform` for regular grids with constant spacing along rows and columns.
This is the main Datashader-style output shape and the most direct input to a
colorized image renderer.

Arrow pseudo-schema:

```text
raster: Struct<
  geometry: Struct<
    kind: Utf8,           // "uniform"
    coordinate_space: Struct<...>?,
    columns: Struct<
      coord: Utf8,        // metadata, e.g. "x", "longitude", "webmercator_x"
      sampling: Utf8?,    // e.g. "linear", "log10", "ln", "sqrt"
      start: Float64,     // outer edge of column 0
      stop: Float64,      // outer edge after final column
      count: UInt32,
    >,
    rows: Struct<
      coord: Utf8,
      sampling: Utf8?,
      start: Float64,     // outer edge of row 0
      stop: Float64,      // outer edge after final row
      count: UInt32,
    >,
  >,
  values: Struct<
    data: List<TCell>,
    null_count: UInt32?,
    non_finite_count: UInt32?,
  >,
>
```

Derived edge positions:

```text
column_edge(i) = columns.start + (columns.stop - columns.start) * i / columns.count
row_edge(i)    = rows.start    + (rows.stop    - rows.start)    * i / rows.count
```

Important details:

- `start` and `stop` preserve orientation. `rows.start > rows.stop` is a valid
  top-to-bottom raster.
- With linear sampling, domain summaries use `min(start, stop)` and
  `max(start, stop)` per dimension.
- With nonlinear sampling, coordinate-domain contribution requires a compatible
  position scale and a known inverse sampling function. The inferred scale
  domain is based on inverse-sampled `start/stop`, not the stored sampled
  coordinates.
- `coord` is descriptive metadata and may be used for validation, titles, CRS
  checks, or coordinate-system-specific interpretation.
- `sampling` describes the numeric space where the axis is uniformly sampled.
  Missing sampling means `"linear"`.
- `start` and `stop` are expressed in sampled coordinates. A log10-sampled
  raster over source x values `1..1000` stores `start = 0`, `stop = 3`, and
  `sampling = "log10"`.
- `coordinate_space` identifies the CRS in which `columns.start/stop` and
  `rows.start/stop` are expressed.
- A Cartesian raster mark maps `columns` to the horizontal/x position scale and
  `rows` to the vertical/y position scale by mark semantics, not by `coord`
  string matching.

Validation:

```text
geometry.kind == "uniform"
columns.count > 0
rows.count > 0
len(values.data) == rows.count * columns.count
```

## Rectilinear Raster Geometry

Use `rectilinear` when cells are still axis-aligned but row or column spacing is
not constant. This matches many gridded scientific datasets and is a natural
bridge between image-like rasters and per-cell rect rendering.

Arrow pseudo-schema:

```text
raster: Struct<
  geometry: Struct<
    kind: Utf8,              // "rectilinear"
    coordinate_space: Struct<...>?,
    columns: Struct<
      coord: Utf8,
      sampling: Utf8?,
      edges: List<Float64>,  // length = columns.count + 1
      start: Float64,        // duplicate of first edge for cheap domains
      stop: Float64,         // duplicate of final edge for cheap domains
      count: UInt32,
    >,
    rows: Struct<
      coord: Utf8,
      sampling: Utf8?,
      edges: List<Float64>,  // length = rows.count + 1
      start: Float64,
      stop: Float64,
      count: UInt32,
    >,
  >,
  values: Struct<
    data: List<TCell>,
    null_count: UInt32?,
    non_finite_count: UInt32?,
  >,
>
```

Cell bounds:

```text
column cell c uses [columns.edges[c], columns.edges[c + 1]]
row cell r    uses [rows.edges[r],    rows.edges[r + 1]]
```

Important details:

- `start` and `stop` intentionally duplicate edge information so scale-domain
  inference does not need to inspect nested lists for linearly sampled axes.
- `sampling` describes the numeric space of the edge coordinates. Missing
  sampling means `"linear"`.
- With nonlinear sampling, coordinate-domain contribution requires a compatible
  position scale and a known inverse sampling function. The inferred scale
  domain is based on inverse-sampled `start/stop`; individual edges are
  inverse-sampled before being passed through the position scale.
- `coordinate_space` identifies the CRS in which `columns.edges` and
  `rows.edges` are expressed.
- Edges should be monotonic, either increasing or decreasing.
- Rectilinear rasters can render as variable-size rects, or be resampled into a
  uniform output raster by a View/Rasterize pipeline.

Validation:

```text
geometry.kind == "rectilinear"
len(columns.edges) == columns.count + 1
len(rows.edges) == rows.count + 1
len(values.data) == rows.count * columns.count
```

## QuadMesh Geometry

Use `quadmesh` for structured curvilinear grids where neighboring cells share
vertices. This is the likely native representation for a future
`QuadMesh2D`/curvilinear raster mark.

For quad meshes, `rows` and `columns` describe topology. Coordinate component
names live with the vertex arrays, because row and column indices are no longer
themselves the plotted coordinates.

Arrow pseudo-schema:

```text
raster: Struct<
  geometry: Struct<
    kind: Utf8,                 // "quadmesh"
    coordinate_space: Struct<...>?,
    columns: Struct<
      coord: Utf8,              // metadata for the column dimension
      count: UInt32,            // cell columns
    >,
    rows: Struct<
      coord: Utf8,              // metadata for the row dimension
      count: UInt32,            // cell rows
    >,
    horizontal: Struct<
      coord: Utf8,              // e.g. "x", "longitude", "webmercator_x"
      vertices: List<Float64>,  // length = (rows.count + 1) * (columns.count + 1)
    >,
    vertical: Struct<
      coord: Utf8,              // e.g. "y", "latitude", "webmercator_y"
      vertices: List<Float64>,  // same length
    >,
  >,
  values: Struct<
    data: List<TCell>,
    null_count: UInt32?,
    non_finite_count: UInt32?,
  >,
>
```

Vertex indexing:

```text
vertex_index = vertex_row * (columns.count + 1) + vertex_column
```

Cell `(row, column)` uses:

```text
(row,     column)
(row,     column + 1)
(row + 1, column + 1)
(row + 1, column)
```

Important details:

- `horizontal` and `vertical` name the two plotted coordinate components
  without hard-coding `x`/`y` into the Arrow schema.
- `coordinate_space` identifies the CRS in which `horizontal.vertices` and
  `vertical.vertices` are expressed.
- A Cartesian mark maps `horizontal` to x and `vertical` to y by mark
  semantics.
- A WebMercator mark/view may require `horizontal.coord` and `vertical.coord`
  to be projected coordinates, or may provide a separate resampling transform
  from longitude/latitude to projected raster output.
- x/y domain inference should use generic ListArray domain inference over
  `horizontal.vertices` and `vertical.vertices`.

Validation:

```text
geometry.kind == "quadmesh"
len(horizontal.vertices) == (rows.count + 1) * (columns.count + 1)
len(vertical.vertices) == (rows.count + 1) * (columns.count + 1)
len(values.data) == rows.count * columns.count
```

## CellMesh Geometry

`cellmesh` is a possible future escape hatch for per-cell quadrilaterals that
do not share vertices. It is more general than `quadmesh`, but heavier and less
structured. Do not implement this before `quadmesh` unless a concrete use case
needs it.

Arrow pseudo-schema:

```text
raster: Struct<
  geometry: Struct<
    kind: Utf8,                 // "cellmesh"
    coordinate_space: Struct<...>?,
    columns: Struct<
      coord: Utf8,
      count: UInt32,
    >,
    rows: Struct<
      coord: Utf8,
      count: UInt32,
    >,
    horizontal: Struct<
      coord: Utf8,
      vertices: List<Float64>,  // length = rows.count * columns.count * 4
    >,
    vertical: Struct<
      coord: Utf8,
      vertices: List<Float64>,  // same length
    >,
  >,
  values: Struct<
    data: List<TCell>,
    null_count: UInt32?,
    non_finite_count: UInt32?,
  >,
>
```

Cell `(row, column)` starts at:

```text
cell_index = row * columns.count + column
vertex_base = cell_index * 4
```

The four vertices are stored in winding order. The exact winding convention
should match the renderer before implementation.

## Public Mark Structure

Raster chart marks should be separate public mark types. Shared options can be
implemented with shared traits, helper structs, or duplicated generated channel
config, but the public mark constructors should stay geometry-specific.

Preferred v1 naming:

```rust
UniformRaster2D::new()
    .raster_with(col("raster"), |r| r.fill(|f| f.scale(...)))
```

The `raster_with` closure should receive a raster channel bundle, not a fill
channel directly. The bundle configures implicit channels whose expressions
come from the raster struct:

```rust
UniformRaster2D::new()
    .raster_with(col("raster"), |r| {
        r.fill(|f| f.scale(...))
            .x(|x| x.axis(|a| a.title("x")))
            .y(|y| y.axis(|a| a.title("y")))
    })
```

Future mark names:

```rust
RectilinearRaster2D::new()
    .raster_with(col("raster"), |r| r.fill(|f| f.scale(...)))

QuadMesh2D::new()
    .raster_with(col("raster"), |r| r.fill(|f| f.scale(...)))

CellMesh2D::new()
    .raster_with(col("raster"), |r| r.fill(|f| f.scale(...)))

PolarRaster2D::new()
    .raster_with(col("raster"), |r| r.fill(|f| f.scale(...)))
```

Avoid this as the initial public API:

```rust
Raster2D::new()
    .raster(col("raster"))
```

That form implies dynamic raster dispatch or a common rendering model that v1
does not provide. Uniform, rectilinear, quadmesh, and cellmesh rasters share
fill color channel concepts, but their geometry options and low-level rendering
paths differ.

Shared mark concepts:

- raster struct column,
- fill/color scale or no-scale direct colors,
- null cell color,
- non-finite numeric cell color,
- full-raster row opacity, with future blend/compositing controls,
- colorbar/legend support,
- clipping to the plot area.

Geometry-specific concepts:

- uniform image interpolation/smoothing,
- rectilinear variable cell edges,
- quadmesh/cellmesh vertex winding, tessellation, and antialiasing,
- mesh bounding-box approximations for interaction or debugging.

## Multiple Raster Rows And Faceting

Compact raster marks should support multiple input rows. Each row renders one
raster instance:

```text
layer | time | raster
------|------|-------------------------
A     | 0    | Struct<geometry, values>
B     | 0    | Struct<geometry, values>
C     | 0    | Struct<geometry, values>
```

`UniformRaster2D` should render three scene images for this dataframe. The
scene mark may group compatible rows into one `SceneImageMark` with
`len = 3`, or emit separate scene image marks. That is an implementation
choice; the public semantics are one raster per row.

Non-raster columns are still ordinary mark data. They should work with existing
plot machinery:

- facet by columns outside the raster struct,
- filter rows before rendering,
- sort/order rows before rendering if ordering support exists,
- drive full-raster opacity or other non-geometry channels,
- participate in hover/item data where supported.

Illustrative example:

```text
Facet by category:
  mark = UniformRaster2D::new()
      .raster(col("raster"))
      .opacity(col("alpha"))
```

In this shape, `category` and `alpha` are normal top-level dataframe columns.
The raster mark should not require these columns to be duplicated inside the
raster struct.

Opacity is row-level. `opacity(col("alpha"))` evaluates one value per raster
row and multiplies that value into every generated cell pixel alpha for that
row after fill/no-scale colorization and after null/non-finite color
substitution. It is intentionally outside the `raster_with` channel bundle
because it is not sourced from raster internals.

Domain behavior across multiple raster rows:

- by default, x domain is the union of every row's column extent after applying
  the axis-specific inverse sampling function when required.
- by default, y domain is the union of every row's row extent after applying
  the axis-specific inverse sampling function when required.
- if the mark has an explicit `transpose()` option enabled, x uses row extents
  and y uses column extents.
- if the fill channel is scaled, fill/color domain is inferred from every row's
  `values.data`, flattened via generic ListArray domain inference.
- if the fill channel is no-scale, cell colors are direct visual values and do
  not contribute to fill/color domain inference.
- null and non-finite cells are excluded from fill/color domains by the rules
  above.
- If the plot is faceted, domain inference follows the existing facet domain
  sharing rules. Each facet sees the raster rows assigned to that facet unless
  the channel/domain is explicitly shared.

The compact mark path should not explode cells into dataframe rows. It should
read each raster struct row, colorize that raster's values, and emit one compact
scene payload for that row.

## Static Versus View-Dependent Domains

Static or precomputed raster data should participate in ordinary automatic
domain inference when the fill channel is scaled:

```text
x domain    <- inverse-sampled raster.geometry.columns.start/stop
y domain    <- inverse-sampled raster.geometry.rows.start/stop
fill domain <- raster.values.data
```

With an explicit mark-level `transpose()` option:

```text
x domain    <- inverse-sampled raster.geometry.rows.start/stop
y domain    <- inverse-sampled raster.geometry.columns.start/stop
fill domain <- raster.values.data
```

With `raster_with(..., |r| r.fill(|f| f.no_scale()))`, only x/y geometry
contributes domains. The direct color value plane does not create a fill scale
or colorbar.

This applies to raster structs loaded from external Arrow data, produced by a
non-view-dependent transform, or authored directly by user code.

Raster structs produced inside a `View` transform are different. Their geometry
is an output of the current view, so using that geometry to infer x/y domains
would create a feedback loop:

```text
View x/y domain -> Rasterize/View output geometry -> inferred x/y domain
```

For raster marks authored inside a `View` closure:

- x/y geometry is render placement only.
- x/y automatic domain inference comes from the `View` domain expressions.
- raster `geometry.columns/rows.start/stop` must not contribute to x/y domains.
- if the fill channel is scaled, fill/color domain may still come from
  `values.data`, because color is not the Cartesian View domain.
- if the fill channel is no-scale, `values.data` remains direct color data and
  contributes no fill/color domain.

This should use the same View domain-fence rule as other View-dependent marks:
Cartesian x/y channels set inside View do not participate in x/y inference, and
the View transform supplies the replacement x/y domain sources.

## Rendering Structure

Raster structs describe cell data plus geometry. Chart marks should validate
those structs, infer domains where scaling is enabled, apply color scales or
direct color coercion, and then compile to compact scene marks. Low-level scene
marks should render already-colorized payloads and should not evaluate
dataframe expressions or Avenger scales.

The intended rendering split:

```text
UniformRaster2D chart mark
  -> SceneImageMark

PolarRaster2D chart mark
  -> ScenePolarRasterMark or SceneQuadMeshMark

RectilinearRaster2D chart mark
  -> SceneRectilinearRasterMark

QuadMesh2D chart mark
  -> SceneQuadMeshMark

CellMesh2D chart mark
  -> SceneCellMeshMark, later
```

### Uniform Raster Rendering

Uniform geometry is affine in its own coordinate space. When the coordinate and
scale compatibility rules above show that this coordinate space maps affinely
to screen pixels, the raster can be represented as an RGBA image plus a
rectangular data extent after color scaling or direct color coercion:

```text
geometry.columns.start/stop/count
geometry.rows.start/stop/count
values.data
  -> color scale or no-scale color coercion
  -> RGBA pixels
  -> SceneImageMark
```

This should be the v1 compact rendering path for compatible Cartesian and
WebMercator Datashader-style output. The chart mark owns:

- schema validation,
- x/y domain contribution from `geometry.columns` and `geometry.rows`,
- fill/color domain contribution from `values.data` when the fill channel is
  scaled,
- scalar-to-RGBA color scaling or direct color coercion,
- null and non-finite cell color handling,
- row-level opacity multiplication into generated RGBA pixels,
- image placement using the raster extent.

The existing image mark/resource path remains the right abstraction for
already-colorized image payloads. A direct-color uniform raster is different:
it is still a grid-valued dataframe column with data-space geometry, but its
cell values are already visual colors selected with `raster_with(...,
|r| r.fill(|f| f.no_scale()))`.

### Polar Raster Rendering

A polar raster may use the same `uniform` or `rectilinear` Arrow geometry as a
cartesian raster, with `coord` metadata identifying `theta` and `r` axes. The
rendering path is different. A uniform grid in `(theta, r)` is not an
axis-aligned rectangular image after the polar coordinate transform; cells
become annular sectors.

Do not render a polar raster by stretching one `SceneImageMark` through the
polar transform. Valid rendering paths are:

```text
PolarRaster2D -> ScenePolarRasterMark
PolarRaster2D -> generated SceneQuadMeshMark
PolarRaster2D -> resample in View -> UniformRaster2D -> SceneImageMark
```

The native polar path should:

- validate that the raster has one theta axis and one radial axis,
- infer theta/r domains from the raster geometry,
- infer fill/color domain from `values.data` when the fill channel is scaled,
- scale theta/r through the plot's normal polar scales,
- generate sector or mesh vertices from scaled theta/r cell edges,
- colorize `values.data` to one color per cell,
- emit a compact scene payload.

The resampled image path is a separate operation. It is appropriate when the
desired output is a screen/cartesian pixel grid, but it loses the native polar
cell boundaries.

### Numeric Value Type Handling

Uniform raster rendering should accept numeric value planes without requiring a
single canonical input type. The implementation should:

```text
1. Validate that values.data is a list whose element type is numeric.
2. Extract the list child array for each raster row.
3. Cast the child array to the scale working type, currently Float32 in the
   existing numeric scale/coercer path.
4. Apply null/non-finite masks before or during colorization.
5. Call the configured fill/color scale on the cast numeric array.
6. Convert scaled colors to RGBA pixels.
```

This follows existing Avenger behavior:

- numeric channel coercion casts Arrow arrays to `Float32`,
- numeric scale implementations cast input arrays before scale math,
- domain span inference casts numeric dataframe columns to `Float32`,
- default scale type inference treats primitive integer and float Arrow types
  as linear numeric data.

The raster mark should not ask users to pre-cast `values.data` to `Float64`.
Precision follows the existing numeric scale path: large integers and decimals
may lose precision when cast to the scale working type, which is acceptable for
color mapping unless a later use case requires higher precision.

### Direct Color Value Handling

No-scale raster rendering should accept color value planes for consistency with
ordinary color channels:

```rust
UniformRaster2D::new()
    .raster_with(col("raster"), |r| r.fill(|f| f.no_scale()))
```

In this mode:

```text
values.data -> Avenger color coercion -> RGBA pixels -> SceneImageMark
```

Implementation requirements:

1. Validate that `values.data` is a list whose child value type can be coerced
   by the same color coercer used for ordinary no-scale fill/stroke channels.
2. Flatten each raster row's list child.
3. Coerce the child values to `ColorOrGradient`.
4. Reject gradients for v1 uniform raster image output, unless a later scene
   representation can preserve them per cell.
5. Convert concrete colors to RGBA pixels.

Required v1 tests should cover `List<Utf8>` CSS colors and
`List<FixedSizeList<Float32, 4>>` normalized RGBA values. Dictionary-encoded
CSS colors should be supported if the list-flattening color coercion path can
preserve the dictionary representation cleanly.

### Scene Image Compatibility For Uniform Raster

The current `SceneImageMark` is sufficient to place a uniform raster as an
axis-aligned rectangular image. It already supports:

- inline RGBA image sources,
- resource-backed image sources with `fallback_key`,
- `x`, `y`, `width`, and `height`,
- `align` and `baseline`,
- `aspect`,
- `smooth`,
- clipping and z-index.

Uniform raster marks should emit scene images with:

```text
image = SceneImageSource::Inline(RgbaImage)
align = left
baseline = top
aspect = false
smooth = false by default
```

The chart mark should first convert raster extent corners to scale-input
coordinates. For linear sampling this is raw `start/stop`; for compatible
nonlinear sampling it is inverse-sampled `start/stop`. It should then transform
those extent corners through the coordinate transform, compute positive
screen-space bounds, and emit:

```text
x = screen_left
y = screen_top
width = screen_right - screen_left
height = screen_bottom - screen_top
```

Do not rely on negative image widths or heights to encode orientation. Current
WGPU and SVG rendering both effectively render the image over the bounding box.
If the transformed column extent or row extent direction is reversed in screen
space, the chart mark should reorder the RGBA pixels before building the
`SceneImageSource`:

```text
screen_start_x > screen_stop_x -> reverse image columns
screen_start_y > screen_stop_y -> reverse image rows
```

This keeps the existing scene mark simple and makes the pixel buffer match the
actual screen orientation.

The inline image buffer should be ordinary row-major RGBA bytes:

```text
RgbaImage.width = image_width
RgbaImage.height = image_height
byte_index = 4 * (image_row * image_width + image_column)
```

Default axis binding:

```text
image_width = columns.count
image_height = rows.count
image_row = raster_row
image_column = raster_column
storage_index = raster_row * columns.count + raster_column
```

With an explicit mark-level `transpose()` option:

```text
image_width = rows.count
image_height = columns.count
image_row = raster_column
image_column = raster_row
storage_index = raster_row * columns.count + raster_column
```

The chart mark should build this buffer after color scaling or direct color
coercion, null/non-finite color substitution, and row-level opacity
multiplication. For v1, emit one `SceneImageMark` per raster row and preserve
the original dataframe-row mapping through `RenderedMarkData::with_source_row_indices`.
Later, compatible rows can be batched into one scene image mark using
`ScalarOrArray::new_array` for `image`, `x`, `y`, `width`, and `height`; this is
only an optimization.

Recommended scenegraph improvement:

```rust
impl SceneImageMark {
    pub fn from_extent(
        image: ScalarOrArray<SceneImageSource>,
        left: ScalarOrArray<f32>,
        top: ScalarOrArray<f32>,
        width: ScalarOrArray<f32>,
        height: ScalarOrArray<f32>,
    ) -> Self
}
```

This is a convenience constructor, not a new serialized scenegraph schema. It
would reduce repeated boilerplate and make the "image covers this rectangle"
intent explicit.

Required renderer improvement before treating `smooth = false` as reliable:

- WGPU currently creates image atlas textures with linear filtering for all
  image marks.
- SVG honors `smooth = false` by emitting `image-rendering="pixelated"`.
- Uniform raster rendering should either fix WGPU image sampling to respect
  `SceneImageMark::smooth`, or explicitly document that v1 WGPU output is
  linearly filtered.

Preferred WGPU fix:

```text
1. Add a nearest image sampler alongside the current linear image sampler.
2. Add an image texture code or vertex flag for nearest image sampling.
3. In add_image_mark, choose linear vs nearest from mark.smooth.
4. Add WGPU image baseline coverage proving smooth=false renders nearest.
```

### Rectilinear Raster Rendering

Rectilinear cells are axis-aligned, but spacing can vary. Rendering them as a
single image would imply uniform spacing unless the data is resampled first.
Therefore native rectilinear rendering should use a dedicated scene mark:

```text
SceneRectilinearRasterMark {
  column_edges,
  row_edges,
  cell_colors,
  row_count,
  column_count,
}
```

Valid rendering paths:

```text
RectilinearRaster2D -> SceneRectilinearRasterMark
RectilinearRaster2D -> resample/rasterize in View -> UniformRaster2D -> SceneImageMark
```

These should remain distinct operations. Native rendering preserves cell
boundaries. Resampling produces a view-dependent uniform image.

### QuadMesh Rendering

Quad meshes need a compact low-level scene mark. Exploding them into many
high-level path-like marks would be a poor primary rendering path.

Suggested scene payload:

```text
SceneQuadMeshMark {
  horizontal_vertices,
  vertical_vertices,
  cell_colors,
  row_count,
  column_count,
}
```

The chart `QuadMesh2D` mark owns:

- schema validation,
- x/y domain contribution from flattened vertex arrays,
- fill/color domain contribution from `values.data` when the fill channel is
  scaled,
- scalar-to-RGBA color scaling or direct color coercion,
- conversion from Arrow list arrays to compact scene buffers.

The scene mark owns only rendering the mesh. It should not know how to look up
`values.data` or apply fill scales.

### Prior Art: HoloViews/Bokeh QuadMesh

HoloViews' `QuadMesh` is useful prior art because it supports both rectilinear
and curvilinear gridded data with one user-facing element, then chooses
different Bokeh glyphs internally.

Relevant source:

- `holoviews/element/raster.py`, `QuadMesh`
- `holoviews/plotting/bokeh/raster.py`, `QuadMeshPlot`
- `holoviews/plotting/bokeh/util.py`, `colormesh`

Observed representation:

- Public data is conceptually `(X, Y, Z)`.
- `X` and `Y` may be 1D coordinate arrays or 2D coordinate arrays.
- `Z` is a 2D value array with shape `(nrows, ncolumns)`.
- `X`/`Y` may be edge arrays with one more coordinate than the value array.
- The element follows matrix orientation: `Z[row, column]` maps to y/x grid
  position.

Observed Bokeh backend behavior:

- HoloViews checks whether either coordinate dimension is irregular.
- If coordinates are not irregular, it renders with Bokeh `quad`:

  ```text
  left, right, bottom, top, value
  ```

- If coordinates are irregular, it renders with Bokeh `patches`:

  ```text
  xs: List<List<Float>>
  ys: List<List<Float>>
  value
  ```

- The irregular path expands each cell into a closed polygon ring. Its
  `colormesh` helper emits five points per cell: four corners plus the first
  corner repeated to close the path.
- The irregular path drops cells whose value or vertices are non-finite.
- Hover centers for irregular cells are computed as the arithmetic mean of the
  cell vertices.
- The implementation has separate flattening/transposition logic for regular
  quads and irregular patches because the two Bokeh glyph families consume
  data in different orders.

Lessons for Avenger:

- Keep `RectilinearRaster2D` and `QuadMesh2D` as separate public marks even if
  a higher-level compound API later accepts both. HoloViews' single element is
  convenient, but internally the render paths are meaningfully different.
- Preserve topology in Avenger's compact scene payloads. For rectilinear data,
  store 1D row/column edges. For quadmesh data, store shared 2D vertex arrays.
  Do not make per-cell polygon rings the canonical scene representation unless
  a renderer backend specifically requires them.
- Add an explicit renderer conversion step only at the backend boundary if a
  backend needs patch-like cells. That conversion can duplicate vertices, close
  rings, and drop invalid cells without changing the Arrow raster schema.
- Be strict about row-major orientation and flattening. HoloViews' backend has
  multiple transpose paths; Avenger should avoid implicit orientation changes.
  Any transpose should be an explicit mark option with baselines for descending
  axes, inverted axes, and curvilinear cells.
- Keep finite-value and finite-geometry masks separate. A non-finite cell value
  may use `non_finite_color`; non-finite geometry should normally make the cell
  unrenderable.
- Support cell centers as derived data for hover, symbols, and labels. The
  arithmetic mean of vertices is a reasonable default, but it should be
  documented as a placement heuristic rather than geometric truth.
- Prefer edge/vertex input in the Arrow schema. HoloViews accepts centers and
  edges, but Avenger should keep center-to-edge inference as an explicit
  transform/helper to avoid ambiguity in non-uniform grids.
- Consider a future `QuadMesh2D -> TriMesh`/triangle-index conversion for GPU
  renderers. HoloViews exposes a `QuadMesh.trimesh()` path, which is a useful
  reminder that a shared-vertex mesh can lower cleanly to triangles without
  changing the public raster struct.

### CellMesh Rendering

`CellMesh2D` is a later, more general path for per-cell quadrilaterals that do
not share topology. It should have a separate scene mark rather than overloading
`SceneQuadMeshMark`:

```text
SceneCellMeshMark {
  horizontal_vertices_per_cell,
  vertical_vertices_per_cell,
  cell_colors,
  cell_count,
}
```

Do not implement this before a concrete use case requires it. `QuadMesh2D`
should cover structured curvilinear grids first.

### Relationship To Explode Transforms

Typed raster explode transforms are not the native rendering path. They are the
public composition path for using existing row-wise marks with raster cells:

```text
col("raster") -> ExplodeUniformRaster2D -> Symbol/Rect/Text
col("raster") -> ExplodeQuadMesh2D      -> Symbol/Text/debug marks
```

The compact mark path and explode path should share validation and extraction
helpers where practical, but they serve different performance and composition
needs.

## Rasterize2D Output Shape

`Rasterize2D` should return ordinary dataframe rows. With no partitioning, it
returns one row:

```text
raster
------
Struct<geometry, values>
```

With partitioning, it returns one row per partition:

```text
category | raster
---------|-------------------------
A        | Struct<geometry, values>
B        | Struct<geometry, values>
C        | Struct<geometry, values>
```

This keeps faceting, filtering, opacity, layer ordering, legends, and other
ordinary mark machinery in the dataframe model. The raster is one logical datum
inside a row, not a replacement for the dataframe itself.

Pre-existing raster data may also arrive in this same multi-row shape without
going through `Rasterize2D`. The compact raster marks should not care whether
the struct column came from a transform, an external Arrow table, or user code.

## Typed Raster Explode Transforms

Raster explosion should be a family of typed transforms, not one polymorphic
`ExplodeRaster2D` transform. Each raster geometry has different valid cell
outputs, and separate transforms give the best type-safe authoring API.

Initial transform family:

```text
ExplodeUniformRaster2D
ExplodeRectilinearRaster2D
ExplodeQuadMesh2D
ExplodeCellMesh2D      // future escape hatch
```

These are the public escape hatches that turn raster-valued rows back into
ordinary cell rows. This lets existing marks such as `Symbol`, `Rect`, and
`Text` interpret raster cells without those marks learning raster-specific
logic.

Conceptually:

```text
one row with Struct<geometry, values>
  -> typed raster explode transform
many rows, one per raster cell
```

Example input:

```text
category | raster
---------|-------------------------
A        | Struct<geometry, values>
```

Example output:

```text
category | raster_row | raster_column | horizontal_center | vertical_center | value
---------|------------|---------------|-------------------|-----------------|------
A        | 0          | 0             | ...               | ...             | ...
A        | 0          | 1             | ...               | ...             | ...
A        | 1          | 0             | ...               | ...             | ...
```

Suggested authoring shape:

```rust
Symbol::new()
    .transform(
        ExplodeUniformRaster2D::new(col("raster")),
        |symbol, cells| {
            symbol
                .x(cells.horizontal_center())
                .y(cells.vertical_center())
                .fill(cells.value())
        },
    )
```

For rect-like rendering:

```rust
Rect::new()
    .transform(
        ExplodeUniformRaster2D::new(col("raster")),
        |rect, cells| {
            rect
                .x(cells.horizontal_start())
                .x2(cells.horizontal_stop())
                .y(cells.vertical_start())
                .y2(cells.vertical_stop())
                .fill(cells.value())
        },
    )
```

The transform should preserve non-raster input columns by repeating their
values for each emitted cell. It should drop the source raster column by default
to avoid duplicating a large nested value into every output row. Add
`.keep_source(true)` later only if debugging or downstream use cases need it.

The transforms may share implementation internally, but the public constructors
and output handles should remain geometry-specific. A common trait may be useful
for shared methods such as `row_index()`, `column_index()`, `value()`,
`horizontal_center()`, and `vertical_center()`, but users should not get methods
that are invalid for the selected geometry.

### Uniform Raster Cells

`ExplodeUniformRaster2D` accepts an ordinary raster expression and returns a
`UniformRasterCells2D` output handle.

Output handle methods:

```rust
cells.row_index()
cells.column_index()

cells.horizontal_start()
cells.horizontal_center()
cells.horizontal_stop()

cells.vertical_start()
cells.vertical_center()
cells.vertical_stop()

cells.value()
```

Cartesian-specific aliases may be added if useful:

```rust
cells.x()
cells.y()
cells.x_start()
cells.x_stop()
cells.y_start()
cells.y_stop()
```

but the underlying transform output should keep using `horizontal` and
`vertical` to match the raster geometry naming.

Expansion formulas:

```text
horizontal_start  = columns.start + column * (columns.stop - columns.start) / columns.count
horizontal_stop   = columns.start + (column + 1) * (columns.stop - columns.start) / columns.count
horizontal_center = (horizontal_start + horizontal_stop) / 2

vertical_start    = rows.start + row * (rows.stop - rows.start) / rows.count
vertical_stop     = rows.start + (row + 1) * (rows.stop - rows.start) / rows.count
vertical_center   = (vertical_start + vertical_stop) / 2

value             = values.data[row * columns.count + column]
```

This naturally preserves reversed orientations because `start` and `stop` are
not sorted before computing cell edges.

### Rectilinear Raster Cells

`ExplodeRectilinearRaster2D` accepts an ordinary raster expression and returns
a `RectilinearRasterCells2D` output handle.

The output handle should match the uniform output handle:

```rust
cells.row_index()
cells.column_index()

cells.horizontal_start()
cells.horizontal_center()
cells.horizontal_stop()

cells.vertical_start()
cells.vertical_center()
cells.vertical_stop()

cells.value()
```

Expansion formulas:

```text
horizontal_start  = columns.edges[column]
horizontal_stop   = columns.edges[column + 1]
horizontal_center = (horizontal_start + horizontal_stop) / 2

vertical_start    = rows.edges[row]
vertical_stop     = rows.edges[row + 1]
vertical_center   = (vertical_start + vertical_stop) / 2

value             = values.data[row * columns.count + column]
```

### QuadMesh Cells

`ExplodeQuadMesh2D` accepts an ordinary raster expression and returns a
`QuadMeshCells2D` output handle.

Output handle methods:

```rust
cells.row_index()
cells.column_index()

cells.horizontal_vertices()
cells.vertical_vertices()

cells.horizontal_center()
cells.vertical_center()

cells.horizontal_min()
cells.horizontal_max()
cells.vertical_min()
cells.vertical_max()

cells.value()
```

Do not expose `horizontal_start()`, `horizontal_stop()`, `vertical_start()`, or
`vertical_stop()` on `QuadMeshCells2D`. Those names imply axis-aligned
rectangular cells. Quad cells should expose vertices and optional bounding-box
summaries instead.

`horizontal_vertices()` and `vertical_vertices()` may be represented as
`List<Float64>` with length four for each output row. If fixed-size list support
is reliable enough in DataFusion, `FixedSizeList<Float64, 4>` is a better
physical representation.

The four vertices are gathered from the shared vertex arrays using the topology
described in the QuadMesh section.

Cell center defaults to the arithmetic mean of the four vertices:

```text
horizontal_center = mean(horizontal_vertices)
vertical_center   = mean(vertical_vertices)
```

This is sufficient for placing symbols or labels. It is not a substitute for a
proper mesh renderer when cells are large, warped, or self-intersecting.

### CellMesh Cells

`ExplodeCellMesh2D` is a future transform that accepts an ordinary raster
expression. It should return a `CellMeshCells2D` handle with the same public
shape as `QuadMeshCells2D`:

```rust
cells.row_index()
cells.column_index()

cells.horizontal_vertices()
cells.vertical_vertices()

cells.horizontal_center()
cells.vertical_center()

cells.horizontal_min()
cells.horizontal_max()
cells.vertical_min()
cells.vertical_max()

cells.value()
```

The difference is physical input storage: `CellMesh2D` reads per-cell vertices
instead of gathering shared vertices from a structured mesh topology.

### Domain And Scale Behavior

After any typed raster explode transform, downstream marks are ordinary row-wise
marks:

- `Symbol` x/y domain inference comes from the emitted center columns.
- `Rect` x/x2/y/y2 domain inference comes from emitted bounds.
- `fill` domain inference comes from the emitted scalar `value` column.

Typed explode transforms should not add special raster-domain contributions.
Their outputs are normal tabular data.

For mesh explodes, rect-style x/x2/y/y2 rendering should use bounding-box
columns only if the user opts into that approximation. The type-safe API should
make the approximation explicit by exposing `horizontal_min/max` and
`vertical_min/max`, not `start/stop`.

### Performance Role

Typed raster explode transforms are not the primary rendering path for dense
rasters. They are composition/debugging/annotation primitives. Dense raster
rendering should use a compact raster mark that consumes the struct directly.

Use cases:

- draw symbols at cell centers,
- draw text labels inside cells,
- render a small raster as ordinary rects,
- inspect rasterized values with existing mark machinery,
- build compound marks from public primitives.

## Naming Summary

Stable top-level fields:

```text
geometry
values
```

Stable shared value fields:

```text
values.data
values.null_count?
values.non_finite_count?
```

Geometry kind strings:

```text
"uniform"
"rectilinear"
"quadmesh"
"cellmesh"
```

Uniform/rectilinear geometry fields:

```text
geometry.coordinate_space?
geometry.columns
geometry.rows
geometry.columns.sampling?
geometry.rows.sampling?
```

Coordinate-system-specific axis names:

```text
cartesian convention: columns.coord = "x", rows.coord = "y"
polar convention:    columns.coord = "theta", rows.coord = "r"
```

Quad/cell mesh coordinate component fields:

```text
geometry.coordinate_space?
geometry.horizontal
geometry.vertical
```

## Open Questions

- Should large rasters use `LargeList<TCell>` instead of `List<TCell>`?
- Do we need `values.association` in v1, or can vertex-associated values wait?
- Should `coord` be required, nullable, or defaulted by dedicated mark
  implementations?
- Should `horizontal`/`vertical` be renamed before implementation to
  `component0`/`component1`, `u`/`v`, or coordinate-system-owned names?
