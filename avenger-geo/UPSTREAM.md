# Upstream sources

The spherical streaming pipeline and projection formulas are Rust ports of
[d3-geo](https://github.com/d3/d3-geo) and
[d3-geo-projection](https://github.com/d3/d3-geo-projection).
Module headers identify the corresponding JavaScript source files.
The complete upstream license notices are in `licenses/`.

The fixture generator uses d3-geo 3.1.1 and d3-geo-projection 4.0.0.
Its npm lockfile pins these versions. Run `npm ci` and `npm run generate`
from `tools/geo-fixtures` to reproduce the fixtures.

The initial Rust implementation was imported from commit
`c261e77c5a43f938972e6642eb5cdd0840a48b60` of this repository.
