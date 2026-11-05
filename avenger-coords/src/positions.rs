//! The positions of a batch of rows along one dimension, as a projection's
//! [`pixels`](crate::PlotProjection::pixels) takes them.

use arrow::array::{Array, ArrayRef, AsArray, Float32Array, Float64Array};
use arrow::buffer::NullBuffer;
use arrow::compute::cast;
use arrow::datatypes::{DataType, Float32Type, Float64Type};
use arrow::error::ArrowError;

/// The positions of a batch of rows along one dimension: Float32 from a scale, or Float64 from
/// data, such as longitudes, with nulls where a row has no position.
#[derive(Clone, Copy, Debug)]
pub enum Positions<'a> {
    F32(&'a Float32Array),
    F64(&'a Float64Array),
}

impl Positions<'_> {
    /// The number of rows.
    pub fn len(&self) -> usize {
        match self {
            Positions::F32(array) => array.len(),
            Positions::F64(array) => array.len(),
        }
    }

    /// Whether there are no rows.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The position of `row`, widened to f64, or `None` where it is null.
    pub fn value(&self, row: usize) -> Option<f64> {
        match self {
            Positions::F32(array) => array.is_valid(row).then(|| f64::from(array.value(row))),
            Positions::F64(array) => array.is_valid(row).then(|| array.value(row)),
        }
    }

    /// The rows that are null, or `None` when none is.
    pub fn nulls(&self) -> Option<&NullBuffer> {
        match self {
            Positions::F32(array) => array.nulls(),
            Positions::F64(array) => array.nulls(),
        }
    }
}

impl<'a> From<&'a Float32Array> for Positions<'a> {
    fn from(array: &'a Float32Array) -> Self {
        Positions::F32(array)
    }
}

impl<'a> From<&'a Float64Array> for Positions<'a> {
    fn from(array: &'a Float64Array) -> Self {
        Positions::F64(array)
    }
}

/// `array` as a column of positions: a Float32 or Float64 array as it is, and another numeric
/// array cast to Float64, or an error for an array that is not numeric.
pub(crate) fn float_column(array: &ArrayRef) -> Result<ArrayRef, ArrowError> {
    match array.data_type() {
        DataType::Float32 | DataType::Float64 => Ok(array.clone()),
        data_type if data_type.is_numeric() => cast(array, &DataType::Float64),
        data_type => Err(ArrowError::InvalidArgumentError(format!(
            "positions are numeric, not {data_type}"
        ))),
    }
}

/// The positions of `column`, a Float32 or Float64 array that [`float_column`] gives.
pub(crate) fn positions(column: &ArrayRef) -> Positions<'_> {
    match column.data_type() {
        DataType::Float32 => Positions::F32(column.as_primitive::<Float32Type>()),
        _ => Positions::F64(column.as_primitive::<Float64Type>()),
    }
}
