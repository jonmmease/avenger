# WebMercator Coordinates

`avenger-chart-webmercator` provides an EPSG:3857 / XYZ-tile coordinate system
for map-like charts. It lives outside the `avenger-chart` facade crate and uses
the same public coordinate, guide, mark, resource, and tool contracts as other
coordinate crates.

## Position Authoring

WebMercator uses projected meters internally:

- `x`: EPSG:3857 easting.
- `y`: EPSG:3857 northing.

Longitude/latitude authoring is a convenience layer on top of those projected
channels. `Symbol<WebMercator>::longitude(...)` writes the projected `x`
channel, and `latitude(...)` writes the projected `y` channel. If data is
already projected, use `projected_x(...)` and `projected_y(...)` instead.

There are no longitude or latitude scale types in this design. Data fitting,
viewport realization, tile math, interaction inversion, and static export all
operate in projected x/y space after longitude/latitude expressions have been
lowered.

```rust
use avenger_chart::prelude::*;
use avenger_chart_webmercator::{
    Symbol, WebMercator, WebMercatorSymbolPositionChannels,
};

let plot = Plot::with_coord(WebMercator::new())
    .mark(
        Symbol::new()
            .longitude(col("lon"))
            .latitude(col("lat"))
            .size(120.0),
    );
```

## Viewport And Sharing

The coordinate owns the map viewport. Runtime state is represented as
`center_x`, `center_y`, and `units_per_pixel`; public helpers expose this as
center plus slippy-map zoom. If center or zoom is omitted, WebMercator infers
the missing viewport parts from projected mark bounds:

- no center and no zoom: infer both from data;
- center only: hold center fixed and infer a zoom that contains data;
- zoom only: hold zoom fixed and infer center from data;
- center and zoom: preserve the authored viewport.

`WebMercatorPanZoom` updates generated viewport params for pan, wheel zoom,
reset, and Shift-drag box zoom. The box zoom preview uses the current viewport
aspect ratio, so the preview rectangle matches the final camera framing.

Shared WebMercator viewports are supported for generated facet/repeat groups.
Authored concat/grid shared viewport domains are rejected because cells can have
different plot-area sizes and authored sharing does not yet provide a single
compatible camera group.

## Tile Layers

Raster tiles are coordinate-owned guide content:

```rust
use avenger_chart_webmercator::{RasterTileLayer, WebMercator};

let coord = WebMercator::new().tiles(
    RasterTileLayer::xyz("https://tile.openstreetmap.org/{z}/{x}/{y}.png")
        .id("osm")
        .max_zoom(19)
        .attribution("OpenStreetMap contributors"),
);
```

`RasterTileLayer::xyz(...)` supports `{z}`, `{x}`, `{y}`, and optional `{s}`
subdomains. Tiles are rendered as resource-backed image scene marks below data
marks, clipped to the plot area, excluded from domains, and non-interactive by
default.

Interactive WGPU hosts request tile images without blocking pan/zoom. Missing
or pending images can render as placeholders and fill in after the image cache
requests render invalidation. SVG, PDF, and documentation PNG exports resolve
image resources before rendering and embed final pixels in the exported output.

Tile providers are intentionally explicit. Avenger does not currently provide a
built-in OpenStreetMap policy object; authors should choose an appropriate tile
service, respect its usage policy, and provide required attribution through
`RasterTileLayer::attribution(...)`.

## Antimeridian

Tiles wrap horizontally across the antimeridian when computing XYZ tile
addresses. Data marks are not duplicated across wrapped worlds yet, so symbols
near +/-180 degrees render at their projected positions rather than appearing
in every visible wrapped copy of the world.

## Export Example

See `avenger-chart-webmercator/examples/export_tiles.rs` for a deterministic
SVG/PDF export example using an inline data-URI tile layer.
