# avenger-coords

The coordinate systems of plots: how a plot's pixels relate to the positions of its two positional dimensions, which tools such as brushes and navigations work in.

## Model

A plot's coordinate system is its kind, the scales of its positional dimensions, and its projection. A scale maps the data of one dimension to a range position, and the plot's projection maps the range positions of both dimensions to the plot's pixels and back. The range of each scale spans its dimension's extent, the positions that the plot shows. A dimension without a scale, such as a map's longitude, takes its data as its positions.

## Projections

`PlotProjection` is the projection of a plot at one size, from the plot's top-left corner:

- `position` and `pixel` convert one point between pixels and range positions. A pixel where the plot shows no position, such as outside a globe, has none, and a position that the plot does not show, such as behind a globe, has no pixel.
- `extent` gives the range positions that the plot shows along each dimension, and `period` gives the period of a dimension that wraps, such as 360 degrees of longitude.
- `is_linear` tells whether straight lines in positions draw as straight lines in pixels, so that paths need no resampling.
- `path` and `polygon` draw a path and a polygon of positions in pixels as the plot bends them, cut where the plot shows no position, with each side of a polygon the short way around a dimension that wraps.
- `enclosed` turns a closed path in pixels, such as a lasso, into polygons of positions, each within one period of a dimension that wraps, so that a planar test tells whether the path encloses a position.

`CartesianProjection` projects a Cartesian plot, whose positions are its pixels. `PolarProjection` projects a polar plot, whose positions are angles in degrees clockwise from the top and distances in pixels from the center, and whose angle wraps around a whole circle.

## Maps

The `geo` feature adds `MapProjection`, the projection of a map through the projector of its cartographic projection from avenger-geo. Its positions are longitudes and latitudes, and longitude wraps every 360 degrees. Its paths run at a steady change of longitude and latitude, so the edge of a box follows a parallel, and its polygons and enclosed paths cut at the antimeridian and at the horizon of a globe. An enclosed path that holds a pole closes through it, so a planar test of longitude and latitude, as well as the spherical test of `avenger_geo::polygon_contains`, tells whether a lasso holds a place.
