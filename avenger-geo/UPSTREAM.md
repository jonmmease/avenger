# Upstream sources

The spherical pipeline and projection formulas are Rust ports of d3-geo and
d3-geo-projection, and the region clip is a port of d3-geo-polygon's polygon
clip. Module headers identify the JavaScript source files.
The pinned parity references are [d3-geo 3.1.1](https://github.com/d3/d3-geo/tree/v3.1.1),
[d3-geo-projection 4.0.0](https://github.com/d3/d3-geo-projection/tree/v4.0.0),
and [d3-geo-polygon 2.0.1](https://github.com/d3/d3-geo-polygon/tree/v2.0.1).
The initial port's exact upstream commit was not recorded.

Local adaptations include Rust geometry sinks, GeoJSON/WKB ingestion,
RFC 7946 ring winding in place of d3's spherical convention, a required view,
an open set of projections, and a numerical inverse for Winkel tripel. A
view region clip lies in the rotated frame, as d3-geo-polygon's does, and an
earth region clip stays on the same places when the projection rotates. A
projection's fixed rotation, which transverse Mercator uses, also places its
center, where d3's transverse Mercator swaps its center's axes. A
projection's bounds clip it within a clip extent in both directions, where
d3's Mercator keeps the clip extent's vertical span and its transverse
Mercator the horizontal one. On the sphere, a ring leaves out points that
repeat the point before them and spikes, where d3 streams them as they are.
Aitoff's inverse finds no point where its
iteration settles away from the target, where d3's returns that point.
The [README](README.md) describes supported behavior and limitations.

Complete notices: [d3-geo](licenses/d3-geo.txt),
[d3-geo-projection](licenses/d3-geo-projection.txt),
[d3-geo-polygon](licenses/d3-geo-polygon.txt), and
[Avenger's BSD license](LICENSE).
