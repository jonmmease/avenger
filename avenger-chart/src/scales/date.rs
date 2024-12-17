use std::sync::Arc;

use arrow::datatypes::DataType;
use avenger_scales::numeric::{
    linear::LinearNumericScale, log::LogNumericScale, pow::PowNumericScale,
    symlog::SymlogNumericScale, ContinuousNumericScale, ContinuousNumericScaleBuilder,
};
use avenger_scales::temporal::date::DateScale as DateContinuousScale;
use chrono::NaiveDate;
use datafusion::{
    error::DataFusionError,
    logical_expr::{ColumnarValue, ScalarUDFImpl, Signature, TypeSignature, Volatility},
};

#[derive(Clone)]
pub struct DateScale {
    name: String,
    signature: Signature,
    builder: ContinuousNumericScaleBuilder<NaiveDate>,
}

impl DateScale {
    pub fn new(builder: ContinuousNumericScaleBuilder<NaiveDate>) -> Self {
        Self {
            name: "date_scale".to_string(),
            signature: Signature::new(
                TypeSignature::Exact(vec![DataType::Float64]),
                Volatility::Immutable,
            ),
            builder,
        }
    }

    pub fn new_date(scale: DateContinuousScale) -> Self {
        Self {
            name: "date_scale".to_string(),
            signature: Signature::new(
                TypeSignature::Exact(vec![DataType::Float64]),
                Volatility::Immutable,
            ),
            builder: scale.builder(),
        }
    }
}

impl std::fmt::Debug for DateScale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DateScale({})", self.name)
    }
}

impl Into<DateScale> for DateContinuousScale {
    fn into(self) -> DateScale {
        DateScale::new_date(self)
    }
}

impl ScalarUDFImpl for DateScale {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType, DataFusionError> {
        Ok(DataType::Float64)
    }

    fn invoke_batch(
        &self,
        args: &[ColumnarValue],
        number_rows: usize,
    ) -> Result<ColumnarValue, DataFusionError> {
        let scale = (self.builder)();
        // scale.scale();
        todo!()
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn test_scale() {
//         println!("test");
//         // let scale = LinearNumericScale::new(&LinearNumericScaleConfig::default());
//     }
// }
