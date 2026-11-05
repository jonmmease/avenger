//! The coordinates of a brush's areas: each area's plot coordinates, and the cell width of each
//! dimension that has one.

use avenger_coords::{Dimension, PlotCoords, PlotProjection};
use avenger_geometry::geo_types::{LineString, MultiPolygon, Polygon};
use avenger_layout::Rect;
use avenger_scales::error::AvengerScaleError;
use avenger_scenegraph::marks::group::Clip;
use geo::algorithm::bool_ops::FillRule;
use geo::BooleanOps;
use lyon_path::geom::point;

use super::draw::rect_clip;

type CoordsFn<State, A> = Box<dyn Fn(&State, &A) -> Option<PlotCoords> + Send + Sync>;

type CellsOf<State, A> = Box<dyn Fn(&State, &A) -> [Option<f64>; 2] + Send + Sync>;

/// How a brush finds the coordinates of each of its areas: the area's plot coordinates, which are
/// the Cartesian coordinates of the area's rectangle without scales unless the app sets them, and
/// the cell width of each dimension that has one.
pub(crate) struct CoordsOf<State, A> {
    pub(crate) coords: Option<CoordsFn<State, A>>,
    pub(crate) cells: Option<CellsOf<State, A>>,
}

impl<State, A> Default for CoordsOf<State, A> {
    fn default() -> Self {
        Self {
            coords: None,
            cells: None,
        }
    }
}

impl<State, A> CoordsOf<State, A> {
    /// The coordinates of the area `key` at `area`, or `None` for an area without coordinates.
    pub(crate) fn coords(&self, app: &State, key: &A, area: Rect) -> Option<Coords> {
        let plot = match &self.coords {
            Some(coords) => coords(app, key)?,
            None => PlotCoords::cartesian([area.width, area.height], [None, None]),
        };
        let cells = (self.cells.as_ref()).map_or([None, None], |cells| cells(app, key));
        Some(Coords { plot, cells })
    }
}

/// The coordinates of one area: its plot coordinates and the cell width of each dimension.
pub(crate) struct Coords {
    plot: PlotCoords,
    cells: [Option<f64>; 2],
}

impl Coords {
    /// The projection between the area's pixels and its range positions.
    pub(crate) fn projection(&self) -> &dyn PlotProjection {
        &**self.plot.projection()
    }

    /// The clip of the area's chrome: the area's rectangle where the plot's outline is that
    /// rectangle, and the outline otherwise, such as a polar plot's circle or a globe's disk.
    pub(crate) fn clip(&self, area: Rect) -> Clip {
        let outline = self.projection().outline();
        let corners = [
            [0.0, 0.0],
            [area.width, 0.0],
            [area.width, area.height],
            [0.0, area.height],
        ];
        let corner = |at: &[f32; 2]| {
            (corners.iter()).any(|c| (c[0] - at[0]).abs() < 1e-3 && (c[1] - at[1]).abs() < 1e-3)
        };
        if matches!(outline.as_slice(), [ring] if ring.len() == 4 && ring.iter().all(corner)) {
            return rect_clip(area);
        }
        let mut builder = lyon_path::Path::builder();
        for ring in &outline {
            let Some((first, rest)) = ring.split_first() else {
                continue;
            };
            builder.begin(point(first[0], first[1]));
            for at in rest {
                builder.line_to(point(at[0], at[1]));
            }
            builder.close();
        }
        Clip::Path(builder.build())
    }

    /// The dimension `index`, 0 or 1.
    pub(crate) fn dimension(&self, index: usize) -> Dimension<'_> {
        self.plot.dimension(index)
    }

    /// The area's two dimensions.
    pub(crate) fn dimensions(&self) -> [Dimension<'_>; 2] {
        [self.dimension(0), self.dimension(1)]
    }

    /// The width of a cell along `dimension` in positions, when a box covers whole cells along
    /// it: when the app sets a cell wider than 0 and the dimension does not wrap.
    pub(crate) fn snap(&self, dimension: usize) -> Option<f64> {
        let wraps = self.dimension(dimension).period().is_some();
        self.cells[dimension].filter(|cell| *cell > 0.0 && !wraps)
    }

    /// The position of the data `value`.
    pub(crate) fn position(&self, value: [f64; 2]) -> Result<[f64; 2], AvengerScaleError> {
        let [x, y] = self.dimensions();
        Ok([x.position(value[0])?, y.position(value[1])?])
    }

    /// The polygons in data that the closed path `path`, in the area's pixels, encloses, as the
    /// projection cuts them. A polygon with a vertex whose data does not map is left out.
    pub(crate) fn enclosed(&self, path: &[[f32; 2]]) -> Vec<Vec<[f64; 2]>> {
        self.plot.enclosed(path)
    }

    /// The region in the area's pixels of the polygons in data `polygons`: their union, with each
    /// polygon filled by the nonzero rule, so polygons wound in opposite directions do not cancel
    /// where they overlap. `None` when a vertex's data does not map.
    pub(crate) fn region(&self, polygons: &[Vec<[f64; 2]>]) -> Option<MultiPolygon<f64>> {
        let mut region = MultiPolygon::new(Vec::new());
        for polygon in polygons {
            let ring: Vec<[f64; 2]> = (polygon.iter())
                .map(|value| self.position(*value).ok())
                .collect::<Option<_>>()?;
            let ring = self.quartered(&ring);
            let rings = self.projection().polygon(&ring).into_iter().map(|ring| {
                let ring: LineString<f64> = (ring.into_iter())
                    .map(|[x, y]| (f64::from(x), f64::from(y)))
                    .collect();
                Polygon::new(ring, Vec::new())
            });
            let shape = MultiPolygon::new(rings.collect());
            region = region.union_with_fill_rule(&shape, FillRule::NonZero);
        }
        Some(region)
    }

    /// `ring`, in range positions, with vertices between its ends along each side that runs a
    /// quarter period or more along a dimension that wraps, such as the side through the center
    /// of a polar plot that closes a lasso around it, so that the side runs the long way as the
    /// projection's polygon runs each side the short way.
    pub(crate) fn quartered(&self, ring: &[[f64; 2]]) -> Vec<[f64; 2]> {
        let mut quartered = Vec::with_capacity(ring.len());
        for (index, from) in ring.iter().enumerate() {
            let to = ring[(index + 1) % ring.len()];
            let steps = (self.dimensions().iter().enumerate())
                .filter_map(|(axis, dimension)| {
                    let quarter = dimension.period()? / 4.0;
                    Some(((to[axis] - from[axis]).abs() / quarter).ceil())
                })
                .fold(1.0, f64::max) as usize;
            quartered.extend((0..steps).map(|step| {
                let along = step as f64 / steps as f64;
                [0, 1].map(|axis| from[axis] + along * (to[axis] - from[axis]))
            }));
        }
        quartered
    }
}
