# Geo Coordinate System

Master design note for general geographic coordinate systems in Avenger: a
d3-geo-style projection engine crate (`avenger-geo`), a `Geo` coordinate
system crate (`avenger-chart-geo`) with graticule/sphere guides and
fit-to-data, point and line marks where line geometry follows the existing
`GeometrySpace` model (great-circle interpolation in `Coordinate` space),
a GeoJSON/`GeoShape` mark backed by WKB geometry columns, adaptive
pan/zoom, and raster tile layers warped through the projection.

Scope is limited to the *blendable* projection families — cylindrical,
pseudocylindrical, and conic projections that share Web Mercator's single
antimeridian-cut topology. This is what lets pan/zoom keep an ordinary
slippy-map camera and adaptively blend the authored projection toward Web
Mercator at high zoom (section 8). Rotation-family projections
(orthographic and other hemisphere azimuthals, true globe rendering) are
deferred to a follow-up note.

This is built as a separate coordinate system from
`avenger-chart-webmercator`, with replacement as the explicit end state:
Web Mercator is one raw projection of the general system, and once the
tile layer and the mercator fast path reach parity (section 9),
`avenger-chart-webmercator` is retired or reduced to a thin preset over
`Geo`.

Status: **Ready for staged implementation planning**. The projection engine
(sections 3–4) and geometry representation (section 7) have clear
boundaries; the coordinate-system integration reuses the proven WebMercator
patterns. Design spikes are flagged in section 12.

---

## 1. Goal

Render data on map projections beyond Web Mercator — Equal Earth, Natural
Earth, Winkel Tripel, Albers and other conics, equirectangular — with:

- **Points** positioned by `longitude`/`latitude` channels.
- **Lines** supporting both display-space geometry (straight pixel segments
  between projected vertices) and coordinate-space geometry (segments follow
  great circles, adaptively resampled through the projection, and cut at
  the antimeridian).
- **GeoJSON geometry** (polygons, multi-geometries) as a mark, with feature
  properties available as ordinary columns for encoding (choropleths are
  joins, not a chart type).
- **Graticules and sphere outline** rendered through the same projection
  pipeline so they bend and clip correctly.
- **Fit-to-data**: solve projection scale/translate so the data's projected
  bounds fill the plot area (the geo analogue of scale-domain inference).
- **Pan/zoom**: a slippy-map camera (re-center + scale-anchored zoom) that
  adaptively blends the authored projection toward Web Mercator at high
  zoom, so interaction stays natural at every scale (section 8).
- **Raster tile underlays** warped through the projection — the
  generalization of the WebMercator tile layer (section 10).

## 2. Design Precedents

Survey conclusions from d3-geo, Vega/Vega-Lite, Observable Plot, ggplot2
`coord_sf`, Plotly, cartopy, and deck.gl (researched 2026-07-02):

- **No serious library models a projection as a scale.** A projection is an
  entangled 2D function — `(x, y) = P(lon, lat)` with x depending on both
  inputs for anything conic, azimuthal, or rotated — so it cannot decompose
  into per-channel scales. Its "axes" are graticule curves, and it carries
  geometry obligations (clipping, cutting, resampling) that scales do not.
- The strongest fit for Avenger is the Observable Plot / ggplot2 position:
  **projection is the panel-level coordinate transform that replaces the
  x/y positional scales**, fed by dedicated `longitude`/`latitude` channels
  (Vega-Lite's API surface). This is exactly Avenger's `CoordinateSystem`
  trait.
- **d3-geo's factoring is the machinery to replicate**: a *raw projection*
  is a pure `(λ, φ) → (x, y)` function; everything else — three-axis
  spherical rotation, antimeridian cutting, small-circle clipping
  (`clip_angle` for azimuthal horizons), adaptive resampling, planar
  rectangle clipping, fit — is shared pipeline infrastructure every
  projection gets for free:

  ```text
  degrees→radians → rotate(λ, φ, γ) → spherical pre-clip
  (antimeridian lune OR small circle) → planar projection with
  adaptive resampling → scale/translate → rectangle post-clip
  ```

- **Adaptive resampling** is the load-bearing stage: a geodesic projects to
  a *curve* under every projection except gnomonic, so linear interpolation
  between projected endpoints is wrong (conic edges sag; discontinuities
  explode). d3 recursively bisects each segment along the great circle,
  comparing the projected spherical midpoint against the linear midpoint,
  until deviation is under a pixel-space `precision` (default ≈0.7px).
  Because the tolerance is in output pixels, accuracy is automatically
  zoom-appropriate. Sphere outlines and graticules are just more geometry
  through the same pipeline — no special-cased map background.
- **Winding order is load-bearing** for spherical polygon clipping: d3 uses
  clockwise exterior rings (spherical convention), which is *opposite*
  RFC 7946 GeoJSON's planar CCW. Ingest must normalize (rewind) so the
  pipeline can trust winding downstream.
- **Mapbox GL JS "adaptive projections"** keep the standard slippy-map
  camera on non-Mercator projections by pointwise-blending the projection
  toward Web Mercator as zoom increases, anchored at the screen center.
  Section 8 adopts this model.

## 3. Crate Boundaries

Two new workspace crates:

```text
avenger-geo          # chart-independent projection engine (like avenger-scales)
avenger-chart-geo    # Geo coordinate system, guides, and marks
```

`avenger-geo` has no chart dependencies (geo-types, lyon_path at most) and
is independently testable against d3-geo reference output. `avenger-chart-geo`
follows the external coordinate-system crate pattern proven by
`avenger-chart-webmercator` and `avenger-chart-polar`.

Rust ecosystem decision (researched 2026-07-02): **implement the projection
engine in-house** rather than depend on `d3_geo_rs`. That crate is the only
existing d3-style stream implementation, but it lacks Natural Earth, has a
generics-heavy API hostile to storing "any projection" behind one type, a
mandatory `web-sys` dependency, and version drift (geo 0.31, wgpu 28 vs our
pins). d3-geo core is ~4–6k lines of well-documented JS with an exhaustive
reference to port tests against; the raw math for the initial projections is
small, and the pipeline stages are a few hundred lines each. `proj` (C
bindings) fails the WASM requirement; `proj4rs` is point-in/point-out CRS
transformation with no clipping/resampling (only relevant if source-data
reprojection ever becomes a requirement). Use the already-pinned `geo`
crate's geodesic/haversine interpolation where convenient, and `d3_geo_rs`
only as a cross-check.

## 4. Projection Engine (`avenger-geo`)

Raw projections are pure math; the pipeline wraps them:

```rust
pub trait RawProjection: Send + Sync {
    /// Radians in, unit-scale planar coordinates out.
    fn project(&self, lambda: f64, phi: f64) -> (f64, f64);
    fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)>;
}
```

Initial projection set: equirectangular, mercator, equal earth (proposed
default, matching Vega-Lite), natural earth 1, winkel tripel, albers /
generic conic equal-area, conic conformal. All are blendable (section
8.1). Rotation-family projections (orthographic, stereographic, gnomonic,
azimuthal equal-area/equidistant) and composite projections (albers-usa)
are future work.

The pipeline is exposed as a stream/visitor API (d3's
`point / line_start / line_end / polygon_start / sphere` protocol) so
geometry never materializes between stages:

- **Rotation**: three-axis spherical rotation `(λ, φ, γ)` before
  projection — oblique aspects of any projection for free, and the basis of
  future globe interaction.
- **Spherical pre-clip**: antimeridian cutting. The slot is designed so
  small-circle `clip_angle` clipping (azimuthal horizons) can occupy it
  later — in d3 the two are exclusive alternatives — but circle clipping
  is deferred with the rotation family.
- **Adaptive resampling**: pixel-space `precision`, great-circle midpoint
  bisection as described in section 2.
- **Planar post-clip**: rectangle clip to the plot area (composes with, and
  reduces load on, the renderer's stencil clipping).
- **Sinks**: lyon path builder, bounds accumulator (for fit), length/area/
  centroid later (label placement).

Plus: `fit_extent` / `fit_size` (solve scale+translate from an object's
projected bounds), graticule and sphere geometry generators, and a
`Projection` configuration type (raw projection kind + rotate + center +
precision + scale/translate) that is plain-serde serializable (enum
dispatch over projection kinds, not trait objects). `RawProjection` stays
object-safe and composable: the pan/zoom blend (section 8) is a `Blend`
wrapper over two raw projections, and the pipeline (clipping, resampling,
fit, graticules) operates on it unchanged.

Testing: golden fixtures generated by running d3-geo in Node over shared
inputs (points, lines crossing the antimeridian, polygons enclosing a
pole, graticules), asserted within tolerance. This mirrors how
`avenger-scales` pins d3-scale behavior.

Numeric policy: f64 throughout the pipeline, cast to f32 at the sink
boundary (consistent with WebMercator's approach; scenegraph is f32).

## 5. Coordinate System (`avenger-chart-geo`)

`Geo` implements `CoordinateSystemCore` / `CoordinateSystem`, patterned on
`avenger-chart-webmercator/src/coord.rs`:

- **Positional architecture (as implemented — scratch/geo decision 1)**:
  `required_channels()` is `["x", "y"]` in the authored projection's raw
  planar units, mapped to pixels by coordinate-owned linear domains —
  exactly the WebMercator machinery, generalized. There are no
  *user-facing* positional scales: marks author positions as
  `.longitude(expr)` / `.latitude(expr)`, which bind x/y to fields of a
  per-projection `geo_project(lon, lat) → struct{x, y}` DataFusion UDF
  (general projections are not separable, so the per-channel expression
  trick WebMercator uses is replaced by the two-argument UDF). Geometry
  that needs great-circle resampling (graticules, geodesic lines,
  GeoShape) bypasses the UDF and streams render-side through the
  `avenger-geo` pipeline into pixels.
- Configuration: projection kind, rotate/center, `precision`, and either
  explicit scale/translate or fit-to-data.
- `CoordinateDomainProvider` / `CoordinateMeasurementProvider`: realize the
  fitted view from inferred lon/lat + geometry-column bounds and the plot
  area, following WebMercator's viewport-realization machinery. Fit has a
  natural aspect ratio, so it interacts with layout the same way
  WebMercator's measurement provider does.
- Guide (`CoordinateGuide` impl): graticule lines and sphere outline,
  generated by `avenger-geo` and rendered through the projection pipeline
  into `SceneLineMark` / `ScenePathMark`. Optional edge labels for
  meridians/parallels are follow-up work (ggplot2's `label_graticule` is
  the precedent — under a nonlinear coord the graticule *is* the axis
  grid).
- Interaction inversion: pixel → inverse pipeline (un-translate/scale →
  raw invert → un-rotate) → lon/lat, exposed via
  `invert_interaction_point`. Not all projections invert everywhere;
  return no value outside the valid region.

Pan/zoom tooling is specified in section 8: a slippy-style camera over the
authored projection with an adaptive Web Mercator blend at high zoom.
Spherical rotation (versor drag for globes) belongs to the deferred
rotation family.

## 6. Marks

- **`Symbol<Geo>`** (and `Text<Geo>` later): `longitude` / `latitude`
  channel builders; each point runs through the point pipeline. Points
  outside the projection's valid region become undefined rather than
  rendering at a fold.
- **`Line<Geo>`** reuses the existing `GeometrySpace` mark option
  (`avenger-chart-core/src/geometry_space.rs`), exactly as `Line<Polar>`
  does:
  - `GeometrySpace::Coordinate` (default): segments are great-circle arcs,
    adaptively resampled through the projection and cut at the antimeridian
    (lowered with `SceneLineMark::defined` breaks or multi-part paths).
  - `GeometrySpace::Display`: project vertices, straight pixel-space
    segments between them.
- **`GeoShape<Geo>`**: consumes a WKB geometry column (section 7); each
  feature streams through the pipeline into a `lyon_path::Path` →
  `ScenePathMark` (lyon's fill tessellator handles polygons with holes;
  ring orientation is normalized at ingest). Fill/stroke/tooltip channels
  encode ordinary columns, so choropleths are DataFusion joins against the
  feature table.

## 7. Geometry Data Representation

GeoJSON features become ordinary Arrow rows: properties as columns, geometry
as a **plain `Binary` column of ISO WKB**, plus `xmin`/`ymin`/`xmax`/`ymax`
Float64 side-columns computed at ingest (so extent inference, filtering, and
all existing DataFusion machinery work untouched).

Pipeline: `geojson` crate → `geo-types` → `wkb` crate (georust; pure Rust,
zero-copy reads via `geo-traits`, no arrow dependency). At mark lowering,
WKB is read zero-copy through `geo-traits` straight into the projection
stream. Ingest normalizes ring winding (section 2) and accepts EWKB while
always writing ISO WKB.

Why not GeoArrow-native from the start (researched 2026-07-02): the spec
is 0.2; `geoarrow-array` 0.8 requires arrow 58 while Avenger currently
pins arrow 55 via DataFusion 48; GeoDataFusion/SedonaDB need DataFusion
≥ 54/50. The chosen representation **is** GeoArrow's `geoarrow.wkb`
serialized encoding, so nothing is thrown away: a workspace DataFusion
upgrade is planned, and section 7.1 covers what it unlocks. Avoid
JSON-string geometry (reparse cost, no bounds pushdown) and opaque structs
in columns (breaks serialization/materialization paths).

Projection executes at render time in Rust (coordinate transform / mark
lowering), not as DataFusion UDFs: it depends on plot-area size and view
state, which the query layer should not see. DataFusion's role is storage,
joins, filtering, and bounds.

TopoJSON ingest (compact world atlases) is a follow-up; the `topojson`
crate is dormant (~2022, pre-1.0 `geojson` types) and would need a shim.

### 7.1 After the DataFusion upgrade: GeoArrow and GeoDataFusion

A workspace DataFusion/arrow upgrade (DataFusion ≥ 54 / arrow ≥ 58) is
planned independently of this work. Because the geometry column is
already byte-identical to the `geoarrow.wkb` encoding, everything below
is additive: no migration of stored data, and no change to the
mark-lowering path, which reads geometry through `geo-traits` either way.

- **Extension-type tagging**: mark the geometry field with
  `ARROW:extension:name = "geoarrow.wkb"` (plus CRS extension metadata).
  Metadata-only; from that point tools that speak GeoArrow (GeoParquet
  readers, Lonboard, SedonaDB, the geoarrow-rs kernels) recognize
  Avenger's geometry columns as-is.
- **Native GeoArrow input**: accept tables using the native encodings
  (`geoarrow.point`, `.linestring`, `.polygon`, `.geometry`, …), either
  normalized to WKB at the boundary via `geoarrow-cast` or consumed
  directly — `geoarrow-array` exposes the same `geo-traits` interface the
  lowering already uses, and the native separated-coordinate layout is
  actually *better* for lowering (contiguous f64 buffers, no per-feature
  WKB decode).
- **GeoParquet / FlatGeobuf IO** via the geoarrow-rs IO crates: a
  GeoParquet file becomes a one-step data source for `GeoShape`, without
  the GeoJSON→WKB ingest path.
- **GeoDataFusion ST_ UDFs** registered in Avenger's own
  `SessionContext` (PostGIS-modeled: constructors, accessors, measures,
  relationships, processing). Chart-pipeline uses:
  - **Spatial joins for choropleths**: point-in-polygon aggregation
    (`ST_Contains`/`ST_Intersects`) directly in the mark's data pipeline,
    instead of requiring pre-joined tables.
  - **`ST_Simplify`** as a data-side transform for zoom-appropriate
    geometry detail — complementing, not replacing, the pixel-space
    resampler (simplify controls input vertex count; the resampler
    controls projected curve fidelity).
  - **`ST_Centroid`** for polygon label anchors (pairs with a future
    `Text<Geo>`).
  - **Exact viewport filtering**: `ST_Intersects` against the view
    region as a second stage after the cheap bbox side-column prefilter.
  - **`ST_Extent`-style aggregates** replacing the hand-rolled bounds
    kernel from phase 4.
- **SedonaDB** is the heavyweight alternative (an embeddable engine that
  owns its own `SessionContext`); Avenger should prefer
  GeoDataFusion-style UDF registration inside its own context.

Caveats. DataFusion does not treat extension types as first-class, and
field metadata can be dropped as columns pass through projections and
UDFs in a plan. The design already tolerates this — geometry is an opaque
pass-through column and filters run on the bbox side-columns — but
"geometry type survives arbitrary user SQL" must be verified against the
upgraded DataFusion before it is promised. GeoArrow is spec 0.2 and
geoarrow-rs is 0.x with breaking releases, so WKB remains the canonical
internal representation; native arrays are accepted input at the
boundary, not a second internal format.

## 8. Pan/Zoom: Camera Model and the Web Mercator Blend

Precedent: Mapbox GL JS "adaptive projections" (v2.6). The insight is to
keep ordinary slippy-map interactions and make the *projection* adapt,
rather than inventing projection-specific gesture machinery: at low zoom
you see the authored projection; as you zoom in, the projection
continuously relaxes into Web Mercator, so street-level interaction is
indistinguishable from a normal map. This also solves the real problem
with pan/zoom on a fixed world projection — zooming into an Albers or
Winkel Tripel just magnifies its distortion, and pans start to feel skewed
away from the projection center.

### 8.1 Which projections qualify

The blend requires the authored projection to be injective over the view,
finite there, and to share Web Mercator's cut topology (one antimeridian
cut, one connected sheet). That classifies the catalog:

| Family | Examples | Pan model | Zoom-in strategy |
| --- | --- | --- | --- |
| Cylindrical / pseudocylindrical / world azimuthal | equirectangular, equal earth, natural earth, winkel tripel | re-center | blend toward Mercator |
| Conic | albers, lambert conformal conic | re-center + north alignment | blend + rotation alignment |
| Globe / hemisphere azimuthal (**deferred**) | orthographic, stereographic, gnomonic | spherical rotation (versor) | local flattening / handoff |
| Interrupted / composite (**deferred**) | goode homolosine, albers-usa | fit-only, planar zoom at most | none |

This note implements the first two rows. The pre-clip slot already encodes
the family split (antimeridian cut vs small-circle horizon), so a future
interaction layer can dispatch on it rather than maintaining a separate
taxonomy.

### 8.2 Camera model

- Camera state is `(center_lon, center_lat, zoom)`, north-up (no bearing
  gesture in v1). Pan re-centers the camera in geographic coordinates;
  zoom scales about the gesture anchor. Gestures never mutate the authored
  projection parameters (conic parallels, rotation) — those remain
  configuration.
- Zoom is scale-anchored to the Web Mercator convention: a given zoom
  level renders at the same ground scale it would on a `WebMercator`
  chart. This gives shared zoom vocabulary with
  `avenger-chart-webmercator` and keeps tile math trivial for the tile
  layer (section 10).
- Fit-to-data provides the initial camera; a configurable zoom floor at
  (or slightly below) the fitted view prevents zooming out into the void.

### 8.3 The adaptive blend

A projection is a pure function of `(λ, φ)`, so the pointwise combination

```text
P_t(λ, φ) = (1 − t) · P_authored(λ, φ) + t · P_mercator(λ, φ)
```

is itself a valid, smooth projection for every `t ∈ [0, 1]` (blended
projections are a classic construction — Winkel Tripel is the arithmetic
mean of equirectangular and Aitoff). `t` is driven by zoom: `t = 0` at and
below a low threshold `z0` (typical fitted world/region views show the
pure authored projection), `t = 1` at and above a high threshold `z1`,
smoothstepped between. Implementation is a `Blend` wrapper implementing
`RawProjection` (section 4); antimeridian cutting, adaptive resampling,
rectangle clipping, fit, and graticules all operate on `P_t` unchanged.

**Anchoring.** A naive blend makes the map drift and rotate as `t`
changes. Each frame applies an affine correction (translate + scale +
rotate) chosen so `P_t` matches position, ground scale, and the direction
of north at the camera center. For cylindrical/pseudocylindrical
projections the rotation term is identity; for conics it is what keeps
north-up coherent (Mapbox's bearing rule — north at the projection center
at low zoom, north at the screen center at high zoom — falls out of this
alignment).

**Inversion.** `P_t` has no closed-form inverse. Interaction inversion
(cursor → lon/lat) runs a few Newton iterations on the 2×2 Jacobian,
seeded from the closed-form inverse of the dominant endpoint
(`P_authored` for small `t`, Mercator for large `t`). Both endpoint
inverses already exist in the engine.

**Polar caveat.** Web Mercator is undefined beyond ±85.05°, so the blend
target degenerates as the camera center approaches a pole. v1 clamps:
hold `t = 0` while the view contains a pole (or clamp the blend anchor's
latitude). The principled fix — blending toward a *locally faithful*
target per region (Mercator at mid-latitudes, polar stereographic near the
poles, the UTM/UPS split) — is future work alongside the rotation family.

### 8.4 Rendering under interaction

View changes re-run the projection pipeline over mark geometry
(re-lowering, matching the WebMercator viewport-tool model). The
resampler's pixel-space precision keeps geometry cost proportional to what
is visible. If per-frame streaming of large `GeoShape` layers proves too
slow during gestures, d3 practice applies — relax `precision` for the
duration of the gesture and restore it on gesture end — and async layers
already have debounced preview materialization to lean on.

A free by-product of the blend machinery: animated projection transitions
(morphing a chart from equal earth to albers) are the same `Blend` wrapper
driven by an animation clock instead of zoom.

## 9. Replacing WebMercator

WebMercator is one raw projection of this system plus a tile layer and a
zoom/viewport vocabulary — all of which this plan builds in general form.
The camera model (section 8.2) deliberately shares Web Mercator zoom
semantics, the blend degenerates to the identity when the authored
projection *is* mercator, and the phase-5 tools mirror the WebMercator
tool surface. Replacement is therefore the end state, executed as the
final phase once the general machinery exists.

Parity requirements (tracked as phase 7):

- **Tile layer parity**: the generalized tile layer (section 10) renders
  mercator-grid tiles with an identity warp when the projection is
  mercator — behavior and baselines must match the current
  `tiles.rs`-based layer.
- **Pre-projected coordinates**: WebMercator marks accept
  `projected_x`/`projected_y` (data already in EPSG:3857 units) because
  positions flow through linear scales in projected units. `Geo` covers
  this with an identity/planar raw projection (the d3 `geoIdentity`
  analog) so pre-projected data bypasses the spherical pipeline.
- **View-change fast path**: WebMercator never re-projects on pan/zoom —
  view changes only touch linear scales. Under a fixed projection with a
  planar camera, `Geo` must recover this for point/rect marks (project
  once, apply the per-frame affine) so large scatters do not regress.
  Resampled line/shape geometry re-streams by necessity (precision is
  pixel-space), exactly as it would in d3.
- **Plumbing parity**: runtime params
  (`center_x`/`center_y`/`units_per_pixel`), shared-viewport containers,
  facet/repeat domain grouping, static export, and the existing visual
  baselines all port.

End state: retire `avenger-chart-webmercator` (preferred — the library is
not public, so there is no compatibility debt) or keep it as a thin
preset (mercator projection + tile config) if the authoring surface in
[`../webmercator.md`](../webmercator.md) is worth preserving verbatim.

## 10. Raster Tile Layers

Warp Web Mercator raster tiles onto any blendable projection,
generalizing the WebMercator tile layer. Precedent: Mapbox GL JS v2.6
adaptive projections (error-driven triangular tile meshes, per-region
zoom selection), OpenLayers raster reprojection (dynamic triangulation to
an error threshold), MapLibre v5's globe.

- **Coverage**: inverse-project the view region (corners plus edge
  samples) to a lon/lat coverage region, then select source tiles from
  the mercator grid. Per-region LOD: under a non-mercator projection one
  view legitimately mixes zoom levels, so sample the local ground scale
  at each candidate tile's center and pick that tile's zoom accordingly.
  The LOD mapping itself is direct because the camera's zoom is
  scale-anchored to Web Mercator (section 8.2).
- **Mesh warping**: subdivide each source tile into a triangle grid,
  project vertices tile-UV → lon/lat → `P_t` on the CPU in f64, and let
  the GPU rasterize the texture-mapped triangles. Subdivision is adaptive
  using the same projected-midpoint-deviation criterion as the section 4
  resampler. Antimeridian and plot-rect handling are mesh cuts/culling.
- **Blend interplay**: tiles warp through `P_t`, the same blended
  projection everything else renders through. As the camera zooms in and
  `t → 1`, the warp converges to the identity and tiles render as
  ordinary unwarped slippy-map tiles — the mesh path is only exercised
  at low/mid zoom, where few tiles are visible. When the authored
  projection *is* mercator, the warp is always the identity: this is the
  WebMercator parity case (section 9).
- **Reuse**: tile fetching, caching, resource-image guide rendering, and
  attribution come from the machinery shipped for WebMercator
  (`avenger-chart-webmercator/src/tiles.rs`; see
  [map-tiles.md](map-tiles.md)). Tile layers remain coordinate-owned
  configuration.
- **Non-mercator tile grids**: some providers (NASA GIBS) publish tiles
  natively in EPSG:4326 or polar stereographic WMTS matrix sets; those
  need only a generalized tile-grid abstraction (grid → per-tile lon/lat
  bounds), not new warping. v1 targets the mercator grid, but the grid
  abstraction should not hard-code it.

## 11. Phasing

### Phase 1 — `avenger-geo` projection engine

Pure math, independent of the chart stack; the bulk of the risk.

- `RawProjection` trait (object-safe) and raw projections:
  equirectangular, mercator, equal earth, natural earth 1, winkel tripel,
  conic equal-area (albers parameterization), conic conformal, and
  identity/planar for pre-projected data (section 9).
- Three-axis spherical rotation.
- Stream pipeline: antimeridian cutting → adaptive resampling
  (pixel-space precision) → scale/translate → rectangle post-clip, with
  the pre-clip slot structured to accept small-circle clipping later.
- `Blend` raw-projection wrapper, anchoring-affine solver, and Newton
  inversion (section 8.3).
- Sinks: lyon path builder, bounds accumulator; `fit_extent`/`fit_size`;
  graticule and sphere generators; serde-serializable `Projection`
  config.
- d3-geo golden-fixture harness (Node script over shared inputs: points,
  antimeridian-crossing lines, pole-enclosing polygons, graticules, per
  projection × rotation).

Exit: fixture suite green within tolerance; invert round-trips
property-tested; `Blend` equals its endpoints at `t = 0` / `t = 1`.

### Phase 2 — `Geo` coordinate system

- `Geo` implements `CoordinateSystemCore`/`CoordinateSystem`
  (typetag-serialized transform); `required_channels()` →
  `["longitude", "latitude"]` with no positional scales.
- Configuration: projection kind, rotate/center, precision, explicit view
  or fit-to-data.
- Domain/measurement providers realize the fitted view from lon/lat
  extents + plot area, with facet/repeat view sharing mirroring
  WebMercator's domain grouping.
- Graticule + sphere guide; interaction inversion (pixel → lon/lat).

Exit: visual baselines of graticule + sphere across the projection set
and rotations; fit baselines against known bounding boxes; a faceted
baseline with a shared view.

### Phase 3 — Point and line marks

- `Symbol<Geo>` with `longitude`/`latitude` channel builders; points
  outside the projection's valid region become undefined.
- `Line<Geo>` with `GeometrySpace` (`Coordinate` default = geodesic arcs
  through the resampler with antimeridian `defined` breaks; `Display` =
  straight projected segments).
- Lon/lat domain inference feeding fit.

Exit: flight-route baselines on equal earth and albers, including an
antimeridian-crossing route and a `Coordinate` vs `Display` comparison.

### Phase 4 — GeoJSON and `GeoShape`

- Ingest: GeoJSON → `geo-types` → ISO WKB `Binary` column + bbox
  side-columns; winding normalization; EWKB accepted on read.
- Geometry bounds feed fit alongside lon/lat channel extents.
- `GeoShape<Geo>`: WKB read zero-copy via `geo-traits`, streamed through
  the pipeline into `lyon_path::Path` → `ScenePathMark`; fill/stroke/
  tooltip channels encode joined columns.

Exit: world choropleth baseline (equal earth, tabular join); US states on
CONUS albers; a pole-enclosing polygon (Antarctica) baseline.

### Phase 5 — Pan/zoom tools

- Camera state (`center`, `zoom`) as runtime params, following the
  WebMercator param plumbing.
- Gestures: drag pan, wheel zoom, reset, Shift+drag box-zoom, with
  preview coverage — mirroring the WebMercator tool surface.
- Adaptive blend wiring: `t(zoom)` ramp, per-frame anchoring, Newton
  cursor inversion, polar clamp; gesture-time precision relaxation if
  profiling demands it.

Exit: interaction tests mirroring WebMercator tool coverage; blend
continuity check across the ramp (no visual jump); an albers-US
zoom-to-street demo.

### Phase 6 — Raster tile layers

- Tile-grid abstraction (mercator grid first, not hard-coded).
- Coverage + per-region LOD selection; adaptive mesh warping through
  `P_t`; identity fast path when the warp is trivial (section 10).
- Coordinate-owned tile configuration; fetching, caching, and attribution
  reused from the WebMercator machinery.

Exit: tile baselines under equal earth and albers at several zooms; pixel
parity with the existing WebMercator tile layer when the projection is
mercator.

### Phase 7 — WebMercator replacement

- View-change fast path: fixed projection + planar camera → project once,
  per-frame affine for point/rect marks; benchmark against current
  WebMercator pan/zoom on large scatters (no-regression gate).
- Pre-projected authoring surface (`projected_x`/`projected_y`
  equivalents) via the identity projection.
- Port `Symbol`/`Rect` authoring, runtime params, shared-viewport
  containers, static export, and visual baselines.
- Retire `avenger-chart-webmercator` (preferred) or reduce it to a thin
  preset over `Geo`; update [`../webmercator.md`](../webmercator.md) and
  [map-tiles.md](map-tiles.md).

Exit: all WebMercator baselines and interaction tests pass under `Geo`;
the old crate is removed or contains no projection/tile logic.

### Out of scope (separate notes)

Rotation-family projections (orthographic and other hemisphere
azimuthals: small-circle clipping, versor rotation, globe rendering);
bearing gestures; polar blend targets (stereographic near the poles);
composite projections (albers-usa — prefer concat panels with per-panel
`Geo` coordinate systems, e.g. Alaska with parallels 55°/65°); TopoJSON
ingest; GeoArrow/GeoDataFusion enablement per section 7.1 (follows the
planned DataFusion upgrade); graticule edge labels.

## 12. Open Decisions

- **Default projection**: equal earth (Vega-Lite's default) vs
  equirectangular (simplest). Proposal: equal earth.
- **Blend ramp**: zoom thresholds `z0`/`z1` and easing for the Web
  Mercator blend; per-family defaults vs authored options; and whether the
  blend is on by default or opt-in per chart.
- **Design spike — resampler/clipping fidelity**: validate the
  stream-pipeline port against d3-geo fixtures early (phase 1), especially
  spherical polygon clipping at the antimeridian, before marks build on
  it.
- **Design spike — fit + layout interaction**: confirm the
  measurement-provider pattern gives acceptable behavior when fit implies
  an aspect ratio the layout cannot honor (letterboxing vs stretching), and
  how that composes with faceting.
- **Graticule density policy**: fixed steps (d3 default: 10°) vs
  zoom-adaptive; start fixed.
- **Tile-grid abstraction scope**: v1 ships the mercator grid only;
  decide how much of the WMTS tile-matrix-set model the abstraction
  commits to up front.
- **Geometry column naming/convention** for `GeoShape` (dedicated channel
  vs well-known column name).
