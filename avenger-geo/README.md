# avenger-geo

A Rust port of [d3-geo](https://github.com/d3/d3-geo) and selected
[d3-geo-projection](https://github.com/d3/d3-geo-projection) algorithms by
Mike Bostock and contributors. See [UPSTREAM.md](UPSTREAM.md) for provenance
and licenses.

Project geographic geometry into paths or polylines without a chart, data
engine, or renderer. The crate runs on native targets and WebAssembly.

`Projection` composes rotation, antimeridian clipping, adaptive great-circle
resampling, and rectangular clipping around a projection formula. The
`projections` module contains Equal Earth, Natural Earth, Mercator,
equirectangular, Winkel tripel, and two conic projections.
`Projection::planar` takes coordinates that are already projected.

Other crates add projections by implementing `ProjectionImpl`, which maps
longitude and latitude in radians to y-up planar units. The trait supplies a
numerical inverse, `bounds` can clip a projection that runs to infinity, as
Mercator does at its poles, and `validate` checks parameters when the
projection builds. Projections serialize through typetag, so an
implementation registers a unique name with `#[typetag::serde(name = "...")]`.
The pipeline cuts geometry at the antimeridian, so a projection's edge must be
the antimeridian, as it is for cylindrical, pseudocylindrical, and conic
projections.

A projection has no default view. Set its scale and translation with
`with_view`, or fit them: a `summarize` method measures geometry at a trial
view, and `fit` sets the view so that the geometry lands in a `range`, the
rectangle `[[x0, y0], [x1, y1]]` in output units. The projection keeps its
aspect ratio, so the geometry fills the range in one dimension and is centered
in the other. `build` returns an error for a projection without a view, for a
view, precision, or clip extent that isn't finite, and for parameters that
`validate` rejects, such as a conic standard parallel outside -90 to 90
degrees.

```rust
use avenger_geo::projections::EqualEarth;
use avenger_geo::{AvengerGeoError, Projection};

fn world_outline() -> Result<lyon_path::Path, AvengerGeoError> {
    let mut projection = Projection::new(EqualEarth);
    let summary = projection.summarize_sphere()?;
    projection.fit([[0.0, 0.0], [640.0, 360.0]], &summary)?;
    Ok(projection.build()?.sphere_path())
}
```

Spherical coordinates enter the pipeline as longitude/latitude **degrees**.
Projection formulas use radians and produce y-up planar units. Display
coordinates are y-down. A planar projection with `with_view(1.0, [0.0, 0.0])`
preserves input coordinates, and `reflect_y: true` flips their y axis.

Geometry is geo-types, or anything else that implements geo-traits, such as a
WKB buffer that `wkb::reader::read_wkb` reads. The whole sphere, which isn't a
geometry, has `Projector::sphere_path` and `Projection::summarize_sphere`.

Polygon rings follow RFC 7946: exterior rings wind counter-clockwise, around
the area on their left, and holes wind clockwise. On the sphere, the winding
selects the side of the ring that the polygon covers, so a clockwise exterior
covers the rest of the globe. Planar input follows the same rule in its own
coordinates: exteriors have positive signed area. Every edge is drawn as a
great circle, so an edge that a sparse file means as a straight line in
longitude and latitude, such as a border along a parallel with two vertices,
draws as an arc.

`Projector::project_point` projects individual points without clipping. The
pipeline clips complete geometry: `Projector::project_to_path` draws it as a
`lyon_path::Path`. Use point projection for markers and apply your viewport
visibility test before drawing them. Paths close polygon rings and keep
LineStrings open, in the same stream. Isolated points are left out. A
non-finite vertex breaks its line, and a broken ring stays open.
`Projector::project_to_sink` feeds the clipped geometry to any `GeoSink` for
other outputs.

Fitting returns an error for an empty range, for a summary without finite
bounds, and for point-sized bounds, and leaves the configuration unchanged.
Non-finite points are left out of a summary, so one bad vertex doesn't stop a
fit. Horizontal and vertical lines can be fitted. Summaries merge, so batches
of data fold from `FitSummary::default()`. `Graticule::lines` returns an error
for an invalid configuration, or when its vertex estimate exceeds one million.

`ingest::geojson_to_features` produces ISO WKB, recomputed bounds, feature IDs,
and JSON properties. String and numeric IDs remain separate from properties,
including for null geometry. Ring cleanup removes zero-area rings and retains
small nonzero polygons and holes. It rewinds rings to RFC 7946 by planar signed
area, which repairs files that wind them the other way. Planar area can't
identify the inside of a polar cap or a ring larger than a hemisphere, so pass
that geometry to the projector directly. It also misreads a polygon that
crosses the antimeridian without being split there, which RFC 7946 asks files
to avoid. The stream omits a ring's duplicated
closing vertex.

The built-in pipeline cuts at the antimeridian. It does not implement spherical
cap clipping or an orthographic projection.

## Arrow arrays

`Projection::summarize_geometry` and `Projector::project_to_paths` take any
GeoArrow array, which `geoarrow_array::array::from_arrow_array` makes from an
Arrow array and its field. `project_to_paths` returns one path per row for a
`ScenePathMark`, with an empty path for a null row. WKB and WKT rows are parsed
as they're read, and converting the array first with `geoarrow_array::cast`
parses them once. Rings are rewound to RFC 7946 by planar area, and zero-area rings are
dropped, as GeoJSON loading does, with the same limits. CRS metadata isn't interpreted: a spherical
projection reads coordinates as longitude/latitude degrees. All edges are drawn
as great circles, including those that GeoArrow's default `edges` treats as
planar.

`Projection::summarize_points` and `Projector::project_points` take longitude
and latitude as numeric Arrow arrays. `project_points` returns `x` and `y` for a
point mark, with NaN where a value is null or a result isn't finite, so the
mark draws nothing there.

`ingest::geojson_to_record_batch` loads GeoJSON as a record batch with one row
per feature:

- `geometry`: ISO WKB as a `geoarrow.wkb` field with CRS `OGC:CRS84`, and null
  for a feature with null geometry.
- `id`: the top-level feature id, only when some feature has one.
- One column per property, sorted by name. A column of numbers is Float64, a
  column of booleans is Boolean, and any other column is Utf8, with non-string
  values as their JSON text. A missing or null value is null.

Ids are typed by the same rule. A property named `geometry` is an error, as is
a property named `id` when features have top-level ids.

## Drawing maps

[avenger-guides-geo](../avenger-guides-geo) draws a projection's sphere and
labeled graticule into an Avenger scene graph, and its projection gallery
example draws the same geometry in six projections.

## Verification

```sh
cargo test --release -p avenger-geo --all-targets
cargo test --release -p avenger-geo --doc
cargo check --release --target wasm32-unknown-unknown -p avenger-geo --lib
```

Twenty-two fixtures compare projections, inverses, streamed geometry, bounds,
and fitting with d3-geo. To regenerate them, run `npm ci` in
`tools/geo-fixtures`, then run `npm run generate`.
