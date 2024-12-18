use std::sync::Arc;

use arrow::{
    array::{ArrayRef, AsArray, Float32Array},
    compute::cast,
    datatypes::{DataType, Float32Type},
};
use avenger_scales::numeric::{linear::LinearNumericScale, ContinuousNumericScale};

use crate::error::AvengerChartError;

pub trait ArrowScaleNumeric {
    fn scale(&self, array: ArrayRef) -> Result<ArrayRef, AvengerChartError>;
    fn invert(&self, array: ArrayRef) -> Result<ArrayRef, AvengerChartError>;
}

impl ArrowScaleNumeric for LinearNumericScale {
    fn scale(&self, array: ArrayRef) -> Result<ArrayRef, AvengerChartError> {
        // cast to f32
        let array = cast(&array, &DataType::Float32)?;
        let array = array.as_primitive::<Float32Type>();

        let scaled = ContinuousNumericScale::scale(self, array.values());
        let scaled = Float32Array::from(scaled.as_vec(array.len(), None));
        Ok(Arc::new(scaled))
    }

    fn invert(&self, array: ArrayRef) -> Result<ArrayRef, AvengerChartError> {
        let array = cast(&array, &DataType::Float32)?;
        let array = array.as_primitive::<Float32Type>();
        let scaled = ContinuousNumericScale::invert(self, array.values());
        let scaled = Float32Array::from(scaled.as_vec(array.len(), None));
        Ok(Arc::new(scaled))
    }
}
