//! The coordinates of a brush's areas: the projection between each area's pixels and the range
//! positions of its two dimensions, and the scales that map the dimensions' data to their ranges.

use std::sync::Arc;

#[cfg(feature = "scales")]
use arrow_array::{ArrayRef, Float64Array};
use avenger_coords::{CartesianProjection, PlotProjection};
use avenger_geometry::geo_types::{LineString, MultiPolygon, Polygon};
use avenger_layout::Rect;
#[cfg(feature = "scales")]
use avenger_scales::scales::ConfiguredScale;
use geo::algorithm::bool_ops::FillRule;
use geo::BooleanOps;

type ProjectionOf<State, A> =
    Box<dyn Fn(&State, &A) -> Option<Arc<dyn PlotProjection>> + Send + Sync>;

#[cfg(feature = "scales")]
type ScaleOf<State, A> = Box<dyn Fn(&State, &A) -> Option<ConfiguredScale> + Send + Sync>;

/// How a brush finds the coordinates of each of its areas: the area's projection, which is the
/// Cartesian projection of the area's rectangle unless
/// the app sets one, and the scale of each dimension that has one.
pub(crate) struct CoordsOf<State, A> {
    pub(crate) projection: Option<ProjectionOf<State, A>>,
    #[cfg(feature = "scales")]
    pub(crate) scales: [Option<ScaleOf<State, A>>; 2],
}

impl<State, A> Default for CoordsOf<State, A> {
    fn default() -> Self {
        Self {
            projection: None,
            #[cfg(feature = "scales")]
            scales: [None, None],
        }
    }
}

impl<State, A> CoordsOf<State, A> {
    /// The coordinates of the area `key` at `area`, `None` for an area without a projection, or an
    /// error for a scale whose range is not numeric.
    #[cfg_attr(not(feature = "scales"), allow(unused_variables))]
    pub(crate) fn coords(
        &self,
        app: &State,
        key: &A,
        area: Rect,
    ) -> Result<Option<Coords>, String> {
        let projection = match &self.projection {
            Some(projection) => match projection(app, key) {
                Some(projection) => projection,
                None => return Ok(None),
            },
            None => Arc::new(CartesianProjection::new([area.width, area.height])),
        };
        let extent = projection.extent();
        let dimension = |index: usize| -> Result<Dimension, String> {
            #[cfg(feature = "scales")]
            let scale = match self.scales[index]
                .as_ref()
                .and_then(|scale| scale(app, key))
            {
                Some(scale) => {
                    let (a, b) = scale.numeric_interval_range().map_err(|e| e.to_string())?;
                    Some(Scaled {
                        scale,
                        range: [a.min(b), a.max(b)],
                    })
                }
                None => None,
            };
            Ok(Dimension {
                extent: extent[index],
                period: projection.period(index),
                #[cfg(feature = "scales")]
                scale,
            })
        };
        let dimensions = [dimension(0)?, dimension(1)?];
        Ok(Some(Coords {
            projection,
            dimensions,
        }))
    }
}

/// The coordinates of one area: its projection and its two dimensions.
pub(crate) struct Coords {
    pub(crate) projection: Arc<dyn PlotProjection>,
    pub(crate) dimensions: [Dimension; 2],
}

impl Coords {
    /// The data at the range position `position`.
    pub(crate) fn value(&self, position: [f64; 2]) -> Result<[f64; 2], String> {
        let [x, y] = &self.dimensions;
        Ok([x.value(position[0])?, y.value(position[1])?])
    }

    /// The range position of the data `value`.
    pub(crate) fn position(&self, value: [f64; 2]) -> Result<[f64; 2], String> {
        let [x, y] = &self.dimensions;
        Ok([x.position(value[0])?, y.position(value[1])?])
    }

    /// The polygons in data that the closed path `path`, in the area's pixels, encloses, as the
    /// projection cuts them. A polygon with a vertex whose data does not map is left out.
    pub(crate) fn enclosed(&self, path: &[[f32; 2]]) -> Vec<Vec<[f64; 2]>> {
        (self.projection.enclosed(path).into_iter())
            .filter_map(|ring| ring.into_iter().map(|at| self.value(at).ok()).collect())
            .collect()
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
            let rings = self.projection.polygon(&ring).into_iter().map(|ring| {
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
    fn quartered(&self, ring: &[[f64; 2]]) -> Vec<[f64; 2]> {
        let mut quartered = Vec::with_capacity(ring.len());
        for (index, from) in ring.iter().enumerate() {
            let to = ring[(index + 1) % ring.len()];
            let steps = (self.dimensions.iter().enumerate())
                .filter_map(|(axis, dimension)| {
                    let quarter = dimension.period? / 4.0;
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

/// One dimension of an area: the range positions that the area shows along it, its period if it
/// wraps, and the scale that maps its data to range positions, if it has one. Without a scale,
/// the range positions are the data, as the degrees of a map are.
pub(crate) struct Dimension {
    /// The range positions that the area shows, from low to high.
    pub(crate) extent: [f64; 2],
    pub(crate) period: Option<f64>,
    #[cfg(feature = "scales")]
    scale: Option<Scaled>,
}

/// A dimension's scale, whose range spans the dimension's extent from its low end to its high end.
#[cfg(feature = "scales")]
struct Scaled {
    scale: ConfiguredScale,
    /// The ends of the scale's range, from low to high.
    range: [f64; 2],
}

impl Dimension {
    /// The data at the range position `position`.
    pub(crate) fn value(&self, position: f64) -> Result<f64, String> {
        #[cfg(feature = "scales")]
        if let Some(Scaled { scale, range }) = &self.scale {
            let [low, high] = self.extent;
            let fraction = (position - low) / (high - low);
            let at = range[0] + fraction * (range[1] - range[0]);
            return scale.invert_scalar(at).map_err(|e| e.to_string());
        }
        Ok(position)
    }

    /// The range position of the data `value`.
    pub(crate) fn position(&self, value: f64) -> Result<f64, String> {
        #[cfg(feature = "scales")]
        if let Some(Scaled { scale, range }) = &self.scale {
            let values: ArrayRef = Arc::new(Float64Array::from(vec![value]));
            let scaled = scale.scale_to_numeric(&values).map_err(|e| e.to_string())?;
            let at = f64::from(scaled.first().copied().unwrap_or(f32::NAN));
            let [low, high] = self.extent;
            return Ok(low + (at - range[0]) / (range[1] - range[0]) * (high - low));
        }
        Ok(value)
    }

    /// Whether the data increase with the range position.
    pub(crate) fn increasing(&self) -> bool {
        let [low, high] = self.extent;
        match (self.value(low), self.value(high)) {
            (Ok(low), Ok(high)) => low <= high,
            _ => true,
        }
    }

    pub(crate) fn length(&self) -> f64 {
        self.extent[1] - self.extent[0]
    }

    /// The range positions of the whole dimension: its extent, or one period from the extent's low
    /// end for a dimension that wraps.
    pub(crate) fn whole(&self) -> [f64; 2] {
        let [low, high] = self.extent;
        [low, self.period.map_or(high, |period| low + period)]
    }

    /// `position` moved inside the extent of a dimension that does not wrap.
    pub(crate) fn clamp(&self, position: f64) -> f64 {
        match self.period {
            Some(_) => position,
            None => position.clamp(self.extent[0], self.extent[1]),
        }
    }

    /// Whether the area shows `position`: always along a dimension that wraps, and otherwise
    /// within the extent, allowing for rounding at its ends.
    pub(crate) fn shows(&self, position: f64) -> bool {
        let slack = self.length().abs() * 1e-6;
        self.period.is_some()
            || (position >= self.extent[0] - slack && position <= self.extent[1] + slack)
    }

    /// `delta` moved by whole periods to within half a period of zero, along a dimension that
    /// wraps, so a change of position runs the short way around.
    pub(crate) fn short(&self, delta: f64) -> f64 {
        match self.period {
            Some(period) => (delta + period / 2.0).rem_euclid(period) - period / 2.0,
            None => delta,
        }
    }

    /// Whether the range from `low` to `high` contains `position`, including its ends, in any
    /// period of a dimension that wraps.
    pub(crate) fn contains(&self, [low, high]: [f64; 2], position: f64) -> bool {
        match self.period {
            Some(period) => (position - low).rem_euclid(period) <= high - low,
            None => low <= position && position <= high,
        }
    }
}
