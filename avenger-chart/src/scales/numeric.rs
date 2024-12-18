use arrow::datatypes::DataType;
use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};
use avenger_scales::{
    numeric::{
        linear::LinearNumericScale, log::LogNumericScale, pow::PowNumericScale,
        symlog::SymlogNumericScale, ContinuousNumericScaleBuilder,
    },
    temporal::date::DateScale,
};
use chrono::NaiveDate;
use datafusion::{
    error::DataFusionError,
    logical_expr::{ColumnarValue, ScalarUDFImpl, Signature, TypeSignature, Volatility},
};

use crate::error::AvengerChartError;

use super::ScaleImpl;

#[derive(Clone)]
pub struct NumericScale<D>
where
    D: 'static + Send + Sync + Clone,
{
    name: String,
    builder: ContinuousNumericScaleBuilder<D>,
}

impl<D> NumericScale<D>
where
    D: 'static + Send + Sync + Clone,
{
    pub fn new(builder: ContinuousNumericScaleBuilder<D>) -> Self {
        Self {
            name: "numeric_scale".to_string(),
            builder,
        }
    }

    pub fn apply(&self, v: &[D]) -> Result<ScalarOrArray<f32>, AvengerChartError> {
        let scale = (self.builder)();
        let v = scale.scale(v);
        Ok(v)
    }

    pub fn apply_scalar(&self, v: D) -> Result<ScalarOrArray<f32>, AvengerChartError> {
        let scale = (self.builder)();
        let v = scale.scale_scalar(v);
        Ok(ScalarOrArray::new_scalar(v))
    }
}

impl NumericScale<f32> {
    pub fn new_linear(scale: LinearNumericScale) -> Self {
        Self {
            name: "linear_scale".to_string(),
            builder: scale.builder(),
        }
    }

    pub fn new_log(scale: LogNumericScale) -> Self {
        Self {
            name: "log_scale".to_string(),
            builder: scale.builder(),
        }
    }

    pub fn new_pow(scale: PowNumericScale) -> Self {
        Self {
            name: "pow_scale".to_string(),
            builder: scale.builder(),
        }
    }

    pub fn new_symlog(scale: SymlogNumericScale) -> Self {
        Self {
            name: "symlog_scale".to_string(),
            builder: scale.builder(),
        }
    }
}

impl NumericScale<NaiveDate> {
    pub fn new_date(scale: DateScale) -> Self {
        Self {
            name: "date_scale".to_string(),
            builder: scale.builder(),
        }
    }
}

impl<D> std::fmt::Debug for NumericScale<D>
where
    D: 'static + Send + Sync + Clone,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NumericScale({})", self.name)
    }
}

// Into implementations
impl Into<NumericScale<f32>> for LinearNumericScale {
    fn into(self) -> NumericScale<f32> {
        NumericScale::new_linear(self)
    }
}

impl Into<ScaleImpl> for LinearNumericScale {
    fn into(self) -> ScaleImpl {
        ScaleImpl::Numeric(self.into())
    }
}

impl Into<NumericScale<f32>> for LogNumericScale {
    fn into(self) -> NumericScale<f32> {
        NumericScale::new_log(self)
    }
}

impl Into<ScaleImpl> for LogNumericScale {
    fn into(self) -> ScaleImpl {
        ScaleImpl::Numeric(self.into())
    }
}

impl Into<NumericScale<f32>> for PowNumericScale {
    fn into(self) -> NumericScale<f32> {
        NumericScale::new_pow(self)
    }
}

impl Into<ScaleImpl> for PowNumericScale {
    fn into(self) -> ScaleImpl {
        ScaleImpl::Numeric(self.into())
    }
}

impl Into<NumericScale<f32>> for SymlogNumericScale {
    fn into(self) -> NumericScale<f32> {
        NumericScale::new_symlog(self)
    }
}

impl Into<ScaleImpl> for SymlogNumericScale {
    fn into(self) -> ScaleImpl {
        ScaleImpl::Numeric(self.into())
    }
}
