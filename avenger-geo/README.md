# avenger-geo

A Rust port of [d3-geo](https://github.com/d3/d3-geo) and selected
[d3-geo-projection](https://github.com/d3/d3-geo-projection) algorithms by
Mike Bostock and contributors. See [UPSTREAM.md](UPSTREAM.md) for provenance
and licenses.

Project geographic geometry into paths or polylines without a chart, data
engine, or renderer. The crate runs on native targets and WebAssembly.

`Projection` composes rotation, antimeridian clipping, adaptive great-circle
resampling, and rectangular clipping. The catalog contains Equal Earth,
Natural Earth, Mercator, equirectangular, Winkel tripel, two conic projections,
and planar identity. `BlendRaw` interpolates projections. Anchoring helpers
keep a selected location and its local north direction stable.

```rust
use avenger_geo::{LyonPathSink, Projection, ProjectionKind, Sphere};

let mut projection = Projection::new(ProjectionKind::EqualEarth);
projection.fit_size([640.0, 360.0], &Sphere).unwrap();
let projector = projection.build();
let mut sink = LyonPathSink::new();
projector.stream(&Sphere, &mut sink);
let outline = sink.finish();
```

Spherical coordinates enter the pipeline as longitude/latitude **degrees**.
Raw projections use radians and produce y-up planar units. Display coordinates
are y-down. Planar identity preserves input coordinates at unit scale and zero
translation. `reflect_y: true` flips their y axis.

`Projector::project` projects individual points without clipping.
`Projector::stream` applies clipping to complete geometry. Use point projection
for markers and apply your viewport visibility test before drawing them.
`LyonPathSink` and `PolylineSink` preserve closed polygon rings and open
LineStrings in the same stream. Isolated points are ignored. Non-finite
vertices break a line, and broken rings remain open.

`fit_extent` and `fit_size` return an error for empty, non-finite, or
point-sized bounds and leave the configuration unchanged. Horizontal and
vertical lines can be fitted. `Graticule::try_lines` checks configuration and
limits its vertex estimate to one million. `lines` panics on the same errors.

`ingest::geojson_to_features` produces ISO WKB, recomputed bounds, feature IDs,
and JSON properties. String and numeric IDs remain separate from properties,
including for null geometry. Ring cleanup removes zero-area rings and retains
small nonzero polygons and holes. It reverses RFC 7946 polygon winding to the
d3 convention using planar signed area. Use `Streamable` directly for geometry
that already has spherical winding, including polar caps. Direct polygon rings
must be closed. The stream omits their duplicated closing vertex.

`ingest::WkbStreamable::new` validates a borrowed WKB buffer once. The result
can be reused for fitting and streaming. Invalid WKB returns an error before
any geometry is emitted.

Projection blending can introduce singularities. Its numerical inverse can
return `None`. The built-in pipeline cuts at the antimeridian. It does not
implement spherical cap clipping or an orthographic projection.

## Example and verification

```sh
cargo run --release -p avenger-geo --example projection_gallery
cargo test --release -p avenger-geo --all-targets
cargo test --release -p avenger-geo --doc
cargo check --release --target wasm32-unknown-unknown -p avenger-geo --lib
```

The example writes SVG and PNG files to `target/geo-gallery`. An optional
output directory is accepted as its first argument. PNG labels use installed
system fonts. The runtime crate has no font or rasterizer dependency.

![Projection gallery](docs/images/projections.png)

Twenty-two fixtures compare projections, inverses, streamed geometry, bounds,
and fitting with d3-geo. To regenerate them, run `npm ci` in
`tools/geo-fixtures`, then run `npm run generate`.
