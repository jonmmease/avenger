# Geo Raster Marks: CRS-Tagged Rasters on the Geo Coordinate System

Status (2026-07-04): Phases 1 and 2 implemented. `geometry.crs` +
`Rasterize2D::frame(...)`, the shared raster-mark core in
avenger-chart-marks, `Mark<Geo> for UniformRaster2D<Geo>` with identity and
warped dispatch (`warped_raster_mesh` with `MercatorY`/`Latitude` axes),
the `avenger_chart_geo::crs` helpers, f64 view-domain params, and the four
examples (`taxi_geo_mercator`, `taxi_geo_albers`, `climate_grid_equal_earth`,
`georegistered_overlay`) all landed on 2026-07-04. Phase 3 (GPU residency
for large inline rasters) and the faceted-raster example remain future
work. §§2–4 below describe the pre-implementation state and design; code
references were verified on the design date.

## 1. Goal

Display datashader-style aggregated rasters — the NYC taxi density raster is
the motivating case — on `Plot<Geo>`: over basemap tiles on
`Geo::mercator()`, and warped correctly on non-Mercator projections and under
the adaptive Mercator blend. Secondarily, make externally produced uniform
grids (EPSG:4326 climate/ocean grids of the xarray/NetCDF world, pre-projected
EPSG:3857 imagery) first-class raster inputs.

The design rests on one schema decision — **the raster carries its
coordinate reference system** — plus a `Mark<Geo>` implementation that
dispatches between axis-aligned and warped rendering exactly the way the tile
guide already does.

## 2. What exists today (verified)

The pieces are all present and unjoined:

- **`Rasterize2D` bins in raw data-domain units and is frame-agnostic.**
  `bin_index` is a pure linear map over whatever units the x/y expressions
  produce (`avenger-chart-transforms/src/rasterize_2d.rs:1630`). The taxi
  parquet's `pickup_x/pickup_y` are already EPSG:3857 meters. Extents and bin
  counts are config expressions, typically wired to view params
  (`v.x().domain_start()`, `v.x().pixels()`).
- **The raster Arrow struct is self-describing — except for its frame.** The
  output row carries `geometry.dimensions[].coords{start, stop, count}`
  (`rasterize_2d.rs:1642-1669`), which is what makes stale-retarget and
  cross-view reuse work with no side-channel state. An extent without a frame
  is only half a description; the frame currently lives in the example
  author's head. `raster-arrow-representations.md` sketched CRS metadata for
  this slot; it never landed.
- **`UniformRaster2D<C>` is generic; only `Mark<Cartesian>` exists**
  (`avenger-chart-marks/src/uniform_raster_2d.rs:16`,
  `avenger-chart-cartesian/src/marks/uniform_raster_2d.rs:55`). The Cartesian
  scene path scales the raster's extent corners through the x/y scales and
  emits one `SceneImageMark` (`aspect: false`), with a two-tier
  (pointer-identity + content-hash) RGBA image cache on the compiled mark.
- **Geo's x/y scales are exactly the right target.** They are coordinate-owned
  forced-linear scales over authored-projection raw units with no
  zero/nice/round (`avenger-chart-geo/src/coord.rs:544-561, 450-479,
  757-777`); their pixel mapping is identical to `tile_pixel_rect`
  (`tiles.rs:1493`). A raster rect positioned through them lands exactly where
  a tile would on an identity-Mercator view.
- **The warp substrate is complete.** `SceneWarpedImageMark` is a general
  scene primitive with GPU, SVG, and PDF paths, participates in preview
  retargeting (`avenger-chart/src/plot/compiled/rendering.rs:951-962`), and
  can opt into the persistent tile texture arrays. `tile_mesh`
  (`tiles.rs:1311`) is tile-specific only in the four lines that derive its
  bounds (`tiles.rs:1318-1321`); its kernel — lerp longitude in u, lerp raw
  mercator-y in v, project through the blend-aware view projector, adaptive
  dyadic-lattice subdivision, straddle culling — is already rect-generic.
- **Gaps.** (a) `View::cartesian()` is the only view kind and
  `resolved_view_params` gates on Cartesian
  (`avenger-chart-core/src/view.rs:136`,
  `avenger-chart/src/plot/compiled/mark_data_runtime.rs:1715`); Geo satisfies
  every input the params need (x/y numeric domains, plot px) but is not
  matched. (b) View domain params round-trip through `f32`
  (`mark_data_runtime.rs:1754`) — ~1 m quantization at EPSG:3857 magnitudes.
  (c) Nothing chart-level emits `SceneWarpedImageMark`; today it is
  guide-only. (d) `MaterializationOutputKind::RgbaImage` exists but nothing
  produces or accepts it; this design does not need it (the RecordBatch path
  is correct — pixels are a per-mark derived cache).

## 3. Design

### 3.1 The raster carries its CRS

Add one nullable field to the raster geometry struct:

```text
geometry: {
    kind: "grid",
    crs:  Utf8 (nullable),          // NEW — e.g. "epsg:3857", "epsg:4326"
    dimensions: [ { name, coords{...} } ],
}
```

Rules:

- **Open string vocabulary, geometry-level, optional.** Absent means "chart
  data units" — today's behavior, fully backward compatible. The tag is
  joint-2D (a CRS is a property of the plane, not of one axis).
- **Core stays CRS-ignorant.** No CRS enum in core crates. The Cartesian mark
  accepts an absent tag and errors on any tag it does not understand
  ("raster declares CRS 'epsg:3857'; Cartesian plots have no CRS — drop
  `.frame(...)` or use a Geo plot"). Interpretation lives in the consuming
  mark crate; an external coordinate-system crate can define and consume its
  own tags without touching core.
- **Declared, not inferred.** Binning is unit-agnostic, so the transform
  cannot derive the frame; `.frame(...)` is an assertion by the author (or by
  a future file-ingestion path reading real CRS metadata). Its value is that
  mismatches become typed errors instead of silently misplaced images — the
  EPSG:3857-meters vs mercator-radian-units factor (R = 6 378 137) is
  otherwise invisible until the image lands 6.4 million times too small.

Binning happens in the **native CRS units** of the data. The raster artifact
then means something independent of any chart — portable, cacheable,
comparable with external tools — and all conversion happens at the display
seam inside the geo crate.

### 3.2 `UniformRaster2D<Geo>`: tile-style dispatch

A `Mark<Geo>` implementation with the same builder surface as the Cartesian
one. At scene build it converts the raster's extent from its declared CRS
into the authored plane, then dispatches exactly like the tile guide
(`tiles.rs:1483`):

- **Identity fast path** — authored projection is unrotated Mercator, blend
  inactive, and the raster CRS is affine to the authored plane (EPSG:3857 is:
  a pure ×1/R scale; absent-tag data likewise): scale the converted extent
  through the x/y `ConfiguredScale`s and emit one axis-aligned
  `SceneImageMark`, reusing the Cartesian implementation's positioning, flip
  handling, and RGBA cache.
- **Warped path** — everything else (rotated/non-Mercator projections, active
  blend, or a CRS that is *not* affine to the plane, e.g. EPSG:4326 whose
  y-axis is nonlinear in mercator-y): build a mesh over the raster's rect
  with the generalized kernel (§3.3) through `measurement.view_projector()`
  and emit `SceneWarpedImageMark`. Blend correctness is free because the
  projector embeds it — the same guarantee tiles rely on. Note EPSG:4326
  rasters take the warped path even on identity-Mercator views.

Both scene forms already participate in preview retargeting, so the
stale-raster pan/zoom experience matches today's Cartesian behavior.

This mark must **not** copy the `Rect<Geo>` pattern of ignoring the blend
(rect renders in the authored plane at blend_t > 0 while tiles/symbols/lines
render in the displayed plane — a known inconsistency, tracked separately).

### 3.3 Generalized raster mesh

Extract the kernel of `tile_mesh` into a rect-generic function in
`avenger-chart-geo`:

```rust
/// The plane a uniform raster's v (row) axis is linear in.
pub enum RasterVAxis {
    /// Rows uniform in raw mercator y (EPSG:3857 sources, tile images).
    MercatorY { top: f64, bottom: f64 },
    /// Rows uniform in geodetic latitude (EPSG:4326 sources).
    Latitude { north: f64, south: f64 },
}

pub fn warped_raster_mesh(
    lon_west: f64,
    lon_east: f64,
    v_axis: RasterVAxis,
    projector: &Projector,
    precision_px: f64,
    plot_width: f32,
    plot_height: f32,
) -> Option<TileMesh>;
```

`tile_mesh` becomes a thin wrapper (tile bounds → `MercatorY`). The only new
code is the `Latitude` branch of `point_at` (lerp latitude directly instead
of inverting mercator-y); the memoized 33×33 lattice, adaptive subdivision,
straddle culling, and offscreen culling are shared unchanged.

### 3.4 View scopes on Geo

`View::cartesian()` semantics are really "linear x/y domains + plot pixels",
which Geo satisfies verbatim. Lift the Cartesian gate in
`resolved_view_params` to accept any coordinate system whose x/y
`ConfiguredScale`s expose numeric interval domains (Geo qualifies; the
existing error remains for anything else). No new view kind is needed; if the
name bothers us later, `View::linear_xy()` can alias it.

Two required companions:

- **f64 view params.** Domain start/end params must pass through as f64
  end-to-end. At EPSG:3857 magnitudes the current f32 hop quantizes extents
  to ~1 m, which breaks deep zoom; with CRS-native binning this fix is
  non-optional.
- **Extent conversion helpers.** View params arrive in authored raw units;
  extent config expressions must be in the raster's CRS. The geo crate
  provides closed-form expression helpers (matching the existing
  builtins-only rule for proto-serializable exprs, `expr.rs:1-12`):

```rust
// avenger-chart-geo
pub mod crs {
    pub const EPSG_3857: &str = "epsg:3857";
    pub const EPSG_4326: &str = "epsg:4326";
    pub const WEB_MERCATOR_RADIUS_M: f64 = 6_378_137.0;

    /// Authored-plane (unrotated Mercator) raw-unit exprs → CRS units.
    /// 3857: x·R, y·R. 4326: x·180/π, atan(sinh(y))·180/π.
    pub fn from_mercator_units_x(crs: &str, expr: Expr) -> Result<Expr, _>;
    pub fn from_mercator_units_y(crs: &str, expr: Expr) -> Result<Expr, _>;
}
```

## 4. Public API summary

```rust
// avenger-chart-transforms — one addition
impl Rasterize2D {
    /// Declare the CRS of the x/y input expressions and extents.
    /// Stamped into geometry.crs; consuming marks validate/convert.
    pub fn frame(self, crs: impl Into<String>) -> Self;
}

// avenger-chart-geo — new
impl Mark<Geo> for UniformRaster2D<Geo> { /* same builder surface */ }
pub mod crs { /* constants + conversion expr helpers, §3.4 */ }
pub fn warped_raster_mesh(...) -> Option<TileMesh>;   // §3.3
pub enum RasterVAxis { ... }

// avenger-chart-core — behavior changes, no new API
// - resolved_view_params accepts Geo (linear x/y + plot px)
// - view domain params carried as f64
```

Authoring shape (taxi on a map):

```rust
let geo = Geo::mercator()
    .tiles(RasterTileLayer::xyz(OSM).attribution("© OpenStreetMap contributors"))
    .adaptive_blend(4.0, 12.0);
let v = View::cartesian().id("density").x_domain(col("x")).y_domain(col("y"));

let raster = Rasterize2D::new(col("pickup_x"), col("pickup_y"))
    .frame(crs::EPSG_3857)
    .x_extent(crs::from_mercator_units_x(crs::EPSG_3857, v.x().domain_start())?, ...)
    .x_bins(v.x().pixels() / lit(2.0))
    .agg_count();

Plot::with_coord(geo).mark(
    UniformRaster2D::new()
        .view(v)
        .raster_with(raster.raster(), |r| r.x_with(raster.x_dim(), ...)
                                           .y_with(raster.y_dim(), ...))
        .fill(...),  // color scale + colorbar as today
)
```

## 5. Examples enabled

1. **`taxi_geo_mercator`** — the NYC taxi density raster over OSM tiles on
   `Geo::mercator()` with `GeoPanZoom`, including the adaptive raster↔scatter
   switch from `taxi_adaptive_points`. (Phase 1.)
2. **`taxi_geo_albers`** — the same raster on Albers USA / under the adaptive
   blend: the density image warps in lockstep with tiles, shapes, and
   graticule through every blend step. (Phase 2.)
3. **`climate_grid_equal_earth`** — an EPSG:4326-uniform grid (temperature or
   sea-surface anomaly, NetCDF→Arrow fixture) on Equal Earth with graticule
   and colorbar: the xarray world's most common raster, displayed without
   resampling. (Phase 2.)
4. **`georegistered_overlay`** — a pre-rasterized, frame-stamped external
   image (e.g. a radar composite in EPSG:3857) placed with no `Rasterize2D`
   at all, demonstrating that the CRS tag, not the transform, is what makes a
   raster displayable. (Phase 2.)
5. **Faceted geo rasters** — `partition_by` already yields one raster row per
   group; per-borough small multiples follow from existing faceting. (Free
   once Phase 1 lands; worth an example.)

## 6. Phases

1. **Identity Mercator** — `Mark<Geo>` impl (fast path only, EPSG:3857 +
   absent-tag), `geometry.crs` field, `.frame(...)`, view-param gate lift,
   f64 params, `crs` helpers. Delivers example 1.
2. **Warped path** — `warped_raster_mesh` extraction + `Latitude` kernel,
   warped dispatch in the mark, 4326 support. Delivers examples 2–4.
3. **Polish** — route large inline raster images through the persistent GPU
   residency system (today `SharedInline` images re-upload via the atlas
   every prepared frame, ~2–4 MB/frame during pan: tolerable, but the known
   next cost); revisit `rectilinear` coords from
   `raster-arrow-representations.md` if a nonuniform-grid source appears.

## 7. Testing

- Schema round-trip + absent-tag compatibility; Cartesian mark errors on
  unknown CRS (typed message).
- Extent conversion unit tests (3857 ×R both axes; 4326 gudermannian y),
  matched against `mercator_y`/`tile_bounds_lonlat`.
- Kernel parity: `warped_raster_mesh` with `MercatorY` bounds of a tile
  reproduces `tile_mesh` bit-for-bit (it is the same code path); at
  identity-Mercator + 3857 the warped path agrees with the fast path within
  the same fp-at-seams tolerance measured for tiles (~0.9998 cross-path).
- Visual: deterministic synthetic point fixture (not the taxi parquet) for
  each example class; blend sweep t ∈ {0, 0.5, 1} for the warped raster.
- Adaptive: view pan/zoom re-rasterizes at f64 precision (regression for the
  ~1 m f32 quantization); stale-preview retarget covers both SceneImageMark
  and SceneWarpedImageMark forms.

## 8. Open decisions

- **Extent-helper ergonomics.** The `from_mercator_units_*` wrapping is
  explicit and a little verbose; a later sugar could let the view accessor
  produce CRS-converted params directly (`v.x().domain_start_in(crs)`), which
  requires the view to know the plot's authored plane — deferred until the
  explicit form proves annoying.
- **CRS vocabulary depth.** Start with exactly two tags. Full proj-string /
  arbitrary-EPSG support belongs to a future ingestion crate, not here.
- **Non-Mercator-authored fast paths.** An EPSG:3857 raster on an authored
  *Albers* plot always warps; a hypothetical "authored-plane-uniform" raster
  (binned in Albers units) would be axis-aligned there. Supported implicitly
  by the absent tag + `View` domains; not worth naming until someone needs it.
- **`Rect<Geo>` blend inconsistency** — separate fix, tracked independently;
  noted here only so the raster mark is not built by analogy to it.
