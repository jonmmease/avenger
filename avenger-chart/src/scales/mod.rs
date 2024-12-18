use ::arrow::{
    array::{ArrayRef, AsArray},
    compute::cast,
    datatypes::{DataType, Float32Type},
};
use avenger_common::{types::ColorOrGradient, value::ScalarOrArray};
use avenger_scales::color::continuous_color::{ContinuousColorScale, LinearSrgbaScale};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use numeric::NumericScale;

use crate::error::AvengerChartError;

pub mod numeric;

pub enum ScaleImpl {
    // Continuous Numeric -> numeric
    Numeric(NumericScale<f32>),
    // Continuous Date -> numeric
    Date(NumericScale<NaiveDate>),
    // Continuous Timestamp -> numeric
    Timestamp(NumericScale<NaiveDateTime>),
    // Continuous Timestamptz -> numeric
    Timestamptz(NumericScale<DateTime<Utc>>),
    // Continuous Numeric -> color
    LinearSrgba(LinearSrgbaScale),
}

impl ScaleImpl {
    /// Apply scale to an Arrow Array, returning an f32 ScalarOrArray
    pub fn apply_to_f32(&self, array: &ArrayRef) -> Result<ScalarOrArray<f32>, AvengerChartError> {
        match self {
            ScaleImpl::Numeric(scale) => {
                // Cast to f32 and downcast to Float32Array
                let array = cast(array, &DataType::Float32)?;
                let array = array.as_primitive::<Float32Type>();

                // Nulls were filtered above, so we can safely ignore them here
                if array.len() == 1 {
                    scale.apply_scalar(array.value(0))
                } else {
                    scale.apply(&array.values())
                }
            }
            ScaleImpl::Date(_scale) => todo!(),
            ScaleImpl::Timestamp(_scale) => todo!(),
            ScaleImpl::Timestamptz(_scale) => todo!(),
            ScaleImpl::LinearSrgba(_scale) => todo!(),
        }
    }

    pub fn apply_to_color_or_gradient(
        &self,
        array: &ArrayRef,
    ) -> Result<ScalarOrArray<ColorOrGradient>, AvengerChartError> {
        match self {
            ScaleImpl::LinearSrgba(scale) => {
                let array = cast(array, &DataType::Float32)?;
                let array = array.as_primitive::<Float32Type>();

                // Nulls were filtered above, so we can safely ignore them here
                let scaled = scale.scale(&array.values()).to_scalar_if_len_one();
                Ok(scaled)
            }
            _ => todo!(),
        }
    }
}

impl From<NumericScale<f32>> for ScaleImpl {
    fn from(scale: NumericScale<f32>) -> Self {
        ScaleImpl::Numeric(scale)
    }
}

impl From<NumericScale<NaiveDate>> for ScaleImpl {
    fn from(scale: NumericScale<NaiveDate>) -> Self {
        ScaleImpl::Date(scale)
    }
}

impl From<NumericScale<NaiveDateTime>> for ScaleImpl {
    fn from(scale: NumericScale<NaiveDateTime>) -> Self {
        ScaleImpl::Timestamp(scale)
    }
}

impl From<NumericScale<DateTime<Utc>>> for ScaleImpl {
    fn from(scale: NumericScale<DateTime<Utc>>) -> Self {
        ScaleImpl::Timestamptz(scale)
    }
}

impl From<LinearSrgbaScale> for ScaleImpl {
    fn from(scale: LinearSrgbaScale) -> Self {
        ScaleImpl::LinearSrgba(scale)
    }
}
