# avenger-geo

A Rust port of [d3-geo](https://github.com/d3/d3-geo) and selected
[d3-geo-projection](https://github.com/d3/d3-geo-projection) and
[d3-geo-polygon](https://github.com/d3/d3-geo-polygon) algorithms by Mike
Bostock and contributors. See [UPSTREAM.md](UPSTREAM.md) for provenance
and licenses.

Project geographic geometry into screen-space geometry without a chart, data
engine, or renderer, and turn that geometry into paths and points for scene
marks. The crate runs on native targets and WebAssembly.

`Projection` composes rotation, spherical clipping, adaptive great-circle
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
`clip` gives the projection's default spherical clip, which must match its
edge: the antimeridian, the default, or a circle for an azimuthal projection.
`rotation` gives a fixed rotation that the pipeline applies after the
caller's, as a transverse projection rolls the sphere by 90 degrees.

A projection has no default view. Set its scale and translation with
`with_view`, or fit them: a `summarize` method measures geometry at a trial
view, and `fit` sets the view so that the geometry lands in a `range`, the
rectangle `[[x0, y0], [x1, y1]]` in output units. The projection keeps its
aspect ratio, so the geometry fills the range in one dimension and is centered
in the other. `build` returns an error for a projection without a view, for a
view, precision, rotation, center, or clip extent that isn't finite, for a
center that projects to infinity, such as Mercator's pole, for a clip extent
beyond ±1e9, where the rectangle clip clamps points as d3's does, for
parameters that `validate` rejects, such as a conic standard parallel outside
-90 to 90 degrees, and for a spherical clip that can't clip.

```rust
use avenger_geo::paths::geometry_path;
use avenger_geo::projections::EqualEarth;
use avenger_geo::{AvengerGeoError, Projection};

fn world_outline() -> Result<lyon_path::Path, AvengerGeoError> {
    let mut projection = Projection::new(EqualEarth);
    let summary = projection.summarize_sphere()?;
    projection.fit([[0.0, 0.0], [640.0, 360.0]], &summary)?;
    Ok(geometry_path(&projection.build()?.project_sphere()))
}
```

Spherical coordinates enter the pipeline as longitude/latitude **degrees**.
Projection formulas use radians and produce y-up planar units. Display
coordinates are y-down. A planar projection with `with_view(1.0, [0.0, 0.0])`
preserves input coordinates, and `reflect_y: true` flips their y axis.

Geometry is geo-types, or anything else that implements geo-traits, such as a
WKB buffer that `wkb::reader::read_wkb` reads. The whole sphere, which isn't a
geometry, has `Projector::project_sphere` and `Projection::summarize_sphere`.

Polygon rings follow RFC 7946: exterior rings wind counter-clockwise, around
the area on their left, and holes wind clockwise. On the sphere, the winding
selects the side of the ring that the polygon covers, so a clockwise exterior
covers the rest of the globe. Planar input follows the same rule in its own
coordinates: exteriors have positive signed area. Every edge is drawn as a
great circle, so an edge that a sparse file means as a straight line in
longitude and latitude, such as a border along a parallel with two vertices,
draws as an arc.

Projection and drawing are two steps. `Projector::project` projects one
geometry through the full pipeline, clipping included, to geo-types geometry in
output units with y down:

| Input | Output |
| --- | --- |
| Point | Point, with NaN coordinates when the clip leaves it out |
| MultiPoint | MultiPoint of the points that the clip keeps |
| LineString, MultiLineString | MultiLineString |
| Polygon, MultiPolygon, Rect, Triangle | MultiPolygon |
| GeometryCollection | GeometryCollection of the points, lines, and polygons that remain |

Lines and polygons become multi-geometries because the antimeridian cut and the
clip can split one into several. Projected exterior rings have positive signed
area and holes negative, and each hole belongs to the smallest exterior that
contains it. A non-finite vertex breaks a line, and a ring leaves it out. A
ring also leaves out points that repeat the point before them and spikes, a
point between two that are one place. Data cut for planar maps has both:
Natural Earth's Antarctica runs along the antimeridian to the south pole and
back, which would make the clip's result turn on rounding.
Projected geometry is in output units, so it must not be summarized or
projected again.

`paths::geometry_path` draws screen-space geometry as a `lyon_path::Path`:
rings close, lines stay open, and points are left out. `Projector::project_point`
and `invert` work on single points without clipping, and
`Projector::project_to_sink` feeds the clipped stream to any `GeoSink` for other
outputs.

Fitting returns an error for an empty range, for a summary without finite
bounds, and for point-sized bounds, and leaves the configuration unchanged.
Non-finite points are left out of a summary, so one bad vertex doesn't stop a
fit. Horizontal and vertical lines can be fitted. Summaries merge, so batches
of data fold from `FitSummary::default()`. `FitSummary::aspect` is the bounds'
width over their height, at which a fit fills its range in both dimensions.
`Graticule::lines` returns an error
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

## Spherical clips

Before it projects, the pipeline keeps part of the sphere: the projection's
default clip, or the `SphericalClip` that `Projection::with_clip` sets. Every
method uses it, fitting included, so a fit frames the part that's kept.

- `Antimeridian` cuts geometry at the antimeridian of the rotated frame, the
  default for cylindrical, pseudocylindrical, and conic projections.
- `Circle { angle }` keeps the cap within `angle` degrees of the projection's
  center, which the rotation places. The angle lies strictly between 0 and
  180.
- `Region { polygon, frame }` keeps the inside of a spherical multipolygon in
  longitude and latitude degrees, whose edges are great circles and whose
  exteriors wind counter-clockwise, as RFC 7946 specifies. Its polygons have
  no holes. Its `frame` says where its coordinates belong.
  `RegionFrame::Earth` places the region on the earth, so it stays on the
  same places when the projection rotates, as a region that keeps a continent
  does. `RegionFrame::View` places it in the rotated frame, after the
  projection's fixed rotation too, so it stays in the same part of the map as
  the sphere turns, as a band of latitudes does.
  `SphericalClip::lon_lat_box` makes a region in a frame between two
  longitudes and two latitudes, with a vertex at least every degree along its
  north and south edges so that they follow their parallels, and crosses the
  antimeridian when `east` is less than `west`.

A circle or a region replaces the antimeridian cut. On a projection whose edge
is the antimeridian, it must not cross the rotated frame's antimeridian. A cap
of 90 degrees or less crosses it only at the poles. A view region that spans
every longitude, as `lon_lat_box(RegionFrame::View, -180.0, -60.0, 180.0,
80.0)` does, has edges along it at every rotation. An earth region with those
edges has them on the frame's antimeridian only while the projection doesn't
turn by longitude.

A projection's fixed rotation applies to its center too, so `with_center`
takes a longitude and latitude of the rotated frame for every projection.

## Arrow arrays

`Projection::summarize_geometry` and `Projector::project_geometry` take any
GeoArrow array, which `geoarrow_array::array::from_arrow_array` makes from an
Arrow array and its field. `project_geometry` projects each row as `project`
does, and returns a native GeoArrow array in output units with a row for each
input row, separated XY coordinates, and no CRS. A null row stays null. Point,
multipoint, line, and polygon arrays keep their kind, as the table above maps
it. Mixed geometry, collections, WKB, and WKT give mixed geometry, whose rows
follow their input rows. WKB and WKT rows are parsed as they're read, and
converting the array first with `geoarrow_array::cast` parses them once. Rings are rewound to RFC 7946 by planar area, and zero-area rings are
dropped, as GeoJSON loading does, with the same limits. CRS metadata isn't interpreted: a spherical
projection reads coordinates as longitude/latitude degrees. All edges are drawn
as great circles, including those that GeoArrow's default `edges` treats as
planar.

`paths::row_parts` splits a screen-space array for path marks and a symbol
mark that share row-aligned styles: a path of each row's polygons, a path of
its lines, which a fill would close, and every point with its row.
`ingest::lon_lat_points` makes a GeoArrow point array with CRS `OGC:CRS84` from
longitude and latitude columns, sharing the values of Float64 columns, for
tables that keep coordinates in two columns.

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
labeled graticule into an Avenger scene graph. Its graticule fits a
projection at each map size and sizes its labels for layout slabs, and its
projection gallery example lays out the same geometry on six maps.

## Verification

```sh
cargo test --release -p avenger-geo --all-targets
cargo test --release -p avenger-geo --doc
cargo check --release --target wasm32-unknown-unknown -p avenger-geo --lib
```

Twenty-eight fixtures compare projections, inverses, streamed geometry, spherical
clips, bounds, and fitting with d3-geo, d3-geo-projection, and d3-geo-polygon.
To regenerate them, run `npm ci` in `tools/geo-fixtures`, then run
`npm run generate`.
