# avenger-coords

Coordinate systems of plots: how a plot's pixels relate to its positions, which have two dimensions. Tools such as brushes and navigations work in those positions.

## Model

A plot's coordinate system consists of its kind, the scales of its positional data, and its projection. The projection maps the plot's positions, which have two dimensions, to the plot's pixels and back.

In a Cartesian, polar, or geographic plot, each dimension of a position is one positional dimension of the plot. Its scale maps the dimension's data to a position in the scale's range, which spans the dimension's extent, and a dimension without a scale, such as a map's longitude, takes its data as its positions. A plot whose positions combine several columns has positions that no single scale gives. A ternary plot's dimensions, for example, are two of its three proportions, with the third implied, and the plot shows a triangle of them rather than the box of its extent.

## Projections

`PlotProjection` is the projection of a plot at one size, from the plot's top-left corner:

- `position` and `pixel` convert one point between pixels and range positions. A pixel where the plot shows no position, such as outside a globe, has none, and a position that the plot does not show, such as behind a globe, has no pixel.
- `pixels` converts the positions of a batch of rows to pixels, as `Positions`: a Float32 column from a scale or a Float64 column from data along each dimension. It returns x and y as Float32 arrays, null where a position is null or the plot does not show it. The default calls `pixel` for each row, and `CartesianProjection` reuses a Float32 column as its pixels.
- `extent` gives the bounds of the range positions that the plot shows along each dimension, and `period` gives the period of a dimension that wraps, such as 360 degrees of longitude.
- `region` gives the polygon of positions that the plot shows, by default the box of the extent, and `clamp` moves a position into it, where a gesture that leaves the plot stays. A projection whose region is not a box, such as a ternary plot's triangle, gives both.
- `is_linear` tells whether straight lines in positions draw as straight lines in pixels, so that paths need no resampling.
- `path` and `polygon` draw a path and a polygon of positions in pixels as the plot bends them, cut where the plot shows no position, with each side of a polygon the short way around a dimension that wraps. The default `path` draws a straight line for a linear projection, and otherwise halves its steps until each lies within a quarter of a pixel of the path. Only a projection with its own way to draw lines, such as a map's projector, which clips them, replaces it.
- `enclosed` turns a closed path in pixels, such as a lasso, into polygons of positions, each within one period of a dimension that wraps, so that a planar point-in-polygon check tells whether the path encloses a position.
- `shows` tells whether the region holds a position, `outline` gives the region in pixels as the rings that a background fills and a clip cuts to, and `bounds` gives the outline's pixel box. A polar plot outlines its circle, with a hole wound the other way, and a map outlines its sphere.
- `tangents` gives the unit pixel directions in which each dimension increases at a position, such as the clockwise and outward directions of a polar plot, for the directions of ticks, labels, and cursors. They are directions of positions: a scale that runs the other way reverses the direction of its data.
- `route` draws the line that the plot draws between two positions, the shortest line in its geometry: a straight line, an arc the short way around a whole circle, or a great circle on a globe. `iso_line` draws the line along which one dimension holds a value, such as a grid line, a polar ring or spoke, or a meridian or a parallel.

`CartesianProjection` projects a Cartesian plot, whose positions are its pixels. `PolarProjection` projects a polar plot, whose positions are angles in degrees clockwise from the top and distances in pixels from the center, and whose angle wraps around a whole circle.

## Coordinates

`PlotCoords` holds a plot's projection and an avenger-scales `ConfiguredScale` for each dimension that has one, which tools such as brushes take as one value. `value(dimension, position)` gives the data at a position, `position(dimension, value)` gives the position of a datum, and `enclosed(path)` turns a closed path in pixels into polygons of data. A scale maps the data to positions directly: its range spans the dimension's extent, in either direction, as a fit at the extent gives it. A dimension without a scale takes its data as its positions, such as the longitude of a map or a proportion of a ternary plot.

`pixels([x, y])` places a batch of rows, such as the points of a scatter plot: it runs each column through its dimension's scale, or takes it as positions without one, and returns the projection's `pixels`. A column without a scale is used as it is when it is Float32 or Float64 and cast to Float64 when it is another numeric type. Positions that a query has already scaled go to the projection's `pixels` directly.

```rust
use avenger_coords::{PlotCoords, PolarProjection};
use std::sync::Arc;

// A whole circle 100 px in radius, with 12 months around it and values from 0 to 10 outward.
let projection = PolarProjection::new([100.0, 100.0], [0.0, 100.0], [0.0, 360.0]);
let theta = LinearScale::configured((0.0, 12.0), (0.0, 360.0));
let radius = LinearScale::configured((0.0, 10.0), (0.0, 100.0));
let coords = PlotCoords::new(Arc::new(projection), [Some(theta), Some(radius)]);
assert_eq!(coords.position(0, 3.0)?, 90.0);
assert_eq!(coords.value(1, 50.0)?, 5.0);
```

`PlotCoords::cartesian(size, scales)` gives the coordinates of a Cartesian plot of `size`.

`contains(pixel)` tells whether the plot shows a position at a pixel. `dimension(d)` gives a view of one dimension: its conversions, its direction, its period in positions and in data, and the conversion of a range of data to positions and back across a wrap. `positions([x, y])` gives the positions of a batch of rows without projecting them.

## Geographic projections

The `geo` feature adds `GeoProjection`, the projection of a map through the projector of its cartographic projection from avenger-geo, at the size of the map's plot. A map has no scales: its positions are the places that the projector draws.

Through a spherical projector, the positions are longitudes and latitudes, and longitude wraps every 360 degrees. Paths run at a steady change of longitude and latitude, so the edge of a box follows a parallel, and polygons and enclosed paths cut at the antimeridian and at the horizon of a globe. An enclosed path that holds a pole closes through it, so either a planar check of longitude and latitude or the spherical check of `avenger_geo::polygon_contains` tells whether a lasso holds a place. Routes follow great circles, as the projector resamples them.

Through a planar projector (`Projection::planar`), the positions are the map's planar coordinates, such as projected meters. The projection is then affine, so its lines are straight, it has no period, and its extent is what the plot shows.
