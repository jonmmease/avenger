# Geo Coordinates

`avenger-chart-geo` provides general geographic map projections for
avenger charts: Equal Earth, Natural Earth, Winkel Tripel, Albers and
other conics, Mercator, and equirectangular, with graticule/sphere
guides, warped raster tiles, pan/zoom tools, and an adaptive Web
Mercator blend for street-level zooming. It lives outside the
`avenger-chart` facade crate and uses the same public coordinate, guide,
mark, resource, and tool contracts as other coordinate crates.

`Geo::mercator()` is the successor to the retired
`avenger-chart-webmercator` crate: same viewport semantics, tools, tile
layers, and pixel output (gated at ≥ 0.9999 image similarity against the
retired suite's baselines, preserved under
`avenger-chart/tests/baselines/webmercator/`).

The projection engine (d3-geo-compatible raw projections, spherical
rotation, antimeridian clipping, adaptive resampling) lives in the
`avenger-geo` crate; design background is in
[`future-work/geo-coordinate-system.md`](future-work/geo-coordinate-system.md).

## Position Authoring

The coordinate works in the projection's *raw planar units* (the y-up
plane produced by projecting with unit scale — radian-scale for
cylindrical projections):

- `x`: raw projected easting.
- `y`: raw projected northing.

Longitude/latitude authoring lowers to closed-form projection
expressions on those channels. `Symbol<Geo>::lon_lat(&geo, ...)` writes
the projected `x`/`y` channels (plus unscaled `lon`/`lat` channels used
by great-circle geometry and the adaptive blend). If data is already
projected, use `projected_x(...)` and `projected_y(...)`.

```rust
use avenger_chart::prelude::*;
use avenger_chart_geo::{Geo, GeoPositionChannels, Symbol};

let geo = Geo::albers_usa_conus();
let plot = Plot::with_coord(geo.clone())
    .mark(
        Symbol::new()
            .lon_lat(&geo, col("lon"), col("lat"))
            .size(120.0),
    );
```

`GeoShape` renders GeoJSON/WKB polygon geometry (see `register_geojson`
for ingest), streamed through the projection pipeline with antimeridian
cutting and adaptive resampling. `Line<Geo>` resamples great-circle
segments in its default coordinate geometry space.

## Viewport And Sharing

The coordinate owns the map viewport. Runtime state is `center_x`,
`center_y`, and `units_per_pixel` in raw units; public helpers expose
this as center plus slippy-style zoom (`world_width / (256 · 2^zoom)`).
If center or zoom is omitted, the view is inferred from projected mark
bounds:

- no center and no zoom: infer both from data;
- center only: hold center fixed and infer a zoom that contains data;
- zoom only: hold zoom fixed and infer center from data;
- center and zoom: preserve the authored viewport.

`GeoPanZoom` updates generated viewport params for pan, wheel zoom,
reset, and Shift-drag box zoom; the box-zoom preview matches the final
camera aspect. Shared viewports are supported for generated facet/repeat
groups; authored concat/grid shared viewport domains are rejected.

`GeoCoordMeasurement::invert_pixel(x, y)` inverts plot pixels back to
`(lon, lat)` for tooltips, including under an active blend (Newton
inversion of the blended projection).

## Adaptive Web Mercator Blend

`Geo::adaptive_blend(BlendConfig::default())` opts a projection into
Mapbox-style behavior: past the configured zoom range the authored
projection morphs pointwise into Web Mercator, anchored at the view
center so position, scale, and north stay fixed while the world
straightens into a slippy map. Blending is clamped off while the view
reaches beyond ±85° latitude.

## Tile Layers

Raster tiles are coordinate-owned guide content:

```rust
use avenger_chart_geo::{Geo, RasterTileLayer};

let coord = Geo::albers_usa_conus().tiles(
    RasterTileLayer::xyz("https://basemaps.cartocdn.com/rastertiles/voyager/{z}/{x}/{y}.png")
        .id("carto")
        .max_zoom(19)
        .attribution("© OpenStreetMap contributors © CARTO"),
);
```

Tiles come from the standard Web-Mercator XYZ grid regardless of the
authored projection. On non-Mercator projections each tile renders as a
textured triangle mesh (`SceneWarpedImageMark`) warped through the
projection — tile edges curve with the graticule — with per-region level
of detail: regions the projection compresses on screen load coarser
tiles. On unrotated Mercator with the blend inactive, tiles take an
identity fast path as plain axis-aligned image marks.

`RasterTileLayer::xyz(...)` supports `{z}`, `{x}`, `{y}`, and optional
`{s}` subdomains, immediate or smooth-zoom loading policies (fallback
zoom levels plus pan/zoom prefetch), data-URI sources for offline use,
and per-layer attribution. Tiles render below data marks (above the
sphere fill, below the graticule), clipped to the plot area, excluded
from domains, and non-interactive.

Interactive WGPU hosts request tile images without blocking pan/zoom;
pending images can render as placeholders and fill in on cache
invalidation. SVG and PDF exports resolve image resources before
rendering; warped tile meshes are software-rasterized and embedded as
images.

Tile providers are intentionally explicit: choose an appropriate tile
service, respect its usage policy, and provide required attribution.

## Antimeridian

Streamed geometry (GeoShape, great-circle lines, the graticule) is cut
at the projection's antimeridian. On the Mercator identity fast path,
tile addresses wrap horizontally across world copies. Warped-tile
discovery on other projections renders each tile once at its canonical
position; data marks are not duplicated across wrapped worlds.

## Export Example

See `avenger-chart-geo/examples/export_tiles.rs` for a deterministic
SVG/PDF export example using an inline data-URI tile layer.
