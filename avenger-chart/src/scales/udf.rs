use std::sync::Arc;
use std::collections::HashMap;

use crate::error::AvengerChartError;
use datafusion::{
    arrow::{
        datatypes::DataType,
    },
    error::DataFusionError,
    logical_expr::{
        ColumnarValue, ScalarUDF, ScalarUDFImpl, ScalarFunctionArgs, Signature, TypeSignature, Volatility,
    },
    scalar::ScalarValue,
};
use avenger_scales::scales::{ScaleConfig, ScaleContext, ScaleImpl, ConfiguredScale};

/// DataFusion UDF that applies a scale transformation to input values
#[derive(Debug, Clone)]
pub struct ScaleUDF {
    signature: Signature,
    scale_impl: Arc<dyn ScaleImpl>,
    range_type: DataType,
}

impl ScaleUDF {
    pub fn new(
        scale_impl: Arc<dyn ScaleImpl>,
        domain_type: DataType,
        range_type: DataType,
        options_type: DataType,
    ) -> Result<Self, AvengerChartError> {
        let signature = Signature::new(
            TypeSignature::Exact(vec![
                DataType::new_list(domain_type.clone(), true),  // Domain array
                DataType::new_list(range_type.clone(), true),   // Range array
                options_type.clone(),                           // Options struct
                domain_type.clone(),                            // Values to scale
            ]),
            Volatility::Immutable,
        );
        
        Ok(Self {
            signature,
            scale_impl,
            range_type,
        })
    }
}

impl ScalarUDFImpl for ScaleUDF {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> &str {
        "scale"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> datafusion::error::Result<DataType> {
        Ok(self.range_type.clone())
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> datafusion::error::Result<ColumnarValue> {
        // Extract domain array
        let domain = match &args.args[0] {
            ColumnarValue::Scalar(ScalarValue::List(domain_arg)) => {
                domain_arg.values().clone()
            }
            ColumnarValue::Array(_) => {
                return Err(DataFusionError::Execution(
                    "Domain must be a scalar list".to_string()
                ));
            }
            _ => {
                return Err(DataFusionError::Execution(format!(
                    "Unexpected domain value: {:?}",
                    args.args[0]
                )))
            }
        };

        // Extract range array
        let range = match &args.args[1] {
            ColumnarValue::Scalar(ScalarValue::List(range_arg)) => {
                range_arg.values().clone()
            }
            ColumnarValue::Array(_) => {
                return Err(DataFusionError::Execution(
                    "Range must be a scalar list".to_string()
                ));
            }
            _ => {
                return Err(DataFusionError::Execution(format!(
                    "Unexpected range value: {:?}",
                    args.args[1]
                )))
            }
        };

        // Extract options struct
        let options = match &args.args[2] {
            ColumnarValue::Scalar(ScalarValue::Struct(options_arg)) => {
                let mut opts = HashMap::new();
                for (idx, field) in options_arg.fields().iter().enumerate() {
                    let column = options_arg.column(idx);
                    if let Ok(scalar) = ScalarValue::try_from_array(&column, 0) {
                        opts.insert(field.name().clone(), scalar);
                    }
                }
                opts
            }
            _ => HashMap::new(),
        };

        // Create configured scale
        let config = ScaleConfig {
            domain,
            range,
            options,
            context: ScaleContext::default(),
        };

        let scale = ConfiguredScale {
            scale_impl: self.scale_impl.clone(),
            config,
        };

        // Apply scale to values
        let scaled = match &args.args[3] {
            ColumnarValue::Array(values) => {
                let scaled_array = scale.scale(&values)
                    .map_err(|e| DataFusionError::Execution(e.to_string()))?;
                ColumnarValue::Array(scaled_array)
            }
            ColumnarValue::Scalar(value) => {
                let scaled_scalar = scale.scale_scalar(value)
                    .map_err(|e| DataFusionError::Execution(e.to_string()))?;
                ColumnarValue::Scalar(scaled_scalar)
            }
        };

        Ok(scaled)
    }
}

/// Create a scale UDF from a scale name and implementation
pub fn create_scale_udf(
    name: &str,
    scale_impl: Arc<dyn ScaleImpl>,
    domain_type: DataType,
    range_type: DataType,
) -> Result<ScalarUDF, AvengerChartError> {
    let options_type = DataType::Struct(vec![].into()); // Empty struct for now
    let scale_udf = ScaleUDF::new(scale_impl, domain_type, range_type, options_type)?;
    Ok(ScalarUDF::new_from_impl(scale_udf))
}