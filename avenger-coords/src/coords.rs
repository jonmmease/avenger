//! The coordinates of plots: a plot's projection between pixels and positions, and the scales
//! that map the data of its dimensions to positions.

use std::sync::Arc;

use arrow::array::{Array, ArrayRef, Float32Array};
use arrow::error::ArrowError;
use avenger_scales::error::AvengerScaleError;
use avenger_scales::scalar::Scalar;
use avenger_scales::scales::ConfiguredScale;

use crate::cartesian::CartesianProjection;
use crate::positions::{float_column, positions};
use crate::projection::{whole, PlotProjection};

/// The coordinates of a plot at one size: its projection between pixels and positions, and the
/// scale of each of its two dimensions that has one.
///
/// A scale maps its dimension's data to the dimension's positions: its range spans the
/// dimension's extent, in either direction, as a fit at the extent gives it. A dimension without a
/// scale, such as the longitude of a map or a proportion of a ternary plot, takes its data as its
/// positions.
#[derive(Clone, Debug)]
pub struct PlotCoords {
    projection: Arc<dyn PlotProjection>,
    scales: [Option<ConfiguredScale>; 2],
}

impl PlotCoords {
    /// The coordinates of `projection`, with `scales[d]` the scale of dimension `d`.
    pub fn new(projection: Arc<dyn PlotProjection>, scales: [Option<ConfiguredScale>; 2]) -> Self {
        Self { projection, scales }
    }

    /// The coordinates of a Cartesian plot of `size`, whose positions are its pixels.
    pub fn cartesian(size: [f32; 2], scales: [Option<ConfiguredScale>; 2]) -> Self {
        Self::new(Arc::new(CartesianProjection::new(size)), scales)
    }

    /// The projection between the plot's pixels and its positions.
    pub fn projection(&self) -> &Arc<dyn PlotProjection> {
        &self.projection
    }

    /// The scale of `dimension`, 0 or 1, or `None` for a dimension that takes its data as its
    /// positions.
    pub fn scale(&self, dimension: usize) -> Option<&ConfiguredScale> {
        self.scales[dimension].as_ref()
    }

    /// The view of `dimension`, 0 or 1.
    pub fn dimension(&self, dimension: usize) -> Dimension<'_> {
        Dimension {
            coords: self,
            index: dimension,
        }
    }

    /// The data at `position` along `dimension`.
    pub fn value(&self, dimension: usize, position: f64) -> Result<f64, AvengerScaleError> {
        self.dimension(dimension).value(position)
    }

    /// The position of the data `value` along `dimension`.
    pub fn position(&self, dimension: usize, value: f64) -> Result<f64, AvengerScaleError> {
        self.dimension(dimension).position(value)
    }

    /// Whether the plot shows a position at `pixel`, from the plot's top-left corner.
    pub fn contains(&self, pixel: [f32; 2]) -> bool {
        (self.projection.position(pixel)).is_some_and(|position| self.projection.shows(position))
    }

    /// The positions of the rows whose data along each dimension is `values`, as a Float32 or a
    /// Float64 column for each dimension: through the dimension's scale, or as positions without
    /// one, where a Float32 or Float64 column is used as it is and another numeric column is cast
    /// to Float64. A column that is not numeric, or columns of different lengths, give an error.
    pub fn positions(&self, values: [&ArrayRef; 2]) -> Result<[ArrayRef; 2], AvengerScaleError> {
        let [x, y] = values;
        if x.len() != y.len() {
            return Err(ArrowError::InvalidArgumentError(format!(
                "the columns of the two dimensions have one length, not {} and {}",
                x.len(),
                y.len()
            ))
            .into());
        }
        Ok([self.column(0, x)?, self.column(1, y)?])
    }

    /// The pixels of the rows whose data along each dimension is `values`: their
    /// [`positions`](Self::positions) through the projection's
    /// [`pixels`](PlotProjection::pixels).
    pub fn pixels(&self, values: [&ArrayRef; 2]) -> Result<[Float32Array; 2], AvengerScaleError> {
        let [x, y] = self.positions(values)?;
        Ok(self.projection.pixels([positions(&x), positions(&y)]))
    }

    /// The positions of the data `values` along `dimension`, as a Float32 or Float64 column.
    fn column(&self, dimension: usize, values: &ArrayRef) -> Result<ArrayRef, AvengerScaleError> {
        let Some(scale) = &self.scales[dimension] else {
            return Ok(float_column(values)?);
        };
        Ok(float_column(&scale.scale(values)?)?)
    }

    /// The polygons of data that the closed path `path`, in pixels, encloses, as the projection's
    /// [`enclosed`](PlotProjection::enclosed) cuts them. A polygon with a vertex whose data does
    /// not map is left out.
    pub fn enclosed(&self, path: &[[f32; 2]]) -> Vec<Vec<[f64; 2]>> {
        (self.projection.enclosed(path).into_iter())
            .filter_map(|ring| {
                (ring.into_iter())
                    .map(|[x, y]| Some([self.value(0, x).ok()?, self.value(1, y).ok()?]))
                    .collect()
            })
            .collect()
    }
}

/// One dimension of plot coordinates: its conversions between data and positions, through its
/// scale, whose range holds the positions, and its period, along which a dimension that wraps runs
/// the short way.
#[derive(Clone, Copy, Debug)]
pub struct Dimension<'a> {
    coords: &'a PlotCoords,
    index: usize,
}

impl Dimension<'_> {
    /// The dimension's scale, or `None` for a dimension that takes its data as its positions.
    pub fn scale(&self) -> Option<&ConfiguredScale> {
        self.coords.scales[self.index].as_ref()
    }

    /// The data at `position`: the scale's inverse of it, or `position` itself without a scale.
    pub fn value(&self, position: f64) -> Result<f64, AvengerScaleError> {
        match self.scale() {
            Some(scale) => scale.invert_scalar(position),
            None => Ok(position),
        }
    }

    /// The position of the data `value`: the scale's value of it, or `value` itself without a
    /// scale.
    pub fn position(&self, value: f64) -> Result<f64, AvengerScaleError> {
        let Some(scale) = self.scale() else {
            return Ok(value);
        };
        let scaled = scale.scale_scalar_to_numeric(&Scalar::from_f64(value))?;
        Ok(scaled.first().copied().map_or(f64::NAN, f64::from))
    }

    /// The positions that the plot shows along the dimension, from low to high.
    pub fn extent(&self) -> [f64; 2] {
        self.coords.projection.extent()[self.index]
    }

    /// The dimension's period in positions, when it wraps.
    pub fn period(&self) -> Option<f64> {
        self.coords.projection.period(self.index)
    }

    /// The dimension's whole span in positions: its extent, or one period from the extent's low
    /// end for a dimension that wraps.
    pub fn whole(&self) -> [f64; 2] {
        whole(self.coords.projection.as_ref(), self.index)
    }

    /// The dimension's period in data, when it wraps: the period without a scale, and the data
    /// that one period of positions spans with a linear or a time scale. `None` for a dimension
    /// that does not wrap, and for another scale, which has no period in data.
    pub fn data_period(&self) -> Option<f64> {
        let period = self.period()?;
        let Some(scale) = self.scale() else {
            return Some(period);
        };
        if !matches!(scale.scale_impl.scale_type(), "linear" | "time") {
            return None;
        }
        let [low, _] = self.extent();
        let [a, b] = [low, low + period].map(|position| self.value(position).ok());
        Some((b? - a?).abs())
    }

    /// Whether the data increase with the position.
    pub fn increasing(&self) -> bool {
        let [low, high] = self.extent();
        match (self.value(low), self.value(high)) {
            (Ok(low), Ok(high)) => low <= high,
            _ => true,
        }
    }

    /// `delta` moved by whole periods to within half a period of zero, along a dimension that
    /// wraps, so that a change of position runs the short way around.
    pub fn short(&self, delta: f64) -> f64 {
        match self.period() {
            Some(period) => (delta + period / 2.0).rem_euclid(period) - period / 2.0,
            None => delta,
        }
    }

    /// The range of data from `first` to `second` as positions from low to high. Along a
    /// dimension that wraps, the range runs up from the position of its first value in the
    /// direction of the data, so a range across the wrap ends past the period's end, and it spans
    /// at most one period.
    pub fn positions_of(&self, [first, second]: [f64; 2]) -> Result<[f64; 2], AvengerScaleError> {
        let [a, b] = [self.position(first)?, self.position(second)?];
        Ok(match self.period() {
            None => [a.min(b), a.max(b)],
            Some(period) => {
                let (start, end) = if self.increasing() { (a, b) } else { (b, a) };
                let width = if end < start {
                    end - start + period
                } else {
                    end - start
                };
                [start, start + width.min(period)]
            }
        })
    }

    /// The range of data from the positions `low` to `high`, the inverse of
    /// [`positions_of`](Self::positions_of). Along a dimension that wraps, the range starts in the
    /// first period of the extent, a range across the wrap has a first value greater than its
    /// second, and a range of a whole period or more is the data of the whole period.
    pub fn values_of(&self, [low, high]: [f64; 2]) -> Result<[f64; 2], AvengerScaleError> {
        let sorted = |[a, b]: [f64; 2]| [a.min(b), a.max(b)];
        match self.period() {
            None => Ok(sorted([self.value(low)?, self.value(high)?])),
            Some(period) if high - low >= period => {
                let [start, end] = self.whole();
                Ok(sorted([self.value(start)?, self.value(end)?]))
            }
            Some(period) => {
                let [start, end] = self.whole();
                let first = start + (low - start).rem_euclid(period);
                let mut second = first + (high - low);
                if second > end {
                    second -= period;
                }
                let [a, b] = [self.value(first)?, self.value(second)?];
                Ok(if self.increasing() { [a, b] } else { [b, a] })
            }
        }
    }
}
