use std::sync::Arc;

use arrow::datatypes::DataType;
use avenger_scales::numeric::{
    linear::LinearNumericScale, log::LogNumericScale, pow::PowNumericScale,
    symlog::SymlogNumericScale, ContinuousNumericScale, ContinuousNumericScaleBuilder,
};
use datafusion::{
    error::DataFusionError,
    logical_expr::{ColumnarValue, ScalarUDFImpl, Signature, TypeSignature, Volatility},
};

#[derive(Clone)]
pub struct NumericScale {
    name: String,
    signature: Signature,
    builder: ContinuousNumericScaleBuilder<f32>,
}

impl NumericScale {
    pub fn new(builder: ContinuousNumericScaleBuilder<f32>) -> Self {
        Self {
            name: "numeric_scale".to_string(),
            signature: Signature::new(
                TypeSignature::Exact(vec![DataType::Float64]),
                Volatility::Immutable,
            ),
            builder,
        }
    }

    pub fn new_linear(scale: LinearNumericScale) -> Self {
        Self {
            name: "linear_scale".to_string(),
            signature: Signature::new(
                TypeSignature::Exact(vec![DataType::Float64]),
                Volatility::Immutable,
            ),
            builder: scale.builder(),
        }
    }

    pub fn new_log(scale: LogNumericScale) -> Self {
        Self {
            name: "log_scale".to_string(),
            signature: Signature::new(
                TypeSignature::Exact(vec![DataType::Float64]),
                Volatility::Immutable,
            ),
            builder: scale.builder(),
        }
    }

    pub fn new_pow(scale: PowNumericScale) -> Self {
        Self {
            name: "pow_scale".to_string(),
            signature: Signature::new(
                TypeSignature::Exact(vec![DataType::Float64]),
                Volatility::Immutable,
            ),
            builder: scale.builder(),
        }
    }

    pub fn new_symlog(scale: SymlogNumericScale) -> Self {
        Self {
            name: "symlog_scale".to_string(),
            signature: Signature::new(
                TypeSignature::Exact(vec![DataType::Float64]),
                Volatility::Immutable,
            ),
            builder: scale.builder(),
        }
    }
}

impl std::fmt::Debug for NumericScale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NumericScale({})", self.name)
    }
}

impl Into<NumericScale> for LinearNumericScale {
    fn into(self) -> NumericScale {
        NumericScale::new_linear(self)
    }
}

impl Into<NumericScale> for LogNumericScale {
    fn into(self) -> NumericScale {
        NumericScale::new_log(self)
    }
}

impl Into<NumericScale> for PowNumericScale {
    fn into(self) -> NumericScale {
        NumericScale::new_pow(self)
    }
}

impl Into<NumericScale> for SymlogNumericScale {
    fn into(self) -> NumericScale {
        NumericScale::new_symlog(self)
    }
}

impl ScalarUDFImpl for NumericScale {
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
