# Upstream sources

The spherical pipeline and projection formulas are Rust ports of d3-geo and
d3-geo-projection. Module headers identify the JavaScript source files.
The pinned parity references are [d3-geo 3.1.1](https://github.com/d3/d3-geo/tree/v3.1.1)
and [d3-geo-projection 4.0.0](https://github.com/d3/d3-geo-projection/tree/v4.0.0).
The initial port's exact upstream commit was not recorded.

Local adaptations include Rust geometry sinks, GeoJSON/WKB ingestion,
projection blending and anchoring, and a numerical inverse for Winkel tripel.
The [README](README.md) describes supported behavior and limitations.

Complete notices: [d3-geo](licenses/d3-geo.txt),
[d3-geo-projection](licenses/d3-geo-projection.txt), and
[Avenger's BSD license](LICENSE).
